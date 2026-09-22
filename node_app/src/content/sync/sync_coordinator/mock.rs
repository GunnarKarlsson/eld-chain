use super::trait_impl::P2pCoordinatorTrait;
use super::SyncMsg;
use crate::capacity::capacity_manager::CapacityManager;
use crate::capacity::missing_content_tracker::MissingContentTracker;
use eld_common::pinboard::PinboardMessageMetadata;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::debug;

/// Mock P2P coordinator for single-node mode
/// Implements the same interface as P2pSyncCoordinator but does nothing
#[derive(Debug)]
pub struct MockP2pSyncCoordinator;

impl MockP2pSyncCoordinator {
    /// Create a new mock P2P coordinator
    pub fn new() -> (Self, mpsc::UnboundedReceiver<SyncMsg>) {
        let (_msg_tx, msg_rx) = mpsc::unbounded_channel();
        (Self, msg_rx)
    }
}

impl P2pCoordinatorTrait for MockP2pSyncCoordinator {
    fn broadcast_announce(&self, _key: String) {
        // No-op in single-node mode
    }

    fn broadcast_content_request(&self, _key: String) {
        // No-op in single-node mode
    }

    fn broadcast_content_response(&self, _key: String, _content: String) {
        // No-op in single-node mode
    }

    fn find_pinboard_and_broadcast_announce(
        &self,
        _pinboard_meta_cache: &BTreeMap<String, PinboardMessageMetadata>,
    ) {
        // No-op in single-node mode
    }

    fn subscribe_to_topic(&self, _topic: &str) -> Result<(), String> {
        // No-op in single-node mode
        Ok(())
    }

    fn publish_to_topic(&self, _topic: &str, _msg: SyncMsg) -> Result<(), String> {
        // No-op in single-node mode
        debug!("Mock sync coordinator publish_to_topic");
        Ok(())
    }

    fn run_message_handler(
        self: Arc<Self>,
        capacity_manager: Arc<CapacityManager>,
        missing_content_tracker: Arc<Mutex<MissingContentTracker>>,
        msg_rx: mpsc::UnboundedReceiver<SyncMsg>,
    ) -> tokio::task::JoinHandle<()> {
        Self::run_message_handler(self, capacity_manager, missing_content_tracker, msg_rx)
    }
}

impl MockP2pSyncCoordinator {
    /// Start the message handler task that processes incoming P2P sync messages
    pub fn run_message_handler(
        self: Arc<Self>,
        _capacity_manager: Arc<CapacityManager>,
        _missing_content_tracker: Arc<Mutex<MissingContentTracker>>,
        mut msg_rx: mpsc::UnboundedReceiver<SyncMsg>,
    ) -> tokio::task::JoinHandle<()> {
        // In single-node mode, just consume messages without processing
        tokio::spawn(async move {
            while let Some(_msg) = msg_rx.recv().await {
                // Messages are consumed but not processed in single-node mode
            }
        })
    }
}
