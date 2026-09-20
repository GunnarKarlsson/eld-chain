use super::abci_rate_limiting::{AbciQueryType, ResultLimiter};
use super::info::{payload_ok_info, QueryProcessorResult};
use crate::storage::traits::ConsensusConnectionStorage;
use eld_common::error::EldError;
use eld_common::utils::to_json_string;
use tracing::info;

pub(crate) fn process_cado_paths_query<S>(
    storage: &S,
    result_limiter: &ResultLimiter,
    query_type: AbciQueryType,
    _path: String,
    data: Vec<u8>,
) -> QueryProcessorResult
where
    S: ConsensusConnectionStorage,
{
    let search_string = String::from_utf8(data.clone()).map_err(|e| EldError::ValidationError {
        field: "search_utf8".to_string(),
        value: format!("{data:?}"),
        details: e.to_string(),
    })?;
    info!("Querying CADO paths with search string: {}", search_string);

    let cado_paths = storage.get_cado_paths_by_prefix(&search_string)?;
    if let Some(error_response) = result_limiter.check_result_limit(cado_paths.len(), query_type) {
        return Err(EldError::ValidationError {
            field: "result_limit".to_string(),
            value: query_type.to_string(),
            details: error_response.log,
        });
    }

    let path_strings: Vec<String> = cado_paths
        .into_iter()
        .map(|path| path.as_str().to_string())
        .collect();

    let info = to_json_string(&path_strings)?;
    Ok(payload_ok_info(
        vec![],
        format!(
            "Found {} CADO paths matching search string",
            path_strings.len()
        ),
        info,
    ))
}
