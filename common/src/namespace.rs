//! On-chain namespace registry: slug rules and [`NamespaceRecord`] CADO payload.

use crate::address::Address;
use crate::cado::{CadoPath, CadoPathKey, CadoType, DeserializableBin};
use crate::error::EldError;
use bincode;
use serde::{Deserialize, Serialize};

const SLUG_MIN_LEN: usize = 3;
const SLUG_MAX_LEN: usize = 32;

/// Immutable CADO payload at `/@eld/namespace/{namespace_slug}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceRecord {
    /// Canonical slug (matches the path name segment).
    pub namespace_slug: String,
    /// Registrant address at registration time.
    pub owner: Address,
    /// Block height when the namespace was registered (`block_height + 1` at deliver).
    pub registered_height: u64,
}

impl NamespaceRecord {
    /// Serializes this record for immutable CADO storage.
    pub fn serialize_bin(&self) -> Result<Vec<u8>, EldError> {
        bincode::serialize(self).map_err(|e| EldError::StorageError {
            operation: "serialize_namespace_record".to_string(),
            details: format!("Failed to serialize NamespaceRecord: {e}"),
        })
    }

    /// Deserializes from CADO payload bytes.
    pub fn deserialize_bin(data: &[u8]) -> Result<Self, EldError> {
        bincode::deserialize(data).map_err(|e| EldError::StorageError {
            operation: "deserialize_namespace_record".to_string(),
            details: format!("Failed to deserialize NamespaceRecord: {e}"),
        })
    }
}

impl DeserializableBin for NamespaceRecord {
    fn deserialize_bin(data: &[u8]) -> Result<Self, EldError> {
        NamespaceRecord::deserialize_bin(data)
    }
}

/// Normalizes user input to a canonical slug (trim + lowercase), then validates format.
pub fn normalize_namespace_slug(input: &str) -> Result<String, EldError> {
    let slug = input.trim().to_ascii_lowercase();
    validate_namespace_slug(&slug)?;
    Ok(slug)
}

/// Validates canonical slug format (does not check reserved names).
pub fn validate_namespace_slug(slug: &str) -> Result<(), EldError> {
    if slug.len() < SLUG_MIN_LEN || slug.len() > SLUG_MAX_LEN {
        return Err(EldError::ValidationError {
            field: "namespace_slug".to_string(),
            value: slug.to_string(),
            details: format!(
                "Namespace slug length must be between {SLUG_MIN_LEN} and {SLUG_MAX_LEN} characters"
            ),
        });
    }

    if slug.starts_with('-') || slug.ends_with('-') {
        return Err(EldError::ValidationError {
            field: "namespace_slug".to_string(),
            value: slug.to_string(),
            details: "Namespace slug must not start or end with '-'".to_string(),
        });
    }

    if slug.contains("--") {
        return Err(EldError::ValidationError {
            field: "namespace_slug".to_string(),
            value: slug.to_string(),
            details: "Namespace slug must not contain consecutive '-'".to_string(),
        });
    }

    if !slug
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(EldError::ValidationError {
            field: "namespace_slug".to_string(),
            value: slug.to_string(),
            details: "Namespace slug may only contain lowercase letters, digits, and '-'"
                .to_string(),
        });
    }

    Ok(())
}

/// Validates content-upload namespace format: ASCII lowercase letters `a`–`z` only, length 3–32.
///
/// Stricter than [`validate_namespace_slug`] (no digits or hyphens). Input must already be
/// trimmed and lowercased (see [`normalize_content_upload_namespace`]).
pub fn validate_content_upload_namespace_slug(slug: &str) -> Result<(), EldError> {
    if slug.len() < SLUG_MIN_LEN || slug.len() > SLUG_MAX_LEN {
        return Err(EldError::ValidationError {
            field: "namespace".to_string(),
            value: slug.to_string(),
            details: format!(
                "Namespace length must be between {SLUG_MIN_LEN} and {SLUG_MAX_LEN} characters"
            ),
        });
    }

    if !slug.chars().all(|c| c.is_ascii_lowercase()) {
        return Err(EldError::ValidationError {
            field: "namespace".to_string(),
            value: slug.to_string(),
            details: "Namespace may only contain lowercase letters a-z".to_string(),
        });
    }

    Ok(())
}

