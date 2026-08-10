# Serving RPC

`--http` starts the RPC server. There is no bespoke JSON-RPC implementation here:
`arbitrum-reth` serves through reth's canonical `RpcAddOns`, swapping in exactly two
Arbitrum-specific pieces.

## Wiring

`crates/arb-reth-node/src/addons.rs` defines the add-on set, and the launcher runs it:

```rust,ignore
let add_ons_ctx = AddOnsContext {
    node: ctx.node_adapter().clone(),
    config: ctx.node_config(),
    beacon_engine_handle,          // a stub — see below
    engine_events: EventSender::default(),
    jwt_secret: ctx.auth_jwt_secret()?,
};
let handle = crate::addons::arb_add_ons().launch_add_ons(add_ons_ctx).await?;
```

Enabling `--http` turns on the full module fleet plus WebSocket and subscriptions, because
reth's RPC configuration lives on `NodeConfig.rpc` rather than as a per-module argument:

```rust,ignore
config.rpc.ws = true;
config.rpc.ws_addr = addr.ip();
config.rpc.ws_port = addr.port();
config.rpc.ws_api = Some(RpcModuleSelection::All);
```

So you get `eth`, `trace`, `debug`, `net`, `web3`, and `txpool` over both HTTP and WS.
(`txpool` is present but empty — there is no pool.)

## The receipt converter

The one place Arbitrum's RPC differs materially from Ethereum's is the receipt: it carries
`gasUsedForL1`.

`ArbReceiptConverter` and `ArbRpcConverter` in `crates/arb-reth-rpc/src/lib.rs` convert
`ArbReceiptEnvelope<Log>` into `ArbTransactionReceipt`. `ArbEthApiBuilder` swaps the
converter into reth's preconfigured eth-api builder and changes nothing else:

```rust,ignore
async fn build_eth_api(self, ctx: EthApiCtx<'_, N>) -> eyre::Result<Self::EthApi> {
    let provider = ctx.components.provider().clone();
    let converter = ArbRpcConverter::new(ArbReceiptConverter::new(provider));
    Ok(ctx.eth_api_builder().with_rpc_converter(converter).build())
}
```

The converter is constructed with a provider because reth's builder passes one, but it never
reads it. `gas_used_for_l1` was stored on the receipt at execution time (see
[ArbOS Execution](ch04-03-arbos-execution.md)), so there is no L1-fee hardfork math to do at
serve time — a genuine simplification relative to op-reth's equivalent, where the L1 fee must
be reconstructed from the block's L1 attributes.

## No engine API

The node is self-driven from its message sources. Nothing external should ever be able to
tell it what the head is.

So the engine API builder is reth's `NoopEngineApiBuilder`, the auth/engine server is
disabled, and the `beacon_engine_handle` passed into the add-ons context is a stub built
around a channel whose receiver is immediately dropped. Any `engine_*` call would return
`EngineUnavailable` — and nothing can reach it in the first place, because the auth server
is not listening.

`ArbPayloadValidatorBuilder` exists purely to satisfy the `RpcAddOns` payload-validator
type bound. It is never invoked.

## Reading across memory and disk

The provider handed to the RPC stack is the same `BlockchainProvider` the engine tree
updates. The launcher is explicit about this:

```rust,ignore
// Clone the in-memory state from the provider so the tree updates the same instance that
// BlockchainProvider serves for RPC queries.
let canonical = provider.canonical_in_memory_state();
```

That shared `CanonicalInMemoryState` is what makes a freshly produced block queryable before
it reaches MDBX. A query resolves against the in-memory overlay first and falls through to
the database, so clients see a single continuous chain.

The driver's `settle_pending_applied` check ties into this directly: it does not consider a
block applied until either the provider reports its hash at that height or the canonical
in-memory head matches. In other words, "applied" is defined as *RPC can see it*.

## Reading a snapshot database directly

There is one shape the normal provider path cannot read. A datadir produced by
`snapshot import` contains only *hashed* state tables — `HashedAccounts`, `HashedStorages`,
`Bytecodes`, and the trie tables — with no `PlainAccountState` or `PlainStorageState` rows.

`crates/arb-reth-node/src/hashed_db.rs` provides free helpers for that case:

```rust,ignore
account_by_address(tx, address)   // keccak the address, read HashedAccounts
code_of(tx, code_hash)            // read Bytecodes
storage_at(tx, address, slot)     // keccak both, dup-cursor into HashedStorages
```

These back the `snapshot read` command, which is how you inspect an imported snapshot before
booting a node on it. See [Bootstrapping a Chain](ch06-00-bootstrapping.md).
