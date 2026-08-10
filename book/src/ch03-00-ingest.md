# Getting Transactions from the Sequencer

The node's job in this phase is narrow and precise: **produce a correctly ordered stream of
`BroadcastFeedMessage`s, with no gaps and no duplicates.** Everything downstream assumes
that ordering is right; nothing downstream can repair it.

There are three producers, and they all write into the same channel, created in
`crates/arb-reth-node/src/commands/node.rs`:

```rust,ignore
let (feed_tx, feed_rx) = tokio::sync::mpsc::channel::<BroadcastFeedMessage>(4096);
```

```text
  ┌───────────────────────────────────────────────────────────────┐
  │ --feed-url        WebSocket relay        tip only, low latency │
  │ --replay-feed     NDJSON file            deterministic, finite │
  │ --l1-rpc          L1 derivation          full history, trustless│
  └───────────────────────────────┬───────────────────────────────┘
                                  │
                     mpsc channel (capacity 4096)
                                  │
                          ArbEngineDriver
```

The channel's *sender* is deliberately kept alive by the node command for the process's
lifetime, even after a finite `--replay-feed` or a bounded `--l1-end-block` run finishes.
Dropping it would close the channel and shut the driver down; holding it keeps the node up
so RPC stays queryable.

## Two very different sources

`--feed-url` and `--l1-rpc` are not redundant paths to the same data — they trade off
against each other:

| | Sequencer relay | L1 derivation |
| --- | --- | --- |
| Latency | Sub-second from sequencing | Minutes (batch posting + confirmations) |
| History | Bounded backlog only | Complete, from chain genesis |
| Trust | Trusts the relay | Trustless: reconstructed from L1 data |
| Cost | One WebSocket | Heavy archive RPC + blob fetches |

The intended production shape runs both. Derivation grinds through the confirmed prefix;
the feed rides the tip. Because an honest sequencer's feed agrees with what it later posts
to L1, the two streams agree, and the driver only has to deduplicate by sequence number.
[Reconciling Two Sources](ch03-07-reconciling.md) covers what happens when they overlap —
and what is not handled if they ever genuinely disagree.

## Chapter map

- [The Sequencer Feed](ch03-01-sequencer-feed.md) — the WebSocket follower: framing,
  resume header, reconnection.
- [L1 Derivation](ch03-02-l1-derivation.md) — the catch-up runtime: windows, prefetch,
  resume checkpoints, supervision.
- [Reading Batches from L1](ch03-03-reading-batches.md) — `SequencerInbox` logs and
  locating each batch's bytes.
- [Blobs](ch03-04-blobs.md) — recovering batch payloads from EIP-4844 sidecars.
- [The Delayed Inbox](ch03-05-delayed-inbox.md) — reconstructing deposits and
  force-included messages from `Bridge` events.
- [Decoding a Batch](ch03-06-decoding-a-batch.md) — framing, brotli, segments, and the
  multiplexer that turns them into messages.
- [Reconciling Two Sources](ch03-07-reconciling.md) — sequence-number dedup in the driver.
