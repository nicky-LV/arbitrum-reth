# Execute to Derive

This is the single most important idea in the codebase. It explains the architecture of
the node, why reth's sync pipeline is unused, and why the block-production path looks like
a payload builder rather than a syncing client.

## How an Ethereum client syncs

reth's staged pipeline is *download-then-execute*:

```text
HeaderStage ──▶ BodyStage ──▶ ExecutionStage ──▶ MerkleStage
 download        download       execute the       compute the state root and
 headers         bodies         stored body       CHECK it against the header
```

The header arrives from a peer with a `state_root` already in it. Execution is a
*verification* step: you replay the body and confirm you arrive at the root the header
claims. If you don't, the peer lied, and you reject the block.

That whole model presupposes a header you downloaded from someone.

## Arbitrum has no such header

On Arbitrum there is no peer to download a header from, and no header to check. The input
is a *sequencer message*; the block — including its state root, its transaction list, its
receipts root, and therefore its hash — is the **output** of executing that message.

```text
message ──▶ [ArbOS state transition] ──▶ transactions, receipts, state root ──▶ header
```

You cannot verify a header before executing, because there is no header before executing.
You mint it.

This is what "execute to derive" means, and it is stated at the top of
`crates/arb-reth-node/src/lib.rs`:

> An Arbitrum node has no such header. It is execute-to-derive: a sequencer message is the
> input, and the block (including its state root) is the output of executing that message.
> We mint it.

## What the node does instead

Since minting a block is exactly what an Ethereum client does when it *builds* a block
locally, `arbitrum-reth` follows reth's payload path rather than its sync path:

```text
produce ──▶ execute ──▶ compute state root ──▶ seal header ──▶ persist executed block
```

Concretely, per message:

1. `ArbEngineDriver` sends a `ForkchoiceUpdated` carrying `ArbPayloadAttributes` — a
   wrapper around the message itself — to reth's engine tree.
2. The engine tree launches a payload job. `ArbPayloadBuilder` executes ArbOS exactly once
   over the parent state.
3. The resulting `BuiltPayloadExecutedBlock` goes back to the tree as
   `InsertExecutedBlock` — the executed block, not a payload to re-execute.
4. A second `ForkchoiceUpdated` names the new block as head, making it canonical.

Steps 3 and 4 are why this is *cheap*: the block is executed once, on the producer side,
and the tree accepts the executed result verbatim. There is no re-execution, and no
consensus check to satisfy — the node's consensus is `NoopConsensus`.

## Where the safety check went

Dropping header verification does not mean dropping correctness checks. The equivalent
guarantee shows up in two other places:

- **State-root parity.** `persist_executed_block` in
  `crates/arb-reth-node/src/persist.rs` recomputes the root from the trie against the
  parent state and errors if it disagrees with the root baked into the header. That is the
  same equation `MerkleStage` checks, moved to the write side.
- **External comparison.** Because the block hash is fully determined by the message
  stream, a node's hash at a given height can be compared against a canonical public RPC.
  That is how the benchmark run in the repository README was validated.

## A consequence worth internalising

Because a wrong block is still a *self-consistent* block, divergence is silent until
something compares you to the rest of the world. Worse, an incorrect block hash can hide
for a while even from a state-root check: `arb-reth`'s own comments note that a
mis-ordered transaction list produces the right state but the wrong
`transactionsRoot`/`receiptsRoot`, and the resulting bad parent hash only becomes visible
once a later L1-advancing block bakes it into ArbOS state via `record_new_l1_block`
(`crates/arb-reth-engine/src/engine.rs`).

This is why transaction ordering inside a block is treated as consensus-critical in
[ArbOS Execution](ch04-03-arbos-execution.md), and why the `rewind` command exists
(see [Recovering from Trouble](ch08-00-recovery.md)).
