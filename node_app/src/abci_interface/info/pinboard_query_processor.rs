use crate::abci_interface::info::info::{payload_ok_info, QueryProcessorResult};
use crate::api::pagination::{PaginationParams, PrefixQueryOptions};
use crate::storage::traits::ConsensusConnectionStorage;
use eld_common::constants::abci_query;
use eld_common::constants::cado::{PATH_PREFIX_PINBOARD, SCOPE_ELD_ROOT};
use eld_common::error::EldError;
use eld_common::pinboard::PinboardMessageMetadata;
use eld_common::utils::to_json_string;
use tracing::warn;

/// Pinboard query paths (data is a UTF-8 path string under [`eld_common::constants::cado::PATH_PREFIX_PINBOARD`]).
pub(crate) fn process_pinboard_query<S>(
    storage: &S,
    _current_height: u64,
    _path: String,
    data: Vec<u8>,
) -> QueryProcessorResult
where
    S: ConsensusConnectionStorage,
{
    let path_str = String::from_utf8(data).map_err(|e| EldError::ValidationError {
        field: "pinboard_query_path_utf8".to_string(),
        value: format!("{:?}", e.as_bytes()),
        details: format!("Invalid UTF-8 in pinboard query path: {e}"),
    })?;

    let parts: Vec<&str> = path_str.split('/').filter(|p| !p.is_empty()).collect();
    // Expect: [SCOPE_ELD_ROOT, PINBOARD, ...]
    if parts.len() < 3 || parts[0] != SCOPE_ELD_ROOT || parts[1] != abci_query::PINBOARD {
        return Err(EldError::ValidationError {
            field: "pinboard_query_path".to_string(),
            value: path_str,
            details: format!("Expected {PATH_PREFIX_PINBOARD}{{post|wallet|tag|gc_metrics}}/..."),
        });
    }

    match parts[2] {
        abci_query::PINBOARD_SEGMENT_POST => Err(EldError::ValidationError {
            field: "pinboard_query_path".to_string(),
            value: path_str,
            details: "Pinboard post/blob ABCI query is disabled; use REST /v1/pinboard/post"
                .to_string(),
        }),
        abci_query::PINBOARD_SEGMENT_WALLET => {
            if parts.len() != 4 && parts.len() != 6 {
                return Err(EldError::ValidationError {
                    field: "pinboard_query_path".to_string(),
                    value: path_str,
                    details: format!(
                        "Expected {}{}/{{wallet}}[/page/page_size]",
                        PATH_PREFIX_PINBOARD,
                        abci_query::PINBOARD_SEGMENT_WALLET
                    ),
                });
            }
            let wallet = parts[3].to_string();
            let (page, page_size) = if parts.len() == 6 {
                (
                    parts[4].parse::<usize>().unwrap_or(0),
                    parts[5].parse::<usize>().unwrap_or(100),
                )
            } else {
                (0, 100)
            };

            let options = PrefixQueryOptions::default().with_pagination(PaginationParams {
                page,
                page_size,
                cursor: None,
            });

            let ids = storage.get_pinboard_message_ids_by_wallet_secure(&wallet, options)?;
            let mut metas = Vec::with_capacity(ids.items.len());
            for message_id in ids.items {
                if let Some(meta) = storage.get_pinboard_metadata(&message_id)? {
                    metas.push(meta);
                } else {
                    warn!(message_id = %message_id, "pinboard wallet index pointed to missing meta");
                }
            }

            #[derive(serde::Serialize)]
            struct PinboardListResponse {
                items: Vec<PinboardMessageMetadata>,
                page: usize,
                page_size: usize,
                has_more: bool,
            }
            let info = to_json_string(&PinboardListResponse {
                items: metas,
                page,
                page_size,
                has_more: ids.has_more,
            })?;
            Ok(payload_ok_info(Vec::new(), "pinboard_wallet_list", info))
        }
        abci_query::PINBOARD_SEGMENT_TAG => {
            if parts.len() != 4 && parts.len() != 6 {
                return Err(EldError::ValidationError {
                    field: "pinboard_query_path".to_string(),
                    value: path_str,
                    details: format!(
                        "Expected {}{}/{{tag}}[/page/page_size]",
                        PATH_PREFIX_PINBOARD,
                        abci_query::PINBOARD_SEGMENT_TAG
                    ),
                });
            }
            let tag = parts[3].to_string();
            let (page, page_size) = if parts.len() == 6 {
                (
                    parts[4].parse::<usize>().unwrap_or(0),
                    parts[5].parse::<usize>().unwrap_or(100),
                )
            } else {
                (0, 100)
            };

            let options = PrefixQueryOptions::default().with_pagination(PaginationParams {
                page,
                page_size,
                cursor: None,
            });

            let ids = storage.get_pinboard_message_ids_by_tag_secure(&tag, options)?;
            let mut metas = Vec::with_capacity(ids.items.len());
            for message_id in ids.items {
                if let Some(meta) = storage.get_pinboard_metadata(&message_id)? {
                    metas.push(meta);
                } else {
                    warn!(message_id = %message_id, "pinboard tag index pointed to missing meta");
                }
            }

            #[derive(serde::Serialize)]
            struct PinboardListResponse {
                items: Vec<PinboardMessageMetadata>,
                page: usize,
                page_size: usize,
                has_more: bool,
            }
            let info = to_json_string(&PinboardListResponse {
                items: metas,
                page,
                page_size,
                has_more: ids.has_more,
            })?;
            Ok(payload_ok_info(Vec::new(), "pinboard_tag_list", info))
        }
        abci_query::PINBOARD_SEGMENT_GC_METRICS => {
            if parts.len() != 3 {
                return Err(EldError::ValidationError {
                    field: "pinboard_query_path".to_string(),
                    value: path_str,
                    details: format!(
                        "Expected {}{}",
                        PATH_PREFIX_PINBOARD,
                        abci_query::PINBOARD_SEGMENT_GC_METRICS
                    ),
                });
            }
            let metrics = storage.get_pinboard_gc_metrics()?;
            let info = to_json_string(&metrics)?;
            Ok(payload_ok_info(Vec::new(), "pinboard_gc_metrics", info))
        }
        other => {
            warn!(path = %path_str, kind = %other, "unknown pinboard query kind");
            Err(EldError::ValidationError {
                field: "pinboard_query_kind".to_string(),
                value: other.to_string(),
                details: "Unknown pinboard query kind".to_string(),
            })
        }
    }
}
