# The Driver Loop

The loop that drains the channel lives in `crates/arb-reth-node/src/launcher.rs`, spawned
as the critical task `arb-engine-driver`. It is small, but two details in it account for a
large share of catch-up throughput.

## Batching

```rust,ignore
const MAX_MESSAGE_BATCH: usize = 64;

let Some(first) = messages_rx.recv().await else { break };
let mut batch = vec![first];
while batch.len() < MAX_MESSAGE_BATCH {
    match messages_rx.try_recv() {
        Ok(msg) => batch.push(msg),
        Err(Empty) => break,
        Err(Disconnected) => { source_closed = true; break }
    }
}
```

The loop blocks for one message, then greedily drains up to 63 more without blocking.

The batch is not a performance trick in itself — the messages are still applied one at a
time, in order. What it provides is a **deterministic proof that another message is
ready**, which is what the overlap below needs. The alternative, asking the receiver
`is_empty()`, is racy: a `false` does not guarantee the next `recv` succeeds promptly.

The consequence is that the loop adapts to its situation automatically. During historical
catch-up, derivation floods the channel, batches fill, and every non-final message overlaps.
At the live tip, messages arrive one per block, batches have length 1, and each block is
fully settled before the next frame arrives — which is what you want when measuring
tip-to-canonical latency.

## Overlapping the forkchoice round trip

Applying one message involves two requests to the engine tree:

1. `ForkchoiceUpdated` with payload attributes → launches the build.
2. `InsertExecutedBlock` + `ForkchoiceUpdated` with the new head → makes it canonical.

Naively, the driver would await the response to (2) before starting (1) for the next
message. That serialises two full producer↔engine round trips per block.

Instead, the driver **queues** the final FCU and returns without awaiting it:

```text
  message N:   send attrs FCU ──▶ build ──▶ InsertExecutedBlock
                                            send final FCU  ─┐
                                                             │ (not awaited)
  message N+1: send attrs FCU ─────────────────────────────┐ │
                                                           │ │
                                            settle N's final FCU ◀┘
                                                           │
                                            await N+1's attrs response
```

`apply_one_native` sends the next message's attributes FCU *first*, then calls
`settle_pending_applied()` to collect the previous block's forkchoice result, and only then
awaits its own response. Because the engine processes requests in order, block N is
guaranteed canonical before block N+1's payload job starts — the ordering is preserved by
the queue, not by the producer's awaiting.

The `defer_tail` parameter to `advance_with_applied_overlap` is what the batching feeds:
it is `true` for every message except the last in a batch, so the final block of a batch is
always fully settled before the loop goes back to `recv()`.

## Settling and the canonicalization check

`settle_pending_applied` awaits the queued FCU task, then confirms the block is genuinely
observable, by either:

- the provider reporting `block_hash(number) == expected`, or
- the canonical in-memory head matching number and hash.

A successful FCU response should already imply this. The explicit check is retained anyway
so that a future change in engine-tree behaviour cannot silently turn the overlap into an
early callback — which would report a block as applied before RPC could see it. If neither
holds, it falls back to `wait_for_head`, polling events and the in-memory head with a
bounded timeout, and errors if the block never becomes canonical.

There is also a small hygiene detail: when the provider check succeeds, the driver drains
the redundant committed-chain events with `while self.obs_rx.try_recv().is_ok() {}`. Without
that, the observation channel would grow unboundedly through a long historical sync.

## Bench accounting

The loop separates two clocks:

```text
recv_ms  time waiting for the next message   → L1-fetch bound
work_ms  time inside advance()               → compute/persist bound
```

Every 1000 blocks it emits a summary at `target: "arb-reth::bench"` with blocks/s and the
`recv` percentage. This single number tells you which half of the system to tune: a high
`recv_pct` means the L1 provider or prefetch depth is the constraint; a low one means
execution, state root, or persistence is.

## Shutdown

When the channel closes — every sender dropped — the loop breaks and calls
`driver.shutdown()`, which asks the engine tree to flush its in-memory tail and terminate.
The driver additionally holds an `EngineTerminationGuard` whose `Drop` sends the same
`Terminate` request, so an early return or a panic still releases persistence handles rather
than leaving the tail unflushed.
