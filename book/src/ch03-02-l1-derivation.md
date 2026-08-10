# L1 Derivation

This is the trustless path, and the only one that can sync history. It reconstructs the
exact message stream the sequencer published, using nothing but data committed to Ethereum:
`SequencerInbox` logs, batch-poster calldata, blob sidecars, and `Bridge` events.

The runtime lives in `crates/arb-reth-sync/src/l1_sync.rs`; the fetch and decode primitives
it drives live in `arb-reth-l1` and `arb-reth-derive`.

## The loop

```text
         ┌──────────────────────────────────────────────────────┐
         │  safe head = L1 head - confirmations                  │
         └───────────────────────┬──────────────────────────────┘
                                 │
   ┌─────────────────────────────▼──────────────────────────────┐
   │  PREFETCH (up to `prefetch_windows` concurrent)            │
   │                                                            │
   │  for each candidate window [from, to]:                     │
   │    batchCount(to) > batchCount(from-1) ?                   │
   │        yes ──▶ spawn resolve_batches(from, to)             │
   │        no  ──▶ binary-search the next batch's block,       │
   │                emit one empty window covering the gap      │
   └─────────────────────────────┬──────────────────────────────┘
                                 │  (FIFO, in order)
   ┌─────────────────────────────▼──────────────────────────────┐
   │  CONSUME oldest window                                      │
   │    ├─ fetch delayed messages covering what the batches read │
   │    ├─ decode each batch  ──▶ DerivedMessage list            │
   │    ├─ convert to BroadcastFeedMessage, number them          │
   │    ├─ drop any block <= db_tip_l2                           │
   │    └─ send the rest into the feed channel                   │
   └─────────────────────────────┬──────────────────────────────┘
                                 │
   ┌─────────────────────────────▼──────────────────────────────┐
   │  CHECKPOINT: queue this window boundary; flush to           │
   │  arb-l1-resume.json once its L2 blocks are durable          │
   └────────────────────────────────────────────────────────────┘
```

## Windows and the batch-count gate

Derivation walks L1 in windows of `batch_window` blocks (default `1_000`, overridable with
`--l1-getlogs-range` when a provider caps `eth_getLogs` spans).

Scanning every window with `getLogs` is wasteful on a sparse chain — most windows of a
young Orbit chain contain no batches at all. So each candidate window is first gated on a
cheap view call, `batchCount()` on the `SequencerInbox` (Nitro's
`InboxReader.GetBatchCount`):

- If the count at the window's end exceeds the count before it, the window delivered
  batches: spawn the real `getLogs` + payload resolution.
- If not, the window is barren. Rather than stepping through barren windows one at a time,
  the runtime binary-searches for the block where the next batch appears and emits **one**
  empty window covering the entire gap.

An empty window still advances the resume cursor, so barren L1 stretches are never
re-scanned after a restart. Dense chains like Arbitrum One pay one extra view call per
productive window — a good trade for collapsing sparse chains from thousands of scans to a
handful.

## Prefetch

`resolve_batches` — the `getLogs` scan plus per-batch payload retrieval — is the dominant
RPC cost, and it does not depend on the delayed-message cursor. So it is safe to run
several windows' worth concurrently. `--l1-prefetch` (default `6`) sets how many
resolutions stay in flight.

The *consumption* side stays strictly ordered: prefetched windows are held in a FIFO and
drained oldest-first, because the delayed cursor and message numbering are inherently
sequential. Prefetch overlaps latency, it does not reorder work.

Within a single window, blob batches are additionally resolved with a bounded concurrency
of 8 (`BATCH_RESOLVE_CONCURRENCY` in `crates/arb-reth-l1/src/sync.rs`). Each blob batch is
a chain of round trips — posting-tx fetch, block fetch, and a sidecar download of roughly
768 KiB — so resolving them serially made blob-era sync latency-bound rather than
CPU-bound.

## Absolute numbering and the drop gate

Every derived message is assigned an **absolute L2 block number**, counting up from
`start_l2_block`:

```text
next_l2 = start_l2_block + 1, +2, +3, …
```

Messages whose block number is at or below `db_tip_l2` — the current *durable* tip —
already exist on disk and are dropped rather than sent. This is what makes a resume safe
when the checkpoint predates the durable tip: persistence can outrun the last written
checkpoint, so the first re-derived blocks legitimately reproduce blocks already stored.
The gate silently discards them and the driver only ever sees `db_tip_l2 + 1` onward.

Note the counter advances for dropped blocks too — they are real blocks in the chain, so
skipping them in the count would misalign everything after.

There is a trap here that the code comments flag emphatically. The number handed to the
driver as `sequence_number` must be the **message index**, not the absolute block number:

```text
sequence_number = absolute_block - genesis_block
```

On a chain whose genesis is block 0 these coincide, so a testnode never catches the bug. On
Arbitrum One, genesis is `22_207_817`; sending absolute numbers would put every message
tens of millions above the driver's cursor, where it buffers them all as "feed-ahead" and
never applies a single block.

## Resume checkpoints

The runtime maintains `arb-l1-resume.json` in the datadir (`L1ResumeLog`, in
`crates/arb-reth-sync/src/resume.rs`). Each entry records an L1 window boundary together
with the L2 block that boundary sits after.

The discipline that makes it recoverable: **a boundary is only appended once its L2 blocks
are durable.** The runtime polls a `persisted_tip` callback — the persisted DB head, *not*
the in-memory canonical head — once per consumed window, and flushes queued boundaries whose
`l2_block` is at or below it. So after a crash, the DB tip is always at or beyond the
last-logged boundary, and resuming from that boundary can only ever re-derive blocks the
drop gate discards.

A failed save is logged, not fatal. The boundaries stay queued in memory and the next
window's save re-persists them.

## Supervision and failure classes

`supervise_l1_sync` wraps the runtime, and the distinction it draws matters:

```rust,ignore
pub enum L1SyncError {
    Provider { operation: &'static str },     // retryable
    Derivation { operation, source },          // terminal
    InvalidRpcUrl,                             // terminal
    PrefetchTask { from, to },                 // terminal
}
```

- **Provider failures** — RPC 429s, 5xx, timeouts, beacon fetch failures — are transient.
  The supervisor restarts from the newest checkpoint that is safe for the current durable
  tip, with unbounded exponential backoff (1s to 30s) plus deterministic 0–20% jitter so a
  fleet sharing an outage does not reconnect in lockstep.
- **Derivation failures** are deterministic: corrupt or unsupported chain data. Retrying
  cannot help, so they stop the worker and the node exits. Turning these into an infinite
  retry loop would hide a real problem behind a tip that never advances.

Note that `L1Error::Rpc` is reclassified as `Provider` by `L1SyncError::l1`, so a transport
failure surfacing from deep inside the decode glue is still treated as retryable.

The HTTP transport also carries its own retry layer for rate limits and 5xx, so most
transient failures never reach the supervisor at all. Beacon blob fetches have separate
retry logic in `BeaconClient`.

## Errors deliberately do not carry URLs

`L1SyncError::Provider` records only an operation name, never the provider's error text.
HTTP client errors routinely embed the full endpoint URL, and endpoint URLs routinely embed
API keys. Dropping the detail keeps credentials out of logs.

## ArbOS versions across upgrades

The runtime carries no ArbOS version. Version advances happen per block, downstream: the
payload builder reads the version from the parent header's encoded `ArbHeaderInfo`, and the
per-block start-block internal transaction applies scheduled ArbOS upgrades when they come
due. A catch-up that crosses an upgrade boundary is therefore wired end to end — though the
source comments are candid that this has not yet been validated against a real mainnet
upgrade crossing.
