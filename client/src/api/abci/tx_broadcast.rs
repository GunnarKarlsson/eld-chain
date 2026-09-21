//! Tendermint broadcast_tx_commit via JSON-RPC.

use crate::api::abci::wire_bytes_to_tx_hash;
use crate::config::client_config::CliConfig;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use eld_common::error::EldError;
use serde_json::Value;
use std::str::FromStr;
use tendermint::Hash;

/// Decoded `deliver_tx` event from a `broadcast_tx_commit` JSON-RPC response.
#[derive(Debug, Clone)]
pub struct DeliverTxEvent {
    pub event_type: String,
    pub attributes: Vec<(String, String)>,
}

pub(crate) fn parse_rpc_response(response: &str) -> Result<(), EldError> {
    let json: Value = serde_json::from_str(response).map_err(|e| EldError::NetworkError {
        operation: "parse RPC response".to_string(),
        details: format!("Failed to parse RPC response JSON: {e}"),
    })?;

    if json["result"].is_null() {
        return Err(EldError::TransactionError {
            tx_type: "unknown".to_string(),
            details: format!("RPC response has null result. Full response: {response}"),
        });
    }

    if json["result"]["check_tx"].is_null() {
        return Err(EldError::TransactionError {
            tx_type: "unknown".to_string(),
            details: format!(
                "RPC response missing check_tx field. Result: {}",
                json["result"]
            ),
        });
    }

    if json["result"]["check_tx"]["code"] != 0 {
        let code = rpc_tx_code(
            &json["result"]["check_tx"]["code"],
            "check_tx.code",
            response,
        )?;
        let log = json["result"]["check_tx"]["log"]
            .as_str()
            .unwrap_or("Unknown error");
        return Err(EldError::TransactionError {
            tx_type: "mempool_validation".to_string(),
            details: format!(
                "Transaction rejected by mempool (code {code}): {log}. Full result: {}",
                json["result"]
            ),
        });
    }

    if json["result"]["deliver_tx"].is_null() {
        return Err(EldError::TransactionError {
            tx_type: "unknown".to_string(),
            details: format!(
                "RPC response missing deliver_tx field. Result: {}",
                json["result"]
            ),
        });
    }

    if json["result"]["deliver_tx"]["code"] != 0 {
        let code = rpc_tx_code(
            &json["result"]["deliver_tx"]["code"],
            "deliver_tx.code",
            response,
        )?;
        let log = json["result"]["deliver_tx"]["log"]
            .as_str()
            .unwrap_or("Unknown error");
        return Err(EldError::TransactionError {
            tx_type: "consensus".to_string(),
            details: format!("Transaction failed at consensus (code {code}): {log}"),
        });
    }

    Ok(())
}

fn rpc_tx_code(value: &Value, field: &str, raw_response: &str) -> Result<u64, EldError> {
    value.as_u64().ok_or_else(|| EldError::NetworkError {
        operation: "parse RPC response".to_string(),
        details: format!("Missing or non-numeric {field} in: {raw_response}"),
    })
}

fn decode_event_attribute(attr: &Value) -> Result<(String, String), EldError> {
    let key_b64 = attr["key"]
        .as_str()
        .ok_or_else(|| EldError::ValidationError {
            field: "event_attribute_key".to_string(),
            value: attr["key"].to_string(),
            details: "Event attribute key is not a string".to_string(),
        })?;
    let value_b64 = attr["value"]
        .as_str()
        .ok_or_else(|| EldError::ValidationError {
            field: "event_attribute_value".to_string(),
            value: attr["value"].to_string(),
            details: "Event attribute value is not a string".to_string(),
        })?;

    let key_bytes = BASE64_STANDARD
        .decode(key_b64)
        .map_err(|e| EldError::ValidationError {
            field: "event_attribute_key".to_string(),
            value: key_b64.to_string(),
            details: format!("Failed to decode base64 key: {e}"),
        })?;
    let value_bytes = BASE64_STANDARD
        .decode(value_b64)
        .map_err(|e| EldError::ValidationError {
            field: "event_attribute_value".to_string(),
            value: value_b64.to_string(),
            details: format!("Failed to decode base64 value: {e}"),
        })?;

    let key = String::from_utf8(key_bytes).map_err(|e| EldError::ValidationError {
        field: "event_attribute_key".to_string(),
        value: key_b64.to_string(),
        details: format!("Failed to convert decoded key bytes to UTF-8: {e}"),
    })?;
    let value = String::from_utf8(value_bytes).map_err(|e| EldError::ValidationError {
        field: "event_attribute_value".to_string(),
        value: value_b64.to_string(),
        details: format!("Failed to convert decoded value bytes to UTF-8: {e}"),
    })?;
    Ok((key, value))
}

