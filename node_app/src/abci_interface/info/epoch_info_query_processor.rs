use super::info::{payload_ok_info, QueryProcessorResult};
use crate::app_state::AppState;
use eld_common::error::EldError;
use eld_common::utils::to_json_string;
use eld_common::validator::EpochInfo;
use std::sync::{Arc, Mutex};

pub(crate) fn process_epoch_info_query(
    state: &Arc<Mutex<AppState>>,
    _path: String,
    _data: Vec<u8>,
    blocks_per_epoch: i64,
    validators_per_epoch: usize,
) -> QueryProcessorResult {
    let state = state.lock().map_err(EldError::from)?;
    let current_epoch = state.envelope.current_epoch;
    let current_block = state.envelope.block_height;

    let blocks_until_next_epoch = blocks_per_epoch - (current_block % blocks_per_epoch);

    let epoch_info = EpochInfo {
        current_epoch,
        current_block,
        blocks_per_epoch,
        validators_per_epoch,
        blocks_until_next_epoch,
    };

    let info = to_json_string(&epoch_info)?;
    Ok(payload_ok_info(vec![], "Epoch info retrieved", info))
}
