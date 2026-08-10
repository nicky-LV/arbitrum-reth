# The Engine Tree

`ArbEngineDriver` does not write to the database. It hands executed blocks to reth's
`EngineApiTreeHandler` — the same component a normal reth node drives from a consensus
client — and the tree owns canonicalization, the in-memory overlay, and asynchronous
persistence.

## What the driver stands up

`ArbEngineDriver::spawn` (`crates/arb-reth-engine/src/engine.rs`) assembles the whole
stack:

```text
persistence service   ◀── storage_v2::spawn_persistence(factory, pruner, metrics)
consensus             ◀── NoopConsensus
payload validator     ◀── BasicEngineValidator wrapping ArbPayloadValidator
payload builder       ◀── ArbPayloadBuilder → ArbPayloadJobGenerator → PayloadBuilderService
overlay manager       ◀── OverlayManager over the state-trie worker pool
tree                  ◀── EngineApiTreeHandler::spawn_new(...)
```

Two of those deserve comment.

**`NoopConsensus`.** There is nothing to validate. The node produced the block itself from
an authoritative message; there is no untrusted peer whose claims need checking.

**`ArbPayloadValidator`.** A minimal implementation that exists to satisfy the engine-tree
generics. It is never meaningfully invoked, because no external party ever submits a payload.

The tree returns a request sender (`to_tree`) and an event receiver. Events are drained on a
background task so the channel can never block, and only one event type is forwarded onward:

```rust,ignore
if let EngineApiEvent::BeaconConsensus(
    ConsensusEngineEvent::CanonicalChainCommitted(header, _)
) = ev { obs_tx.send((header.number, header.hash())); }
```

`CanonicalBlockAdded` is deliberately *not* forwarded. It fires when an executed block is
inserted as pending, which is not proof that RPC-visible canonical state has advanced.
Treating it as such would let the driver report a block applied before anyone could query it.

## The two requests

Per block, the driver sends:

```rust,ignore
// 1. launch the build
FromEngine::Request(EngineApiRequest::Beacon(BeaconEngineMessage::ForkchoiceUpdated {
    state: ForkchoiceState { head_block_hash: parent, safe_block_hash: parent,
                             finalized_block_hash: B256::ZERO },
    payload_attrs: Some(attributes),
    tx: fcu_tx,
}))

// 2a. hand back the executed result — no re-execution
FromEngine::Request(EngineApiRequest::InsertExecutedBlock(built))

// 2b. make it canonical
FromEngine::Request(EngineApiRequest::Beacon(BeaconEngineMessage::ForkchoiceUpdated {
    state: ForkchoiceState { head_block_hash: new_hash, safe_block_hash: new_hash,
                             finalized_block_hash: B256::ZERO },
    payload_attrs: None,
    tx: fcu_tx,
}))
```

`safe` tracks `head`; `finalized` stays zero. Arbitrum finality is an L1 property, not
something this node tracks.

Between (1) and (2a), the driver resolves the payload job with `PayloadKind::Earliest` —
there is no competitive transaction selection to wait for, so the deterministic
single-message build is taken as soon as it exists.

## `TreeConfig` knobs

`ArbEngineTuning::to_tree_config` maps the CLI flags onto reth's `TreeConfig`:

| Knob | Default | Meaning |
| --- | --- | --- |
| `persistence_threshold` | 2 | Canonical lead over the last persisted block before a persistence batch. |
| `memory_block_buffer_target` | 0 | Recent blocks retained in memory before flushing. |
| `persistence_backpressure_threshold` | 16 | Hard stall on production once this many blocks are unpersisted. |
| `cross_block_cache_size` | 256 MiB | Account, storage, and bytecode cache across blocks. |
| `share_execution_cache_with_payload_builder` | `true` | Safe: one payload at a time. |
| `share_sparse_trie_with_payload_builder` | `false` | Opt-in overlap of state-root work. |

The first three are reth's stock values, tuned for a live validator: persist promptly, hold
almost nothing. Bulk historical sync wants the opposite — see
[Observability and Tuning](ch07-00-observability.md).

The cache size default differs from reth's on purpose. reth's generic `TreeConfig` default
is 4 GiB, which makes its fixed-size cache tables needlessly sparse for a serial producer
touching a comparatively small working set; 256 MiB keeps them dense.

`to_tree_config` sets the backpressure threshold *first*, because `TreeConfig` validates its
invariants after every builder call and a deep configuration would otherwise trip a debug
assertion mid-construction.

## Canonical before durable

The most important consequence of this design:

```text
   block produced
        │
        ├──▶ InsertExecutedBlock + FCU ──▶ canonical IN MEMORY   (milliseconds)
        │                                       │
        │                                       └──▶ RPC can serve it NOW
        │
        └──▶ persistence service ──▶ MDBX + static files          (batched, async)
```

Production waits only for in-memory canonicalization. Persistence happens in the background,
in batches, and is deliberately outside the critical-path timing.

This has two practical consequences that recur throughout the book:

- **RPC sees blocks before disk does.** `BlockchainProvider` reads across the in-memory
  overlay and the database, so a freshly produced block is queryable immediately. See
  [Serving RPC](ch05-02-rpc.md).
- **`last_block_number()` is the *durable* tip, not the canonical head.** L1 derivation's
  resume checkpointing depends on this distinction: a checkpoint may only be recorded once
  its blocks are on disk. See [L1 Derivation](ch03-02-l1-derivation.md).

## Termination

`EngineTerminationGuard` wraps the tree sender and sends `FromOrchestrator::Terminate` on
`Drop`, using an atomic swap so the request is sent at most once. `ArbEngineDriver::shutdown`
does the same explicitly and awaits the response.

Either way, the tree flushes its in-memory tail before the process exits. If it did not, a
clean shutdown would lose exactly the unpersisted window that `memory_block_buffer_target`
and `persistence_threshold` are configured to hold.