/// Normalizes content-upload namespace input: trim + ASCII lowercase only (no format validation).
pub fn normalize_content_upload_namespace(input: &str) -> String {
    input.trim().to_ascii_lowercase()
}

/// Resolves optional upload namespace: `None` or empty/whitespace → `None`, else canonical slug.
pub fn resolve_optional_namespace(input: Option<String>) -> Result<Option<String>, EldError> {
    match input {
        None => Ok(None),
        Some(s) if s.trim().is_empty() => Ok(None),
        Some(s) => {
            let normalized = normalize_content_upload_namespace(&s);
            validate_content_upload_namespace_slug(&normalized)?;
            Ok(Some(normalized))
        }
    }
}

/// When namespace is set, ensures it is registered and owned by `signer`.
pub fn validate_namespace_upload_authorization<F>(
    namespace: Option<String>,
    signer: Address,
    lookup: F,
) -> Result<Option<String>, EldError>
where
    F: FnOnce(String) -> Result<Option<NamespaceRecord>, EldError>,
{
    let Some(canonical) = resolve_optional_namespace(namespace)? else {
        return Ok(None);
    };

    let record = lookup(canonical.clone())?.ok_or_else(|| EldError::ValidationError {
        field: "namespace".to_string(),
        value: canonical.clone(),
        details: "Namespace is not registered".to_string(),
    })?;

    if record.owner != signer {
        return Err(EldError::ValidationError {
            field: "namespace".to_string(),
            value: canonical,
            details: "Signer is not the namespace owner".to_string(),
        });
    }

    Ok(Some(canonical))
}

/// Returns true if `namespace_slug` is reserved and cannot be registered.
pub fn is_reserved_namespace_slug(namespace_slug: &str) -> bool {
    RESERVED_NAMESPACE_SLUGS.contains(&namespace_slug)
}

/// Maps a slug to the on-chain namespace registration [`CadoPath`] (`/@eld/namespace/{slug}`).
///
/// Normalizes and validates the slug, rejects reserved names. Does not check registration.
pub fn slug_to_namespace_cadopath(slug: &str) -> Result<CadoPath, EldError> {
    let canonical = normalize_namespace_slug(slug)?;
    if is_reserved_namespace_slug(&canonical) {
        return Err(EldError::ValidationError {
            field: "namespace_slug".to_string(),
            value: canonical,
            details: "Namespace slug is reserved".to_string(),
        });
    }
    CadoPath::new(CadoType::Namespace, CadoPathKey::NamespaceSlug(&canonical))
}

