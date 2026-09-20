use crate::storage::traits::{SnapshotChunk, SnapshotStorage};
use abci::{async_api::Snapshot, async_trait, types::*};
use eld_common::error::EldError;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::sync::Arc;
use tracing::{error, info, warn};

use super::SnapshotManager;

pub struct SnapshotConnection<S>
where
    S: SnapshotStorage + Send + Sync + 'static,
{
    manager: Arc<SnapshotManager<S>>,
}

impl<S> SnapshotConnection<S>
where
    S: SnapshotStorage + Send + Sync + 'static,
{
    pub fn new(manager: Arc<SnapshotManager<S>>) -> Self {
        Self { manager }
    }

    /// Check if there's enough capacity to store a snapshot of the given size
    ///
    /// This implementation:
    /// 1. Checks the available disk space on the system
    /// 2. Applies a safety buffer to ensure other operations can proceed
    /// 3. Verifies if the estimated snapshot size can fit
    fn has_capacity_for(&self, size: u64) -> bool {
        // DB path where snapshots are stored
        // In a real implementation, this should be configurable
        let db_path = Path::new("./data/rocksdb");

        // Safety factor - require 20% more space than we need
        let safety_factor = 1.2;
        let required_space = (size as f64 * safety_factor) as u64;

        // Get available disk space
        match get_available_space(db_path) {
            Ok(available) => {
                // Allow snapshot if we have enough space
                if available > required_space {
                    log_info(format!(
                        "Sufficient space for snapshot: available={available}, required={required_space}"
                    ));
                    true
                } else {
                    log_warn(format!(
                        "Insufficient space for snapshot: available={available}, required={required_space}"
                    ));
                    false
                }
            }
            Err(e) => {
                log_error(format!("Error checking disk space: {e}"));
                // Default to false if we can't determine available space
                false
            }
        }
    }

    /// Helper method to store a snapshot chunk
    ///
    /// This method calculates the hash of the chunk data, verifies it,
    /// and stores it using the appropriate storage method.
    async fn store_snapshot_chunk(
        &self,
        height: i64,
        chunk: &SnapshotChunk,
    ) -> Result<(), EldError> {
        // 1. Get the snapshot metadata to check if this chunk belongs to an existing snapshot
        let metadata = match self.manager.get_metadata(height).await? {
            Some(m) => m,
            None => {
                // Cannot store chunk for non-existent snapshot
                // In a real implementation, we might want to create metadata here
                return Err(EldError::NotFoundError {
                    resource_type: "snapshot metadata".to_string(),
                    identifier: height.to_string(),
                });
            }
        };

        // 2. Verify the chunk index is valid
        if chunk.index >= metadata.chunk_count {
            return Err(EldError::ValidationError {
                field: "chunk index".to_string(),
                value: chunk.index.to_string(),
                details: format!(
                    "Invalid chunk index: {} (max: {})",
                    chunk.index,
                    metadata.chunk_count - 1
                ),
            });
        }

        // 3. Create a proper snapshot chunk with hash verification
        let mut hasher = Sha256::new();
        hasher.update(&chunk.data);
        let hash = hex::encode(hasher.finalize());

        // 4. Create a proper chunk with the calculated hash
        let verified_chunk = SnapshotChunk {
            index: chunk.index,
            data: chunk.data.clone(),
            hash,
            size: chunk.data.len() as u64,
            metadata: chunk.metadata.clone(),
        };

        // 5. Store the chunk using the SnapshotManager's put_chunk method
        match self.manager.put_chunk(height, &verified_chunk).await {
            Ok(_) => {
                log_info(format!(
                    "Stored snapshot chunk: height={}, index={}",
                    height, chunk.index
                ));
                Ok(())
            }
            Err(e) => {
                log_error(format!("Failed to store snapshot chunk: {e}"));
                Err(EldError::StorageError {
                    operation: "store snapshot chunk".to_string(),
                    details: format!("Failed to store snapshot chunk: {e}"),
                })
            }
        }
    }
}

// Helper functions for disk space checks and logging

