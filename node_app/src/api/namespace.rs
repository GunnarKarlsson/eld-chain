//! Namespace registry read API (`GET /v1/namespace/{namespace_slug}`, `GET /v1/namespaces`).

use super::{check_rate_limit, ApiEndpointType, ApiError, RateLimitState};
use crate::app_state::AppState;
use crate::storage::rocksdb::RocksDBStorage;
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use eld_client::api::rest::{
    NamespaceListItem, NamespaceListPagination, NamespaceListResponse,
    NamespaceNotRegisteredResponse, NamespaceRegisteredResponse,
};
use eld_common::cado::{CadoBody, CadoPath, CadoPathKey, CadoType};
use eld_common::{
    error::EldError,
    namespace::{normalize_namespace_slug, NamespaceRecord},
};
use serde::Deserialize;
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock as TokioRwLock;
use tracing::debug;

/// Query parameters for `GET /v1/namespaces`.
#[derive(Debug, Deserialize)]
pub struct NamespaceListParams {
    /// With `after_namespace_slug`, return namespaces strictly older than the cursor in list order.
    pub after_registered_height: Option<u64>,
    /// Must be paired with `after_registered_height` when continuing a listing.
    pub after_namespace_slug: Option<String>,
    pub limit: Option<u32>,
}

/// Resolves a namespace from committed RocksDB CADO (canonical slug already normalized).
pub fn lookup_namespace_record_from_storage(
    storage: &RocksDBStorage,
    canonical_slug: &str,
) -> Result<Option<NamespaceRecord>, EldError> {
    let path = CadoPath::new(
        CadoType::Namespace,
        CadoPathKey::NamespaceSlug(canonical_slug),
    )?;
    let Some(cado) = storage.get_cado_by_path(path)? else {
        return Ok(None);
    };
    let CadoBody::Immutable(immutable) = cado else {
        return Err(EldError::ValidationError {
            field: "namespace".to_string(),
            value: canonical_slug.to_string(),
            details: "Namespace registry CADO is not immutable".to_string(),
        });
    };
    let record = NamespaceRecord::deserialize_bin(immutable.data())?;
    Ok(Some(record))
}

/// Resolves a namespace: current block staging, committed app cache, then RocksDB.
pub fn lookup_namespace_record(
    current: Option<&AppState>,
    committed: &AppState,
    namespace_slug: &str,
) -> Result<Option<NamespaceRecord>, EldError> {
    if let Some(current) = current {
        if let Some(record) = current.envelope.resolve_namespace(namespace_slug)? {
            return Ok(Some(record));
        }
    }
    committed.envelope.resolve_namespace(namespace_slug)
}

/// Collects all registered namespaces from committed index and optional current-block staging.
pub fn collect_namespace_records(
    committed: &AppState,
    current: Option<&AppState>,
) -> Result<Vec<NamespaceRecord>, EldError> {
    let mut records = committed
        .envelope
        .collect_namespace_records(current.map(|state| &state.envelope))?;
    sort_namespace_records(&mut records);
    Ok(records)
}

/// Sorts by `registered_height` descending, then `namespace_slug` ascending.
pub fn sort_namespace_records(records: &mut [NamespaceRecord]) {
    records.sort_by(|a, b| {
        b.registered_height
            .cmp(&a.registered_height)
            .then_with(|| a.namespace_slug.cmp(&b.namespace_slug))
    });
}

/// Keeps rows strictly after `(after_height, after_slug)` in list order.
pub fn filter_namespace_records_after_cursor(
    records: &[NamespaceRecord],
    after_height: u64,
    after_slug: &str,
) -> Vec<NamespaceRecord> {
    records
        .iter()
        .filter(|record| {
            record.registered_height < after_height
                || (record.registered_height == after_height
                    && record.namespace_slug.as_str() > after_slug)
        })
        .cloned()
        .collect()
}

/// Returns up to `limit` rows and whether additional rows exist (`limit + 1` probe).
pub fn paginate_namespace_records(
    records: Vec<NamespaceRecord>,
    limit: u32,
) -> (Vec<NamespaceRecord>, bool) {
    let limit = limit as usize;
    let has_next = records.len() > limit;
    let page = records.into_iter().take(limit).collect();
    (page, has_next)
}

pub fn namespace_record_to_list_item(
    record: &NamespaceRecord,
) -> Result<NamespaceListItem, EldError> {
    NamespaceListItem::from_record(record)
}