/// Scope names without `@` (see [`crate::constants::cado::ALLOWED_SCOPES`]), plus fixed reserved slugs.
const RESERVED_NAMESPACE_SLUGS: &[&str] = &[
    "eld",
    "user",
    "public",
    "contract",
    "test",
    "other",
    "pinboard",
    "admin",
    "namespace",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::cado::PATH_PREFIX_NAMESPACE_REGISTRY;

    #[test]
    fn normalize_namespace_slug_lowercases_and_trims() {
        assert_eq!(normalize_namespace_slug("  Peter  ").unwrap(), "peter");
    }

    #[test]
    fn validate_namespace_slug_accepts_valid() {
        assert!(validate_namespace_slug("peter").is_ok());
        assert!(validate_namespace_slug("a-b-c").is_ok());
        assert!(validate_namespace_slug("abc").is_ok());
        assert!(validate_namespace_slug(&"a".repeat(32)).is_ok());
    }

    #[test]
    fn validate_namespace_slug_rejects_invalid() {
        assert!(validate_namespace_slug("ab").is_err());
        assert!(validate_namespace_slug(&"a".repeat(33)).is_err());
        assert!(validate_namespace_slug("-abc").is_err());
        assert!(validate_namespace_slug("abc-").is_err());
        assert!(validate_namespace_slug("a--b").is_err());
        assert!(validate_namespace_slug("UPPER").is_err());
        assert!(validate_namespace_slug("bad_slug").is_err());
    }

    #[test]
    fn reserved_namespace_slugs_include_scopes_and_fixed_names() {
        assert!(is_reserved_namespace_slug("eld"));
        assert!(is_reserved_namespace_slug("user"));
        assert!(is_reserved_namespace_slug("pinboard"));
        assert!(is_reserved_namespace_slug("namespace"));
        assert!(!is_reserved_namespace_slug("peter"));
    }

    #[test]
    fn slug_to_namespace_cadopath_builds_expected_path() {
        let path = slug_to_namespace_cadopath("peter").expect("path");
        assert_eq!(path.as_str(), "/@eld/namespace/peter");
        assert_eq!(
            path.as_str(),
            format!("{PATH_PREFIX_NAMESPACE_REGISTRY}peter")
        );
    }

    #[test]
    fn slug_to_namespace_cadopath_rejects_reserved() {
        assert!(slug_to_namespace_cadopath("eld").is_err());
        assert!(slug_to_namespace_cadopath("pinboard").is_err());
    }

    #[test]
    fn normalize_content_upload_namespace_trims_and_lowercases_only() {
        assert_eq!(normalize_content_upload_namespace("peterpan"), "peterpan");
        assert_eq!(
            normalize_content_upload_namespace("  peterPan  "),
            "peterpan"
        );
        assert_eq!(normalize_content_upload_namespace("peter-pan"), "peter-pan");
    }

    #[test]
    fn content_upload_namespace_letter_only_rules() {
        assert!(resolve_optional_namespace(Some("peterpan".to_string())).is_ok());
        assert!(resolve_optional_namespace(Some("peterPan".to_string())).is_ok());
        assert!(resolve_optional_namespace(Some("peter-pan".to_string())).is_err());
        assert!(resolve_optional_namespace(Some("peterpan1".to_string())).is_err());
        assert!(resolve_optional_namespace(Some("ab".to_string())).is_err());
    }

    #[test]
    fn resolve_optional_namespace_empty_skips() {
        assert_eq!(resolve_optional_namespace(None).unwrap(), None);
        assert_eq!(
            resolve_optional_namespace(Some("".to_string())).unwrap(),
            None
        );
        assert_eq!(
            resolve_optional_namespace(Some("  ".to_string())).unwrap(),
            None
        );
        assert_eq!(
            resolve_optional_namespace(Some("peter".to_string())).unwrap(),
            Some("peter".to_string())
        );
    }

    #[test]
    fn validate_namespace_upload_authorization_owner_and_registry() {
        let owner =
            Address::parse_hex_str("0xe17404c417fa10cc04fdf73604fcacca8d0a687c").expect("address");
        let other =
            Address::parse_hex_str("0x23b1f0b6199479b5d04fb54e21df14d51530b7b1").expect("address");
        let record = NamespaceRecord {
            namespace_slug: "peterpan".to_string(),
            owner,
            registered_height: 1,
        };

        let lookup_ok = |_| Ok(Some(record.clone()));
        assert_eq!(
            validate_namespace_upload_authorization(Some("peterpan".to_string()), owner, lookup_ok)
                .unwrap(),
            Some("peterpan".to_string())
        );

        let lookup_missing = |_| Ok(None);
        assert!(validate_namespace_upload_authorization(
            Some("peterpan".to_string()),
            owner,
            lookup_missing
        )
        .is_err());

        let lookup_ok2 = |_| Ok(Some(record.clone()));
        assert!(validate_namespace_upload_authorization(
            Some("peterpan".to_string()),
            other,
            lookup_ok2
        )
        .is_err());

        assert_eq!(
            validate_namespace_upload_authorization(None, owner, |_| {
                panic!("lookup must not run")
            })
            .unwrap(),
            None
        );
    }

    #[test]
    fn namespace_record_bincode_roundtrip() {
        let owner =
            Address::parse_hex_str("0xe17404c417fa10cc04fdf73604fcacca8d0a687c").expect("address");
        let record = NamespaceRecord {
            namespace_slug: "peter".to_string(),
            owner,
            registered_height: 42,
        };
        let bytes = record.serialize_bin().expect("serialize");
        let decoded = NamespaceRecord::deserialize_bin(&bytes).expect("deserialize");
        assert_eq!(record, decoded);
    }
}
