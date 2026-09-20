use super::info::{payload_ok_info, AccountCount, QueryProcessorResult};
use crate::storage::traits::ConsensusConnectionStorage;
use eld_common::constants::cado::PATH_PREFIX_ACCOUNT;
use eld_common::utils::to_json_string;

pub(crate) fn process_account_count_query<S>(
    storage: &S,
    _path: String,
    _data: Vec<u8>,
) -> QueryProcessorResult
where
    S: ConsensusConnectionStorage,
{
    let count = storage.get_cado_paths_by_prefix(PATH_PREFIX_ACCOUNT)?.len();

    Ok(payload_ok_info(
        vec![],
        "Account count".to_string(),
        to_json_string(&AccountCount { count })?,
    ))
}
