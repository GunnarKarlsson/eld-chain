//! Namespace registry HTTP types for namespace REST endpoints.
//!
//! Wire fields that are Eld account addresses stay as hex `String`s. Convert from
//! [`crate::namespace::NamespaceRecord`]'s typed [`crate::address::Address`] only at
//! these constructors.

use crate::error::EldError;
use crate::namespace::{slug_to_namespace_cadopath, NamespaceRecord};
use serde::{Deserialize, Serialize};

/// JSON body when the namespace is registered (`200`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NamespaceRegisteredResponse {
    pub registered: bool,
    pub namespace_slug: String,
    pub scope: String,
    pub owner: String,
    pub registered_height: u64,
    pub registry_path: String,
}

impl NamespaceRegisteredResponse {
    /// Builds the REST DTO; formats `owner` as canonical `0x` hex at the wire edge.
    pub fn from_record(record: &NamespaceRecord) -> Result<Self, EldError> {
        let registry_path = slug_to_namespace_cadopath(&record.namespace_slug)?;
        Ok(Self {
            registered: true,
            namespace_slug: record.namespace_slug.clone(),
            scope: format!("@{}", record.namespace_slug),
            owner: record.owner.hex_with_prefix(),
            registered_height: record.registered_height,
            registry_path: registry_path.as_str().to_string(),
        })
    }
}

/// JSON body when the namespace is not registered (`404`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NamespaceNotRegisteredResponse {
    pub registered: bool,
    pub namespace_slug: String,
}

impl NamespaceNotRegisteredResponse {
    /// Builds the 404 body for an unregistered slug.
    pub fn from_slug(namespace_slug: String) -> Self {
        Self {
            registered: false,
            namespace_slug,
        }
    }
}

/// One registered namespace in `GET /v1/namespaces` list responses.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NamespaceListItem {
    pub namespace_slug: String,
    pub scope: String,
    pub owner: String,
    pub registered_height: u64,
    pub registry_path: String,
}

impl NamespaceListItem {
    /// Builds a list row; formats `owner` as canonical `0x` hex at the wire edge.
    pub fn from_record(record: &NamespaceRecord) -> Result<Self, EldError> {
        let registry_path = slug_to_namespace_cadopath(&record.namespace_slug)?;
        Ok(Self {
            namespace_slug: record.namespace_slug.clone(),
            scope: format!("@{}", record.namespace_slug),
            owner: record.owner.hex_with_prefix(),
            registered_height: record.registered_height,
            registry_path: registry_path.as_str().to_string(),
        })
    }
}

/// Pagination metadata for `GET /v1/namespaces`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NamespaceListPagination {
    pub limit: u32,
    pub has_next: bool,
    pub total: Option<u64>,
}

/// Response body for `GET /v1/namespaces`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NamespaceListResponse {
    pub namespaces: Vec<NamespaceListItem>,
    pub pagination: NamespaceListPagination,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::address::Address;

    fn sample_record() -> NamespaceRecord {
        NamespaceRecord {
            namespace_slug: "peter".to_string(),
            owner: Address::parse_hex_str("0xe17404c417fa10cc04fdf73604fcacca8d0a687c")
                .expect("owner"),
            registered_height: 42,
        }
    }

    #[test]
    fn registered_response_formats_owner_as_prefixed_hex() {
        let resp = NamespaceRegisteredResponse::from_record(&sample_record()).expect("from_record");
        assert!(resp.registered);
        assert_eq!(resp.namespace_slug, "peter");
        assert_eq!(resp.scope, "@peter");
        assert_eq!(resp.owner, "0xe17404c417fa10cc04fdf73604fcacca8d0a687c");
        assert_eq!(resp.registered_height, 42);
        assert!(resp.registry_path.contains("peter"));

        let json = serde_json::to_string(&resp).expect("serialize");
        assert!(json.contains("0xe17404c417fa10cc04fdf73604fcacca8d0a687c"));
    }

    #[test]
    fn list_item_formats_owner_as_prefixed_hex() {
        let item = NamespaceListItem::from_record(&sample_record()).expect("from_record");
        assert_eq!(item.owner, "0xe17404c417fa10cc04fdf73604fcacca8d0a687c");
        assert_eq!(item.scope, "@peter");
    }

    #[test]
    fn not_registered_response_from_slug() {
        let body = NamespaceNotRegisteredResponse::from_slug("missing".to_string());
        assert!(!body.registered);
        assert_eq!(body.namespace_slug, "missing");
    }
}
