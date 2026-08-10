# The Delayed Inbox

Not every Arbitrum transaction goes through the sequencer. Deposits from L1, and any
transaction a user force-includes when the sequencer censors them, arrive through the
**delayed inbox**: they are delivered to an L1 contract, and a later sequencer batch
declares how many of them it has consumed.

Reconstructing these from L1 is the job of `DelayedInboxReader`
(`crates/arb-reth-l1/src/delayed.rs`), and it is more subtle than reading batches because
the message is split across two events.

## Two events, one message

```text
Bridge                          Inbox
──────                          ─────
MessageDelivered                InboxMessageDelivered        (body inline in the event)
  messageIndex                    or
  beforeInboxAcc                InboxMessageDeliveredFromOrigin (body in tx calldata)
  inbox
  kind
  sender
  messageDataHash  ◀────────────────── keccak256(body) must match
  baseFeeL1
  timestamp
```

`MessageDelivered` on the **`Bridge`** carries the metadata and, crucially, the
authoritative per-chain index numbering. The body comes from an inbox contract, either
inline or recoverable from the delivering transaction's calldata.

Pairing them is not simply "match by index". Other Arbitrum deployments share these event
signatures *and reuse the index space*, so bodies are paired to metadata by
`(index, inbox-address)`, where the inbox address is taken from `MessageDelivered.inbox`.
Then `keccak256(body)` is checked against the event's `messageDataHash` — a mismatch is an
error, not a warning.

This is also why `BRIDGE_MAINNET` is pinned as a constant: the `Bridge` proxy is the sole
emitter of `MessageDelivered` for Arbitrum One, so filtering on that address is what makes
the index space unambiguous.

## The accumulator chain

Each delayed message hashes as (from `Messages.sol`):

```text
messageHash = keccak256(
    kind(1) ++ sender(20) ++ blockNumber(8) ++ timestamp(8)
    ++ inboxSeqNum(32) ++ baseFeeL1(32) ++ messageDataHash(32)
)

accumulator = keccak256(beforeInboxAcc ++ messageHash)
```

Because each accumulator folds in the previous one, a reconstructed run of delayed messages
can be checked end to end: recompute the chain and confirm it matches the on-chain
accumulators. `verify_accumulator_chain` does this. It is the integrity check that makes
delayed reconstruction trustless rather than merely plausible.

The `request_id` stamped on the resulting message header is `BigToHash(messageIndex)` — a
non-null `request_id` is how a delayed message is distinguished from a sequencer message in
the feed representation.

## Coverage: the hard part

The obvious implementation — scan the same L1 range as the batches — is wrong.

A delayed message can be delivered to the inbox and then sit there indefinitely before a
batch consumes it. A deposit made in January can be read by a batch posted in March. So the
delayed scan range has nothing to do with the batch range.

`fetch_delayed_map_covering` (`crates/arb-reth-l1/src/sync.rs`) handles this by walking L1
blocks **backward** from the range end, widening until every delayed index the window's
batches consume is in hand. The step size is `delayed_window`, default `10_000` blocks —
sized to stay under typical `eth_getLogs` range caps, since most providers reject much
wider spans.

```text
                    batches in this window consume delayed indices 900..905
                                                    │
   L1 blocks ───────────────────────────────────────┼──────────▶
              ◀──── scan backward until 900..905 all found ─────┤
                     (10,000-block steps by default)         range end
```

A `DelayedCache` is threaded forward across consecutive windows, so a message fetched for
one window is not re-fetched for the next. Without that cache, the backward scan would
dominate the RPC bill.

## What the multiplexer sees

The decoder does not know any of this. It sees a `DelayedSource` trait:

```rust,ignore
trait DelayedSource {
    fn get(&self, index: u64) -> Option<DerivedMessage>;
}
```

When the multiplexer hits a `DelayedMessages` segment, it pulls exactly one message at the
running cursor and advances. Batches with no delayed segments pass `NoDelayed`, a source
that returns nothing.

Keeping this behind a trait is what makes `arb-reth-derive` a pure offline decoder with no
network dependency — the property that lets batch decoding be tested against fixtures
without an L1 endpoint.

## Failure modes

Two multiplexer errors relate to delayed messages, and they mean different things:

- `DelayedMissing(index)` — the source could not supply an index the batch asked for. In
  practice, the backward coverage scan did not reach far enough.
- `DelayedPastCount { read, after }` — the batch tried to read past its own declared
  `afterDelayedMessagesRead`. That is a malformed batch, not a fetch problem.
