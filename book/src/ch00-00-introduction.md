# Introduction

`arbitrum-reth` is an Arbitrum execution and derivation node built on
[reth](https://github.com/paradigmxyz/reth). It reads Arbitrum's inputs — sequencer
messages, whether they arrive over a relay socket or are recovered from Ethereum L1 —
executes ArbOS state transitions through
[`arbitrum-revm`](https://github.com/nuntax/arbitrum-revm), and writes the resulting L2
chain into a reth MDBX database that it then serves over JSON-RPC.

This book answers one question in depth:

> **How does a message become queryable state?**

Everything else follows from that. The three parts of the answer map onto the three core
chapters:

| Question | Chapter |
| --- | --- |
| Where do transactions come from? | [Getting Transactions from the Sequencer](ch03-00-ingest.md) |
| How are they executed? | [Applying Messages](ch04-00-applying.md) |
| How does the state become readable? | [Making State Available](ch05-00-state.md) |

## Who this book is for

You are comfortable with Rust and have at least a rough model of how an Ethereum
execution client works: blocks, receipts, a state trie, JSON-RPC. You do **not** need to
know Arbitrum or Nitro internals — the parts that matter are introduced as they come up.

If you have read reth's source before, several things will look familiar but subtly
rearranged. Chapter 2 explains why: Arbitrum inverts the usual relationship between a
block header and its execution, and that inversion is the reason `arbitrum-reth` skips
reth's staged sync pipeline entirely.

## How to read it

Chapters 1 and 2 are short and worth reading in order — chapter 2 in particular sets up
vocabulary (*message*, *derivation*, *execute-to-derive*) that the rest of the book uses
without re-explaining.

Chapters 3, 4 and 5 are the substance, and each is written to be readable on its own if
you already have the big picture. Chapters 6 through 8 are operational: bootstrapping,
metrics, and recovery.

Throughout, claims about behaviour are anchored to the code with paths like
`crates/arb-reth-engine/src/engine.rs`. Where a design decision has a non-obvious reason,
the reason is in the text — most of them are also recorded as comments at the cited
location, often in more detail than there is room for here.

## A note on Nitro

Arbitrum's reference implementation is [Nitro](https://github.com/OffchainLabs/nitro),
written in Go. `arbitrum-reth` reimplements Nitro's derivation and block-production
semantics on top of reth and revm; consensus-relevant behaviour must match Nitro exactly
or the chain forks. So Nitro is cited often, by file and function, as the normative
reference — for example, the batch framing in
[Decoding a Batch](ch03-06-decoding-a-batch.md) mirrors `nitro/arbstate/inbox.go`. You do
not need to read Nitro to follow the book, but the citations are there when a detail looks
arbitrary and you want to know whether it is.
