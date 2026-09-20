use crate::capacity::capacity_manager::CapacityManager;
use crate::capacity::missing_content_tracker::MissingContentTracker;
use crate::content::sync::sync_coordinator::P2pCoordinatorTrait;
use eld_common::error::EldError;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::Duration;
use tracing::{error, info};

/// Configuration for periodic sync service
#[derive(Debug, Clone)]
pub struct PeriodicSyncConfig {
    /// Interval for periodic sync check (seconds)
    pub check_interval_secs: u64, // Default: 300 (5 minutes)
}

impl Default for PeriodicSyncConfig {
    fn default() -> Self {
        Self {
            check_interval_secs: 300,
        }
    }
}

/// Periodic sync service that checks for missing content and requests it
pub struct PeriodicSyncService {
    missing_content_tracker: Arc<Mutex<MissingContentTracker>>,
    capacity_manager: Arc<CapacityManager>,
    p2p_coordinator: Arc<dyn P2pCoordinatorTrait>,
    config: PeriodicSyncConfig,
}

impl PeriodicSyncService {
    /// Create a new periodic sync service
    pub fn new(
        capacity_manager: Arc<CapacityManager>,
        p2p_coordinator: Arc<dyn P2pCoordinatorTrait>,
        config: PeriodicSyncConfig,
    ) -> Result<Self, EldError> {
        // Get capacity directory from capacity manager config
        let capacity_dir = capacity_manager.config().capacity_dir.as_path();
        let mut tracker = MissingContentTracker::new(capacity_dir);
        tracker.load()?;

        Ok(Self {
            missing_content_tracker: Arc::new(Mutex::new(tracker)),
            capacity_manager,
            p2p_coordinator,
            config,
        })
    }

    /// Start the periodic sync service
    pub async fn start(&self) -> Result<(), EldError> {
        let interval = Duration::from_secs(self.config.check_interval_secs);
        let mut interval_timer = tokio::time::interval(interval);
        // Skip the first tick to avoid immediate execution
        interval_timer.tick().await;

        info!(
            "Periodic sync service started (interval: {} seconds)",
            self.config.check_interval_secs
        );

        loop {
            interval_timer.tick().await;
            info!("Running periodic content sync check");

            if let Err(e) = self.check_and_request_missing().await {
                error!(error = %e, "Error in periodic sync check");
            }
        }
    }

    /// Check missing content and request it
    async fn check_and_request_missing(&self) -> Result<(), EldError> {
        // Get all missing content
        // TODO: Limit the get to N items as a type of iteration
        let missing = {
            let tracker = self.missing_content_tracker.lock().unwrap();
            tracker.get_all_missing_content()?
        };

        if missing.is_empty() {
            info!("No missing content found, skipping sync");
            return Ok(());
        }

        info!(count = missing.len(), "Found missing content entries");

        for record in missing {
            // Double-check: is content still missing? (check in slots)
            match self
                .capacity_manager
                .get_content_from_slots(&record.content_id.hex_with_prefix())
                .await
            {
                Ok(_) => {
                    // Content exists! Remove from missing list (cleanup)
                    info!(
                        manifest_id = %record.manifest_id,
                        "Content now exists in slots, removing from missing list"
                    );
                    let mut tracker = self.missing_content_tracker.lock().unwrap();
                    tracker.mark_content_synced(&record.manifest_id.hex_with_prefix())?;
                }
                Err(_) => {
                    // Still missing - request it
                    info!(
                        manifest_id = %record.manifest_id,
                        content_id = %record.content_id,
                        attempt_count = record.attempt_count,
                        "Requesting missing content"
                    );

                    // Request from all peers
                    let content_key = record
                        .content_id
                        .hex_with_prefix()
                        .trim_start_matches("0x")
                        .to_string();
                    info!(
                        content_key = %content_key,
                        "Periodic sync broadcasting ContentRequest"
                    );
                    self.p2p_coordinator.broadcast_content_request(content_key);

                    // Update attempt tracking
                    let mut updated_record = record;
                    updated_record.attempt_count += 1;
                    updated_record.last_attempt = Some(
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map_err(|e| EldError::StorageError {
                                operation: "get_system_time".to_string(),
                                details: format!("Failed to get system time: {e}"),
                            })?
                            .as_secs(),
                    );

                    // Store updated record
                    let mut tracker = self.missing_content_tracker.lock().unwrap();
                    tracker.update_missing_content_record(&updated_record)?;
                }
            }
        }

        Ok(())
    }
}
