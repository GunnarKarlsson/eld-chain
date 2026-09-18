use crate::content_id::ContentId;
use crate::manifest_id::ManifestId;
use serde::{Deserialize, Serialize};

/// Record for tracking missing content in the sync system.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MissingContentRecord {
    pub manifest_id: ManifestId,
    pub content_id: ContentId,
    /// Block height where the content was encountered.
    pub block_height: u64,
    /// Timestamp when first discovered.
    pub discovered_at: u64,
    /// Number of fetch attempts.
    pub attempt_count: u32,
    /// Timestamp of last fetch attempt.
    pub last_attempt: Option<u64>,
}