fn parse_namespace_list_continuation(
    params: &NamespaceListParams,
) -> Result<Option<(u64, String)>, ApiError> {
    match (
        params.after_registered_height,
        params.after_namespace_slug.as_deref(),
    ) {
        (None, None) => Ok(None),
        (Some(height), Some(slug)) => {
            let canonical = normalize_namespace_slug(slug).map_err(|e| ApiError::BadRequest {
                message: "Invalid after_namespace_slug".to_string(),
                details: Some(e.to_string()),
            })?;
            Ok(Some((height, canonical)))
        }
        _ => Err(ApiError::BadRequest {
            message: "Invalid namespace list continuation".to_string(),
            details: Some(
                "Provide both after_registered_height and after_namespace_slug, or neither."
                    .to_string(),
            ),
        }),
    }
}

/// `GET /v1/namespace/{namespace_slug}`
pub async fn handle_get_namespace(
    State(rate_limit_state): State<Arc<TokioRwLock<RateLimitState>>>,
    State(committed_state): State<Arc<Mutex<AppState>>>,
    State(current_state): State<Arc<Mutex<Option<AppState>>>>,
    Path(namespace_slug): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    check_rate_limit(&rate_limit_state, ApiEndpointType::General, &headers).await?;

    let canonical =
        normalize_namespace_slug(&namespace_slug).map_err(|e| ApiError::BadRequest {
            message: "Invalid namespace_slug".to_string(),
            details: Some(e.to_string()),
        })?;

    let current_snapshot = current_state
        .lock()
        .map_err(|_| ApiError::InternalServerError {
            message: "Failed to read current application state".to_string(),
            details: None,
        })?
        .clone();

    let committed = committed_state
        .lock()
        .map_err(|_| ApiError::InternalServerError {
            message: "Failed to read committed application state".to_string(),
            details: None,
        })?;

    let record = lookup_namespace_record(current_snapshot.as_ref(), &committed, &canonical)
        .map_err(|e| ApiError::InternalServerError {
            message: "Failed to resolve namespace".to_string(),
            details: Some(e.to_string()),
        })?;

    debug!(
        namespace_slug = %canonical,
        registered = record.is_some(),
        "namespace lookup"
    );

    if let Some(record) = record {
        let body = NamespaceRegisteredResponse::from_record(&record).map_err(|e| {
            ApiError::InternalServerError {
                message: "Failed to build registry path".to_string(),
                details: Some(e.to_string()),
            }
        })?;
        return Ok((StatusCode::OK, Json(body)).into_response());
    }

    let body = NamespaceNotRegisteredResponse::from_slug(canonical);
    Ok((StatusCode::NOT_FOUND, Json(body)).into_response())
}

