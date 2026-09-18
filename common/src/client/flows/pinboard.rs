//! Pinboard POST, content fetch, and pinboard query flows.

use crate::app_api::AppApi;
use crate::client::ChainClient;
use crate::constants::abci_query;
use crate::constants::cado::PATH_PREFIX_PINBOARD;
use crate::pinboard_api::{PinboardMessageParams, PostMessageSubmitRequest};
use crate::tx::PostMessageUserRequestInput;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use std::fs;
use tracing::{error, info, warn};

/// Submit a pinboard message: signs user commitment locally, POSTs to node's upload API.
pub(crate) async fn post_pinboard_message(client: &ChainClient, input: PinboardMessageParams) {
    let PinboardMessageParams {
        wallet_name,
        file_path,
        content_type,
        expires_height,
        visibility,
        topic,
        tags,
        user_fee_amount,
        namespace,
    } = input;

    let wallet = match client.get_wallet_by_name(wallet_name).await {
        Some(w) => w,
        None => {
            error!("Wallet not found");
            return;
        }
    };

    let message_bytes = match fs::read(&file_path) {
        Ok(b) => b,
        Err(e) => {
            error!(%e, "Failed to read message file");
            return;
        }
    };

    let user_request = match wallet.sign_post_message_user_commitment(
        &message_bytes,
        PostMessageUserRequestInput {
            expires_height,
            visibility,
            topic,
            tags,
            content_type,
            fee_amount: user_fee_amount,
            namespace,
        },
    ) {
        Ok(r) => r,
        Err(e) => {
            error!(%e, "Failed to build signed user request");
            return;
        }
    };

    let submit = PostMessageSubmitRequest {
        user: user_request,
        message_b64: BASE64_STANDARD.encode(&message_bytes),
        idempotency_key: None,
    };

    let app_api = AppApi::new(client.config.get_app_base_url());

    match app_api.submit_pinboard_message(submit).await {
        Ok(resp) => {
            info!(
                message_id = %resp.message_id,
                tx_hash = %resp.tx_hash,
                "Pinboard message accepted by node"
            );
            println!("status: {}", resp.status);
            println!("message_id: {}", resp.message_id);
            println!("content_key: {}", resp.content_key);
            println!("tx_hash: {}", resp.tx_hash);
            println!("origin_validator: {}", resp.origin_validator);
            println!("received_timestamp: {}", resp.received_timestamp);
            if let Some(content_path) = &resp.content_path {
                println!("content_path: {content_path}");
            }
        }
        Err(e) => {
            error!(%e, "Pinboard submit failed");
        }
    }
}

