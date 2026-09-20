use super::info::{payload_ok, QueryProcessorResult};
use crate::config::ConsensusConfig;
use eld_common::error::EldError;
use eld_common::tx::Tx;
use eld_common::utils::to_json_bytes;
use std::sync::{Arc, Mutex};
use tracing::{error, info};

pub(crate) fn process_estimate_fee_query(
    consensus_config: &Arc<Mutex<ConsensusConfig>>,
    _path: String,
    data: Vec<u8>,
) -> QueryProcessorResult {
    info!("estimate fee section");
    let hex_str = match String::from_utf8(data.clone()) {
        Ok(s) => s,
        Err(e) => {
            error!("{:?}", e);
            return Err(EldError::ValidationError {
                field: "fee_query_hex_utf8".to_string(),
                value: format!("{data:?}"),
                details: e.to_string(),
            });
        }
    };

    let tx_bytes = match hex::decode(&hex_str) {
        Ok(bytes) => bytes,
        Err(e) => {
            error!("{:?}", e);
            return Err(EldError::ValidationError {
                field: "fee_query_hex".to_string(),
                value: hex_str,
                details: e.to_string(),
            });
        }
    };

    let tx_json = match String::from_utf8(tx_bytes) {
        Ok(json) => json,
        Err(e) => {
            error!("{:?}", e);
            return Err(EldError::ValidationError {
                field: "fee_query_decoded_utf8".to_string(),
                value: "<binary>".to_string(),
                details: e.to_string(),
            });
        }
    };

    let tx: Tx = match serde_json::from_str(&tx_json) {
        Ok(tx) => tx,
        Err(e) => {
            error!("{:?}", e);
            return Err(EldError::ValidationError {
                field: "fee_query_tx_json".to_string(),
                value: tx_json,
                details: e.to_string(),
            });
        }
    };

    info!(
        tx_type = %tx.payload.r#type,
        tx_json_len = tx_json.len(),
        "estimate_fee query"
    );

    let fee_config = consensus_config
        .lock()
        .map_err(EldError::from)?
        .fee_config
        .clone();

    let fee = eld_common::fee::calculate_dynamic_fee(&tx, &fee_config)?;
    info!("calculated fee: {}", fee);
    let fee_response = serde_json::json!({
        "estimated_fee": fee.amount(),
    });

    let value = to_json_bytes(&fee_response)?;
    Ok(payload_ok(value, "Fee estimation successful"))
}
