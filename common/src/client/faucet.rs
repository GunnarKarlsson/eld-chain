//! HTTP client for the dev faucet endpoint.

use crate::client_config::CliConfig;
use crate::endpoint::resolve_faucet_request_url;
use std::time::Duration;
use tracing::{error, info};

fn faucet_http_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .build()
}

pub async fn request_faucet(config: &CliConfig, address: String) {
    if let Err(e) = crate::validation::validate_address(&address) {
        error!(%e);
        return;
    }

    let url = match resolve_faucet_request_url(
        config.faucet_url.as_deref(),
        &config.faucet_host,
        &config.faucet_port,
        &config.faucet_end_point,
    ) {
        Ok(url) => url,
        Err(e) => {
            error!(%e);
            return;
        }
    };

    let client = match faucet_http_client() {
        Ok(client) => client,
        Err(e) => {
            error!("Failed to create faucet HTTP client: {e}");
            return;
        }
    };

    let request_body = serde_json::json!({
        "address": address
    });

    match client.post(&url).json(&request_body).send().await {
        Ok(response) => match response.status() {
            reqwest::StatusCode::OK => {
                info!("✅ Successfully requested tokens from faucet");
                if let Ok(text) = response.text().await {
                    info!("Response: {}", text);
                }
            }
            status => {
                error!(
                    "❌ Failed to request tokens from faucet. Status: {}",
                    status
                );
                if let Ok(text) = response.text().await {
                    error!("Error details: {}", text);
                }
            }
        },
        Err(e) => {
            error!("❌ Failed to send request to faucet: {}", e);
        }
    }
}
