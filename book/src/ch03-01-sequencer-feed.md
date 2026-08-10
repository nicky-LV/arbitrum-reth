# The Sequencer Feed

`--feed-url` is the low-latency path: it follows the sequencer's relay directly instead of
waiting for batches to reach L1. The whole follower is one spawned task in
`crates/arb-reth-node/src/commands/node.rs`, and it is short enough to describe completely.

```text
  connect (with resume header)
        │
        ▼
  read frame ──▶ parse as Root { version, messages: [BroadcastFeedMessage] }
        │              │
        │              ├─ record ingress timestamps (metrics)
        │              ├─ next_seq = msg.sequence_number + 1
        │              └─ send into the feed channel
        │
        ▼
  disconnect ──▶ sleep 2s ──▶ reconnect from next_seq
```

## Asking the relay where to start

Before the WebSocket upgrade, the follower sets a request header:

```text
Arbitrum-Requested-Sequence-Number: <next_seq>
```

The initial value is derived from the node's own tip:

```rust,ignore
let feed_start_seq = handle.provider
    .last_block_number()
    .unwrap_or(feed_genesis_block)
    .saturating_sub(feed_genesis_block)
    + 1;
```

That is the message-index form of "one past my current block" — the
`block = seq + genesis_block` relation from
[The Big Picture](ch02-00-the-big-picture.md), rearranged.

This is an **optimisation, not a correctness mechanism**. A relay holds only a bounded
backlog; if the requested sequence predates what it retains, it simply streams what it has.
The driver's sequence guard deduplicates and buffers regardless, so a wrong hint costs
redundant work, not correctness.

On reconnect the header carries the *live* `next_seq`, which advances as messages arrive —
so a dropped connection resumes near where it stopped rather than replaying the whole
backlog.

## Frames

Each data frame is a JSON object of the form:

```json
{ "version": 1, "messages": [ { "sequenceNumber": …, "message": { … } } ] }
```

The follower deserialises into a local `Root` type and iterates `messages`. Frames that
carry no messages — the relay also publishes confirmed-sequence-number notices — deserialise
with an empty or absent list and are skipped. Binary frames are treated as UTF-8 text;
ping/pong frames are ignored; a `Close` frame breaks the read loop into the reconnect path.

A frame that fails to parse is logged at `debug` and skipped rather than killing the
follower. That is intentional: relay implementations may add frame types, and an unknown
frame is not a reason to stop following the tip.

## Latency instrumentation

The follower is the only place with a meaningful notion of *ingress time*, so it is where
feed-latency tracking starts. `FeedLatencyTracker`
(`crates/arb-reth-node/src/metrics.rs`) records three timestamps per message:

| Sample | Taken |
| --- | --- |
| `record_frame_arrival` | Before decoding the frame — so decode cost is inside the measurement. It also takes the wall clock at that same edge and the message's sequencer stamp, the pair the `sequenced_*` metrics are built from. |
| `record_ready_for_channel` | After parsing, before the channel send. |
| `record_driver_dequeue` | In the driver loop, when the message is picked up. |

The driver later calls `record_canonical` when the message's block becomes the canonical
in-memory head, closing the loop. That end-to-end span is
`reth_arb_reth_feed_frame_to_canonical_seconds`; measured against the sequencer's own stamp
instead of ingress it is `reth_arb_reth_feed_sequenced_to_state_seconds`. Both are described in
[Observability and Tuning](ch07-00-observability.md).

The tracker is created only when `--feed-url` is set. L1-derived and replayed messages
drive the identical engine callback but have no ingress sample, so they are deliberately
excluded from the `feed_*` metric family — mixing catch-up blocks into a tip-latency
histogram would make it meaningless.

## Reconnection

There is no backoff ladder here: a failed connect or a dropped socket sleeps a flat two
seconds and retries, forever. The only conditions that stop the follower permanently are an
unparseable URL (logged as an error) and the feed channel closing (which means the node is
shutting down anyway).

That simplicity is reasonable because the feed is never the source of truth. If it stays
down, L1 derivation keeps the chain advancing — just with batch-posting latency instead of
sequencing latency.

## TLS note

`wss://` relays need a process-default rustls `CryptoProvider`. The dependency tree carries
both `aws-lc-rs` and `ring`, so rustls cannot pick one on its own, and the first
`connect_async` would fail. `main()` in `crates/arb-reth-node/src/bin/arb-reth.rs`
installs `aws_lc_rs` explicitly at startup for this reason.
