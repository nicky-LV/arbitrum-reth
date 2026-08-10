# The Message

Everything in this node is a function of one type. Before following it through the
pipeline, it is worth knowing what it contains.

## `BroadcastFeedMessage`

This is the unit that flows through the channel between ingest and execution. It comes
from `arbitrum-alloy`'s `sequencer::feed` module and is, in shape, exactly what a sequencer
relay publishes over its WebSocket:

```rust,ignore
BroadcastFeedMessage {
    sequence_number: u64,
    message_with_meta_data: MessageWithMetadata {
        l1_incoming_message: L1IncomingMessage {
            header: Header {
                kind: u8,
                sender: String,        // hex address
                block_number: u64,     // L1 block
                timestamp: u64,        // L1 timestamp
                request_id: Value,     // null for sequencer messages
                base_fee_l1: Value,
            },
            l2msg: String,             // base64
            legacy_batch_gas_cost: Option<..>,
            batch_data_stats: Option<BatchDataStats>,
        },
        delayed_messages_read: u64,
    },
}
```

Three fields carry most of the weight:

- **`sequence_number`** is the message index. Combined with the chain's genesis block it
  determines the L2 block number: `block = sequence_number + genesis_block`. It is also
  the dedup key when two producers are running.
- **`l2msg`** is the payload: a base64-encoded L2 message that expands into zero or more
  signed transactions. Its internal structure is described in
  [Decoding a Batch](ch03-06-decoding-a-batch.md).
- **`delayed_messages_read`** is the cumulative count of delayed-inbox messages consumed as
  of this block. Nitro encodes it into the block header's `nonce` field, so it is
  consensus-relevant, not bookkeeping.

The header's `kind` selects the message family. The two you meet most are kind `3`
(`L2Message`, an ordinary sequencer batch entry) and kind `13` (`BatchPostingReport`, the
message ArbOS uses to account for the L1 cost of posting a batch). Delayed-inbox messages
carry their own kinds and a non-null `request_id`.

## The canonical binary form: `DerivedMessage`

The feed type is a JSON transport DTO — hex strings, base64, `serde_json::Value` fields.
The L1 derivation pipeline works in a typed binary form instead
(`crates/arb-reth-derive/src/message.rs`):

```rust,ignore
DerivedMessage {
    header: L1IncomingMessageHeader {
        kind: u8,
        poster: Address,
        block_number: u64,
        timestamp: u64,
        request_id: Option<B256>,   // None for sequencer messages
        l1_base_fee: U256,
    },
    l2_msg: Vec<u8>,
    delayed_messages_read: u64,
}
```

This mirrors Nitro's `MessageWithMetadata`. It is what the batch multiplexer emits.

## Why derivation converts to the feed shape

`arb-reth-l1`'s `feed` module converts `DerivedMessage` back into `BroadcastFeedMessage`
before handing it to the driver. That looks like a pointless round trip through a JSON-ish
representation — it is not.

The reason is that there is exactly **one** validated path from a message to a block:
`digest_message`, driven from a `BroadcastFeedMessage`. Making L1-derived messages take
that same path means L1 catch-up and live feed following exercise identical code. A bug in
message interpretation cannot manifest on one source and not the other.

The cost is real but small: `l2msg` gets base64-encoded on the way out and decoded again by
`parse_message`. The conversion lives in
`crates/arb-reth-l1/src/feed.rs`.

One field cannot survive the trip on its own: a `BatchPostingReport` needs
`batch_data_stats` (token count and gas cost of the batch it reports on), which is not
carried on a `DerivedMessage`. Derivation reconstructs it from the batch bytes and attaches
it during assembly — see [Reading Batches from L1](ch03-03-reading-batches.md).

## The virtual batch poster

Sequencer messages are stamped with a synthetic sender:

```text
0xA4B000000000000000000073657175656e636572
```

Those trailing bytes spell `sequencer` in ASCII. It is Nitro's
`arbos/l1pricing.BatchPosterAddress`, defined here in
`crates/arb-reth-derive/src/message.rs`. It matters during execution: the block's
coinbase equals the message poster, and the assembler only marks a block as tip-collecting
when the beneficiary is this address — delayed-message blocks, whose poster is the real L1
sender, never collect tips.
