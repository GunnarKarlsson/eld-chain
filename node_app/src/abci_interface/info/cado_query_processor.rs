use super::info::{payload_ok, payload_ok_info, QueryProcessorResult};
use crate::storage::traits::ConsensusConnectionStorage;
use eld_common::cado::CadoPath;
use eld_common::error::EldError;
use eld_common::utils::to_json_string;
use tracing::error;

pub(crate) fn process_cado_query<S>(
    storage: &S,
    _path: String,
    data: Vec<u8>,
) -> QueryProcessorResult
where
    S: ConsensusConnectionStorage,
{
    let path_str = match String::from_utf8(data.clone()) {
        Ok(path) => path,
        Err(e) => {
            error!("{:?}", e);
            return Err(EldError::ValidationError {
                field: "cado_path_utf8".to_string(),
                value: format!("{data:?}"),
                details: e.to_string(),
            });
        }
    };

    let cado_path = match CadoPath::parse(&path_str) {
        Ok(path) => path,
        Err(e) => {
            error!("{:?}", e);
            return Err(e);
        }
    };

    match storage.get_cado_by_path(cado_path) {
        Ok(Some(cado)) => match to_json_string(&cado) {
            Ok(info) => Ok(payload_ok_info(vec![], "CADO found", info)),
            Err(e) => {
                error!("{:?}", e);
                Err(e)
            }
        },
        Ok(None) => Ok(payload_ok(vec![], "CADO not found".to_string())),
        Err(e) => {
            error!("{:?}", e);
            Err(e)
        }
    }
}
