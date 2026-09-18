//! Tendermint broadcast_tx_commit via JSON-RPC.

use crate::client_config::CliConfig;
use crate::error::EldError;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use serde_json::Value;
use tracing::{error, info};

pub(crate) fn parse_rpc_response(response: &str) -> Result<(), EldError> {
    let json: Value = serde_json::from_str(response).map_err(|e| EldError::NetworkError {
        operation: "parse RPC response".to_string(),
        details: format!("Failed to parse RPC response JSON: {e}"),
    })?;

    if json["result"].is_null() {
        error!(
            "mempool error: RPC response has null result. Full response: {}",
            response
        );
        return Err(EldError::TransactionError {
            tx_type: "unknown".to_string(),
            details: "RPC response has null result".to_string(),
        });
    }

    if json["result"]["check_tx"].is_null() {
        error!(
            "mempool error: RPC response missing check_tx field. Result: {}",
            json["result"]
        );
        return Err(EldError::TransactionError {
            tx_type: "unknown".to_string(),
            details: "RPC response missing check_tx field".to_string(),
        });
    }

    if json["result"]["check_tx"]["code"] != 0 {
        let code = json["result"]["check_tx"]["code"].as_u64().unwrap_or(0);
        let log = json["result"]["check_tx"]["log"]
            .as_str()
            .unwrap_or("Unknown error");
        error!(
            "mempool error: check_tx failed with code {}. Full result: {}",
            code, json["result"]
        );
        return Err(EldError::TransactionError {
            tx_type: "mempool_validation".to_string(),
            details: format!("Transaction rejected by mempool (code {code}): {log}"),
        });
    }

    if json["result"]["deliver_tx"].is_null() {
        error!(
            "tx processing error: RPC response missing deliver_tx field. Result: {}",
            json["result"]
        );
        return Err(EldError::TransactionError {
            tx_type: "unknown".to_string(),
            details: "RPC response missing deliver_tx field".to_string(),
        });
    }

    if json["result"]["deliver_tx"]["code"] != 0 {
        let code = json["result"]["deliver_tx"]["code"].as_u64().unwrap_or(0);
        let log = json["result"]["deliver_tx"]["log"]
            .as_str()
            .unwrap_or("Unknown error");
        error!("tx processing error:\nError: {} : Code: {}", log, code);
        return Err(EldError::TransactionError {
            tx_type: "consensus".to_string(),
            details: format!("Transaction failed at consensus (code {code}): {log}"),
        });
    }

    if let Some(events) = json["result"]["deliver_tx"]["events"].as_array() {
        for event in events {
            info!("\nEvent Type: {}", event["type"]);
            if let Some(attributes) = event["attributes"].as_array() {
                for attr in attributes {
                    let key = BASE64_STANDARD
                        .decode(attr["key"].as_str().expect("Failed to get key as string"))
                        .expect("Failed to decode base64 key");
                    let value = BASE64_STANDARD
                        .decode(
                            attr["value"]
                                .as_str()
                                .expect("Failed to get value as string"),
                        )
                        .expect("Failed to decode base64 value");
                    info!(
                        "{}: {}",
                        String::from_utf8(key)
                            .expect("Failed to convert decoded key bytes to UTF-8"),
                        String::from_utf8(value)
                            .expect("Failed to convert decoded value bytes to UTF-8")
                    );
                }
            }
        }
    }

    Ok(())
}

pub async fn send_tx_rpc(config: &CliConfig, hex_encoded: &str) -> Result<Value, EldError> {
    let client = reqwest::Client::new();
    let url = config.get_node_url();
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
        .map_err(|e| {
            error!("Failed to send RPC request: {}", e);
            EldError::NetworkError {
                operation: "RPC request".to_string(),
                details: format!("Failed to send RPC request: {e}"),
            }
        })?;

    if !response.status().is_success() {
        let status = response.status();
        error!("RPC request failed with status: {}", status);
        return Err(EldError::NetworkError {
            operation: "RPC request".to_string(),
            details: format!("RPC request failed with status: {status}"),
        });
    }

    let response_text = response.text().await.map_err(|e| {
        error!("Failed to read RPC response: {}", e);
        EldError::NetworkError {
            operation: "RPC response reading".to_string(),
            details: format!("Failed to read RPC response: {e}"),
        }
    })?;

    parse_rpc_response(&response_text)?;

    serde_json::from_str(&response_text).map_err(|e| {
        error!("Failed to parse RPC response JSON: {}", e);
        EldError::NetworkError {
            operation: "parse RPC response".to_string(),
            details: format!("Failed to parse RPC response JSON: {e}"),
        }
    })
}
