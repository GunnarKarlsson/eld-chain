use super::commands::P2pCommand;
use super::coordinator::P2pSyncCoordinator;
use super::SyncMsg;
use crate::capacity::capacity_manager::CapacityManager;
use crate::capacity::missing_content_tracker::MissingContentTracker;
use eld_common::pinboard::PinboardMessageMetadata;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// Trait for P2P coordinator operations
pub trait P2pCoordinatorTrait: Send + Sync {
    fn broadcast_announce(&self, key: String);
    fn broadcast_content_request(&self, key: String);
    fn broadcast_content_response(&self, key: String, content: String);
    fn find_pinboard_and_broadcast_announce(
        &self,
        pinboard_meta_cache: &BTreeMap<String, PinboardMessageMetadata>,
    );
    fn subscribe_to_topic(&self, topic: &str) -> Result<(), String>;
    fn publish_to_topic(&self, topic: &str, msg: SyncMsg) -> Result<(), String>;

    /// Start the message handler task that processes incoming P2P sync messages.
    ///
    /// Implementations should typically delegate to their internal message loop
    /// and return the spawned task handle.
    fn run_message_handler(
        self: Arc<Self>,
        capacity_manager: Arc<CapacityManager>,
        missing_content_tracker: Arc<Mutex<MissingContentTracker>>,
        msg_rx: mpsc::UnboundedReceiver<SyncMsg>,
    ) -> tokio::task::JoinHandle<()>;
}

impl P2pCoordinatorTrait for P2pSyncCoordinator {
    fn broadcast_announce(&self, key: String) {
        let _ = self.cmd_tx.send(P2pCommand::BroadcastAnnounce { key });
    }

    fn broadcast_content_request(&self, key: String) {
        let _ = self
            .cmd_tx
            .send(P2pCommand::BroadcastContentRequest { key });
    }

    fn broadcast_content_response(&self, key: String, content: String) {
        let _ = self
            .cmd_tx
            .send(P2pCommand::BroadcastContentResponse { key, content });
    }

    fn find_pinboard_and_broadcast_announce(
        &self,
        pinboard_meta_cache: &BTreeMap<String, PinboardMessageMetadata>,
    ) {
        let cache_size = pinboard_meta_cache.len();
        let mut announced = 0u32;
        for (message_id, meta) in pinboard_meta_cache {
            info!(
                message_id = %message_id,
                content_key = %meta.content_key,
                "Announcing pinboard blob via P2P"
            );
            // Compatibility mode: continue using SyncMsg::Announce { content_id } where the field
            // carries the pinboard content_key.
            self.broadcast_announce(meta.content_key.clone());
            announced += 1;
        }
        info!(
            cache_size = cache_size,
            announced = announced,
            "Pinboard announce scan finished"
        );
    }

    fn subscribe_to_topic(&self, topic: &str) -> Result<(), String> {
        self.cmd_tx
            .send(P2pCommand::SubscribeToTopic {
                topic: topic.to_string(),
            })
            .map_err(|e| format!("Failed to send subscribe command: {e}"))?;
        Ok(())
    }

    fn publish_to_topic(&self, topic: &str, msg: SyncMsg) -> Result<(), String> {
        debug!(topic = %topic, "publish_to_topic called");
        match self.cmd_tx.send(P2pCommand::PublishToTopic {
            topic: topic.to_string(),
            msg,
        }) {
            Ok(_) => {
                debug!("PublishToTopic command sent to channel");
                Ok(())
            }
            Err(e) => {
                warn!(error = %e, "Failed to send PublishToTopic command to channel");
                Err(format!("Failed to send command: {e}"))
            }
        }
    }

    fn run_message_handler(
        self: Arc<Self>,
        capacity_manager: Arc<CapacityManager>,
        missing_content_tracker: Arc<Mutex<MissingContentTracker>>,
        msg_rx: mpsc::UnboundedReceiver<SyncMsg>,
    ) -> tokio::task::JoinHandle<()> {
        P2pSyncCoordinator::run_message_handler(
            self,
            capacity_manager,
            missing_content_tracker,
            msg_rx,
        )
    }
}
