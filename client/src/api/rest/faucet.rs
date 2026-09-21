//! HTTP client for the dev faucet endpoint.

use crate::config::client_config::CliConfig;
use eld_common::error::EldError;
use std::time::Duration;

fn faucet_http_client() -> Result<reqwest::Client, EldError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| EldError::NetworkError {
            operation: "create faucet HTTP client".to_string(),
            details: format!("Failed to create faucet HTTP client: {e}"),
        })
}

pub async fn request_faucet(config: &CliConfig, address: String) -> Result<String, EldError> {
    eld_common::validation::validate_address(&address)?;

    let url = config.get_faucet_request_url()?;
    let client = faucet_http_client()?;

    let request_body = serde_json::json!({
        "address": address
    });

    let response = client
        .post(&url)
        .json(&request_body)
        .send()
        .await
        .map_err(|e| EldError::NetworkError {
            operation: "request faucet".to_string(),
            details: format!("Failed to send request to faucet: {e}"),
        })?;

    match response.status() {
        reqwest::StatusCode::OK => {
            let text = response.text().await.map_err(|e| EldError::NetworkError {
                operation: "request faucet".to_string(),
                details: format!("Failed to read faucet response: {e}"),
            })?;
            Ok(text)
        }
        status => {
            let details = match response.text().await {
                Ok(text) => {
                    format!("Failed to request tokens from faucet. Status: {status}. {text}")
                }
                Err(e) => {
                    format!(
                        "Failed to request tokens from faucet. Status: {status}. Also failed to read body: {e}"
                    )
                }
            };
            Err(EldError::NetworkError {
                operation: "request faucet".to_string(),
                details,
            })
        }
    }
}
