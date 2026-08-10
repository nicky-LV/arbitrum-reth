# Observability and Tuning

```sh
arb-reth node --metrics 127.0.0.1:9001 …
```

serves reth's Prometheus endpoint. Bind it to loopback unless the metrics network is
deliberately exposed.

Telemetry never blocks the feed reader or the block producer: under contention a sample is
dropped instead. `reth_arb_reth_feed_tracking_dropped_total` should stay at zero in normal
operation, and a non-zero value means you are losing measurements, not blocks.

## Which family to use

There are two families measuring block production, and picking the wrong one produces
nonsense.

| Family | Covers | Use for |
| --- | --- | --- |
| `reth_arb_reth_feed_*` | Only messages seen on the live WebSocket feed | Tip latency, MEV-facing measurements |
| `reth_arb_reth_engine_block_*` | **Every** produced block, including L1 catch-up | Sync throughput benchmarking |

The `feed_*` family deliberately excludes L1-derived blocks, because they have no meaningful
ingress timestamp. Mixing catch-up blocks into a tip-latency histogram would make it
meaningless.

Conversely: samples collected while burning through a feed backlog include that backlog. To
judge tip latency, measure a node that is already at the tip.

## Sequencer to state

Two spans answer "how stale is the state I am acting on":

| Series | Spans | Exact? |
| --- | --- | --- |
| `reth_arb_reth_feed_sequenced_to_state_seconds` | Sequencer's own stamp → canonical here | ±1s stamp bias |
| `reth_arb_reth_feed_sequenced_to_received_seconds` | Sequencer's own stamp → this node's ingress | ±1s stamp bias |
| `reth_arb_reth_feed_frame_to_canonical_seconds` | Ingress → canonical here | exact |

The first is the sum of the other two, by construction.

The sequencer stamps whole seconds while blocks arrive far faster than that, so every
`sequenced_*` sample also carries the sub-second remainder of its own stamp — a positive bias
of up to a second. `quantile="0"` over a window is the tightest estimate of true latency; the
mean sits roughly half a stamping second high. Only `frame_to_canonical_seconds` is free of
this, because both of its edges are local monotonic instants.

The `sequenced_*` pair is the only place a wall clock is compared across machines. A negative
sample means this node's clock trails the sequencer's; the series is signed rather than clamped
so that stays visible instead of reading as zero latency.

## Feed to canonical

`reth_arb_reth_feed_frame_to_canonical_seconds` spans from receiving a WebSocket data frame
through decoding, queueing, block production, forkchoice, and in-memory canonicalization. It
ends when the shared provider state used by RPC has the new canonical head. It excludes an
RPC client's own network round trip and response serialization.

To localise a delay, the phases decompose (all names carry the `reth_arb_reth_` prefix):

```text
ingress and ordering
    feed_frame_decode_seconds
    feed_channel_wait_seconds
    feed_sequencing_wait_seconds

exclusive outer phases   (means add up to engine_apply_total_seconds)
    feed_payload_attributes_seconds
    feed_payload_job_seconds
    feed_engine_handoff_seconds
    feed_engine_apply_overhead_seconds

inside the payload job
    feed_payload_job_launch_seconds
    feed_payload_job_resolve_seconds
    feed_payload_job_overhead_seconds     (job minus the builder's measured production)

inside production
    feed_block_*_seconds                  parent state, message prep, execution, finalisation

nested inside engine_handoff — do NOT add to the outer phases
    feed_engine_insert_seconds
    feed_engine_forkchoice_seconds
    feed_canonicalization_wait_seconds
```

The "exclusive" versus "nested" distinction is the one people get wrong. The four outer
phases partition the total; the three engine diagnostics are *inside* `engine_handoff` and
double-count if added.

## ArbOS execution

Per-transaction-family series, labelled `legacy`, `eip1559`, `deposit`, `unsigned`,
`contract`, `retry`, `submit_retryable`, `internal`. Names below omit the
`reth_arb_reth_` prefix that the Prometheus endpoint applies:

- `arbos_transaction_execution_seconds` — the full ArbOS handler transition, including
  applicable pre-execution hooks and the EVM or protocol transaction body.
- `arbos_transaction_commit_seconds` — receipt construction plus the in-memory state commit.
- `arbos_transaction_gas_used` / `arbos_transaction_l1_gas_used` — L2 gas and L1 poster gas.
- `arbos_pre_execution_system_call_seconds` — the EIP-2935 prelude.
- `arbos_post_execution_header_info_seconds` — the ArbOS header-field read in `finish`.

