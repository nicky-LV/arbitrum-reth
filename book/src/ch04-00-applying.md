# Applying Messages

One message in, one block out. This chapter follows a single `BroadcastFeedMessage` from
the moment it leaves the channel to the moment its block is the canonical head.

```text
  message
     │
     ▼
  ┌──────────────────────────────────────────────────────────────┐
  │ ArbEngineDriver                                              │
  │   reconcile against next_seq  ────────────▶ drop / buffer    │
  │                              (see ch. 3.7)                   │
  └──────────────────────────┬───────────────────────────────────┘
                             │ apply
  ┌──────────────────────────▼───────────────────────────────────┐
  │ ForkchoiceUpdated { head = parent, attrs = the message }     │
  │   ──▶ engine tree validates, creates optional sparse-trie    │
  │       task, returns a payload id                             │
  └──────────────────────────┬───────────────────────────────────┘
                             │
  ┌──────────────────────────▼───────────────────────────────────┐
  │ ArbPayloadBuilder::build (serial, one at a time)             │
  │   digest_message ──▶ block env ──▶ ArbBlockExecutor          │
  │     ├─ EIP-2935 parent-hash system call                      │
  │     ├─ InternalTxStartBlock (0x6a)                           │
  │     ├─ scheduled retries (FIFO) interleaved with user txs    │
  │     └─ finish: read ArbOS header info                        │
  │   ──▶ hashed post-state ──▶ state root ──▶ ArbBlockAssembler │
  └──────────────────────────┬───────────────────────────────────┘
                             │ BuiltPayloadExecutedBlock
  ┌──────────────────────────▼───────────────────────────────────┐
  │ InsertExecutedBlock   (no re-execution)                      │
  │ ForkchoiceUpdated { head = new block }                       │
  │   ──▶ canonical in memory; persistence happens in background │
  └──────────────────────────────────────────────────────────────┘
```

## Why it goes through the payload builder at all

A reasonable first reaction: if the driver already knows the message and could execute it
directly, why route through reth's payload-job machinery — an FCU round trip, a payload id,
a job to resolve?

Because that machinery owns things worth reusing. The engine tree constructs the sparse
state-root task and hands its handle to the builder, so state-root computation can overlap
execution. It manages the cross-block execution cache. It validates attributes. And
critically, it is the code path reth exercises and tests for locally built blocks — taking
it means Arbitrum block production inherits reth's behaviour rather than reimplementing it.

The cost is two channel round trips per block, and the driver works hard to overlap them
(see [The Driver Loop](ch04-01-driver-loop.md)).

## Chapter map

- [The Driver Loop](ch04-01-driver-loop.md) — batching, overlap, and the pipelining trick
  that keeps the engine busy.
- [From Message to Block Environment](ch04-02-payload-attributes.md) —
  `digest_message`, timestamps, ArbOS version resolution.
- [ArbOS Execution](ch04-03-arbos-execution.md) — the EVM bridge, the start-block
  transaction, transaction ordering, retries.
- [Sealing the Block](ch04-04-sealing.md) — state root, header assembly, Arbitrum header
  metadata.
- [The Engine Tree](ch04-05-engine-tree.md) — insertion, canonicalization, and what
  "canonical" means before persistence.
