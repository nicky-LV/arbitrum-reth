# Bootstrapping a Chain

A node needs a block 0 — or something that can stand in for one. There are three ways to
get there, and they correspond to the three boot-mode flag groups.

```text
--chain-info + --genesis   Orbit chain from a Nitro genesis definition
--chain                    chain-config JSON (ArbOS genesis allocation)
--snapshot-head            a datadir seeded by `snapshot import`
```

## Genesis from a chain config

`crates/arb-reth-node/src/genesis.rs` converts an `ArbosInitConfig` into a reth `ChainSpec`
whose genesis allocation is the ArbOS-initialized state.

The allocation is not a static list. `arb_genesis_accounts` **re-runs the ArbOS init
procedure against an empty state** and enumerates every account written to block 0's trie.
That is the only way to guarantee the resulting root matches what Nitro's
`InitializeArbosInDatabase` produces.

The genesis *header* is reproduced field for field from Nitro's
`arbosState.MakeGenesisBlock`:

```text
London format
nonce      = 1
gasLimit   = 1 << 50
baseFee    = 0.1 gwei
difficulty = 1
extraData  = 32 zero bytes
mixHash    = encodes the ArbOS version
```

so both the genesis state root and the genesis block hash match the real chain. This has
been validated byte-for-byte against the nitro-testnode (block 0 hash `0xb88471…`, state
root `0xff8927…`).

Two entry points parse the init parameters: `arbos_init_from_chain_config_json` for a raw
chain-config blob, and `arbos_init_from_parsed` for a `ParsedInitMessage` decoded from an L1
`Initialize` message.

> `--initial-l1-base-fee` is part of the genesis *state*, defaulting to Nitro's
> `DefaultInitialL1BaseFee` of 50 gwei. A chain created with a different value — a
> nitro-testnode commonly uses a tiny one — will not reproduce its genesis root unless you
> pass the matching value. If your genesis root is wrong and everything else looks right,
> check this first.

## Orbit chains

`--chain-info chaininfo.json --genesis genesis.json` boots an Orbit chain end to end. The
two files are complementary and must be given together:

| File | Supplies |
| --- | --- |
| `genesis.json` | The chain spec and prealloc: geth-style `alloc`, `arbOSInit.initialL1BaseFee`, `serializedChainConfig`. |
| `chaininfo.json` | The L1 rollup deployment: chain id, parent chain id, `SequencerInbox`, `Bridge`, `DeployedAt`. |

Nitro applies the prealloc first and then runs ArbOS init, so on an address conflict the
ArbOS-written account wins. `arb_chain_spec_with_alloc` layers them in that order.

The rollup addresses are resolved as one coherent set — `RollupDeployment` in
`crates/arb-reth-node/src/commands/node.rs`, mirroring Nitro's
`chaininfo.GetRollupAddressesConfig`. `--l1-sequencer-inbox` and `--l1-bridge` must be set
together or not at all; you do not mix one chain's inbox with another's bridge, and passing
only one is an error.

## Snapshots

Syncing Arbitrum One from Nitro genesis is impractical. The realistic path is importing a
Nitro state export.

### The stream format

`snapshot import` reads a line-oriented export from Nitro's state dumper:

```text
A <accountHash:64hex> <nonce:dec> <balance:hex> <codeHash:64hex> <storageRoot:64hex>
C <codeHash:64hex> <code:hex>
S <slotHash:64hex> <value:hex>
```

- `A` starts an account; subsequent `S` lines belong to it.
- `C` declares bytecode by its keccak hash and may appear anywhere.
- All hashes are already-hashed 64-hex keys.
- Balances and values may be odd-length hex.

The importer writes directly into `HashedAccounts`, `HashedStorages`, and `Bytecodes`, then
drives reth's trie computation to verify the root:

```sh
arb-reth snapshot import \
  --state  /tmp/arb1_genesis_state.stream \
  --blocks /tmp/arb1_head_block.stream \
  --out    /tmp/arbreth-mdbx \
  --expect 0x7f2bfc4481d02bfcfc606ebb949384ef78d03a0f30a2dc9cccd652eb80926ae1
```

`--expect` is not optional in spirit. An import that produces the wrong root produces a node
that computes wrong state roots forever after, and the failure will not be obvious.

### Related subcommands

| Command | Purpose |
| --- | --- |
| `snapshot build-preimages` | Build reth's slot-preimage sidecar from a Nitro Classic export. |
| `snapshot read` | Inspect accounts, code, and storage in an imported datadir. |
| `snapshot repair-history` | Add missing history-boundary metadata to an older import. |
| `genesis verify` | Verify the Arbitrum One Nitro-genesis state root from a classic export. |
| `genesis verify-export` | Verify the hashed state-trie root of a `reth-export --mode state` stream on stdin. |

### Booting on a snapshot

```sh
arb-reth node --datadir /data/arb1 --snapshot-head /data/head.stream --l1-rpc … --http
```

`--snapshot-head` points at the `reth-export --mode blocks` head stream
(`H <num> <hash> <headerRLP>`). The node builds its chain spec from that head header so the
genesis-hash check accepts the imported database, and resumes from the snapshot's head
block. Do not also pass `--chain`.

## Finding the L1 resume point

Booting is only half the problem: derivation has to know **where in L1** to start, so that
its first new block is `db_tip + 1`.

Precedence, resolved in `commands/node.rs`:

1. An explicit `--l1-start-block` override.
2. The persisted `arb-l1-resume.json` checkpoint.
3. The genesis-snapshot bootstrap: batch 0's delivery block, found on-chain from the
   deployment anchor.

Three optional overrides refine it:

- **`--l1-start-delayed`** — the delayed cursor before the start block. Defaults to the
  durable tip header's `nonce`, which encodes exactly this, so it rarely needs setting.
- **`--l1-start-l2-block`** — the L2 block the start boundary sits *after*. Defaults to the
  durable DB tip, which is correct when the start block is the batch boundary the tip was
  built from.
- **`--l2-genesis-block`** — the chain's genesis anchor. Defaults to `22_207_817` for
  Arbitrum One, `0` for a custom deployment.

> **The mid-batch case.** If a snapshot was cut *inside* a batch, the tip is not on a batch
> boundary, and defaulting `--l1-start-l2-block` to the DB tip is wrong. Pass the batch's
> first message index minus one instead. Re-derived blocks at or below the DB tip are
> dropped by the gate described in [L1 Derivation](ch03-02-l1-derivation.md), so derivation
> realigns without rewinding below the snapshot's own genesis.

> **`--l1-start-block` must be a batch boundary the current tip was built from.** It is a
> forcing override with no validation. A wrong value produces a chain that derives cleanly
> and is silently wrong.