/// `GET /v1/namespaces` — paginated list of registered custom namespaces.
pub async fn handle_list_namespaces(
    State(rate_limit_state): State<Arc<TokioRwLock<RateLimitState>>>,
    State(committed_state): State<Arc<Mutex<AppState>>>,
    State(current_state): State<Arc<Mutex<Option<AppState>>>>,
    headers: HeaderMap,
    Query(params): Query<NamespaceListParams>,
) -> Result<Json<NamespaceListResponse>, ApiError> {
    check_rate_limit(&rate_limit_state, ApiEndpointType::General, &headers).await?;

    let limit = params.limit.unwrap_or(50).min(100);
    if limit == 0 {
        return Err(ApiError::BadRequest {
            message: "Invalid limit".to_string(),
            details: Some("Limit must be > 0".to_string()),
        });
    }

    let continuation = parse_namespace_list_continuation(&params)?;

    let current_snapshot = current_state
        .lock()
        .map_err(|_| ApiError::InternalServerError {
            message: "Failed to read current application state".to_string(),
            details: None,
        })?
        .clone();

    let committed_snapshot = committed_state
        .lock()
        .map_err(|_| ApiError::InternalServerError {
            message: "Failed to read committed application state".to_string(),
            details: None,
        })?
        .clone();

    let all_records = collect_namespace_records(&committed_snapshot, current_snapshot.as_ref())
        .map_err(|e| ApiError::InternalServerError {
            message: "Failed to list namespaces".to_string(),
            details: Some(e.to_string()),
        })?;

    let total = if continuation.is_none() {
        Some(all_records.len() as u64)
    } else {
        None
    };

    let page_records = match continuation {
        None => all_records,
        Some((after_height, after_slug)) => {
            filter_namespace_records_after_cursor(&all_records, after_height, &after_slug)
        }
    };

    let (page_records, has_next) = paginate_namespace_records(page_records, limit);

    let namespaces = page_records
        .iter()
        .map(namespace_record_to_list_item)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| ApiError::InternalServerError {
            message: "Failed to build namespace list response".to_string(),
            details: Some(e.to_string()),
        })?;

    debug!(
        limit,
        has_next,
        returned = namespaces.len(),
        total = ?total,
        "namespace list"
    );

    Ok(Json(NamespaceListResponse {
        namespaces,
        pagination: NamespaceListPagination {
            limit,
            has_next,
            total,
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use eld_common::address::Address;
    use eld_common::cado::{CADOMetadata, CadoType};
    use eld_common::namespace::slug_to_namespace_cadopath;

    fn sample_record(slug: &str, height: u64) -> NamespaceRecord {
        let owner =
            Address::parse_hex_str("0xe17404c417fa10cc04fdf73604fcacca8d0a687c").expect("address");
        NamespaceRecord {
            namespace_slug: slug.to_string(),
            owner,
            registered_height: height,
        }
    }

    #[test]
    fn lookup_prefers_current_block_cache() {
        let record = sample_record("peter", 99);

        let mut current = AppState::default();
        current
            .envelope
            .namespace_registry_cache
            .insert("peter".to_string(), record.clone());

        let committed = AppState::default();
        let found = lookup_namespace_record(Some(&current), &committed, "peter").expect("lookup");
        assert_eq!(found, Some(record));
    }

    #[test]
    fn lookup_committed_cache_when_not_staged() {
        let record = sample_record("peter", 42);
        let path = slug_to_namespace_cadopath("peter").expect("path");
        let bytes = record.serialize_bin().expect("serialize");
        let cado = eld_common::cado::CadoBody::immutable(
            bytes,
            CADOMetadata::new(CadoType::Namespace, "peter"),
        );

        let mut committed = AppState::default();
        committed
            .envelope
            .committed_cado_cache
            .insert(path.as_str().as_bytes(), cado);

        let found = lookup_namespace_record(None, &committed, "peter").expect("lookup");
        assert_eq!(found.as_ref(), Some(&record));
    }

    #[test]
    fn sort_orders_by_height_desc_then_slug_asc() {
        let mut records = vec![
            sample_record("beta", 100),
            sample_record("alpha", 100),
            sample_record("zeta", 50),
            sample_record("gamma", 200),
        ];
        sort_namespace_records(&mut records);
        let slugs: Vec<_> = records
            .iter()
            .map(|r| (r.namespace_slug.as_str(), r.registered_height))
            .collect();
        assert_eq!(
            slugs,
            vec![("gamma", 200), ("alpha", 100), ("beta", 100), ("zeta", 50)]
        );
    }

    #[test]
    fn staging_overrides_committed_record_for_same_slug() {
        let mut committed = AppState::default();
        committed
            .envelope
            .namespace_registry_index
            .insert("peter".to_string(), sample_record("peter", 10));

        let mut current = AppState::default();
        current
            .envelope
            .namespace_registry_cache
            .insert("peter".to_string(), sample_record("peter", 99));

        let records = collect_namespace_records(&committed, Some(&current)).expect("collect");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].registered_height, 99);
    }

    #[test]
    fn collect_namespace_records_uses_committed_index() {
        let mut committed = AppState::default();
        committed
            .envelope
            .namespace_registry_index
            .insert("alpha".to_string(), sample_record("alpha", 100));
        committed
            .envelope
            .namespace_registry_index
            .insert("beta".to_string(), sample_record("beta", 50));

        let records = collect_namespace_records(&committed, None).expect("collect");
        let slugs: Vec<_> = records
            .iter()
            .map(|record| record.namespace_slug.as_str())
            .collect();
        assert_eq!(slugs, vec!["alpha", "beta"]);
    }

    #[test]
    fn cursor_filter_returns_older_rows_in_list_order() {
        let records = vec![
            sample_record("gamma", 200),
            sample_record("alpha", 100),
            sample_record("beta", 100),
            sample_record("zeta", 50),
        ];

        let filtered = filter_namespace_records_after_cursor(&records, 100, "alpha");
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].namespace_slug, "beta");
        assert_eq!(filtered[1].namespace_slug, "zeta");
    }

    #[test]
    fn pagination_uses_limit_plus_one_probe() {
        let records = vec![
            sample_record("gamma", 200),
            sample_record("alpha", 100),
            sample_record("beta", 100),
        ];

        let (page, has_next) = paginate_namespace_records(records, 2);
        assert!(has_next);
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].namespace_slug, "gamma");
        assert_eq!(page[1].namespace_slug, "alpha");
    }

    #[test]
    fn continuation_params_must_be_paired() {
        let only_height = NamespaceListParams {
            after_registered_height: Some(10),
            after_namespace_slug: None,
            limit: None,
        };
        assert!(parse_namespace_list_continuation(&only_height).is_err());

        let only_slug = NamespaceListParams {
            after_registered_height: None,
            after_namespace_slug: Some("peter".to_string()),
            limit: None,
        };
        assert!(parse_namespace_list_continuation(&only_slug).is_err());

        let paired = NamespaceListParams {
            after_registered_height: Some(10),
            after_namespace_slug: Some("peter".to_string()),
            limit: None,
        };
        assert_eq!(
            parse_namespace_list_continuation(&paired).expect("paired"),
            Some((10, "peter".to_string()))
        );
    }
}
