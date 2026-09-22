use super::coordinator::P2pSyncCoordinator;
use super::trait_impl::P2pCoordinatorTrait;
use super::SyncMsg;
use eld_common::constants::p2p::P2P_TOPIC_CONTENT_SYNC;
use tracing::{error, info};

impl P2pSyncCoordinator {
    pub(super) async fn handle_content_inventory_request(&self, timestamp: u64) {
        info!(
            timestamp = timestamp,
            "P2P ContentInventoryRequest received"
        );
        // Legacy manifest inventory has been removed; respond with empty list.
        let items_json = match serde_json::to_string(&Vec::<serde_json::Value>::new()) {
            Ok(json) => json,
            Err(e) => {
                error!(
                    error = %e,
                    "Failed to serialize content inventory items to JSON"
                );
                "[]".to_string() // Send empty array on serialization error
            }
        };

        info!(
            item_count = 0,
            last_updated_block = 0,
            "Sending ContentInventoryResponse"
        );

        // Publish response on P2P_TOPIC_CONTENT_SYNC
        let response = SyncMsg::ContentInventoryResponse { items_json };
        if let Err(e) = self.publish_to_topic(P2P_TOPIC_CONTENT_SYNC, response) {
            error!(
                error = %e,
                "Failed to publish ContentInventoryResponse"
            );
        }
    }

    pub(super) fn handle_content_inventory_response(&self, items_json: String) {
        info!(
            items_json_len = items_json.len(),
            "P2P ContentInventoryResponse received"
        );
        // Parse and log the inventory items
        match serde_json::from_str::<Vec<serde_json::Value>>(&items_json) {
            Ok(items) => {
                info!(
                    item_count = items.len(),
                    "ContentInventoryResponse contains {} items",
                    items.len()
                );
                if items.len() > 5 {
                    info!("... and {} more items", items.len() - 5);
                }
            }
            Err(e) => {
                error!(
                    error = %e,
                    "Failed to parse ContentInventoryResponse JSON"
                );
            }
        }
    }
}
