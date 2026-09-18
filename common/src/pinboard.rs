use crate::constants::{abci_query, cado};
use crate::error::EldError;
use crate::namespace::validate_content_upload_namespace_slug;
use crate::Address;
use serde::{Deserialize, Serialize};

/// Secondary pinboard metadata key for custom-namespace content: `/@{namespace}/{message_id}`.
pub fn pinboard_namespace_content_path(namespace: &str, message_id: &str) -> String {
    format!("/@{namespace}/{message_id}")
}

/// True when REST/storage should use legacy Eld pinboard lookup (`/@eld/pinboard/post/...`).
pub fn path_uses_eld_pinboard_lookup(path: &str) -> bool {
    path.contains(cado::SCOPE_ELD_ROOT)
}

/// Standard Eld pinboard post path: `/@eld/pinboard/post/{wallet}/{message_id}`.
pub fn pinboard_eld_post_cado_path(wallet: &str, message_id: &str) -> String {
    format!(
        "{}{}/{}/{}",
        cado::PATH_PREFIX_PINBOARD,
        abci_query::PINBOARD_SEGMENT_POST,
        wallet,
        message_id
    )
}

/// Response `cado_path` / `content_path` for a committed post.
pub fn pinboard_response_cado_path(meta: &PinboardMessageMetadata) -> String {
    if let Some(ns) = meta.namespace.as_ref() {
        pinboard_namespace_content_path(ns, &meta.message_id)
    } else {
        pinboard_eld_post_cado_path(&meta.original_signer.hex_with_prefix(), &meta.message_id)
    }
}

/// Parses `/@{namespace}/{message_id}` (non-Eld paths only). Returns canonical `(namespace, message_id)`.
pub fn parse_pinboard_namespace_content_path(path: &str) -> Result<(String, String), EldError> {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() != 2 {
        return Err(EldError::ValidationError {
            field: "path".to_string(),
            value: path.to_string(),
            details: "Expected /@{namespace}/{message_id}".to_string(),
        });
    }

    let scope = parts[0];
    if !scope.starts_with('@') || scope.len() < 2 {
        return Err(EldError::ValidationError {
            field: "path".to_string(),
            value: path.to_string(),
            details: "Namespace scope must start with @".to_string(),
        });
    }
    let namespace_slug = &scope[1..];
    validate_content_upload_namespace_slug(namespace_slug)?;

    if parts[1].is_empty() {
        return Err(EldError::ValidationError {
            field: "path".to_string(),
            value: path.to_string(),
            details: "message_id cannot be empty".to_string(),
        });
    }

    Ok((namespace_slug.to_string(), parts[1].to_string()))
}

/// Persisted metadata for a pinboard post (PostMessage).
///
/// This record is intentionally small and does not contain the message bytes; it is used for
/// indexing, TTL/expiry checks, and blob refcount management.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PinboardMessageMetadata {
    /// Unique logical post id (commitment hash).
    pub message_id: String,
    /// Original user wallet address (0x + lowercase hex).
    pub original_signer: Address,
    /// Content-addressed blob identity: hex(SHA-256(message_bytes)).
    pub content_key: String,
    /// MIME type for the message body (must be one of the pinboard allowlist values).
    pub content_type: String,
    /// Block height after which the post is expired.
    pub expires_height: u64,
    /// Visibility policy string (currently application-defined).
    pub visibility: String,
    /// Optional topic string (currently application-defined).
    pub topic: Option<String>,
    /// Application-level filter tags (canonicalized; max 4, each max 64 bytes in tx validation).
    pub tags: Vec<String>,
    /// Block height at which the message was committed (for ordering/indexes).
    pub committed_height: u64,
    /// Timestamp recorded by the validator at submission time (for UI).
    pub received_timestamp: u64,
    /// Optional custom namespace slug (letter-only canonical form); set when upload used a namespace.
    #[serde(default)]
    pub namespace: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Address;

    #[test]
    fn pinboard_namespace_content_path_format() {
        assert_eq!(
            pinboard_namespace_content_path("captainhook", "abc123"),
            "/@captainhook/abc123"
        );
    }

    #[test]
    fn path_uses_eld_pinboard_lookup_detects_eld_scope() {
        assert!(path_uses_eld_pinboard_lookup(
            "/@eld/pinboard/post/0xabc/msg1"
        ));
        assert!(!path_uses_eld_pinboard_lookup("/@captainhook/msg1"));
    }

    #[test]
    fn parse_pinboard_namespace_content_path_valid() {
        let (ns, mid) =
            parse_pinboard_namespace_content_path("/@captainhook/deadbeef").expect("parse");
        assert_eq!(ns, "captainhook");
        assert_eq!(mid, "deadbeef");
    }

    #[test]
    fn pinboard_metadata_serde_roundtrip_with_namespace() {
        let owner =
            Address::parse_hex_str("0xe17404c417fa10cc04fdf73604fcacca8d0a687c").expect("address");
        let meta = PinboardMessageMetadata {
            message_id: "mid".to_string(),
            original_signer: owner,
            content_key: "ck".to_string(),
            content_type: "text/plain".to_string(),
            expires_height: 100,
            visibility: "public".to_string(),
            topic: None,
            tags: vec![],
            committed_height: 10,
            received_timestamp: 1,
            namespace: Some("peterpan".to_string()),
        };
        let json = serde_json::to_string(&meta).expect("serialize");
        let decoded: PinboardMessageMetadata = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(meta, decoded);
    }
}
