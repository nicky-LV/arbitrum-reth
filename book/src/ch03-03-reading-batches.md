# Reading Batches from L1

A *batch* is the unit the sequencer posts to Ethereum. Finding one and getting its bytes
back is the job of `SequencerInboxReader` in `crates/arb-reth-l1/src/reader.rs`.

## The event

Every posted batch emits:

```solidity
event SequencerBatchDelivered(
    uint256 indexed batchSequenceNumber,
    bytes32 indexed beforeAcc,
    bytes32 indexed afterAcc,
    bytes32 delayedAcc,
    uint256 afterDelayedMessagesRead,
    TimeBounds timeBounds,
    uint8 dataLocation
);
```

The reader filters on this event's topic-0 over an L1 block range. The `sol!` macro in
`crates/arb-reth-l1/src/contracts.rs` exists only to derive that topic hash and the call
selectors — the non-indexed data is decoded by hand in
`arb_reth_derive::batch::parse_sequencer_batch_delivered`, which frees callers from ABI
layout arithmetic and yields a `BatchHeader` directly.

Two fields drive everything downstream:

- **`afterDelayedMessagesRead`** — the delayed cursor after this batch. The previous
  batch's value is this batch's starting cursor, and the multiplexer needs it to index
  `DelayedMessages` segments.
- **`dataLocation`** — where the batch's bytes actually live.

## Where the bytes live

`dataLocation` selects one of three retrieval strategies, modelled as `BatchPayload`:

```text
TxInput            ──▶ Calldata(bytes)   decode the posting tx's calldata
SeparateBatchEvent ──▶ Calldata(bytes)   read a SequencerBatchData event
Blob               ──▶ Blob { versioned_hashes, block_number }
NoData             ──▶ None              a delayed-only batch
```

`TxInput` and `SeparateBatchEvent` both yield the same payload shape — a header-flag byte
followed by compressed segments — so downstream decode treats them identically.

### Calldata

`extract_calldata_payload` in `crates/arb-reth-l1/src/lib.rs` dispatches on the posting
transaction's 4-byte selector. Three variants are recognised:

| Function | Notes |
| --- | --- |
| `addSequencerL2BatchFromOrigin` | The current calldata poster. |
| `addSequencerL2BatchFromOrigin` (legacy signature) | Older deployments. |
| `addSequencerL2BatchFromOriginDelayProof` | Selector `0x69cacded`; current nitro-testnode and Orbit chains. |

Each decodes to the same `data` argument. An unrecognised selector surfaces as
`L1Error::UnknownSelector` rather than being silently skipped — a new poster variant should
be a loud failure, not a hole in the derived chain.

There is a unit test pinning the delay-proof selector to `0x69cacded` specifically because
falling through to `UnknownSelector` there would break every current testnode and Orbit
chain.

### Blobs

The blob path resolves separately, via the beacon API. See [Blobs](ch03-04-blobs.md).

### Empty batches are legal

An empty payload is a *valid* empty sequencer message, not an error. Nitro's
`ParseSequencerMessage` logs "empty sequencer message" and returns zero segments; so does
`decode_payload_messages`. Early Arbitrum One batches do exactly this — batch 0 is a
`SeparateBatchEvent` with empty data.

The multiplexer still runs on an empty payload, because a batch with no segments can still
*read delayed messages*: if `afterDelayedMessages > before_delayed_count`, those
force-included messages must be emitted. Only the segment list is empty.

## Assembly into feed messages

`crates/arb-reth-l1/src/assemble.rs` turns resolved batches into the numbered feed stream.
For each batch, in order: decode into `DerivedMessage`s, then convert each to a
`BroadcastFeedMessage` numbered from `seq_start`.

The one wrinkle is **batch posting reports** (kind 13). ArbOS charges L1 posting costs by
processing a report message describing a previously posted batch, and that report needs
`BatchDataStats` — token count and gas cost — computed from the batch's serialized bytes.
Those stats are not carried on a `DerivedMessage`.

The resolution is an accumulating map:

```text
as each batch is resolved:
    report_stats[batch_data_hash(batch)] = batch_data_stats(batch)

when a BatchPostingReport message appears:
    look up report_stats[report_data_hash(l2_msg)]
```

This works because of a posting-order property: a report is posted in the same L1
transaction as the batch it reports on, and is consumed by a *later* batch. So the reported
batch is always seen before its report. A lookup miss surfaces as
`L1Error::Missing("batch posting report stats (batch not seen)")`.

The map (`ReportStatsCache`) is threaded across consecutive windows rather than rebuilt per
window, so the fetches behind it are paid once per L1 range.

## Locating a batch by number

`SequencerInboxReader::delivery_block_of_batch` finds the L1 block that delivered a given
batch sequence number, walking `getLogs` windows forward from an anchor and giving up after
a bounded number of windows. This is used at startup to resolve a resume point's batch
boundary on-chain — for instance, a genesis snapshot whose resume point is batch 0, anchored
at the `SequencerInbox` deployment block.
