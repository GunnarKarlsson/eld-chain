//! Submits [`VerifiedProofTx`](eld_common::tx::VerifiedProofTx) with:
//! - persisted dedup on `(sender, challenge_id)` via [`VerifiedProofSubmissionClaimStorage`](crate::storage::traits::VerifiedProofSubmissionClaimStorage),
//! - one serialized pipeline per instance (mutex),
//! - optimistic local nonce via [`SequentialOptimisticNonceSender`](super::SequentialOptimisticNonceSender),
//! - CP-signed chunk proofs embedded in the tx for consensus verification.
//!
//! **Claim policy:** check claim before broadcast; persist claim only after successful
//! `send_tx_rpc` so a failed submit can be retried for the same `challenge_id`.
//!
//! **Nonce on failure** (under `send_serial` — no other VP submit until this returns):
//! definitive CheckTx/DeliverTx-style errors → [`reuse_nonce`](super::SequentialOptimisticNonceSender::reuse_nonce);
//! [`NetworkError`](eld_common::error::EldError::NetworkError) → set local next from account CADO.

use crate::storage::traits::VerifiedProofSubmissionClaimStorage;
use crate::wallet::SequentialOptimisticNonceSender;
use eld_common::capacity_proof::ChunkProof;
use eld_common::error::EldError;
use eld_common::tx::{Payload, Tx, TxSig, VerifiedProofTx};
use eld_common::Address;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tracing::{error, info};

/// Hex placeholder matching a real Ed25519 signature length for size-stable fee estimation.
/// (`TxSig::LEN` bytes → `TxSig::LEN * 2` hex chars.)
const DUMMY_TX_SIG: &str =
    "00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000";
// Compile-time check that the dummy signature length matches the expected length.
const _: () = assert!(DUMMY_TX_SIG.len() == TxSig::LEN * 2);

/// Fields from a validated P2P capacity challenge response, forwarded into `VerifiedProofTx`.
pub struct VerifiedProofSubmissionProofs {
    pub proofs: Vec<ChunkProof>,
    pub generated_at: u64,
    pub provider_pubkey: String,
    pub provider_signature: String,
    /// `true` records a failed proof. `false` is the reward path.
    pub failed: bool,
}

/// Capacity-validator path uses the wallet from `ELD_CAPACITY_VALIDATOR_WALLET_NAME`; the type stays generic via
/// [`VerifiedProofChainSubmitter::wallet_name`].
pub struct VerifiedProofChainSubmitter {
    wallet_name: String,
    cli: Arc<eld_client::facade::ChainClient>,
    consensus_config: Arc<std::sync::Mutex<crate::config::ConsensusConfig>>,
    protocol: eld_common::protocol_constants::ProtocolHandle,
    claim_store: Arc<dyn VerifiedProofSubmissionClaimStorage>,
    nonce_sender: SequentialOptimisticNonceSender,
    /// Serializes claim + nonce + broadcast (required for non-transactional claim store).
    send_serial: Mutex<()>,
}

impl VerifiedProofChainSubmitter {
    pub fn new(
        wallet_name: String,
        cli: Arc<eld_client::facade::ChainClient>,
        consensus_config: Arc<std::sync::Mutex<crate::config::ConsensusConfig>>,
        protocol: eld_common::protocol_constants::ProtocolHandle,
        claim_store: Arc<dyn VerifiedProofSubmissionClaimStorage>,
    ) -> Self {
        Self {
            wallet_name,
            cli,
            consensus_config,
            protocol,
            claim_store,
            nonce_sender: SequentialOptimisticNonceSender::new(),
            send_serial: Mutex::new(()),
        }
    }

    /// Broadcast `VerifiedProof` after local validation succeeded.
    pub async fn submit_verified_proof(
        &self,
        capacity_provider: &str,
        challenge_id: &str,
        block_height: i64,
        proof_fields: VerifiedProofSubmissionProofs,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _lane = self.send_serial.lock().await;

        let wallet = self
            .cli
            .get_wallet_by_name(self.wallet_name.clone())
            .await
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { format!("{e}").into() })?
            .ok_or_else(|| -> Box<dyn std::error::Error + Send + Sync> {
                format!("Wallet '{}' not found", self.wallet_name).into()
            })?;

