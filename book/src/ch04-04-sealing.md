# Sealing the Block

Execution has produced receipts and a state delta. Turning that into a sealed block means
computing the state root and assembling the header — and on Arbitrum, the header carries
metadata no Ethereum header does.

## The state root

reth's generic block builder owns post-state hashing and state-root computation:

```text
BundleState ──▶ HashedPostState ──▶ state_root_with_updates(parent) ──▶ (root, TrieUpdates)
```

`arb-reth-engine` wraps the builder's provider so those two phases can be timed
individually — `finish_hashed_state` and `finish_state_root` — without changing consensus
behaviour.

### The sparse trie option

`--share-sparse-trie-with-payload-builder` lets the engine tree hand the builder a
`PayloadStateRootHandle`: a background task that consumes state updates *as execution
produces them* and computes the root concurrently, rather than after the fact.

The builder feeds it via a state hook installed on the EVM database. When execution
finishes, dropping the hook signals that the task has seen every transition — including the
EIP-2935 prelude and the start-block transaction.

It is **opt-in**, and there are two reasons.

The first is ownership: reth's conservative default assumes a node may build payloads in
parallel, where sharing the sparse trie task would race. The Arbitrum driver builds exactly
one payload at a time, so it is safe here.

The second is correctness, and it is more interesting. The pinned sparse converter cannot
faithfully represent two situations:

- **storage wipes** (a `SELFDESTRUCT`), and
- **created-empty accounts**, on chains before ArbOS 30.

So the builder watches for them. A state hook accumulates hazard flags:

```rust,ignore
fn sparse_root_hazards(state: &EvmState, preserve_created_empty_accounts: bool) -> u8 {
    // SPARSE_HAZARD_SELFDESTRUCT   if any account is_selfdestructed()
    // SPARSE_HAZARD_CREATED_EMPTY  if any account is_created() && is_empty()
}
```

If either fires, the builder waits for the task to release its proof workers and preserved
trie cache, **discards its result**, and lets `finish` compute the authoritative root from
the merged bundle state. The fallback is counted:
`state_root_task_native_success` versus `state_root_task_fallback`. Watching that ratio is
how you tell whether the option is actually helping on your workload.

## Header assembly

`ArbBlockAssembler` (`crates/arb-reth-evm/src/block.rs`) builds the header. Most fields are
ordinary — `transactions_root`, `receipts_root`, `logs_bloom`, `gas_used`, `state_root`.
Four are Arbitrum-specific:

| Field | Value |
| --- | --- |
| `difficulty` | Always `1`. Nitro sets L2 block difficulty to 1 unconditionally. |
| `nonce` | `delayed_messages_read` big-endian — Nitro's `EncodeNonce`. |
| `mix_hash` | `ArbHeaderInfo::encode_mix_hash()` — send count, L1 block number, ArbOS version. |
| `extra_data` | `ArbHeaderInfo::encode_extra_data()` — the send-Merkle root. |

Several standard fields are explicitly `None`: `withdrawals_root`,
`parent_beacon_block_root`, `blob_gas_used`, `excess_blob_gas`, `requests_hash`,
`block_access_list_hash` (EIP-7928 is not used on Arbitrum), `slot_number`.

### `ArbHeaderInfo`

This is the load-bearing structure. It is what makes the ArbOS version readable from the
chain itself, which is what lets [From Message to Block
Environment](ch04-02-payload-attributes.md) resolve the version from the parent header:

```rust,ignore
ArbHeaderInfo {
    send_root: B256,            // ArbOS outbox Merkle root
    send_count: u64,
    l1_block_number: u64,       // as ArbOS recorded it post-execution
    arbos_format_version: u64,
    collect_tips: bool,
}
```

Note `l1_block_number` here is the value **ArbOS recorded during execution**, not the raw
`l1BlockNumber` from the message context. Those can differ, and Nitro packs the recorded one.

### `collect_tips`

One subtle rule:

```rust,ignore
let collect_tips = computed.collect_tips
    && evm_env.block_env.beneficiary() == BATCH_POSTER_ADDRESS;
```

Delayed-message blocks — where the coinbase is the real L1 sender rather than the virtual
batch-poster address — never collect tips, per Nitro's `block_processor.go`. All
transactions in a block share a coinbase, so this is decidable at assembly time.

## The result

The assembler returns a `Block<ArbTxEnvelope>`; the builder wraps it as a
`BuiltPayloadExecutedBlock<ArbPrimitives>` carrying the recovered block, the execution
output, and the computed trie data. That bundle is what goes to the engine tree — already
executed, already rooted, already sealed.

## The parity check

Before any of this is trusted on disk, `persist_executed_block`
(`crates/arb-reth-node/src/persist.rs`) recomputes the root independently from the trie
against the parent state and errors on mismatch:

```text
1. HashedPostState::from_bundle_state(bundle)
2. state_root_with_updates(hashed) against the latest committed state
3. assert computed == header.state_root
4. build ExecutedBlock with the precomputed ComputedTrieData
5. save_blocks(...) then commit()
```

Step 3 is the moved-to-the-write-side equivalent of `MerkleStage`'s check, as described in
[Execute to Derive](ch02-01-execute-to-derive.md). It is what stands between a subtly wrong
execution and a corrupted database.
