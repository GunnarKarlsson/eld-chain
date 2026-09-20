use super::info::{payload_ok_info, QueryProcessorResult};
use crate::app_state::AppState;
use eld_common::coin::Coin;
use eld_common::error::EldError;
use eld_common::utils::to_json_string;
use eld_common::validator::CapacityValidatorsInfo;
use std::sync::{Arc, Mutex};
use tracing::{debug, error};

pub(crate) fn process_capacity_validators_query(
    state: &Arc<Mutex<AppState>>,
    _path: String,
    _data: Vec<u8>,
) -> QueryProcessorResult {
    let state = state.lock().map_err(EldError::from)?;
    let capacity_validators = state.envelope.capacity_validators.clone();
    let current_epoch = state.envelope.current_epoch;

    debug!("Processing capacity_validators query");
    debug!(
        "Registered capacity validators len: {}",
        capacity_validators.len(),
    );

    let total_stake = match capacity_validators
        .iter()
        .try_fold(Coin::zero(), |acc, sp| acc + sp.stake)
    {
        Ok(coin) => coin,
        Err(e) => {
            error!("Failed to sum capacity validator stakes: {}", e);
            return Err(e);
        }
    };

    let total_capacity: u64 = capacity_validators
        .iter()
        .map(|sp| sp.storage_capacity)
        .sum();

    let mut sorted = capacity_validators;
    sorted.sort_by(|a, b| b.storage_capacity.cmp(&a.storage_capacity));

    let info = CapacityValidatorsInfo {
        capacity_validators: sorted,
        total_stake,
        total_capacity,
        current_epoch,
    };
    let info = to_json_string(&info)?;

    Ok(payload_ok_info(
        vec![],
        "Capacity validators retrieved",
        info,
    ))
}
