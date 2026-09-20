//! Extension trait for dispatching a validated [`Tx`] to [`TransactionProcessor`].

use super::tx_processor::TransactionProcessor;
use crate::errors::response_deliver_tx_error_validation_failed;
use abci::types::ResponseDeliverTx;
use async_trait::async_trait;
use eld_common::tx::{PayloadInner, Tx, UnstakeTx};

/// Extension trait so a validated [`Tx`] can dispatch to [`TransactionProcessor`] (e.g. [`ConsensusConnection`](super::consensus::ConsensusConnection)).
#[async_trait]
pub trait ConsensusTxDeliver {
    /// Runs the payload-specific deliver handler for this transaction.
    async fn process<C>(&self, conn: &C) -> ResponseDeliverTx
    where
        C: TransactionProcessor + Send + Sync;
}

#[async_trait]
impl ConsensusTxDeliver for Tx {
    async fn process<C>(&self, conn: &C) -> ResponseDeliverTx
    where
        C: TransactionProcessor + Send + Sync,
    {
        match self.payload.inner.clone() {
            PayloadInner::VerifiedProof(verified_proof_tx) => {
                conn.process_verified_proof(verified_proof_tx).await
            }
            PayloadInner::RegisterCapacity(register_capacity_tx) => {
                conn.process_register_capacity(register_capacity_tx, self.public_key.clone())
                    .await
            }
            PayloadInner::UnregisterCapacity(unregister_capacity_tx) => {
                conn.process_unregister_capacity(unregister_capacity_tx)
                    .await
            }
            PayloadInner::UpdateCapacityMerkleRoot(update_tx) => {
                conn.process_update_capacity_merkle_root(update_tx).await
            }
            PayloadInner::Transfer(transfer_tx) => conn.process_transfer(transfer_tx).await,
            PayloadInner::Stake(stake_tx) => {
                // TODO: Fix unstake parsing in serde to distinguish between stake and unstake
                if self.payload.r#type == "Unstake" {
                    match UnstakeTx::new(stake_tx.sender, stake_tx.amount) {
                        Ok(unstake_tx) => conn.process_unstake(unstake_tx).await,
                        Err(e) => response_deliver_tx_error_validation_failed(e.to_string()),
                    }
                } else {
                    conn.process_stake(stake_tx).await
                }
            }
            PayloadInner::Unstake(unstake_tx) => conn.process_unstake(unstake_tx).await,
            PayloadInner::PostMessage(post_message_tx) => {
                conn.process_post_message(post_message_tx).await
            }
            PayloadInner::AddNamespace(add_namespace_tx) => {
                conn.process_add_namespace(add_namespace_tx).await
            }
        }
    }
}
