# Documentation

Use the release binary in the examples below. Replace paths and endpoints with values for the target chain.

- [node](commands/node.md): derive and serve a chain.
- [snapshot](commands/snapshot.md): import or inspect Nitro state exports.
- [genesis](commands/genesis.md): verify state roots before import.
- [rewind](commands/rewind.md): remove a bad local suffix.
- [dump-blocks](commands/dump-blocks.md): inspect persisted blocks.
- [chains](chains/README.md): chain-specific operator notes.
- [observability](observability/README.md): Prometheus metrics for the execute and persist loop.

Every command exposes its complete flag list through `arb-reth <command> --help`.

For how the node works internally rather than how to invoke it, see [The arbitrum-reth Book](../book/); run `./open_book.sh` at the repository root to serve it.
