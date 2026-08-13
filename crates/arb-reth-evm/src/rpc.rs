//! RPC compatibility impls for `arb-reth-evm` types (gated behind the `rpc` feature).
//!
//! - `TryIntoTxEnv<ArbTx, ArbSpecId, ArbBlockEnv> for ArbTransactionRequest`: satisfies the
//!   `TxEnvConverter<ArbTransactionRequest, ArbEvmConfig>` blanket.
//! - `BuildPendingEnv<Header> for ArbNextBlockEnvAttributes`: satisfies the
//!   `PendingEnvBuilder<ArbEvmConfig>` blanket so `EthApiBuilder::build()` compiles.

use alloy_evm::rpc::{EthTxEnvError, TryIntoTxEnv};
use arb_revm::ArbTransaction;
use arbitrum_alloy_rpc_types::ArbTransactionRequest;

use crate::{ArbBlockEnv, ArbNextBlockEnvAttributes, ArbTx};

use arb_revm::ArbSpecId;

impl TryIntoTxEnv<ArbTx, ArbSpecId, ArbBlockEnv> for ArbTransactionRequest {
    type Err = EthTxEnvError;

    fn try_into_tx_env(
        self,
        evm_env: &alloy_evm::EvmEnv<ArbSpecId, ArbBlockEnv>,
    ) -> Result<ArbTx, EthTxEnvError> {
        let tx_env: revm::context::TxEnv = self.inner.try_into_tx_env(evm_env)?;
        // No retry_meta for RPC sim; encoded_2718 not needed.
        Ok(ArbTx(ArbTransaction::new(tx_env)))
    }
}

use alloy_consensus::BlockHeader as AlloyBlockHeader;
use alloy_primitives::B256;
use alloy_rpc_types_eth::BlockOverrides;
use reth_primitives_traits::SealedHeader;
use arbitrum_alloy_consensus::header::ArbHeaderInfo;
use reth_rpc_eth_api::helpers::pending_block::BuildPendingEnv;

impl<H: AlloyBlockHeader> BuildPendingEnv<H> for ArbNextBlockEnvAttributes {
    fn build_pending_env(
        parent: &SealedHeader<H>,
        _block_overrides: Option<&BlockOverrides>,
    ) -> Self {
        // The parent header DOES carry both, contrary to what this comment used to
        // claim: ArbOS packs send_count / l1_block_number / arbos_version into
        // `mix_hash` and the send root into `extra_data`, which is exactly what
        // `spec_for_header` / `l1_block_number_for_header` read on the canonical
        // path. `decode_header_parts` takes those two byte strings, so it works
        // through the generic `AlloyBlockHeader` bound as well.
        //
        // Leaving `arbos_format_version` at 0 was not cosmetic: the pending EVM's
        // spec comes from `ArbSpecId::from_arbos_version(attributes
        // .arbos_format_version)`, so 0 activated NOTHING and every `eth_call`
        // against `pending` failed with `EVM error: NotActivated` the moment it
        // touched a post-Merge opcode — which is any Uniswap v4 pool, since the
        // PoolManager needs Cancun transient storage.
        //
        // Reading it from the parent is correct for every block except one that
        // itself performs an ArbOS upgrade, where the child's version is one
        // higher; that block falls back to the parent's spec for its pending env
        // only, and is corrected the moment the block is canonical.
        let info = parent
            .mix_hash()
            .and_then(|mix| {
                ArbHeaderInfo::decode_header_parts(parent.extra_data().as_ref(), mix.as_slice())
                    .ok()
            })
            .filter(|i| i.is_arbitrum());
        Self {
            timestamp: parent.timestamp().saturating_add(1),
            suggested_fee_recipient: parent.beneficiary(),
            prev_randao: B256::ZERO,
            gas_limit: parent.gas_limit(),
            l1_block_number: info.as_ref().map(|i| i.l1_block_number).unwrap_or(0),
            l1_base_fee_wei: alloy_primitives::U256::ZERO,
            arbos_format_version: info.as_ref().map(|i| i.arbos_format_version).unwrap_or(0),
            delayed_messages_read: parent.nonce().map(|n| u64::from_be_bytes(n.0)).unwrap_or(0),
            extra_data: alloy_primitives::Bytes::default(),
            withdrawals: None,
            finish_timing_out: Default::default(),
        }
    }
}
