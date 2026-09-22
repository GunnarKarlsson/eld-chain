use super::coordinator::P2pSyncCoordinator;
use super::trait_impl::P2pCoordinatorTrait;
use crate::capacity::capacity_manager::CapacityManager;
use crate::capacity::missing_content_tracker::MissingContentTracker;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use eld_common::constants::pinboard::MAX_CHUNK_SIZE;
use sha2::{Digest, Sha256};
use std::sync::Mutex;
use tracing::{error, info, warn};

impl P2pSyncCoordinator {
    pub(super) async fn handle_announce(
        &self,
        capacity_manager: &CapacityManager,
        missing_content_tracker: &Mutex<MissingContentTracker>,
        content_id: String,
    ) {
        // Compatibility mode: keep SyncMsg field name `content_id`, but treat it as a generic blob key.
        let content_key = content_id;
        info!(%content_key, "P2P Announce received (pinboard content_key)");
        let has_temp_blob = self
            .storage
            .get_pinboard_temp_blob(&content_key)
            .map(|blob| blob.is_some())
            .unwrap_or(false);
        if has_temp_blob {
            info!(%content_key, "Pinboard temp blob exists locally, ignoring Announce");
            return;
        }
        match capacity_manager.get_content_from_slots(&content_key).await {
            Ok(_) => {
                info!(%content_key, "Slot content exists locally, ignoring Announce");
            }
            Err(_) => {
                info!(%content_key, "Content not found locally — sending ContentRequest");
                if let Ok(mut tracker) = missing_content_tracker.lock() {
                    if let Err(e) = tracker.upsert_missing_content_by_content_id(&content_key) {
                        warn!(
                            %content_key,
                            error = %e,
                            "Failed to track missing content key"
                        );
                    }
                }
                self.broadcast_content_request(content_key);
            }
        }
    }

    pub(super) async fn handle_content_request(
        &self,
        capacity_manager: &CapacityManager,
        content_id: String,
    ) {
        // Compatibility mode: field name is `content_id`, value is interpreted as pinboard content_key.
        let content_key = content_id;
        info!(%content_key, "P2P ContentRequest received (pinboard content_key)");
        let pinboard_temp_blob = self
            .storage
            .get_pinboard_temp_blob(&content_key)
            .ok()
            .flatten();
        if let Some(content_bytes) = pinboard_temp_blob {
            info!(
                %content_key,
                content_size = content_bytes.len(),
                "Sending ContentResponse for locally available pinboard temp blob"
            );
            let content_base64 = BASE64_STANDARD.encode(&content_bytes);
            self.broadcast_content_response(content_key, content_base64);
            return;
        }

        match capacity_manager.get_content_from_slots(&content_key).await {
            Ok(content_bytes) => {
                info!(
                    %content_key,
                    content_size = content_bytes.len(),
                    "Sending ContentResponse for locally available slot content"
                );
                // Encode content as base64 for transmission
                let content_base64 = BASE64_STANDARD.encode(&content_bytes);
                self.broadcast_content_response(content_key, content_base64);
            }
            Err(_) => {
                info!(%content_key, "ContentRequest received but content not found locally");
            }
        }
    }

    pub(super) async fn handle_content_response(
        &self,
        capacity_manager: &CapacityManager,
        missing_content_tracker: &Mutex<MissingContentTracker>,
        content_id: String,
        content: String,
    ) {
        // Compatibility mode: field name is `content_id`, value is interpreted as pinboard content_key.
        let content_key = content_id;
        info!(
            %content_key,
            content_size = content.len(),
            "P2P ContentResponse received"
        );

        if capacity_manager
            .get_content_from_slots(&content_key)
            .await
            .is_ok()
        {
            info!(%content_key, "Slot content already exists locally, ignoring ContentResponse");
            return;
        }

        let content_bytes = match BASE64_STANDARD.decode(&content) {
            Ok(content_bytes) => content_bytes,
            Err(e) => {
                error!(
                    %content_key,
                    error = %e,
                    "Failed to decode base64 content from ContentResponse"
                );
                return;
            }
        };

        let computed_key = hex::encode(Sha256::digest(&content_bytes));
        if computed_key != content_key {
            warn!(
                %content_key,
                %computed_key,
                decoded_size = content_bytes.len(),
                "Rejecting ContentResponse: sha256(content) did not match content_key"
            );
            return;
        }
        info!(
            %content_key,
            decoded_size = content_bytes.len(),
            "Verified ContentResponse hash matches content_key"
        );

        match capacity_manager.get_content_from_slots(&content_key).await {
            Ok(_) => {
                info!(
                    content_key = %content_key,
                    "P2P pinboard capacity mirror: content already in slots, skipping store_content_chunks"
                );
            }
            Err(_) => {
                let chunks: Vec<Vec<u8>> = content_bytes
                    .chunks(MAX_CHUNK_SIZE)
                    .map(|c| c.to_vec())
                    .collect();
                match capacity_manager
                    .store_content_chunks(content_key.clone(), chunks)
                    .await
                {
                    Ok(_) => {
                        info!(
                            content_key = %content_key,
                            byte_len = content_bytes.len(),
                            "PINBOARD_P2P_SYNC_CAPACITY_WRITE_OK: pinboard content stored to capacity slots after ContentResponse"
                        );
                    }
                    Err(e) => {
                        error!(
                            content_key = %content_key,
                            error = %e,
                            "P2P pinboard capacity store failed after ContentResponse"
                        );
                    }
                }
            }
        }

        // Remove from missing content list (best effort).
        if let Ok(mut tracker) = missing_content_tracker.lock() {
            if let Err(e) = tracker.mark_content_synced_by_content_id(&content_key) {
                warn!(
                    content_id = %content_key,
                    error = %e,
                    "Failed to remove content from missing list"
                );
            }
        }
    }
}
