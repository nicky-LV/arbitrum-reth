# Installation

## Building

The workspace pins a Rust toolchain (`rust-version = "1.95"`, edition 2024) and depends on
specific git revisions of `arbitrum-revm` and `arbitrum-alloy`, recorded in the root
`Cargo.toml` under `[workspace.dependencies]`. Pinning is deliberate: `arb-revm` is the
ArbOS state-transition implementation, and a revision bump can change consensus behaviour.

```sh
cargo build --release -p arbitrum-reth-node --bin arbitrum-reth
./target/release/arbitrum-reth --help
```

The build compiles a Stylus runtime (a vendored Wasmer) as part of `arb-revm`'s `stylus`
feature, so the first build is slow and memory-hungry.

## Build profiles

The root `Cargo.toml` defines several profiles beyond cargo's defaults:

| Profile | Use |
| --- | --- |
| `release` | Default: `opt-level = 3`, thin LTO, stripped. |
| `maxperf` | Fat LTO, `codegen-units = 1`. Slowest build, fastest binary. |
| `profiling` | `release` plus full debug info and no stripping. |
| `hivetests` | Optimised but keeps debug assertions and overflow checks. |
| `reproducible` | `panic = "abort"`, no incremental, single codegen unit. |

Use `--profile profiling` when you intend to attach a profiler; symbol-stripped `release`
binaries produce useless flamegraphs.

## Checks

```sh
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Some integration tests are `#[ignore]`d because they need a live L1 endpoint or hours of
CPU — for example the launcher's deep-persistence soak test in
`crates/arb-reth-node/src/launcher.rs`. They are opt-in via `cargo test -- --ignored`.

## What you need to run a node

Deriving a chain from L1 requires:

- **An archive-capable L1 execution RPC.** Derivation issues historical `eth_getLogs` over
  wide ranges and fetches old transactions by hash. A pruned endpoint cannot serve this.
- **A beacon (consensus-layer) REST endpoint**, if the L2 range you are deriving includes
  post-Dencun blob batches. Standard beacon nodes retain blob sidecars for roughly 18 days,
  so deep historical sync needs a blob-archive provider. See [Blobs](ch03-04-blobs.md).
- **A boot source**: either an imported Nitro snapshot, or a genesis definition for a fresh
  chain. See [Bootstrapping a Chain](ch06-00-bootstrapping.md).

Following only the live sequencer relay (`--feed-url` with `--no-l1-derive`) still needs
`--l1-rpc` at startup to bootstrap chain information, and only works on a datadir already
at the relay's retained range.
