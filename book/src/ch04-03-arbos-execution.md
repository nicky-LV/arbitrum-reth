# ArbOS Execution

The state transition itself lives in a separate repository, `arbitrum-revm`, which has no
reth dependency. `arb-reth-evm` is the adapter that presents it through reth's extension
points. This chapter covers the adapter and the block-level execution sequence.

## The layers

```text
reth                    alloy-evm                     arb-reth-evm            arb-revm
────                    ─────────                     ────────────            ────────
ConfigureEvm      ◀──   —                             ArbEvmConfig
BlockExecutorFactory ◀─ BlockExecutorFactory     ◀──   ArbBlockExecutorFactory
                        BlockExecutor            ◀──   ArbBlockExecutor   ──▶  execute_message
                        BlockAssembler           ◀──   ArbBlockAssembler       hooks
                        EvmFactory               ◀──   ArbEvmFactory
                        Evm                      ◀──   ArbEvm             ──▶  ArbEvm/ArbHandler
```

The mapping mirrors `alloy-op-evm`'s `OpEvm` / `OpBlockExecutor` almost line for line, which
is deliberate — it makes the Arbitrum layer reviewable against a known-good analogue.

`ArbEvm` is thin: it owns an `arb_revm::ArbEvm` over an `ArbContext<DB>` with the Arbitrum
precompile set, and `transact_raw` dispatches to `inspect_tx` or `transact` depending on
whether an inspector is enabled. Both route through `ArbHandler`, which is where the real
Arbitrum semantics live: ArbOS gas charging, poster fee, the `NUMBER` override, and Arbitrum
precompiles.

## The block sequence

Executing one block runs this sequence:

```text
1. apply_pre_execution_changes
      EIP-2935 history-storage system call (parent hash), from SYSTEM_ADDRESS
2. InternalTxStartBlock (0x6a)          ← a real transaction, with a receipt
3. loop:
      scheduled retry (FIFO)  if any
      else next user tx       if any
      else break
4. finish
      read post-execution ArbOS header info into the shared cell
```

### 1. The EIP-2935 prelude

`apply_pre_execution_changes` writes the parent block hash into the history-storage contract
under `SYSTEM_ADDRESS`, mirroring Nitro's `ProcessParentBlockHash`. On chains before ArbOS
v40 this is a no-op state transition — which matches `execute_message`, so it is performed
unconditionally rather than version-gated here.

### 2. The start-block transaction

Every Arbitrum block begins with Nitro's `InternalTxStartBlock`, transaction type `0x6a`,
carrying calldata for:

```text
ArbosActs.startBlock(l1BaseFee, l1BlockNumber, l2BlockNumber, timeLastBlock)
```

It updates the L2 pricing model, records L1 block advancement into ArbOS's `Blockhashes`
state, and applies any scheduled ArbOS version upgrade.

> **It is not run in `apply_pre_execution_changes`.** It is a real block transaction with its
> own receipt, so the caller drives it through `execute_transaction` like any other. Running
> it as a pre-execution change would exclude it from the transactions and receipts roots and
> diverge from Nitro. The comment in `crates/arb-reth-evm/src/block.rs` says so explicitly.

This is also the only transaction that *must* succeed. A failure here aborts block
production with "internal start-block tx failed".

### 3. Transaction ordering

Ordering mirrors Nitro's `arbos/block_processor.go`:

```text
start-block internal tx
then, each iteration:
    any scheduled redeem (FIFO)   ← drained before the next user tx
    else the next sequenced user tx
```

A user transaction that calls `redeem()` schedules an `ArbitrumRetryTx`, which Nitro runs
*immediately after it*. After each successful transaction the executor scans its logs for
redeem-scheduled events and pushes the resulting retries onto the front queue.

> **Why this ordering is consensus-critical.** Appending scheduled retries to the back of a
> single queue — running them after all remaining user transactions — does not change
> execution, state, or gas: the transactions are independent. It changes only the *order* of
> the block's transaction list. That diverges `transactionsRoot` and `receiptsRoot`, and
> therefore the block hash, from Nitro. A state-root parity check would not catch it. The
> wrong hash stays invisible until a later L1-advancing block bakes it into ArbOS state via
> `record_new_l1_block`.

### Invalid transactions are dropped, not fatal

A derived transaction that is *invalid* under the state transition — lack of funds, nonce
too high; a validation failure, not a revert — is dropped and block production continues
without it. This mirrors Nitro (`block_processor.go`), and it is real on mainnet: an
unsigned or contract transaction from the delayed inbox whose sender cannot pay yields a
block containing only the internal start-block transaction.

revm rejects such a transaction before applying it, so nothing enters the block. The
executor logs at `debug` and moves on. Only the internal transaction is exempt.

### 4. `finish`

After all transactions commit, the executor reads post-execution ArbOS state — the
send-Merkle root and count, the ArbOS version, and the L1 block number ArbOS recorded — into
an `ArbBlockHeaderInfo`, and stores it in a shared cell for the assembler.

That shared cell is the only channel from executor to assembler, because reth's
`BasicBlockBuilder` hands the assembler the *builder's* context, not the executor's EVM. It
works because `create_executor` receives a clone of the context and the field is an
`Arc<Mutex<…>>`, so both sides see the same cell.

## Sender recovery

Sender recovery is pure per-transaction work, so the sequenced user transactions are
recovered **in parallel with rayon** before the execution loop starts, rather than one
`ecrecover` at a time inside it.

Results — including failures — are carried per transaction so the loop still reports the
same first-in-order error it would have hit serially. Retries scheduled mid-block and the
internal start-block transaction carry their sender in the envelope and stay on the cheap
inline path.

## Receipts and `gas_used_for_l1`

Arbitrum receipts carry an extra field: `gasUsedForL1`, the L2-gas equivalent of the L1
posting cost attributed to that transaction.

It is not computed in the receipt builder. The ArbOS handler sets `chain().poster_gas`
during pre-execution, and the executor reads it straight off the chain context:

```rust,ignore
let gas_used_for_l1 = self.evm.ctx().chain.poster_gas;
```

That value is stored on the receipt at execution time, which is why the RPC converter in
[Serving RPC](ch05-02-rpc.md) needs no L1-fee hardfork math — unlike op-reth's equivalent.

## Metrics sampling

The executor exports per-transaction-family histograms — execution time, commit time, L2 gas,
L1 poster gas — labelled by a bounded set of families (`legacy`, `eip1559`, `deposit`,
`unsigned`, `contract`, `retry`, `submit_retryable`, `internal`). Labels deliberately exclude
addresses, hashes, block numbers, and ArbOS version to keep Prometheus cardinality bounded.

`ARB_EXECUTION_METRICS_SAMPLE_RATE` controls sampling of the detailed histograms: `1` (the
default) records every transaction, `16` records one of each type per 16-transaction window
and is the recommended throughput setting, `0` disables them. Block-level metrics, MGas/s,
state root, persistence, and feed-to-canonical are never affected by this setting.
