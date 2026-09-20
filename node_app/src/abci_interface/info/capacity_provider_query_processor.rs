use super::info::{payload_ok_info, QueryProcessorResult};
use crate::app_state::AppState;
use eld_common::address::Address;
use eld_common::error::EldError;
use eld_common::utils::to_json_string;
use std::sync::{Arc, Mutex};

pub(crate) fn process_capacity_provider_query(
    state: &Arc<Mutex<AppState>>,
    _path: String,
    data: Vec<u8>,
) -> QueryProcessorResult {
    let state = state.lock().map_err(EldError::from)?;
    let capacity_validators = state.envelope.capacity_validators.clone();

    let address = match String::from_utf8(data.clone()) {
        Ok(addr) => addr,
        Err(e) => {
            return Err(EldError::ValidationError {
                field: "capacity_provider_address_utf8".to_string(),
                value: format!("{data:?}"),
                details: e.to_string(),
            });
        }
    };
    let parsed_address = Address::parse_hex_str(&address)?;
    let provider = capacity_validators
        .iter()
        .find(|p| p.address == parsed_address);

    match provider {
        Some(info) => match to_json_string(info) {
            Ok(info_json) => Ok(payload_ok_info(
                vec![],
                "Capacity provider info retrieved",
                info_json,
            )),
            Err(e) => Err(e),
        },
        None => Err(EldError::NotFoundError {
            resource_type: "capacity_provider".to_string(),
            identifier: parsed_address.hex_with_prefix(),
        }),
    }
}