pub(crate) async fn get_content(client: &ChainClient, content_id: String) {
    info!("Getting content with ID: {}", content_id);

    let http = reqwest::Client::new();

    let base_url = client.config.get_app_base_url();
    let url = format!("{base_url}content/{content_id}");
    info!("Requesting content from: {}", url);

    match http.get(&url).send().await {
        Ok(response) => {
            if response.status().is_success() {
                match response.text().await {
                    Ok(text) => {
                        info!(
                            "Content retrieved successfully. Response length: {} characters",
                            text.len()
                        );

                        match serde_json::from_str::<Vec<u8>>(&text) {
                            Ok(bytes) => {
                                info!(
                                    "Successfully parsed JSON array of bytes. Size: {} bytes",
                                    bytes.len()
                                );

                                match String::from_utf8(bytes.clone()) {
                                    Ok(content_string) => {
                                        match serde_json::from_str::<serde_json::Value>(
                                            &content_string,
                                        ) {
                                            Ok(json) => {
                                                info!("Content is valid JSON:");
                                                info!(
                                                    "{}",
                                                    serde_json::to_string_pretty(&json).unwrap()
                                                );
                                            }
                                            Err(e) => {
                                                info!("Content is not valid JSON: {}", e);
                                                info!("Raw content:");
                                                info!("{}", content_string);
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        info!("Content is not valid UTF-8: {}", e);
                                        info!("Content is binary data ({} bytes)", bytes.len());
                                        info!("Format: unknown");
                                    }
                                }
                            }
                            Err(_) => match serde_json::from_str::<serde_json::Value>(&text) {
                                Ok(json) => {
                                    info!("Response is valid JSON:");
                                    info!("{}", serde_json::to_string_pretty(&json).unwrap());
                                }
                                Err(e) => {
                                    info!("Response is not valid JSON: {}", e);
                                    info!("Raw response:");
                                    info!("{}", text);
                                }
                            },
                        }
                    }
                    Err(e) => info!("Error reading response body: {}", e.to_string()),
                }
            } else {
                info!("Error: HTTP status {}", response.status());
            }
        }
        Err(e) => info!("Error making request: {}", e.to_string()),
    }
}

pub(crate) async fn pinboard_get_post(client: &ChainClient, wallet: String, message_id: String) {
    let path = format!(
        "{}{}/{}/{}",
        PATH_PREFIX_PINBOARD,
        abci_query::PINBOARD_SEGMENT_POST,
        wallet,
        message_id
    );

    let base_url = client
        .config
        .get_app_base_url()
        .trim_end_matches('/')
        .to_string();
    let url = format!(
        "{}/v1/pinboard/post?path={}",
        base_url,
        urlencoding::encode(&path)
    );

    match reqwest::Client::new().get(&url).send().await {
        Ok(response) => {
            if !response.status().is_success() {
                let status = response.status();
                match response.text().await {
                    Ok(body) => error!(
                        "Pinboard REST query failed for {}: HTTP {} body={}",
                        path, status, body
                    ),
                    Err(e) => error!(
                        "Pinboard REST query failed for {}: HTTP {} and failed reading body: {}",
                        path, status, e
                    ),
                }
                return;
            }

            let v: serde_json::Value = match response.json().await {
                Ok(v) => v,
                Err(e) => {
                    error!("Pinboard REST JSON parse failed for {}: {}", path, e);
                    return;
                }
            };
            info!("Pinboard REST response for {}:\n{}", path, v);

            if let Some(message_b64) = v.get("message_b64").and_then(|m| m.as_str()) {
                match BASE64_STANDARD.decode(message_b64.as_bytes()) {
                    Ok(decoded) => match String::from_utf8(decoded.clone()) {
                        Ok(text) => info!("Pinboard decoded message:\n{}", text),
                        Err(_) => {
                            info!("Pinboard decoded message (hex): 0x{}", hex::encode(decoded))
                        }
                    },
                    Err(e) => warn!("Failed to decode pinboard message_b64: {}", e),
                }
            } else {
                let blob_status = v
                    .get("blob_status")
                    .and_then(|s| s.as_str())
                    .unwrap_or("unknown");
                warn!(
                    "Pinboard REST response did not include message_b64 (blob_status={})",
                    blob_status
                );
            }
        }
        Err(e) => error!("Pinboard REST query failed for {} ({}): {}", path, url, e),
    }
}

pub(crate) async fn pinboard_list_by_wallet(
    client: &ChainClient,
    wallet: String,
    page: usize,
    page_size: usize,
) {
    let api = crate::abci_api::AbciHttpApi::new(client.config.get_node_url().to_owned());
    let path = format!(
        "{}{}/{}/{}/{}",
        PATH_PREFIX_PINBOARD,
        abci_query::PINBOARD_SEGMENT_WALLET,
        wallet,
        page,
        page_size
    );
    match api.pinboard_query(path.clone()).await {
        Ok(v) => info!("Pinboard response for {}:\n{}", path, v),
        Err(e) => error!("Pinboard query failed for {}: {}", path, e),
    }
}

pub(crate) async fn pinboard_list_by_tag(
    client: &ChainClient,
    tag: String,
    page: usize,
    page_size: usize,
) {
    let api = crate::abci_api::AbciHttpApi::new(client.config.get_node_url().to_owned());
    let path = format!(
        "{}{}/{}/{}/{}",
        PATH_PREFIX_PINBOARD,
        abci_query::PINBOARD_SEGMENT_TAG,
        tag,
        page,
        page_size
    );
    match api.pinboard_query(path.clone()).await {
        Ok(v) => info!("Pinboard response for {}:\n{}", path, v),
        Err(e) => error!("Pinboard query failed for {}: {}", path, e),
    }
}
