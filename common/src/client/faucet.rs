//! HTTP client for the dev faucet endpoint.

use crate::client_config::CliConfig;
use crate::error::EldError;
use std::time::Duration;
use tracing::{error, info};

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

pub async fn request_faucet(config: &CliConfig, address: String) -> Result<(), EldError> {
    crate::validation::validate_address(&address)?;

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
        .map_err(|e| {
            error!("Failed to send request to faucet: {e}");
            EldError::NetworkError {
                operation: "request faucet".to_string(),
                details: format!("Failed to send request to faucet: {e}"),
            }
        })?;

    match response.status() {
        reqwest::StatusCode::OK => {
            info!("✅ Successfully requested tokens from faucet");
            if let Ok(text) = response.text().await {
                info!("Response: {}", text);
            }
            Ok(())
        }
        status => {
            let details = match response.text().await {
                Ok(text) => {
                    error!("Failed to request tokens from faucet. Status: {status}. {text}");
                    format!("Failed to request tokens from faucet. Status: {status}. {text}")
                }
                Err(_) => {
                    error!("Failed to request tokens from faucet. Status: {status}");
                    format!("Failed to request tokens from faucet. Status: {status}")
                }
            };
            Err(EldError::NetworkError {
                operation: "request faucet".to_string(),
                details,
            })
        }
    }
}
