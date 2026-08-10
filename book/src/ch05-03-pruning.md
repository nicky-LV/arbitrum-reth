# Pruning and History

With no pruning flags, `arbitrum-reth` is an **archive node**: it retains all historical
state, receipts, and transaction lookups.

## How pruning is wired

Pruning is not a separate process. `ArbEngineDriver::spawn` builds a pruner and hands it to
the persistence service, which runs it after commit batches at the configured interval:

```rust,ignore
let pruner = match prune_builder {
    Some(builder) => builder.build_with_provider_factory(factory.clone()),
    None => Pruner::new_with_factory(factory.clone(), vec![], 5, 0, None, finished_exex_height_rx),
};
```

The `None` arm is a real pruner with an **empty segment set** — a no-op that keeps
everything. That is what "archive by default" means mechanically.

The same `PruneConfig` is applied to *both* the provider factory and the persistence
pruner. This matters more than it looks: the launcher carries a regression test
(`crates/arb-reth-node/src/launcher.rs`) for the case where a first batch deletes
transaction-sender static files and a second batch must not recreate them. Configuring only
one side would make persistence and pruning disagree about which segments exist.

## Profiles

| Flag | Effect |
| --- | --- |
| `--full` | reth's full-node profile: prunes sender recovery completely, retains the unwind-safe recent window for account history, storage history, and receipts. |
| `--minimal` | More aggressive: also prunes transaction lookups, receipts, and static-file data per reth's minimal-storage profile. |
| `--prune.block-interval N` | How often the persistence service may prune. |
| `--prune.minimum-distance N` | Minimum recent block window pruning must retain. |
| `--prune.*` | Granular per-segment rules, e.g. `--prune.account-history.distance`, `--prune.receipts.distance`, `--prune.transaction-lookup.full`, `--prune.sender-recovery.full`. |

`--full` and `--minimal` are mutually exclusive. For granular control, invoke
`arb-reth node --help` and use the `--prune.*` flags directly — each segment supports
`.full`, `.distance <BLOCKS>`, and `.before <BLOCK_NUMBER>` variants.

## When to enable it

> **Prune only after the initial import or catch-up has completed.**

Two reasons. First, a pruned node cannot serve arbitrary historical state, receipts, or
transaction lookups that were deliberately removed — and catch-up tooling and verification
generally want them. Second, pruning during bulk sync spends I/O competing with the write
path you are trying to make fast.

The reverse is also true and worth stating plainly: **pruning is not reversible.** Restoring
pruned history means re-syncing that range.

## Interaction with derivation

Pruning affects what the node can *serve*; it does not affect what it can *derive*. L1
derivation reads from L1, not from local history, and its resume checkpoint records L1 and
L2 block numbers rather than state.

There is one caveat, in [Recovering from Trouble](ch08-00-recovery.md): `rewind` needs a
resume-log boundary old enough to restart derivation from. Aggressive pruning does not
truncate the resume log — it is a separate JSON file — but a heavily pruned node has fewer
options for verifying where a divergence began.

## Snapshot history boundaries

A datadir created by `snapshot import` starts mid-chain, so it has no history below the
snapshot's head. Reth records that as history boundary metadata, and a snapshot imported by
an older version of the tool may lack it. `snapshot repair-history` adds the missing
metadata to an existing import, which is required before pruning behaves sensibly on such a
datadir.
