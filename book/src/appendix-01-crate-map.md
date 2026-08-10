# Appendix A: Crate Map

Eight crates under `crates/`, roughly 20k lines of Rust. The dependency direction runs from
pure decoders at the bottom to the node binary at the top.

```text
                       arb-reth-node          (binary, CLI, launcher, genesis, RPC add-ons)
                       ┌──────┴───────┐
              arb-reth-sync      arb-reth-rpc  (catch-up runtime)   (receipt converters)
                    │
              arb-reth-l1                      (L1 fetch: inbox, delayed, beacon)
                    │
            arb-reth-derive                    (pure decode: batch, blob, multiplexer)

              arb-reth-engine                  (engine-tree driver, payload builder, storage v2)
                    │
               arb-reth-evm                    (ConfigureEvm, BlockExecutor, BlockAssembler)
                    │
                 arb-revm                      (external: ArbOS state transition)

            arb-reth-genesis                   (Nitro state export → reth genesis)
```

## `arb-reth-derive`

Pure, offline decode. No network I/O at all, which is what makes it testable against
fixtures.

| Module | Contents |
| --- | --- |
| `batch` | 40-byte timeBounds header, header flags, brotli, RLP segments, `parse_sequencer_batch_delivered`. |
| `multiplexer` | `extract_messages`: walk segments → `DerivedMessage`s. |
| `blob` | EIP-4844 field-element decode (`DecodeBlobs` port). |
| `delayed` | `DelayedMessage`, accumulator hashing, the `DelayedSource` trait. |
| `l2message` | `parse_l2_message`: L2 message → signed transaction encodings. |
| `message` | `DerivedMessage`, `L1IncomingMessageHeader`, `BATCH_POSTER_ADDRESS`. |

## `arb-reth-l1`

The fetch layer over an L1 RPC. Turns chain data into decoder input.

| Module | Contents |
| --- | --- |
| `reader` | `SequencerInboxReader`, `DeliveredBatch`, `BatchPayload`. |
| `delayed` | `DelayedInboxReader`, `verify_accumulator_chain`. |
| `beacon` | `BeaconClient`: slot math, sidecar fetch, versioned-hash matching. |
| `contracts` | ABI surface and mainnet addresses. |
| `sync` | `derive_range`, `resolve_batches`, delayed coverage, caches. |
| `assemble` | Batches + delayed → numbered `BroadcastFeedMessage` stream. |
| `feed` | `DerivedMessage` → `BroadcastFeedMessage`. |
| `batch_serialize` | `batch_data_hash`, `batch_data_stats` for posting reports. |

## `arb-reth-sync`

| Module | Contents |
| --- | --- |
| `l1_sync` | `run_l1_sync`, `supervise_l1_sync`, `L1SyncConfig`, `L1SyncError`. |
| `resume` | `L1ResumeLog`, `L1ResumeCheckpoint` — the `arb-l1-resume.json` log. |

## `arb-reth-evm`

Bridges `arb-revm` into reth's EVM and block-execution extension points. Mirrors
`alloy-op-evm`.

| Module | Contents |
| --- | --- |
| `lib` | `ArbEvm`, `ArbEvmFactory`. |
| `block` | `ArbBlockExecutor`, `ArbBlockExecutorFactory`, `ArbBlockAssembler`, `ArbBlockExecutionCtx`. |
| `config` | `ArbEvmConfig` (`ConfigureEvm`), `ArbNextBlockEnvAttributes`. |
| `tx` | `ArbTx` newtype over `ArbTransaction<TxEnv>`. |
| `precompiles` | `ArbPrecompilesMap`, version-gated ArbOS precompiles. |
| `rpc` | (feature `rpc`) `TryIntoTxEnv` for `ArbTransactionRequest`. |

## `arb-reth-engine`

| Module | Contents |
| --- | --- |
| `engine` | `ArbEngineDriver`, `produce_with_timing`, `ArbEngineTuning`, `wait_for_head`. |
| `native_payload` | `ArbPayloadBuilder`, `ArbPayloadJobGenerator`. |
| `storage_v2` | Persistence proxy adding slot preimages. |
| `engine_spike` | `ArbPayloadValidator`. |
| `lib` | `ArbPayloadTypes`, `ArbPayloadAttributes`, `ArbBuiltPayload`, `ArbExecutionData`. |

## `arb-reth-node`

The binary and everything that wires the rest together.

| Module | Contents |
| --- | --- |
| `bin/arb-reth` | CLI dispatch. |
| `commands/node` | `NodeArgs`, boot-mode resolution, feed follower, derivation startup. |
| `commands/snapshot` | Import, read, build-preimages, repair-history. |
| `commands/genesis` | `verify`, `verify-export`. |
| `commands/rewind` | Unwind to an earlier L2 block. |
| `commands/dump_blocks` | Inspect persisted blocks. |
| `launcher` | `ArbLauncher` (custom `LaunchNode`), the driver loop. |
| `genesis` | ArbOS genesis chain-spec builder, chain-info / Nitro genesis parsing. |
| `addons` | `ArbEthApiBuilder`, `ArbPayloadValidatorBuilder`, RPC add-on set. |
| `executor` | `ArbExecutorBuilder` — the one non-noop component. |
| `persist` | `persist_executed_block`. |
| `hashed_db` | Direct hashed-table reads for snapshot databases. |
| `metrics` | `FeedLatencyTracker`. |
| `pooled` | `ArbPooledTransaction` (needed only to parameterise the noop pool). |
| `lib` | `ArbNode`: the `NodeTypes` / `Node` impls. |

## `arb-reth-rpc`

`ArbReceiptConverter` and `ArbRpcConverter` — receipt conversion surfacing
`gas_used_for_l1`.

## `arb-reth-genesis`

Converts a Nitro classic-state export into a reth-readable genesis, reproducing Nitro's
`InitializeArbosInDatabase` via `arb_revm::arbos_init` and verifying the state root.

## External dependencies

| Crate | Role |
| --- | --- |
| `arb-revm` | The ArbOS state transition. No reth dependency; pinned by git revision. |
| `arbitrum-alloy` | Arbitrum consensus types, sequencer feed types, RPC types, network. |
| `revm` | Pinned to 42.0.1 to unify with `arb-revm`. |
| `reth-*` | Provider stack, engine tree, RPC, MDBX, tries, pruning. |

## `ArbNode`'s type parameters

The four associated types that let reth's generic infrastructure instantiate for Arbitrum
(`crates/arb-reth-node/src/lib.rs`):

```rust,ignore
impl NodeTypes for ArbNode {
    type Primitives = ArbPrimitives;                  // arb-alloy tx/receipt/block types
    type ChainSpec  = reth_chainspec::ChainSpec;      // reth's stock spec
    type Storage    = EthStorage<ArbTxEnvelope, Header>;
    type Payload    = ArbPayloadTypes;                // stub; satisfies PayloadTypes
}
```

and the components, four of which are noops:

```rust,ignore
ComponentsBuilder::default()
    .node_types::<N>()
    .executor(ArbExecutorBuilder)          // the only real one
    .noop_pool::<ArbPooledTransaction>()
    .noop_payload()
    .noop_network::<ArbNetworkPrimitives>()
    .noop_consensus()
```
