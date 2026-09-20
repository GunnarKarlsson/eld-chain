use crate::abci_interface::info::info::{payload_ok_info, QueryProcessorResult};
use crate::api::pagination::{PaginationParams, PrefixQueryOptions};
use crate::storage::traits::{ConsensusConnectionStorage, PinboardGlobalFeedOrder};
use eld_common::error::EldError;
use eld_common::pinboard::PinboardMessageMetadata;
use eld_common::utils::to_json_string;
use serde::Deserialize;
use tracing::warn;

#[derive(Debug, Clone, Deserialize)]
struct PinboardFeedRequest {
    /// "asc" (oldest first) or "desc" (newest first). Default: "desc".
    #[serde(default)]
    order: Option<PinboardGlobalFeedOrder>,
    /// 0-based page. Default: 0.
    #[serde(default)]
    page: Option<usize>,
    /// Items per page. Default: 100.
    #[serde(default)]
    page_size: Option<usize>,
}

impl Default for PinboardFeedRequest {
    fn default() -> Self {
        Self {
            order: Some(PinboardGlobalFeedOrder::Desc),
            page: Some(0),
            page_size: Some(100),
        }
    }
}

/// Global pinboard feed query (data is UTF-8 JSON):
/// `{"order":"asc"|"desc","page":0,"page_size":100}` (all fields optional).
pub(crate) fn process_pinboard_feed_query<S>(
    storage: &S,
    _path: String,
    data: Vec<u8>,
) -> QueryProcessorResult
where
    S: ConsensusConnectionStorage,
{
    let req: PinboardFeedRequest = if data.is_empty() {
        PinboardFeedRequest::default()
    } else {
        serde_json::from_slice(&data).map_err(|e| EldError::ValidationError {
            field: "pinboard_feed_query_json".to_string(),
            value: String::from_utf8_lossy(&data).to_string(),
            details: format!("Invalid JSON for pinboard feed query: {e}"),
        })?
    };

    let order = req.order.unwrap_or(PinboardGlobalFeedOrder::Desc);
    let page = req.page.unwrap_or(0);
    let page_size = req.page_size.unwrap_or(100);

    let options = PrefixQueryOptions::default().with_pagination(PaginationParams {
        page,
        page_size,
        cursor: None,
    });

    let ids = storage.get_pinboard_message_ids_global_secure(order, options)?;
    let mut metas = Vec::with_capacity(ids.items.len());
    for message_id in ids.items {
        if let Some(meta) = storage.get_pinboard_metadata(&message_id)? {
            metas.push(meta);
        } else {
            warn!(message_id = %message_id, "pinboard global index pointed to missing meta");
        }
    }

    #[derive(serde::Serialize)]
    struct PinboardFeedResponse {
        items: Vec<PinboardMessageMetadata>,
        order: PinboardGlobalFeedOrder,
        page: usize,
        page_size: usize,
        has_more: bool,
    }

    let info = to_json_string(&PinboardFeedResponse {
        items: metas,
        order,
        page,
        page_size,
        has_more: ids.has_more,
    })?;

    Ok(payload_ok_info(Vec::new(), "pinboard_feed", info))
}
