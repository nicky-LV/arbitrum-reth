//! Executed-block push: a pre-canonical notification for co-located consumers.
//!
//! Emitted from inside block production the moment the transaction loop
//! finishes — BEFORE the state-root wait, header assembly, canonicalization
//! and the newHeads RPC leg. On a derived chain the block's outcome is final
//! at that point (the node built the block itself from the sequencer feed;
//! the state root is header bookkeeping, not validation that can fail), so a
//! consumer that only needs the logs — an arb engine reacting to pool state —
//! gains the entire post-execution tail (~2.5ms p50, up to ~11ms when the
//! state-root wait spikes) plus its own `eth_getBlockReceipts` round trip.
//!
//! The payload deliberately carries receipts in the same shape the consumer
//! would have read from `eth_getBlockReceipts`: per tx, the sender and the
//! full logs (empty for a reverted tx). It is NOT a sealed block: there is no
//! block hash yet (the header does not exist), and the notification is not a
//! canonicality claim — a consumer must keep following `newHeads` for
//! confirmation and treat a pushed block that never canonicalizes as a
//! divergence to resync from.

use alloy_primitives::{Address, B256, Bytes};
use serde::Serialize;

/// One log, in `eth_getBlockReceipts` field layout (hex-encoded on the wire
/// via the alloy serde impls: `address`/`topics` as 0x-hex, `data` as 0x-hex).
#[derive(Debug, Clone, Serialize)]
pub struct PushLog {
    pub address: Address,
    pub topics: Vec<B256>,
    pub data: Bytes,
}

/// One executed transaction: sender, success flag, and its full logs
/// (unfiltered; empty when the tx reverted — same as its receipt).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutedTx {
    pub from: Address,
    pub status: bool,
    pub logs: Vec<PushLog>,
}

/// The pre-canonical executed-block notification, one per produced block.
///
/// `block_number`/`sequence_number`/`timestamp` are JSON numbers (all fit in
/// 2^53 by orders of magnitude). Includes ArbOS-internal txs, exactly like
/// `eth_getBlockReceipts`, so a consumer can assert payload equality against
/// the canonical receipts.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutedBlock {
    pub block_number: u64,
    /// Feed sequence number — the join key for `arb_getFeedIngress`.
    pub sequence_number: u64,
    pub timestamp: u64,
    pub txs: Vec<ExecutedTx>,
}
