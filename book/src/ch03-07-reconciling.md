# Reconciling Two Sources

When `--l1-rpc` and `--feed-url` run together, two independent producers push into one
channel. Derivation walks the confirmed prefix; the feed rides the tip. Their outputs
overlap, and they arrive interleaved.

The reconciliation is the driver's, not the producers'. It is Nitro's `TransactionStreamer`
model, implemented in `ArbEngineDriver::advance_with_applied_inner`
(`crates/arb-reth-engine/src/engine.rs`).

## The cursor

The driver holds one number:

```rust,ignore
next_seq: u64   // the next message index to apply
```

seeded at startup from the resumed tip:

```rust,ignore
let next_seq = genesis_tip.number.saturating_sub(genesis_block) + 1;
```

Every incoming message is classified against it:

```text
              seq < next_seq          seq == next_seq         seq > next_seq
                    │                       │                       │
                 DROP                    APPLY                   BUFFER
          already applied by      produce the block,      hold in `pending`
          the other producer      then drain any          (BTreeMap keyed
          (idempotent)            now-contiguous          by sequence)
                                  buffered messages
```

After applying, the driver drains `pending` forward as long as the next sequence is
present, so a gap closing releases everything behind it in one pass. Stragglers that fall
below the cursor afterwards — a feed duplicate that lost the race to L1 — are discarded by a
final `retain`.

The buffer is capped at `MAX_PENDING = 50_000` messages. Beyond that, feed-ahead messages
are simply not buffered; they will be re-derived from L1 anyway. The cap bounds memory if
the feed runs far ahead of derivation, which is exactly what happens when derivation is
slow or stalled.

## Why index dedup is enough

Nitro does more than this: it byte-compares feed messages against what L1 later confirms
and can resequence on disagreement. `arbitrum-reth` deliberately does not.

The reasoning, stated in `crates/arb-reth-node/src/commands/node.rs`: on an honest
sequencer, the feed and L1 never disagree — what the sequencer broadcasts is what it
subsequently posts. So index dedup is sufficient in the only case that occurs in practice.

The gap is real and acknowledged: **a feed-versus-L1 content disagreement is not handled.**
If the sequencer published a message that L1 later contradicts, L1 is authoritative and the
node would need to reorg. That reorg logic is future work. This is not a subtle limitation
to be discovered later — it is why the node is labelled experimental.

## Two sources, one path

Everything that reaches the driver is a `BroadcastFeedMessage`, regardless of origin. The
driver cannot tell an L1-derived message from a live relay message, and that is the point:

```text
  relay ──────┐
              ├──▶ BroadcastFeedMessage ──▶ digest_message ──▶ ArbOS ──▶ block
  L1 derive ──┤
              │
  replay file ┘
```

One validated state-transition path, exercised identically by every source. The only
observable difference is metrics: only relay messages carry an ingress timestamp, so only
they appear in the `feed_*` latency family. Everything appears in
`reth_arb_reth_engine_block_*`.

## The drop gate, one layer up

Note that there are *two* deduplication mechanisms, at different layers, and they are not
redundant:

| Layer | Mechanism | Against |
| --- | --- | --- |
| L1 sync runtime | absolute block number ≤ `db_tip_l2` → drop before sending | Blocks already **on disk** after a resume |
| Driver | `seq < next_seq` → drop before applying | Messages already **applied** this session |

The first exists because a resume checkpoint may predate the durable tip. The second exists
because two producers overlap. A single mechanism could not cover both: the runtime does not
know what the driver has applied in memory, and the driver does not know what a fresh
resume will re-derive.

## `--no-l1-derive`

Setting this makes the feed the sole producer. It is the right choice only for a datadir
already inside the relay's retained range — resuming a node that was recently synced.

The flag exists because the two producers must not *both* be authoritative for a range they
both cover from scratch. Genesis is still bootstrapped from `--l1-rpc` (chain id, spec,
initial L1 base fee) even when derivation itself is skipped.
