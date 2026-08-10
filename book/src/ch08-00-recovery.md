# Recovering from Trouble

Because the node mints its own blocks, a wrong block is still an internally consistent
block. That shapes what can go wrong and how you fix it.

## Divergence

The failure that matters most: your chain and the canonical chain disagree at some height.

**Detecting it.** The only reliable check is external comparison. Query a public canonical
RPC for a block hash and state root at a height you have, and compare. `dump-blocks` prints
headers, transaction hashes, and receipt status straight from a datadir, which is usually
enough to bisect where the disagreement starts.

**Why the state root alone is not enough.** As
[ArbOS Execution](ch04-03-arbos-execution.md) explains, a mis-ordered transaction list
produces the correct state and the wrong `transactionsRoot` / `receiptsRoot`. State-root
parity passes; the block hash is wrong. The divergence only becomes materially visible once
a later L1-advancing block bakes the bad parent hash into ArbOS state via
`record_new_l1_block`. So compare **hashes**, not just roots.

**Fixing it.** `arb-reth rewind` removes the local suffix from a confirmed divergence point:

```sh
arb-reth rewind --datadir /data/arb1 --to <L2_BLOCK>
```

Then restart the node. Derivation resumes from the newest resume-log boundary that is safe
for the new tip, and re-derives forward.

> Rewind to a block you have **confirmed** is correct, not to the first one you suspect is
> wrong. Rewinding too far is cheap — derivation replays it. Rewinding not far enough leaves
> the bad block in place and wastes the whole cycle.

## Derivation stopped

The node stops when L1 derivation cannot resume — deliberately. The alternative, parking
forever behind a tip that can never advance, hides the problem.

Check the error class first:

- **`transient L1 provider failure during …`** — retryable. The supervisor should already be
  backing off and retrying with unbounded exponential backoff. If it is retrying forever,
  your endpoint is the problem: rate limits, a non-archive execution RPC, or a beacon node
  without blob retention.
- **`deterministic L1 derivation failure during …: …`** — terminal, and a real bug or
  genuinely unsupported chain data. The inner error names the stage:
  `UnknownSelector` means a batch-poster function this build does not recognise; `Batch` or
  `Mux` errors mean the framing or segment decode disagreed with the chain.
- **`invalid L1 RPC URL`** — configuration.

Provider errors deliberately do not include the endpoint URL, because those URLs carry API
keys. The operation name is what you get; correlate it with the request pattern.

## Sync is slow

Read `recv_pct` from the bench line first (see
[Observability and Tuning](ch07-00-observability.md)). It splits the problem cleanly:

**Starved (high `recv_pct`).**
- Raise `--l1-prefetch` until the provider rate-limits.
- Check `--l1-getlogs-range` matches the provider's `eth_getLogs` cap — set too low, it
  multiplies request count.
- In the blob era, the beacon endpoint is usually the bottleneck. A blob-archive endpoint is
  not optional for deep history.

**Compute or persist bound (low `recv_pct`).**
- Deepen persistence batching (`--persistence-threshold 128`, `--persistence-backpressure
  512`).
- Check `backpressure_active`: if production stalls regularly, the disk is the limit.
- Try `--share-sparse-trie-with-payload-builder`, then check the
  `state_root_task_native_success` / `fallback` ratio to confirm it is actually being used.

## Restarted and lost blocks

Expected if you ran `--no-fsync`. MDBX rolled back to its last synced meta page, and the
node lost a suffix of recently produced blocks.

This is recoverable by design: derivation resumes from the newest resume-log boundary at or
below the durable tip and re-derives forward. The drop gate discards anything already on
disk.

> A cautionary note recorded in this repository's own operational history: `--no-fsync` has
> destroyed a datadir outright. "Crash-consistent" is a statement about MDBX's meta pages,
> not a promise about your directory surviving every failure mode. Keep the retained input
> streams that let you re-import, and treat re-import time as the real cost of using the flag.

## Genesis root mismatch on a fresh chain

The node refuses to boot because the computed genesis state root does not match the chain's.

Check, in order:

1. `--initial-l1-base-fee`. It is part of genesis *state* and defaults to 50 gwei; a
   nitro-testnode commonly uses a tiny value. This is the most common cause.
2. That `--chain-info` and `--genesis` are from the *same* deployment. The rollup addresses
   and the genesis allocation must describe one chain.
3. That you are not also passing `--chain` alongside `--snapshot-head` — snapshot boot builds
   its spec from the head header instead.

Use `genesis verify` to check a state export's root independently of booting a node.

## Derivation runs but produces no blocks

Classic symptom of a `genesis_block` mismatch. Every message lands far above the driver's
`next_seq`, so all of them are buffered as feed-ahead and none are applied.

Check `--l2-genesis-block`: `22_207_817` for Arbitrum One, `0` for a fresh chain. The
relation is `block = sequence_number + genesis_block`, and it is used in the driver cursor,
the sync runtime's numbering, and the feed follower's resume header — all three have to agree.

## Snapshot imported but the node will not start

- Ensure you passed `--snapshot-head` (the blocks stream), not just `--datadir`.
- Do not pass `--chain` with `--snapshot-head`.
- An import from an older tool version may lack history-boundary metadata; run
  `snapshot repair-history` against the datadir.
- Verify the import's state root with `snapshot read` or `genesis verify-export` before
  spending hours on a node that will compute wrong roots regardless.