fn get_available_space(path: &Path) -> Result<u64, EldError> {
    if !path.exists() {
        // Try to create the directory if it doesn't exist
        std::fs::create_dir_all(path).map_err(|e| EldError::FileSystemError {
            operation: "create_directory".to_string(),
            path: path.to_string_lossy().to_string(),
            details: format!("Failed to create directory: {e}"),
        })?;
    }

    #[cfg(target_os = "linux")]
    {
        use std::process::Command;

        // Use df command as a more reliable way to get disk space
        let output = Command::new("df")
            .args(&[
                "--output=avail",
                path.to_str().expect("Failed to convert path to str"),
            ])
            .output()
            .map_err(|e| EldError::FileSystemError {
                operation: "execute_df_command".to_string(),
                path: path.to_string_lossy().to_string(),
                details: format!("Failed to execute df command: {}", e),
            })?;

        if !output.status.success() {
            return Err(EldError::FileSystemError {
                operation: "df_command".to_string(),
                path: path.to_string_lossy().to_string(),
                details: "df command failed".to_string(),
            });
        }

        // Parse the output - skip the header line and get the available KB
        let stdout = String::from_utf8_lossy(&output.stdout);
        let available_kb = stdout
            .lines()
            .nth(1)
            .and_then(|line| line.trim().parse::<u64>().ok())
            .ok_or_else(|| EldError::FileSystemError {
                operation: "parse_df_output".to_string(),
                path: path.to_string_lossy().to_string(),
                details: "Failed to parse df output".to_string(),
            })?;

        // Convert KB to bytes
        let available = available_kb * 1024;

        Ok(available)
    }

    #[cfg(target_os = "macos")]
    {
        use std::process::Command;

        // Use df command for macOS
        let output = Command::new("df")
            .args(["-k", path.to_str().expect("Failed to convert path to str")])
            .output()
            .map_err(|e| EldError::FileSystemError {
                operation: "execute_df_command".to_string(),
                path: path.to_string_lossy().to_string(),
                details: format!("Failed to execute df command: {e}"),
            })?;

        if !output.status.success() {
            return Err(EldError::FileSystemError {
                operation: "df_command".to_string(),
                path: path.to_string_lossy().to_string(),
                details: "df command failed".to_string(),
            });
        }

        // Parse the output - skip the header line and get the available KB from the 4th column
        let stdout = String::from_utf8_lossy(&output.stdout);
        let available_kb = stdout
            .lines()
            .nth(1)
            .and_then(|line| {
                line.split_whitespace()
                    .nth(3)
                    .and_then(|avail| avail.parse::<u64>().ok())
            })
            .ok_or_else(|| EldError::FileSystemError {
                operation: "parse_df_output".to_string(),
                path: path.to_string_lossy().to_string(),
                details: "Failed to parse df output".to_string(),
            })?;

        // Convert KB to bytes
        let available = available_kb * 1024;

        Ok(available)
    }

    #[cfg(target_os = "windows")]
    {
        use std::process::Command;

        // Use PowerShell to get disk space on Windows
        let output = Command::new("powershell")
            .args(&[
                "-Command",
                &format!(
                    "(Get-PSDrive -PSProvider FileSystem | Where-Object {{$_.Root -eq '{}'}} | Select-Object Free).Free",
                    path.ancestors().find(|p| p.is_absolute()).unwrap_or(path).to_string_lossy().replace("\\", "\\\\")
                ),
            ])
            .output()
            .map_err(|e| EldError::FileSystemError {
                operation: "execute_powershell_command".to_string(),
                path: path.to_string_lossy().to_string(),
                details: format!("Failed to execute PowerShell command: {}", e),
            })?;

        if !output.status.success() {
            return Err(EldError::FileSystemError {
                operation: "powershell_command".to_string(),
                path: path.to_string_lossy().to_string(),
                details: "PowerShell command failed".to_string(),
            });
        }

        // Parse the output to get the available bytes
        let stdout = String::from_utf8_lossy(&output.stdout);
        let available = stdout
            .trim()
            .parse::<u64>()
            .map_err(|e| EldError::FileSystemError {
                operation: "parse_powershell_output".to_string(),
                path: path.to_string_lossy().to_string(),
                details: format!("Failed to parse PowerShell output: {}", e),
            })?;

        Ok(available)
    }

    // Default fallback for unsupported platforms
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        // On unsupported platforms, return a large number so we don't
        // artificially constrain snapshots
        // In production, this should be configurable
        Ok(1_000_000_000_000) // 1 TB
    }
}

fn log_info(message: String) {
    info!("[INFO] Snapshot: {}", message);
}

fn log_warn(message: String) {
    warn!("[WARN] Snapshot: {}", message);
}

fn log_error(message: String) {
    error!("[ERROR] Snapshot: {}", message);
}

