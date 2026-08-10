# Getting Started

Before dissecting the node, it helps to have one running — or at least to know what the
command line looks like, because the flags map almost one-to-one onto the subsystems the
rest of the book describes.

This chapter covers building the binary and the shape of a `node` invocation. It is
deliberately brief; the exhaustive flag reference is `arb-reth <command> --help` and the
operator guides under [`docs/`](https://github.com/nicky-LV/arbitrum-reth/tree/main/docs).

`arb-reth` is a single binary with five subcommands:

| Command | Purpose |
| --- | --- |
| `node` | Run a node: derive from L1, follow a sequencer feed, serve RPC. |
| `snapshot import` / `snapshot read` | Convert and inspect a Nitro state export. |
| `genesis verify` / `genesis verify-export` | Verify a genesis or exported state root. |
| `rewind` | Remove a local block suffix after a confirmed divergence. |
| `dump-blocks` | Print headers, transaction hashes, and receipt status from a datadir. |

Dispatch lives in `crates/arb-reth-node/src/bin/arb-reth.rs`; each subcommand's
implementation is a module under `crates/arb-reth-node/src/commands/`.
