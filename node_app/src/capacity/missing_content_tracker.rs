use eld_common::error::EldError;
use eld_common::missing_content::MissingContentRecord;
use eld_common::ContentId;
use eld_common::ManifestId;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::info;

/// File-based missing content tracker
/// Stores missing content records in a JSON file
pub struct MissingContentTracker {
    records: HashMap<String, MissingContentRecord>, // manifest_id -> record
    file_path: PathBuf,
}

impl MissingContentTracker {
    fn with_0x_prefix(input: &str) -> String {
        if input.starts_with("0x") {
            input.to_string()
        } else {
            format!("0x{input}")
        }
    }

    fn parse_content_id(input: &str) -> Result<ContentId, EldError> {
        Self::with_0x_prefix(input).parse::<ContentId>()
    }

    /// Create a new missing content tracker
    pub fn new(capacity_dir: &Path) -> Self {
        let file_path = capacity_dir.join("missing-content.json");
        Self {
            records: HashMap::new(),
            file_path,
        }
    }

    /// Load missing content records from disk
    pub fn load(&mut self) -> Result<(), EldError> {
        if !self.file_path.exists() {
            info!(
                missing_content_file = %self.file_path.display(),
                "Missing content file does not exist, starting with empty tracker"
            );
            return Ok(());
        }

        let file_content =
            std::fs::read_to_string(&self.file_path).map_err(|e| EldError::StorageError {
                operation: "load_missing_content".to_string(),
                details: format!("Failed to read missing content file: {e}"),
            })?;

        let records: HashMap<String, MissingContentRecord> = serde_json::from_str(&file_content)
            .map_err(|e| EldError::StorageError {
                operation: "load_missing_content".to_string(),
                details: format!("Failed to deserialize missing content file: {e}"),
            })?;

        info!(
            missing_content_file = %self.file_path.display(),
            record_count = records.len(),
            "Loaded missing content records from disk"
        );

        self.records = records;
        Ok(())
    }

    /// Save missing content records to disk
    pub fn save(&self) -> Result<(), EldError> {
        // Ensure parent directory exists
        if let Some(parent) = self.file_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| EldError::StorageError {
                operation: "save_missing_content".to_string(),
                details: format!("Failed to create directory: {e}"),
            })?;
        }

        let file_content =
            serde_json::to_string_pretty(&self.records).map_err(|e| EldError::StorageError {
                operation: "save_missing_content".to_string(),
                details: format!("Failed to serialize missing content: {e}"),
            })?;

        std::fs::write(&self.file_path, file_content).map_err(|e| EldError::StorageError {
            operation: "save_missing_content".to_string(),
            details: format!("Failed to write missing content file: {e}"),
        })?;

        info!(
            missing_content_file = %self.file_path.display(),
            record_count = self.records.len(),
            "Saved missing content records to disk"
        );

        Ok(())
    }

    /// Remove content from missing list
    pub fn mark_content_synced(&mut self, manifest_id: &str) -> Result<(), EldError> {
        if self.records.remove(manifest_id).is_some() {
            self.save()?;
            info!(manifest_id = %manifest_id, "Removed content from missing list");
        }
        Ok(())
    }

    /// Find and remove missing content by content_id
    pub fn mark_content_synced_by_content_id(&mut self, content_id: &str) -> Result<(), EldError> {
        let needle = Self::parse_content_id(content_id)?;
        let mut found = false;
        self.records.retain(|manifest_id, record| {
            if record.content_id == needle {
                found = true;
                info!(
                    manifest_id = %manifest_id,
                    content_id = %content_id,
                    "Removed content from missing list after successful sync"
                );
                false // Remove this record
            } else {
                true // Keep this record
            }
        });

        if found {
            self.save()?;
        }

        Ok(())
    }

    /// Add or update a missing entry by content key.
    /// Uses the content id bytes as a synthetic manifest id for pinboard sync flow.
    pub fn upsert_missing_content_by_content_id(
        &mut self,
        content_id: &str,
    ) -> Result<(), EldError> {
        let content_id = Self::parse_content_id(content_id)?;
        let manifest_id = ManifestId::new(*content_id.as_bytes());
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| EldError::StorageError {
                operation: "missing_content_now".to_string(),
                details: format!("Failed to get system time: {e}"),
            })?
            .as_secs();

        let key = manifest_id.to_string();
        let mut record = self
            .records
            .get(&key)
            .cloned()
            .unwrap_or(MissingContentRecord {
                manifest_id,
                content_id,
                block_height: 0,
                discovered_at: now,
                attempt_count: 0,
                last_attempt: None,
            });
        record.last_attempt = Some(now);
        self.records.insert(key, record);
        self.save()?;
        Ok(())
    }

    /// Get all missing content records
    pub fn get_all_missing_content(&self) -> Result<Vec<MissingContentRecord>, EldError> {
        Ok(self.records.values().cloned().collect())
    }

    /// Update an existing missing content record
    pub fn update_missing_content_record(
        &mut self,
        record: &MissingContentRecord,
    ) -> Result<(), EldError> {
        self.records
            .insert(record.manifest_id.to_string(), record.clone());
        self.save()?;
        Ok(())
    }
}