        let sender_str = wallet.address.hex_with_prefix();
        if self
            .claim_store
            .has_verified_proof_submission_claim(&sender_str, challenge_id)
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { format!("{e}").into() })?
        {
            info!(
                challenge_id = %challenge_id,
                "Skipping VerifiedProof submit: persisted claim already exists for this sender"
            );
            return Ok(());
        }

        let cli_nonce = self.cli.clone();
        let addr_nonce = sender_str.clone();
        let assigned_nonce = self
            .nonce_sender
            .take_next_nonce(move || {
                let cli = cli_nonce.clone();
                let addr = addr_nonce.clone();
                async move { cli.get_next_nonce_for_account_cado(addr).await }
            })
            .await
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;

        let hex_encoded = match self
            .build_signed_verified_proof_hex(
                &wallet,
                capacity_provider,
                challenge_id,
                block_height,
                proof_fields,
                assigned_nonce,
            )
            .await
        {
            Ok(hex) => hex,
            Err(e) => {
                self.nonce_sender.reuse_nonce(assigned_nonce).await;
                return Err(e);
            }
        };

        info!(
            challenge_id = %challenge_id,
            capacity_provider = %capacity_provider,
            "Submitting VerifiedProof transaction to Tendermint RPC"
        );

        match self.cli.send_tx_rpc(&hex_encoded).await {
            Ok(response) => {
                crate::broadcast_log::log_deliver_tx_events(&response);
                info!(
                    challenge_id = %challenge_id,
                    capacity_provider = %capacity_provider,
                    "Successfully submitted VerifiedProof transaction to Tendermint RPC"
                );
                // Claim only after successful broadcast so failures can retry this challenge_id.
                match self
                    .claim_store
                    .insert_verified_proof_submission_claim(&sender_str, challenge_id)
                {
                    Ok(true) => {}
                    Ok(false) => {
                        info!(
                            challenge_id = %challenge_id,
                            "VerifiedProof claim already present after successful broadcast"
                        );
                    }
                    Err(e) => {
                        error!(
                            challenge_id = %challenge_id,
                            error = %e,
                            "Failed to persist VerifiedProof submission claim after successful broadcast"
                        );
                        return Err(format!("Failed to persist submission claim: {e}").into());
                    }
                }
                Ok(())
            }
            Err(e) => {
                match &e {
                    EldError::NetworkError { .. } => {
                        // Inclusion unknown: sync local next from account (still under send_serial).
                        match self
                            .cli
                            .get_next_nonce_for_account_cado(sender_str.clone())
                            .await
                        {
                            Ok(Some(next)) => self.nonce_sender.reuse_nonce(next).await,
                            Ok(None) | Err(_) => {
                                self.nonce_sender.reuse_nonce(assigned_nonce).await
                            }
                        }
                    }
                    _ => {
                        self.nonce_sender.reuse_nonce(assigned_nonce).await;
                    }
                }
                error!(
                    challenge_id = %challenge_id,
                    capacity_provider = %capacity_provider,
                    error = %e,
                    "Failed to submit VerifiedProof transaction to Tendermint RPC"
                );
                Err(format!("Failed to submit transaction: {e}").into())
            }
        }
    }

    async fn build_signed_verified_proof_hex(
        &self,
        wallet: &eld_common::wallet::Wallet,
        capacity_provider: &str,
        challenge_id: &str,
        block_height: i64,
        proof_fields: VerifiedProofSubmissionProofs,
        assigned_nonce: eld_common::nonce::Nonce,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let chain_id = self
            .consensus_config
            .lock()
            .map_err(|e| format!("Failed to acquire consensus config lock: {e}"))?
            .chain_id
            .clone();

        let verified_at_timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| format!("Failed to get timestamp: {e}"))?
            .as_secs();

        let provider_addr = Address::parse_hex_str(capacity_provider)
            .map_err(|e| format!("Invalid capacity_provider address: {e}"))?;

        let mut verified_proof_tx = VerifiedProofTx::new(
            wallet.address,
            provider_addr,
            challenge_id.to_string(),
            block_height,
            block_height,
            verified_at_timestamp,
            proof_fields.proofs,
            proof_fields.generated_at,
            proof_fields.provider_pubkey,
            proof_fields.provider_signature,
        )
        .map_err(|e| format!("Invalid VerifiedProofTx: {e}"))?;
        verified_proof_tx.failed = proof_fields.failed;

        let mut tx = Tx::new(
            assigned_nonce,
            Payload::new(verified_proof_tx),
            wallet.verifying_key(),
        );

        let fee_config = self
            .protocol
            .get()
            .ok_or_else(|| "protocol constants are not loaded".to_string())?
            .fee_config();

        // Estimate fee on a size-stable sig so check_tx (signed tx) matches; empty sig underpays by
        // one size-KB when signing crosses a 1024-byte boundary.
        let mut tx_for_fee = tx.clone();
        tx_for_fee.sig = TxSig::new(DUMMY_TX_SIG.to_string())
            .map_err(|e| format!("Invalid dummy signature: {e}"))?;

        let dynamic_fee = eld_common::fee::calculate_dynamic_fee(&tx_for_fee, &fee_config)
            .map_err(|e| format!("Failed to calculate fee: {e}"))?;
        tx.fee = dynamic_fee.into();

        wallet
            .sign(&mut tx, &chain_id)
            .map_err(|e| format!("Failed to sign transaction: {e}"))?;

        let json = serde_json::to_string(&tx)
            .map_err(|e| format!("Failed to serialize transaction: {e}"))?;
        Ok(hex::encode(&json))
    }
}
