# Decoding a Batch

At this point the node has a batch's raw bytes, however they were sourced. This chapter
turns those bytes into messages. All of it is pure, offline, network-free code in
`arb-reth-derive`, mirroring `nitro/arbstate/inbox.go`.

```text
batch bytes
    │
    ├─ 40-byte timeBounds header ─────────▶ BatchHeader
    │
    ├─ header flag byte (0x00 = brotli, …)
    │
    ├─ brotli decompress
    │
    ├─ RLP segment list ──────────────────▶ Vec<Segment>
    │
    └─ multiplexer walk ──────────────────▶ Vec<DerivedMessage>
```

## The timeBounds header

Every batch starts with 40 bytes: five big-endian `u64`s.

```rust,ignore
struct BatchHeader {
    min_timestamp: u64,
    max_timestamp: u64,
    min_l1_block: u64,
    max_l1_block: u64,
    after_delayed_messages: u64,
}
```

The first four bound what the batch may claim about time and L1 height; the fifth is the
delayed cursor after this batch.

## The header flag

The byte immediately after the header selects the payload encoding
(`nitro/daprovider/util.go`):

| Flag | Meaning |
| --- | --- |
| `0x00` | Brotli-compressed segments — the common case. |
| `0x01` | DA certificate. |
| `0x08` | AnyTrust tree. |
| `0x20` | Zeroheavy. |
| `0x40` | L1-authenticated. |
| `0x50` | Blob hashes (`L1_AUTHENTICATED \| 0x10`). |
| `0x80` | AnyTrust. |

`decompress_payload` handles the brotli path; flags resolved at a different layer (blobs,
DA) surface as a `BatchError` here rather than being guessed at.

## Segments

The decompressed payload is an RLP list of segments. Each segment's first byte is its kind:

| Kind | Name | Effect |
| --- | --- | --- |
| `0` | `L2Message` | Emit one sequencer message with this body. |
| `1` | `L2MessageBrotli` | Same, after decompressing the body. |
| `2` | `DelayedMessages` | Pull one message from the delayed source, advance the cursor. |
| `3` | `AdvanceTimestamp` | Add a delta to the running timestamp. |
| `4` | `AdvanceL1Block` | Add a delta to the running L1 block. |

## The multiplexer

`extract_messages` (`crates/arb-reth-derive/src/multiplexer.rs`) walks the segments,
maintaining a running timestamp, L1 block, and delayed cursor. Each segment emits at most
one message.

Two details here are consensus-critical and both are counterintuitive enough that the code
carries long comments justifying them.

### The running values start at zero

The timestamp and L1 block counters start at **0**, not at the header's `min_timestamp` /
`min_l1_block`. The `[min, max]` bounds are applied as a clamp only at message-emit time.

Seeding from `min_*` looks natural and is wrong. The first `Advance*` segment carries an
*absolute* value, so `min + absolute` overshoots `max` and clamps to `max` — giving every
message in the batch a ceiling L1 block number. That corrupts the ArbOS `Blockhashes` state
on the very first derived block.

### Advance deltas wrap

Nitro performs `timestamp += advancing` on a Go `uint64`, which wraps on overflow. That is
not a corner case to be defended against — it is load-bearing. A batch encodes a
*decrement* as an advance of `u64::MAX`, i.e. `-1 mod 2^64`. Rejecting or saturating the
overflow would decode such batches differently from Nitro and fork the chain.

### Size limits

An `L2MessageBrotli` segment whose decompressed body exceeds Nitro's
`MaxL2MessageSize` (262,144 bytes) is rejected with `OversizeL2Message`.

## Inside an L2 message

The multiplexer's output still holds an opaque `l2_msg`. Its structure is parsed by
`parse_l2_message` (`crates/arb-reth-derive/src/l2message.rs`), mirroring
`nitro/arbos/parse_l2.go`. The first byte is a sub-kind:

| Kind | Name | Contains |
| --- | --- | --- |
| `0` | `UnsignedUserTx` | An Arbitrum-constructed transaction (no standard signed hash). |
| `1` | `ContractTx` | Likewise. |
| `2` | `NonMutatingCall` | — |
| `3` | `Batch` | A nested sequence of `[u64 BE length][message]`. |
| `4` | `SignedTx` | One EIP-2718 transaction encoding. |
| `7` | `SignedCompressedTx` | One brotli-compressed EIP-2718 encoding. |

`Batch` nests, bounded at Nitro's depth limit of 16 (`BatchTooDeep`). Nitro stops
*gracefully* at the first read error or oversize sub-message inside a batch rather than
failing the whole message, and this parser matches that behaviour — another case where
faithfulness to the reference implementation outranks strictness.

The result is a `ParsedL2` holding the signed transaction encodings (whose `keccak256` is
the transaction hash) plus a count of Arbitrum-constructed sub-messages that have no such
hash.

## Where decoding is used

Note that `parse_l2_message` is *not* on the block-production path. The node re-encodes
derived messages into the feed shape and lets `digest_message` — the single validated entry
point described in [From Message to Block Environment](ch04-02-payload-attributes.md) — do
the parsing. `parse_l2_message` exists for tooling and for tests that need to inspect a
batch's transaction hashes without executing anything.
