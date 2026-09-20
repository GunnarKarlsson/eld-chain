use super::info::{payload_ok_info, QueryProcessorResult};
use crate::app_state::AppState;
use eld_common::coin::Coin;
use eld_common::error::EldError;
use eld_common::utils::to_json_string;
use eld_common::validator::ActiveValidatorsInfo;
use std::sync::{Arc, Mutex};
use tracing::{debug, error};

pub(crate) fn process_active_validators_query(
    state: &Arc<Mutex<AppState>>,
    _path: String,
    _data: Vec<u8>,
) -> QueryProcessorResult {
    let state = state.lock().map_err(EldError::from)?;
    let active_validators = state.envelope.active_validators.clone();
    let current_epoch = state.envelope.current_epoch;

    debug!("Processing active_validators query");
    debug!("Current active validators len: {}", active_validators.len());

    let total_stake = match active_validators
        .iter()
        .try_fold(Coin::zero(), |acc, v| acc + v.stake)
    {
        Ok(coin) => coin,
        Err(e) => {
            error!("Failed to sum validator stakes: {}", e);
            return Err(e);
        }
    };

    debug!("Total stake: {}", total_stake);

    let mut sorted_validators = active_validators;
    sorted_validators.sort_by(|a, b| b.stake.cmp(&a.stake));

    let active_validators_info = ActiveValidatorsInfo {
        validators: sorted_validators,
        total_stake,
        current_epoch,
    };

    let info = to_json_string(&active_validators_info)?;

    Ok(payload_ok_info(
        vec![],
        "Active validators info retrieved",
        info,
    ))
}
