use super::coordinator::P2pSyncCoordinator;
use super::SyncMsg;
use crate::app_state::AppState;
use crate::capacity::capacity_manager::CapacityManager;
use crate::capacity::missing_content_tracker::MissingContentTracker;
use eld_common::constants::p2p::P2P_TOPIC_CONTENT_SYNC;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::debug;

impl P2pSyncCoordinator {
    /// Start the message handler task that processes incoming P2P sync messages
    pub fn run_message_handler(
        self: Arc<Self>,
        capacity_manager: Arc<CapacityManager>,
        missing_content_tracker: Arc<Mutex<MissingContentTracker>>,
        mut msg_rx: mpsc::UnboundedReceiver<SyncMsg>,
    ) -> tokio::task::JoinHandle<()> {
        let committed_state_ref = self.committed_state.clone();
        tokio::spawn(async move {
            while let Some(msg) = msg_rx.recv().await {
                let committed_state = committed_state_ref.lock().ok().and_then(|s| s.clone());
                self.handle_sync_message(
                    &capacity_manager,
                    &missing_content_tracker,
                    committed_state.as_ref(),
                    msg,
                )
                .await;
            }
        })
    }

    /// Handle a single sync message
    async fn handle_sync_message(
        &self,
        capacity_manager: &CapacityManager,
        missing_content_tracker: &Mutex<MissingContentTracker>,
        committed_state: Option<&Arc<std::sync::Mutex<AppState>>>,
        msg: SyncMsg,
    ) {
        match msg {
            SyncMsg::Heartbeat { timestamp } => {
                debug!(
                    timestamp = timestamp,
                    "P2P Heartbeat received from storage validator"
                );
            }
            SyncMsg::ContentSyncHeartbeat { timestamp } => {
                debug!(
                    timestamp = timestamp,
                    topic = P2P_TOPIC_CONTENT_SYNC,
                    "P2P ContentSyncHeartbeat received"
                );
            }
            SyncMsg::ContentSyncHeartbeatResponse {
                provider_id: provider,
                timestamp,
            } => {
                debug!(
                    provider = %provider,
                    timestamp = timestamp,
                    topic = P2P_TOPIC_CONTENT_SYNC,
                    "P2P ContentSyncHeartbeatResponse received from capacity provider"
                );
            }
            SyncMsg::Announce { content_id } => {
                self.handle_announce(capacity_manager, missing_content_tracker, content_id)
                    .await;
            }
            SyncMsg::ContentRequest { content_id } => {
                self.handle_content_request(capacity_manager, content_id)
                    .await;
            }
            SyncMsg::ContentResponse {
                content_id,
                content,
            } => {
                self.handle_content_response(
                    capacity_manager,
                    missing_content_tracker,
                    content_id,
                    content,
                )
                .await;
            }
            SyncMsg::CapacityChallenge {
                challenge_id: challenge_id_str,
                challenger,
                provider_id: provider,
                chunk_indices,
                block_height,
                merkle_root,
                seed,
                expiration_block,
                timestamp,
            } => {
                self.handle_capacity_challenge(
                    capacity_manager,
                    challenge_id_str,
                    challenger,
                    provider,
                    chunk_indices,
                    block_height,
                    merkle_root,
                    seed,
                    expiration_block,
                    timestamp,
                )
                .await;
            }
            SyncMsg::CapacityChallengeResponse {
                challenge_id,
                provider_id: provider,
                challenger,
                block_height,
                proofs,
                generated_at,
                provider_pubkey,
                provider_signature,
            } => {
                self.handle_capacity_challenge_response(
                    committed_state,
                    challenge_id,
                    provider,
                    challenger,
                    block_height,
                    proofs,
                    generated_at,
                    provider_pubkey,
                    provider_signature,
                )
                .await;
            }
            SyncMsg::ContentInventoryRequest { timestamp } => {
                self.handle_content_inventory_request(timestamp).await;
            }
            SyncMsg::ContentInventoryResponse { items_json } => {
                self.handle_content_inventory_response(items_json);
            }
        }
    }
}
