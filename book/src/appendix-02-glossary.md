# Appendix B: Glossary

**AnyTrust** — An Arbitrum data-availability mode where batch data is held by a committee
rather than posted in full to L1. Signalled by batch header flags `0x80` / `0x08`.

**ArbOS** — Arbitrum's operating-system layer inside the L2 state: gas pricing, L1 fee
accounting, retryables, the outbox, precompiles. Its version is encoded in each block header
and advanced by the start-block transaction.

**`ArbHeaderInfo`** — Arbitrum header metadata packed into `extra_data` and `mix_hash`: send
root, send count, L1 block number, ArbOS version, and the tip-collection flag. Reading it
from the parent header is how the node resolves the ArbOS version for the next block.

**Batch** — The unit the sequencer posts to L1: a 40-byte timeBounds header, a flag byte, and
a compressed segment list. Decodes into one or more messages.

**`BatchPostingReport`** — Message kind 13. Tells ArbOS what a previously posted batch cost
on L1 so it can charge users. Needs `BatchDataStats` reconstructed from the batch bytes.

**Batch poster address** — `0xA4B0…73657175656e636572` (trailing bytes spell `sequencer`).
The virtual sender stamped on sequencer messages and the coinbase of sequencer blocks.

**`BroadcastFeedMessage`** — The unit flowing between ingest and execution. What a relay
publishes, and what L1 derivation converts into.

**Canonical (in memory)** — A block accepted as head by the engine tree and visible through
`CanonicalInMemoryState`. Queryable over RPC immediately; persisted later.

**Delayed inbox** — The path for L1 deposits and force-included transactions. Delivered to
the `Bridge`, consumed later by a batch's `DelayedMessages` segments.

**`delayedMessagesRead`** — Cumulative count of delayed messages consumed as of a block.
Encoded in the block header's `nonce` (Nitro's `EncodeNonce`).

**`DerivedMessage`** — The typed binary message form produced by the multiplexer; Nitro's
`MessageWithMetadata`.

**Derivation** — Reconstructing the L2 message stream from L1 data alone. The trustless sync
path.

**Durable tip** — The persisted database head, `last_block_number()`. Distinct from the
canonical head, which may be ahead of it in memory. Resume checkpointing depends on the
difference.

**Engine tree** — reth's `EngineApiTreeHandler`: owns block insertion, canonicalization, the
in-memory overlay, and the persistence service.

**Execute-to-derive** — Arbitrum's model: the block, including its state root, is the
*output* of executing a message, not something to verify against a downloaded header. See
[Execute to Derive](ch02-01-execute-to-derive.md).

**FCU** — `ForkchoiceUpdated`. Sent twice per block: once with payload attributes to launch a
build, once with the new head to canonicalize it.

**Feed-ahead** — A message whose sequence number exceeds the driver's cursor, buffered until
derivation closes the gap.

**`gasUsedForL1`** — The Arbitrum receipt field recording L1 posting cost as L2-gas
equivalent. Set from `chain().poster_gas` at execution time.

**Genesis block number** — The chain's Nitro-genesis anchor: `22_207_817` for Arbitrum One,
`0` for a fresh chain. `block = sequence_number + genesis_block`.

**`InternalTxStartBlock`** — Transaction type `0x6a`, the mandatory first transaction of every
Arbitrum block. Carries `ArbosActs.startBlock(...)`; updates pricing, records L1 block
advancement, applies ArbOS upgrades.

**Message index / sequence number** — The position of a message in the stream. Determines the
block number and is the dedup key when two producers run.

**Multiplexer** — `extract_messages`: walks a decoded batch's segments and emits messages.
Nitro's `inboxMultiplexer`.

**Nitro** — Arbitrum's reference implementation, in Go. The normative source for consensus
behaviour.

**Orbit chain** — A chain built with the Arbitrum stack, deployed independently of Arbitrum
One. Boots from `chaininfo.json` plus `genesis.json`.

**Poster gas** — ArbOS's L2-gas-equivalent charge for L1 posting cost, computed during
pre-execution and recorded as `gasUsedForL1`.

**Relay / feed** — A sequencer's WebSocket broadcast of messages as they are sequenced. A tip
source with a bounded backlog, not a history source.

**Resume log** — `arb-l1-resume.json` in the datadir. Records L1 window boundaries and the L2
blocks they sit after, appended only once those blocks are durable.

**Retryable / retry** — Arbitrum's mechanism for L1→L2 calls that may be redeemed later.
Redeeming schedules an `ArbitrumRetryTx`, which Nitro runs immediately after the redeeming
transaction — an ordering constraint that is consensus-critical.

**Safe head** — `L1 head - confirmations` (default 8). Derivation never reads past it, as a
reorg safety margin.

**Segment** — One entry in a decoded batch: an L2 message, a delayed-message reference, or a
timestamp / L1-block advance.

**`SequencerInbox`** — The L1 contract sequencer batches are posted to. Emits
`SequencerBatchDelivered`.

**Sidecar** — An EIP-4844 blob's data, retrievable only from the consensus layer, addressed
by slot, and retained for roughly 18 days by a stock beacon node.

**Sparse trie task** — reth's background state-root computation consuming state updates as
execution produces them. Opt-in here; falls back on storage wipes and pre-ArbOS-30
created-empty accounts.

**Storage V2 / slot preimages** — reth stores canonical state under hashed keys while unwind
changesets use plain slot keys. The `SlotPreimages` sidecar bridges the two;
`arbitrum-reth` populates it via a persistence proxy since it runs no staged sync.

**timeBounds** — The 40-byte batch header: min/max timestamp, min/max L1 block, and
`afterDelayedMessages`.

**Window** — A range of L1 blocks derivation processes as one unit. Default 1000 blocks;
gated on `batchCount()` so barren stretches collapse.