And from `arb-revm` itself:

```text
reth_arb_revm_arbos_handler_phase_seconds{phase, tx_type, mode}
    phase = pre_execution   ArbOS gas charging and filtering
          | execution       the protocol or EVM frame
          | end_tx_hook     fee distribution, refunds, backlog updates
    mode  = execute         block production
          | inspect         debug tracing — exclude from latency views
```

Filtering out `mode="inspect"` matters: a single `debug_traceTransaction` will otherwise
skew your execution histograms.

Labels deliberately exclude addresses, transaction hashes, block numbers, and ArbOS version,
to keep cardinality bounded. For per-contract or opcode detail, use a one-off profiler.

`ARB_EXECUTION_METRICS_SAMPLE_RATE` gates only these detailed histograms: `1` (default)
records everything, `16` records one transaction of each type per 16-transaction window and
is the recommended throughput setting, `0` disables them. Block production, MGas/s,
persistence, state root, and feed-to-canonical are never sampled by it.

## Execute and persist

The reth metrics that describe the path this node actually uses:

| Metric | Tells you |
| --- | --- |
| `reth_blockchain_tree_in_mem_state_num_blocks` | Size of the unpersisted in-memory window. |
| `reth_consensus_engine_beacon_backpressure_active` | Whether production is stalled on persistence. |
| `reth_consensus_engine_beacon_backpressure_stall_duration` | How long it stalled. |
| `reth_consensus_engine_beacon_persistence_duration` | Persistence batch latency. |
| `reth_consensus_engine_persistence_save_blocks_*` | Batch size and latency. |
| `reth_consensus_engine_beacon_inserted_already_executed_blocks` | Engine-tree throughput on the insert path. |

reth's standard pipeline and `newPayload` metrics are exported too, but they do not drive
this node's loop — ignore them.

## The one-line throughput signal

The driver logs a summary every 1000 blocks at `target: "arb-reth::bench"`:

```text
bench: 1000-block window  blocks=… blk_per_s=… recv_ms=… work_ms=… recv_pct=…
```

`recv_pct` is the share of wall time spent *waiting for messages* rather than producing
blocks. It tells you which half of the system is the constraint:

- **High `recv_pct`** — starved. The L1 provider, `--l1-prefetch`, or `--l1-getlogs-range`
  is the limit. In the blob era, suspect beacon sidecar fetch latency first.
- **Low `recv_pct`** — compute or persist bound. Look at
  `reth_arb_reth_engine_block_*` to see whether execution, state root, or the engine handoff
  dominates, and at backpressure to see whether the disk is the real limit.

## Tuning for bulk sync

The defaults target a live node. For historical catch-up, the benchmark configuration in the
README used:

```sh
--persistence-threshold 128 \
--memory-buffer-target 0 \
--persistence-backpressure 512 \
--share-sparse-trie-with-payload-builder
```

That is: commit in large batches, let production run up to 512 blocks ahead of the disk, and
overlap state-root computation with ArbOS execution.

`--share-sparse-trie-with-payload-builder` only pays off with useful state-root worker
parallelism, and it silently falls back on storage wipes and pre-ArbOS-30 created-empty
accounts (see [Sealing the Block](ch04-04-sealing.md)). Watch the ratio of
`state_root_task_native_success` to `state_root_task_fallback`: a high fallback rate means
you are paying for the task and discarding its output.

`--engine.cross-block-cache-size` defaults to 256 MiB, chosen to keep reth's fixed-size cache
tables dense for a serial producer. Raising it toward reth's generic 4 GiB is not
automatically better.

> The node builds only one Arbitrum payload at a time. `--share-execution-cache-with-payload-builder`
> and `--share-sparse-trie-with-payload-builder` are safe *because* of that. Do not carry
> these settings into a node that can run concurrent payload jobs without first reviewing
> reth's cache and sparse-trie ownership rules.

## Per-block tracing

Independent of Prometheus, every produced block logs:

```text
target: arb-reth::engine        number, hash, state_root, gas_used   "produced block"
target: arb-reth::engine::timing  us_attributes, us_payload_job, us_produce,
                                  us_handoff, us_insert, us_fcu, us_wait, us_total
```

The `timing` line is at `debug`. It is the fastest way to see where a single slow block went
without setting up a scrape.
