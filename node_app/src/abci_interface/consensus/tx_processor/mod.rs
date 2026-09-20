use super::consensus::ConsensusConnection;
use crate::storage::traits::ConsensusConnectionStorage;
use abci::types::ResponseDeliverTx;
use async_trait::async_trait;
use eld_common::tx::{
    AddNamespaceTx, PostMessageTx, RegisterCapacityTx, StakeTx, TransferTx, TxPublicKey,
    UnregisterCapacityTx, UnstakeTx, UpdateCapacityMerkleRootTx, VerifiedProofTx,
};

pub mod add_namespace_processor;
pub mod post_message_processor;
pub mod register_capacity_processor;
pub mod stake_processor;
pub mod transfer_processor;
pub mod unregister_capacity_processor;
pub mod unstake_processor;
pub mod update_capacity_merkle_root_processor;
pub mod verified_proof_processor;

#[async_trait]
pub trait TransactionProcessor: Send + Sync {
    async fn process_verified_proof(&self, verified_proof_tx: VerifiedProofTx)
        -> ResponseDeliverTx;

    async fn process_register_capacity(
        &self,
        register_capacity_tx: RegisterCapacityTx,
        public_key: TxPublicKey,
    ) -> ResponseDeliverTx;

    async fn process_unregister_capacity(
        &self,
        unregister_capacity_tx: UnregisterCapacityTx,
    ) -> ResponseDeliverTx;

    async fn process_update_capacity_merkle_root(
        &self,
        update_tx: UpdateCapacityMerkleRootTx,
    ) -> ResponseDeliverTx;

    async fn process_transfer(&self, transfer_tx: TransferTx) -> ResponseDeliverTx;

    async fn process_stake(&self, stake_tx: StakeTx) -> ResponseDeliverTx;

    async fn process_unstake(&self, unstake_tx: UnstakeTx) -> ResponseDeliverTx;

    async fn process_post_message(&self, post_message_tx: PostMessageTx) -> ResponseDeliverTx;

    async fn process_add_namespace(&self, add_namespace_tx: AddNamespaceTx) -> ResponseDeliverTx;
}

#[async_trait]
impl<S> TransactionProcessor for ConsensusConnection<S>
where
    S: ConsensusConnectionStorage,
{
    async fn process_verified_proof(
        &self,
        verified_proof_tx: VerifiedProofTx,
    ) -> ResponseDeliverTx {
        verified_proof_processor::process_verified_proof_tx(self, verified_proof_tx).await
    }

    async fn process_register_capacity(
        &self,
        register_capacity_tx: RegisterCapacityTx,
        public_key: TxPublicKey,
    ) -> ResponseDeliverTx {
        register_capacity_processor::process_register_capacity_tx(
            self,
            register_capacity_tx,
            public_key,
        )
        .await
    }

    async fn process_unregister_capacity(
        &self,
        unregister_capacity_tx: UnregisterCapacityTx,
    ) -> ResponseDeliverTx {
        unregister_capacity_processor::process_unregister_capacity_tx(self, unregister_capacity_tx)
            .await
    }

    async fn process_update_capacity_merkle_root(
        &self,
        update_tx: UpdateCapacityMerkleRootTx,
    ) -> ResponseDeliverTx {
        update_capacity_merkle_root_processor::process_update_capacity_merkle_root_tx(
            self, update_tx,
        )
        .await
    }

    async fn process_transfer(&self, transfer_tx: TransferTx) -> ResponseDeliverTx {
        transfer_processor::process_transfer_tx(self, transfer_tx).await
    }

    async fn process_stake(&self, stake_tx: StakeTx) -> ResponseDeliverTx {
        stake_processor::process_stake_tx(self, stake_tx).await
    }

    async fn process_unstake(&self, unstake_tx: UnstakeTx) -> ResponseDeliverTx {
        unstake_processor::process_unstake_tx(self, unstake_tx).await
    }

    async fn process_post_message(&self, post_message_tx: PostMessageTx) -> ResponseDeliverTx {
        post_message_processor::process_post_message_tx(self, post_message_tx).await
    }

    async fn process_add_namespace(&self, add_namespace_tx: AddNamespaceTx) -> ResponseDeliverTx {
        add_namespace_processor::process_add_namespace_tx(self, add_namespace_tx).await
    }
}
