//! P2P heartbeat tasks (protocol variants on `SyncMsg` for now; to be removed later).

use super::sync_coordinator::{P2pCoordinatorTrait, P2pSyncCoordinator};
use eld_common::constants::p2p::{ELD_STORAGE_CHALLENGE_TOPIC_PREFIX, P2P_TOPIC_CONTENT_SYNC};
use eld_common::sync_msg::SyncMsg;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

impl P2pSyncCoordinator {
    /// Send heartbeats every 5 seconds to all registered capacity providers.
    pub fn start_heartbeat_task(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
            // Skip the first tick to avoid immediate execution
            interval.tick().await;

            info!("Heartbeat task started (sending heartbeats every 5 seconds to registered capacity providers)");

            loop {
                interval.tick().await;

                let committed_state = match self.committed_state.lock() {
                    Ok(state) => state.clone(),
                    Err(e) => {
                        warn!(
                            error = %e,
                            "Failed to acquire committed_state lock for heartbeat"
                        );
                        continue;
                    }
                };

                let capacity_validators = match committed_state {
                    Some(state_lock) => {
                        let state = match state_lock.lock() {
                            Ok(state) => state,
                            Err(e) => {
                                warn!(
                                    error = %e,
                                    "Failed to acquire app_state lock for heartbeat"
                                );
                                continue;
                            }
                        };
                        state.envelope.capacity_validators.clone()
                    }
                    None => {
                        continue;
                    }
                };

                if capacity_validators.is_empty() {
                    info!("No registered capacity providers, skipping heartbeat");
                    continue;
                }

                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                for provider in &capacity_validators {
                    let capacity_provider = &provider.address;
                    let challenge_topic =
                        format!("{ELD_STORAGE_CHALLENGE_TOPIC_PREFIX}{capacity_provider}");

                    let heartbeat_msg = SyncMsg::Heartbeat { timestamp };

                    match self.publish_to_topic(&challenge_topic, heartbeat_msg) {
                        Ok(_) => {
                            info!(
                                capacity_provider = %capacity_provider,
                                topic = %challenge_topic,
                                timestamp = timestamp,
                                "Sent heartbeat to capacity provider"
                            );
                        }
                        Err(e) => {
                            warn!(
                                capacity_provider = %capacity_provider,
                                topic = %challenge_topic,
                                error = %e,
                                "Failed to send heartbeat to capacity provider"
                            );
                        }
                    }
                }

                info!(
                    provider_count = capacity_validators.len(),
                    "Sent heartbeats to all registered capacity providers"
                );
            }
        })
    }

    /// Send `ContentSyncHeartbeat` every 5 seconds on `P2P_TOPIC_CONTENT_SYNC`.
    pub fn start_content_sync_heartbeat_task(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
            interval.tick().await;

            info!(
                "Content sync heartbeat task started (sending ContentSyncHeartbeat every 5 seconds to {} topic)",
                P2P_TOPIC_CONTENT_SYNC
            );

            loop {
                interval.tick().await;

                let timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                let content_sync_topic = P2P_TOPIC_CONTENT_SYNC;
                let heartbeat_msg = SyncMsg::ContentSyncHeartbeat { timestamp };

                match self.publish_to_topic(content_sync_topic, heartbeat_msg) {
                    Ok(_) => {
                        info!(
                            topic = %content_sync_topic,
                            timestamp = timestamp,
                            "Sent ContentSyncHeartbeat to {} topic",
                            content_sync_topic
                        );
                    }
                    Err(e) => {
                        warn!(
                            topic = %content_sync_topic,
                            error = %e,
                            "Failed to send ContentSyncHeartbeat to {} topic",
                            content_sync_topic
                        );
                    }
                }
            }
        })
    }
}
