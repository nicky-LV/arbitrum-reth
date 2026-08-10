# The Big Picture

Here is the whole node on one page. Every box is expanded somewhere later in the book.

```text
                        ┌──────────────────────────────────────────────┐
   Ethereum L1          │            MESSAGE SOURCES                   │
   ┌──────────┐         │                                              │
   │Sequencer │──logs──▶│  arb-reth-l1  ──▶  arb-reth-derive           │
   │  Inbox   │         │  (fetch batches,   (decode framing,          │
   └──────────┘         │   blobs, delayed    segments, multiplexer)   │
   ┌──────────┐         │   inbox)                    │                │
   │  Bridge  │──logs──▶│                             ▼                │
   └──────────┘         │              arb-reth-sync::run_l1_sync      │
   ┌──────────┐         │                             │                │
   │  Beacon  │─blobs──▶│                             │                │
   └──────────┘         │                             │                │
                        │  Sequencer relay ───────────┤                │
   ┌──────────┐  ws     │  (--feed-url, tip only)     │                │
   │  Relay   │────────▶│                             │                │
   └──────────┘         │  Replay file ───────────────┤                │
                        │  (--replay-feed)            │                │
                        └─────────────────────────────┼────────────────┘
                                                      │
                                    mpsc::channel<BroadcastFeedMessage>(4096)
                                                      │
                        ┌─────────────────────────────▼────────────────┐
                        │              ArbEngineDriver                 │
                        │  sequence reconciliation (drop / apply /     │
                        │  buffer), then one block per message         │
                        └─────────────────────────────┬────────────────┘
                                                      │
                        ┌─────────────────────────────▼────────────────┐
                        │      reth engine tree + payload builder      │
                        │                                              │
                        │  ArbPayloadBuilder ──▶ arb-reth-evm ──▶      │
                        │                        arb-revm (ArbOS)      │
                        │                             │                │
                        │  execute ▸ state root ▸ seal header          │
                        │                             │                │
                        │  InsertExecutedBlock ▸ ForkchoiceUpdated     │
                        └──────────┬──────────────────┬────────────────┘
                                   │                  │
                    in-memory canonical state    persistence service
                                   │                  │
                                   ▼                  ▼
                        ┌──────────────────┐   ┌──────────────┐
                        │ BlockchainProvider│  │ MDBX + static│
                        │   (RPC reads)     │  │    files     │
                        └────────┬──────────┘   └──────────────┘
                                 │
                          eth_* JSON-RPC
```

## The three movements

**Ingest.** Something produces an ordered stream of `BroadcastFeedMessage`s. There are
three producers and they all write into one bounded channel. The interesting one is L1
derivation, which reconstructs that stream from nothing but Ethereum logs, calldata, and
blob sidecars.

**Apply.** A single consumer — `ArbEngineDriver` — drains the channel. For each message it
does not already have, it produces exactly one L2 block: build the block environment,
execute ArbOS, compute the state root, seal the header, and hand the executed block to
reth's engine tree.

**Serve.** The engine tree makes the block canonical in memory immediately and persists it
to MDBX asynchronously. `BlockchainProvider` reads across both layers, and reth's standard
RPC stack serves `eth_*` on top of it with Arbitrum-aware receipt conversion.

## One message, one block

The invariant that shapes everything: **each sequencer message produces exactly one L2
block.** There is no transaction pool doing selection, no fee-market competition among
builders, no reorg between competing heads. A message's position in the stream determines
its block number:

```text
L2 block number = message sequence number + genesis block number
```

`genesis_block` is the chain's Nitro-genesis anchor (`22_207_817` for Arbitrum One, `0` for
a fresh chain). This one equation shows up in the driver's dedup cursor, in the L1 sync
runtime's numbering, and in the feed follower's resume header. Getting it wrong on a chain
whose genesis is not block 0 makes every message land far above the driver's cursor, so it
buffers everything and produces nothing — a failure mode the code comments call out
explicitly in `crates/arb-reth-sync/src/l1_sync.rs`.

## What is *not* here

Several things a normal reth node has are deliberately absent:

- **No staged sync pipeline.** Chapter [Execute to Derive](ch02-01-execute-to-derive.md)
  explains why the download-then-execute model cannot apply.
- **No transaction pool.** `ArbNode`'s components builder wires
  `NoopTransactionPoolBuilder`. Arbitrum has no p2p transaction gossip; the sequencer
  decides ordering.
- **No p2p network.** `NoopNetworkBuilder`.
- **No consensus engine orchestrator, and no engine API.** The node is self-driven from its
  message sources, so the beacon-engine handle passed to the RPC add-ons is a stub and the
  engine API builder is reth's `NoopEngineApiBuilder`.
- **No payload builder service in the usual sense.** There is one, but it builds
  deterministic single-message payloads, not fee-maximising ones.

`ArbNode`'s `Node` impl in `crates/arb-reth-node/src/lib.rs` makes this concrete: of the
five component builders, four are noops and only the executor is real.
