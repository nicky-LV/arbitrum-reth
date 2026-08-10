# From Message to Block Environment

Before ArbOS can execute anything, the message has to become a block environment: a
timestamp, a coinbase, a gas limit, an L1 block number, an ArbOS version, and a list of
transactions. This chapter covers that translation.

## `ArbPayloadAttributes`

The driver wraps the message almost unchanged:

```rust,ignore
pub struct ArbPayloadAttributes {
    /// max(message L1 timestamp, parent timestamp)
    pub timestamp: u64,
    /// The ordered message ArbOS expands into the block's transactions.
    pub message: BroadcastFeedMessage,
}
```

Two things are worth noting.

**The timestamp is a max, not a copy.** Arbitrum's next-block timestamp is
`max(message.l1_timestamp, parent.timestamp)`. L1 timestamps are not strictly increasing
relative to L2 block times, and a block whose timestamp went backwards would be invalid.

**Payload ids must not alias.** reth derives a payload id by hashing the parent hash and the
attributes. Two Arbitrum messages with equal timestamps would produce the same id and
collide as build jobs. `ArbPayloadAttributes::payload_id` therefore feeds the message
sequence number through the `slot_number` field, which makes every id distinct. There is a
unit test for exactly this in `crates/arb-reth-engine/src/lib.rs`.

## `digest_message`

The real translation happens in `arb_revm::executor::digest_message`, called from
`produce_with_timing` (`crates/arb-reth-engine/src/engine.rs`). It takes:

- the `BroadcastFeedMessage`,
- an `ArbParentHeader` (number, timestamp, beneficiary, basefee, gas limit, difficulty,
  prevrandao),
- an `ArbExecCfg` carrying the chain id,
- and the **ArbOS version**,

and returns an `ArbExecutionInput`: the parsed message plus the ordered transaction list
ArbOS will execute.

This is the single validated message→execution entry point mentioned throughout chapter 3.
It is why L1 derivation converts back into the feed representation instead of feeding a
binary type directly.

## Resolving the ArbOS version

The version is not carried on the message. It is read from the **parent header**:

```rust,ignore
let arbos_version = ArbHeaderInfo::decode_header(parent_header)
    .ok()
    .map(|i| i.arbos_format_version as u8);
let version = arbos_version.unwrap_or(0);
```

`ArbHeaderInfo` is packed into the header's `extra_data` and `mix_hash` (see
[Sealing the Block](ch04-04-sealing.md)). Reading it from the parent means the version
advances naturally as the chain does: an ArbOS upgrade is applied by a block's start-block
internal transaction, that block's header records the new version, and the next block's
message is parsed under it. No component needs to track upgrade schedules.

## `ArbNextBlockEnvAttributes`

`digest_message`'s output is converted into the attributes reth's `ConfigureEvm` consumes:

```rust,ignore
ArbNextBlockEnvAttributes {
    timestamp: input.message.l1_timestamp.max(parent.timestamp),
    suggested_fee_recipient: input.message.poster,   // block coinbase
    prev_randao: B256::ZERO,
    gas_limit: input.cfg.block_gas_limit,
    l1_block_number: input.message.l1_block_number,
    l1_base_fee_wei: input.message.l1_base_fee_wei,
    arbos_format_version: version as u64,
    delayed_messages_read: input.message.delayed_messages_read,
    extra_data: Bytes::default(),
    withdrawals: None,
    finish_timing_out: …,
}
```

`ArbEvmConfig` (`crates/arb-reth-evm/src/config.rs`) turns this into an `EvmEnv` plus an
`ArbBlockExecutionCtx`. It needs only the chain id as configuration — unlike Optimism, whose
spec is keyed by a timestamp-based chain spec, Arbitrum's spec comes from the header itself.

## `NUMBER` returns the L1 block number

This is the Arbitrum quirk most likely to surprise someone reading the EVM layer. On
Arbitrum, the `NUMBER` opcode returns the **L1** block number, not the L2 one.

`arb_revm` implements this by overriding `opNumber` to read `chain().l1_block_number`, and
the plumbing to get the right value there runs through three places:

```text
ArbHeaderInfo (parent header)
    ──▶ ArbEvmConfig::evm_env / context_for_block
        ──▶ ArbBlockExecutionCtx::l1_block_number
            ──▶ ArbBlockExecutorFactory::create_executor threads it into ArbChainContext
```

Any executor built through `ArbEvmConfig` sees the correct L1 block number. One built by
hand from `ArbEvmFactory` with a default chain context does not — which is fine for a bare
value transfer that never reads `NUMBER`, and is exactly the caveat the factory's doc
comment records.

## The two state providers

`produce_with_timing` takes **two** independent state providers over the parent:

```rust,ignore
produce_with_timing(
    evm_config, chain_id, parent, feed_msg,
    exec_state_provider,   // reads during execution
    trie_state_provider,   // reads during trie construction
    state_root_task,
)
```

They must be separate instances. Sharing one would let execution reads and trie-build reads
interfere, corrupting one or the other. The comment in the source says exactly this, and it
is the kind of constraint that is invisible until it produces a wrong state root.

## Base fee comes from state, not the parent header

One correction happens after the environment is built and before the first transaction.

ArbOS stores `L2PricingState.BaseFeeWei` as the fee for the *next* block: each block's
start-block transaction computes and stores its successor's fee. So the current block's base
fee is the value sitting in state at block start — what the parent's update produced.

The block env, however, was seeded from the parent header's base fee, which is the fee from
*two* blocks back. Those agree only while the fee sits at the `minBaseFee` floor. So
`produce_with_timing` reads the value out of ArbOS state and overwrites the block env before
executing:

```rust,ignore
let block_base_fee = ArbosState::open()
    .l2_pricing
    .base_fee_wei
    .get(builder.evm_mut().ctx_mut().journal_mut())?;
builder.evm_mut().ctx_mut().modify_block(|b| b.basefee = block_base_fee);
```

This must happen *before* the start-block transaction runs, since that transaction
overwrites the stored value with the next block's fee. Getting it wrong only matters once
the gas backlog pushes the fee off its floor — so, again, a bug that a quiet testnet never
reveals.
