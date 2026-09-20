use super::abci_rate_limiting::{AbciQueryType, ResultLimiter};
use super::info::{payload_ok_info, QueryProcessorResult};
use crate::storage::traits::ConsensusConnectionStorage;
use eld_common::error::EldError;
use eld_common::utils::to_json_string;
use tracing::info;

pub(crate) fn process_cado_list_query<S>(
    storage: &S,
    result_limiter: &ResultLimiter,
    query_type: AbciQueryType,
    _path: String,
    data: Vec<u8>,
) -> QueryProcessorResult
where
    S: ConsensusConnectionStorage,
{
    let prefix = String::from_utf8(data.clone()).map_err(|e| EldError::ValidationError {
        field: "prefix_utf8".to_string(),
        value: format!("{data:?}"),
        details: e.to_string(),
    })?;
    info!("Querying CADOs with prefix: {}", prefix);

    let cados = storage.get_cados_by_prefix(&prefix)?;
    if let Some(error_response) = result_limiter.check_result_limit(cados.len(), query_type) {
        return Err(EldError::ValidationError {
            field: "result_limit".to_string(),
            value: query_type.to_string(),
            details: error_response.log,
        });
    }

    let info = to_json_string(&cados)?;
    Ok(payload_ok_info(
        vec![],
        format!("Found {} CADOs matching prefix", cados.len()),
        info,
    ))
}
