# Blobs

Since Dencun, Arbitrum posts most batches as EIP-4844 blobs rather than calldata. Blob
contents are not available from an execution-layer RPC — only their versioned hashes are —
so recovering a blob batch means going to the consensus layer.

## Finding the sidecars

`BeaconClient` (`crates/arb-reth-l1/src/beacon.rs`) is a minimal REST client over a beacon
node's `/eth/v1/...` API.

Blob sidecars are addressed by **slot**, not by execution block, so the client converts:

```text
slot = (execution_block_timestamp - genesis_time) / seconds_per_slot
```

Post-merge this is exact — the execution payload timestamp equals the slot time. Mainnet
constants are baked in (`MAINNET_GENESIS_TIME = 1_606_824_023`, `SECONDS_PER_SLOT = 12`),
with `with_timing` available for other chains.

Having fetched a slot's sidecars, the client matches them to the batch's blob hashes by
computing `kzg_to_versioned_hash` over each sidecar's KZG commitment and selecting the ones
the batch's transaction referenced.

> **Retention.** Beacon nodes keep blob sidecars for roughly 18 days. Deep historical sync
> across the blob era therefore requires a blob-archive endpoint, not a stock beacon node.
> A missing sidecar surfaces as a provider error, which the supervisor treats as retryable —
> so a node pointed at a non-archive beacon will retry forever rather than fail loudly. If
> derivation stalls in the blob era, check this first.

## Decoding a blob

Blob encoding is a faithful port of Nitro's `util/blobs/blobs.go::DecodeBlobs`, in
`crates/arb-reth-derive/src/blob.rs`. It exists because a blob field element cannot hold 32
arbitrary bytes: the BLS modulus is just under 2^254, so only 254 of each 256 bits are
usable.

Arbitrum's encoding puts 31 payload bytes in bytes `1..32` of each field element, and packs
6 spare bits into byte `0`:

```text
field element (32 bytes):
  ┌────────┬───────────────────────────────────────────────┐
  │ byte 0 │              bytes 1..32                       │
  │ 6 bits │              31 payload bytes                  │
  └────────┴───────────────────────────────────────────────┘

per blob: 4096 field elements
usable:   254 * 4096 / 8 = 130,048 bytes
```

Decoding is two passes **per blob**:

1. Append the 31-byte body of every field element.
2. Reassemble bytes from the 6 spare bits in byte 0 of every field element, LSB-first
   through a small accumulator.

Then the concatenation of all blobs is RLP-decoded as a single byte string; trailing zero
padding is ignored.

> **The interleaving is load-bearing.** The passes are interleaved *per blob* — all bodies
> of blob 0, then all spares of blob 0, then blob 1. Doing all bodies first and all spares
> second across every blob would corrupt any multi-blob batch. The module comment calls this
> out explicitly, and it is the kind of bug that passes single-blob tests.

Two failures are possible: the spare-bit accumulator not draining to zero (a corrupt blob),
and the reassembled stream not being valid RLP. Both are `BlobDecodeError` variants.

## After decoding

The decoded byte stream is exactly the same shape the calldata path produces: a header-flag
byte followed by compressed segments. From there, blob and calldata batches are
indistinguishable — both go through `decode_payload_messages`, described in
[Decoding a Batch](ch03-06-decoding-a-batch.md).

This is why `decode_batch_messages` refuses to handle a `BatchPayload::Blob` directly: the
blob must first be resolved to a payload via the reader's blob path, and only then decoded.
Calling it on an unresolved blob batch returns
`L1Error::UnsupportedDataLocation(BLOB_HASHES)`.

## Cost

Blob batches dominate sync latency in the post-Dencun range. Each one is a chain of round
trips — fetch the posting transaction, fetch its block for the timestamp, then download
roughly 768 KiB of sidecar data — and none of it is CPU work. This is precisely why
`resolve_batches` runs up to 8 batch resolutions concurrently within a window and why
`--l1-prefetch` overlaps whole windows. Without both, blocks that execute in milliseconds
would sit behind seconds of network wait.
