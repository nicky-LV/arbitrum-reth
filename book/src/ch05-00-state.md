# Making State Available

A block that has been executed and canonicalized is not yet *useful*. This chapter covers
the last leg: how the executed block reaches durable storage, and how a JSON-RPC client
reads the resulting state.

```text
   ArbEngineDriver
        │
        │ InsertExecutedBlock + ForkchoiceUpdated
        ▼
   ┌──────────────────────────────────────────────────────────────┐
   │  engine tree                                                 │
   │    CanonicalInMemoryState  ◀── updated synchronously         │
   │    persistence service     ◀── batched, background           │
   └───────────┬───────────────────────────────┬──────────────────┘
               │                               │
               │                    storage_v2 proxy (slot preimages)
               │                               │
               │                               ▼
               │                    ┌────────────────────────┐
               │                    │  MDBX + static files   │
               │                    │  + pruner              │
               │                    └───────────┬────────────┘
               │                                │
   ┌───────────▼────────────────────────────────▼──────────────────┐
   │  BlockchainProvider — reads across memory and disk            │
   └───────────────────────────┬───────────────────────────────────┘
                               │
   ┌───────────────────────────▼───────────────────────────────────┐
   │  reth RpcAddOns: eth / trace / debug / net / web3 / txpool     │
   │  + ArbRpcConverter (gasUsedForL1 on receipts)                  │
   │  engine API: Noop                                              │
   └───────────────────────────────────────────────────────────────┘
```

## The key asymmetry

The two arrows out of the engine tree have very different timing:

- **In-memory canonicalization** happens synchronously, in milliseconds. As soon as it
  completes, the block is queryable.
- **Persistence** happens in batches, in the background, governed by
  `persistence_threshold`, `memory_block_buffer_target`, and
  `persistence_backpressure_threshold`.

`BlockchainProvider` unifies both, so RPC clients never see the seam. But internal
components sometimes need to know which side of it they are on — L1 derivation's resume
checkpointing is the main example, and it deliberately reads the durable tip via
`last_block_number()` rather than the canonical head.

## Chapter map

- [Persistence](ch05-01-persistence.md) — the persistence service, the storage-v2 proxy,
  and the durability knobs.
- [Serving RPC](ch05-02-rpc.md) — reth's RPC add-ons, the Arbitrum receipt converter, and
  why there is no engine API.
- [Pruning and History](ch05-03-pruning.md) — archive by default, and what the pruning
  profiles remove.
