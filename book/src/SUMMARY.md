# Summary

[The arbitrum-reth Book](title-page.md)
[Introduction](ch00-00-introduction.md)

# Getting Started

- [Getting Started](ch01-00-getting-started.md)
    - [Installation](ch01-01-installation.md)
    - [Running a Node](ch01-02-running-a-node.md)

- [The Big Picture](ch02-00-the-big-picture.md)
    - [Execute to Derive](ch02-01-execute-to-derive.md)
    - [The Message](ch02-02-the-message.md)

# How the Node Works

- [Getting Transactions from the Sequencer](ch03-00-ingest.md)
    - [The Sequencer Feed](ch03-01-sequencer-feed.md)
    - [L1 Derivation](ch03-02-l1-derivation.md)
    - [Reading Batches from L1](ch03-03-reading-batches.md)
    - [Blobs](ch03-04-blobs.md)
    - [The Delayed Inbox](ch03-05-delayed-inbox.md)
    - [Decoding a Batch](ch03-06-decoding-a-batch.md)
    - [Reconciling Two Sources](ch03-07-reconciling.md)

- [Applying Messages](ch04-00-applying.md)
    - [The Driver Loop](ch04-01-driver-loop.md)
    - [From Message to Block Environment](ch04-02-payload-attributes.md)
    - [ArbOS Execution](ch04-03-arbos-execution.md)
    - [Sealing the Block](ch04-04-sealing.md)
    - [The Engine Tree](ch04-05-engine-tree.md)

- [Making State Available](ch05-00-state.md)
    - [Persistence](ch05-01-persistence.md)
    - [Serving RPC](ch05-02-rpc.md)
    - [Pruning and History](ch05-03-pruning.md)

# Operating a Node

- [Bootstrapping a Chain](ch06-00-bootstrapping.md)
- [Observability and Tuning](ch07-00-observability.md)
- [Recovering from Trouble](ch08-00-recovery.md)

# Appendices

- [Appendix A: Crate Map](appendix-01-crate-map.md)
- [Appendix B: Glossary](appendix-02-glossary.md)