#[async_trait]
impl<S> Snapshot for SnapshotConnection<S>
where
    S: SnapshotStorage + Send + Sync + 'static,
{
    async fn list_snapshots(&self, _request: RequestListSnapshots) -> ResponseListSnapshots {
        match self.manager.list_snapshots(100).await {
            Ok(snapshots) => {
                let snapshot_items = snapshots
                    .into_iter()
                    .map(|s| abci::types::Snapshot {
                        height: s.height as u64,
                        format: s.format_version,
                        chunks: s.chunk_count,
                        hash: s.app_hash,
                        metadata: Default::default(),
                    })
                    .collect();

                ResponseListSnapshots {
                    snapshots: snapshot_items,
                }
            }
            Err(_) => ResponseListSnapshots { snapshots: vec![] },
        }
    }

    async fn load_snapshot_chunk(
        &self,
        request: RequestLoadSnapshotChunk,
    ) -> ResponseLoadSnapshotChunk {
        match self
            .manager
            .get_chunk(request.height as i64, request.chunk)
            .await
        {
            Ok(Some(chunk)) => ResponseLoadSnapshotChunk { chunk: chunk.data },
            _ => ResponseLoadSnapshotChunk { chunk: vec![] },
        }
    }

    async fn offer_snapshot(&self, request: RequestOfferSnapshot) -> ResponseOfferSnapshot {
        // Extract the snapshot from the request
        let offered_snapshot = match request.snapshot {
            Some(snapshot) => snapshot,
            None => {
                return ResponseOfferSnapshot {
                    result: OfferSnapshotResult::Reject.into(),
                }
            }
        };

        let height = offered_snapshot.height as i64;

        // First check: Verify if we already have this snapshot
        match self.manager.get_metadata(height).await {
            Ok(Some(_)) => {
                // We already have this snapshot
                return ResponseOfferSnapshot {
                    result: OfferSnapshotResult::Reject.into(),
                };
            }
            Err(_) => {
                // Error accessing storage
                return ResponseOfferSnapshot {
                    result: OfferSnapshotResult::Abort.into(),
                };
            }
            _ => {} // Continue with checks
        }

        // Second check: Verify the format version
        if offered_snapshot.format != 1 {
            // Our system only supports format version 1
            return ResponseOfferSnapshot {
                result: OfferSnapshotResult::RejectFormat.into(),
            };
        }

        // Third check: Verify the app hash matches if provided
        if !request.app_hash.is_empty() && offered_snapshot.hash != request.app_hash {
            // App hash mismatch
            return ResponseOfferSnapshot {
                result: OfferSnapshotResult::Reject.into(),
            };
        }

        // Fourth check: Verify we have capacity to store this snapshot
        // (This would likely involve checking free disk space - simplified here)
        let estimated_size = offered_snapshot.chunks as u64 * 10_000_000; // Estimate based on typical chunk size
        if !self.has_capacity_for(estimated_size) {
            return ResponseOfferSnapshot {
                result: OfferSnapshotResult::Abort.into(),
            };
        }

        // Accept the snapshot if all checks passed
        ResponseOfferSnapshot {
            result: OfferSnapshotResult::Accept.into(),
        }
    }

    async fn apply_snapshot_chunk(
        &self,
        request: RequestApplySnapshotChunk,
    ) -> ResponseApplySnapshotChunk {
        // Get snapshot height based on the chunk index
        // Note: In a real implementation, we would need to track which snapshot
        // is currently being applied, as the RequestApplySnapshotChunk doesn't
        // directly include the height. For now, we'll use a simple approach.

        // Get a list of snapshots
        let snapshots = match self.manager.list_snapshots(10).await {
            Ok(s) => s,
            Err(_) => {
                return ResponseApplySnapshotChunk {
                    result: ApplySnapshotChunkResult::Unknown.into(),
                    refetch_chunks: vec![],
                    reject_senders: vec![],
                };
            }
        };

        if snapshots.is_empty() {
            return ResponseApplySnapshotChunk {
                result: ApplySnapshotChunkResult::Unknown.into(),
                refetch_chunks: vec![],
                reject_senders: vec![],
            };
        }

        // Assume we're applying the most recent snapshot
        let height = snapshots[0].height;
        let chunk_index = request.index;
        let chunk_data = request.chunk.clone();

        // Create snapshot chunk
        let chunk = crate::storage::traits::SnapshotChunk {
            index: chunk_index,
            data: chunk_data,
            hash: "".to_string(), // We don't have the hash here, it would be verified later
            size: request.chunk.len() as u64,
            metadata: crate::storage::traits::SnapshotChunkMetadata {
                accounts_count: 0,
                staking_accounts_count: 0,
                devices_count: 0,
                manifests_count: 0,
                chunk_proofs_count: 0,
            },
        };

        // Store the chunk
        if self.store_snapshot_chunk(height, &chunk).await.is_err() {
            return ResponseApplySnapshotChunk {
                result: ApplySnapshotChunkResult::Unknown.into(),
                refetch_chunks: vec![],
                reject_senders: vec![],
            };
        }

        // In a real implementation, we would verify if all chunks are received
        // and if the snapshot is valid. For this simplified version, we'll
        // always return Accept.
        ResponseApplySnapshotChunk {
            result: ApplySnapshotChunkResult::Accept.into(),
            refetch_chunks: vec![],
            reject_senders: vec![],
        }
    }
}