/// Decoded `deliver_tx` events from a successful `broadcast_tx_commit` body.
/// Malformed attributes are omitted (same as the previous skip-and-continue behavior).
pub fn deliver_tx_events(response: &Value) -> Vec<DeliverTxEvent> {
    let Some(events) = response["result"]["deliver_tx"]["events"].as_array() else {
        return Vec::new();
    };
    events
        .iter()
        .map(|event| {
            let event_type = event["type"].as_str().unwrap_or("").to_string();
            let attributes = event["attributes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|attr| decode_event_attribute(attr).ok())
                .collect();
            DeliverTxEvent {
                event_type,
                attributes,
            }
        })
        .collect()
}

/// Resolve the committed transaction hash from a successful `broadcast_tx_commit` body.
///
/// Uses the RPC `result.hash` when present; otherwise derives the hash from Eld wire bytes
/// (UTF-8 hex of the signed JSON), matching block indexing.
pub fn broadcast_tx_hash(response: &Value, tx_wire_hex: &str) -> Result<Hash, EldError> {
    if let Some(hash_str) = response["result"]["hash"].as_str() {
        return Hash::from_str(hash_str).map_err(|e| EldError::ValidationError {
            field: "broadcast_tx_hash".to_string(),
            value: hash_str.to_string(),
            details: format!("Failed to parse Tendermint tx hash: {e}"),
        });
    }

    Ok(wire_bytes_to_tx_hash(tx_wire_hex.as_bytes()))
}

pub async fn send_tx_rpc(config: &CliConfig, hex_encoded: &str) -> Result<Value, EldError> {
    let client = reqwest::Client::new();
    let url = config.get_node_url()?;
    let tx_base64 = base64::engine::general_purpose::STANDARD.encode(hex_encoded);

    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "broadcast_tx_commit",
        "params": [tx_base64]
    });

    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| EldError::NetworkError {
            operation: "RPC request".to_string(),
            details: format!("Failed to send RPC request: {e}"),
        })?;

    if !response.status().is_success() {
        let status = response.status();
        return Err(EldError::NetworkError {
            operation: "RPC request".to_string(),
            details: format!("RPC request failed with status: {status}"),
        });
    }

    let response_text = response.text().await.map_err(|e| EldError::NetworkError {
        operation: "RPC response reading".to_string(),
        details: format!("Failed to read RPC response: {e}"),
    })?;

    parse_rpc_response(&response_text)?;

    serde_json::from_str(&response_text).map_err(|e| EldError::NetworkError {
        operation: "parse RPC response".to_string(),
        details: format!("Failed to parse RPC response JSON: {e}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::abci::wire_bytes_to_tx_hash;

    #[test]
    fn broadcast_tx_hash_uses_rpc_result_when_present() {
        let wire_hex = "7b7d";
        let expected = wire_bytes_to_tx_hash(wire_hex.as_bytes());
        let response = serde_json::json!({
            "result": {
                "hash": expected.to_string()
            }
        });

        let hash = broadcast_tx_hash(&response, wire_hex).expect("hash");
        assert_eq!(hash, expected);
    }

    #[test]
    fn broadcast_tx_hash_falls_back_to_wire_bytes() {
        let wire_hex = "7b226e6f6e6365223a317d";
        let response = serde_json::json!({ "result": {} });

        let hash = broadcast_tx_hash(&response, wire_hex).expect("hash");
        assert_eq!(hash, wire_bytes_to_tx_hash(wire_hex.as_bytes()));
    }
}
