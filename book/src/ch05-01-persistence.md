# Persistence

Blocks reach MDBX through reth's engine-tree persistence service, running on its own OS
thread. `arbitrum-reth` inserts a thin proxy in front of it.

## The write path

```text
engine tree
    │  PersistenceAction::SaveBlocks(ExecutedBlock[])
    ▼
storage_v2 proxy  ── enrich with slot preimages ──▶ reth persistence service
                                                        │
                                                        ├──▶ save_blocks(...)
                                                        │      MDBX tables + static files
                                                        ├──▶ commit()
                                                        └──▶ pruner (at the configured interval)
```

Each `ExecutedBlock` carries the recovered block, the execution output, and precomputed
`ComputedTrieData`, so the persistence service writes rather than recomputes.

## The storage-v2 proxy

This is `arbitrum-reth`-specific and worth understanding, because it exists to patch a
mismatch between two reth subsystems.

In Storage V2, reth stores canonical state under **hashed** keys. Its unwind changesets,
however, record **plain** storage-slot keys. Something has to bridge the two so an unwind
can map a hashed slot back to the plain key it came from. In reth's staged sync, the
`SlotPreimages` sidecar table is populated by a pipeline stage.

`arbitrum-reth` does not run staged sync. So `crates/arb-reth-engine/src/storage_v2.rs`
spawns a proxy thread that sits in front of reth's stock persistence service, enriches each
`SaveBlocks` batch with the slot preimages it needs, and forwards it unchanged otherwise.

A related helper runs earlier, on the driver's side: `mark_live_hashed_storage_wipes` is
called on each built payload before insertion, tagging storage wipes so the persistence
layer handles them correctly. Wipes are also the hazard that disqualifies the sparse
state-root task, as described in [Sealing the Block](ch04-04-sealing.md) — the same
underlying difficulty, surfacing in two places.

`PersistenceProxyGuard` joins the proxy thread once the engine tree's request sender is
dropped, so shutdown does not race the final batch.

## The three knobs

| Flag | Default | Effect |
| --- | --- | --- |
| `--persistence-threshold` | 2 | Persist once the canonical tip is this far ahead of the last persisted block. |
| `--memory-buffer-target` | 0 | Keep this many recent blocks in memory before flushing. |
| `--persistence-backpressure` | 16 | Stall block production once this many blocks are unpersisted. |

The defaults are reth's, and they are right for a node at the tip: persist promptly, hold
almost nothing, keep the crash window tiny.

They are wrong for bulk historical sync, where committing every two blocks turns MDBX
transaction overhead into the bottleneck. The benchmark configuration in the repository
README used `--persistence-threshold 128`, `--memory-buffer-target 0`, and
`--persistence-backpressure 512` — commit in batches of roughly 128 blocks, and allow
production to run up to 512 blocks ahead of the disk.

The backpressure threshold is what keeps that bounded. Without it, a producer faster than
the disk would grow the in-memory window without limit until the process is killed. With it,
production stalls until persistence catches up, and the stall is visible as
`reth_consensus_engine_beacon_backpressure_active`.

## Durability and `--no-fsync`

`--no-fsync` opens MDBX in `SafeNoSync`. Each block still commits — so the parent state is
visible to the child — but the OS flushes lazily, removing roughly 50 ms of fsync latency per
block.

What you keep: crash consistency. MDBX rolls back to the last synced meta page on restart;
the database is never torn.

What you lose: a suffix of recently produced blocks, which L1 derivation can reproduce.

> This is a bulk-sync tool, not a durability setting. It is appropriate for an initial
> import you are prepared to re-run, and inappropriate for a node expected to survive power
> loss without re-sync. Treat losing the datadir as the realistic downside, not the
> theoretical one.

## The standalone persist primitive

`persist_executed_block` in `crates/arb-reth-node/src/persist.rs` is a synchronous
"execute once, persist" primitive used by tests and tooling rather than the running node —
the node persists through the engine tree. It is worth reading because it states the
contract explicitly:

```text
1. HashedPostState::from_bundle_state::<KeccakKeyHasher>(bundle)
2. (root, trie_updates) = state_root_with_updates(hashed)  against the PARENT state
3. assert root == recovered_block.header().state_root
4. ExecutedBlock { block, output, ComputedTrieData }
5. provider_rw.save_blocks(...)  then  commit()
```

Step 3 is the parity check discussed in [Execute to Derive](ch02-01-execute-to-derive.md):
the caller must have baked the correct root into the header, and a mismatch is an error, not
a correction.

## The deadlock rule

From `crates/arb-reth-node/src/launcher.rs`, stated as a rule rather than a comment:

> **Never hold a read provider across a `provider_rw()` / `save_blocks()` call.**

MDBX readers and the writer contend; holding a read transaction open across a write acquires
them in an order that can deadlock. Any new code touching both sides has to respect this.
