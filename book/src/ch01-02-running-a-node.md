# Running a Node

A snapshot-seeded Arbitrum One node looks like this:

```sh
arb-reth node \
  --datadir /data/arb1 \
  --snapshot-head /data/head.stream \
  --l1-rpc https://your-archive-rpc.example \
  --l1-beacon https://your-beacon-api.example \
  --http --http.port 8545
```

An Orbit chain booting from a Nitro genesis instead:

```sh
arb-reth node \
  --datadir /data/orbit \
  --chain-info chaininfo.json \
  --genesis genesis.json \
  --l1-rpc https://your-archive-rpc.example \
  --http
```

## Reading the flags as a system diagram

The flag groups correspond directly to the subsystems in the rest of this book.

```text
  boot source            message sources              execution + storage         serving
  ───────────            ───────────────              ───────────────────         ───────
  --snapshot-head        --l1-rpc / --l1-beacon       --persistence-threshold     --http
  --chain-info           --feed-url                   --memory-buffer-target      --http.addr
  --genesis              --replay-feed                --persistence-backpressure  --http.port
  --chain                --no-l1-derive               --engine.cross-block-       --metrics
                                                        cache-size
                                                      --no-fsync
                                                      --full / --minimal
                                                      --prune.*
```

Everything under "message sources" ends up writing into the same in-process channel of
`BroadcastFeedMessage`s; everything under "execution + storage" tunes the single consumer
of that channel. That channel is the spine of the node, and
[The Big Picture](ch02-00-the-big-picture.md) traces it end to end.

## Choosing message sources

There are three producers, and they are not mutually exclusive:

- **`--l1-rpc`** starts trustless L1 derivation. This is the only source that can sync
  history, and it is the authoritative one. Covered in
  [L1 Derivation](ch03-02-l1-derivation.md).
- **`--feed-url`** connects to a sequencer relay. A relay is a *tip* source with a bounded
  backlog, not a history source, so it cannot sync a chain from scratch. Covered in
  [The Sequencer Feed](ch03-01-sequencer-feed.md).
- **`--replay-feed <NDJSON>`** pushes a file of newline-delimited `BroadcastFeedMessage`
  JSON objects into the channel, then holds the node up for RPC inspection. Useful for
  deterministic testing.

Running `--l1-rpc` and `--feed-url` together is the intended production shape: derivation
fills the confirmed prefix while the feed rides the tip, and the driver reconciles the two
by message sequence number so nothing is applied twice. See
[Reconciling Two Sources](ch03-07-reconciling.md).

`--no-l1-derive` makes the feed the sole producer. It still needs `--l1-rpc` to bootstrap
chain information at startup, and it is only appropriate for a datadir already inside the
relay's retained range.

## Defaults worth knowing

| Flag | Default | Notes |
| --- | --- | --- |
| `--chain-id` | `42161` | Arbitrum One. |
| `--persistence-threshold` | `2` | Blocks of canonical lead before a persistence batch. |
| `--memory-buffer-target` | `0` | Recent blocks held in memory before flushing. |
| `--persistence-backpressure` | `16` | Unpersisted blocks before production stalls. |
| `--engine.cross-block-cache-size` | `256` MiB | reth's generic default is 4 GiB, needlessly sparse for a serial producer. |
| `--share-execution-cache-with-payload-builder` | `true` | Safe here: only one payload builds at a time. |
| `--share-sparse-trie-with-payload-builder` | `false` | Opt-in; overlaps state-root work with execution. |
| `--l1-prefetch` | `6` | Concurrent L1 window resolutions during catch-up. |
| `--http.addr` / `--http.port` | `127.0.0.1` / `8545` | |

The three persistence defaults are reth's stock values, which suit a live node at the tip.
Bulk historical catch-up wants much deeper batching — see
[Observability and Tuning](ch07-00-observability.md).

> **`--no-fsync` is a loaded gun.** It opens MDBX in `SafeNoSync`, dropping the per-commit
> fsync during bulk sync. It stays crash-consistent in the sense that MDBX rolls back to
> the last synced meta page, and derivation can reproduce the lost suffix — but only if
> your resume checkpoint and datadir survive intact. Do not reach for it casually.
