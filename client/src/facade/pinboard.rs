//! Pinboard POST, content fetch, and pinboard query flows.

use super::ChainClient;
use crate::api::rest::AppApi;
use crate::api::rest::{PinboardMessageParams, PostMessageSubmitRequest};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use eld_common::constants::abci_query;
use eld_common::constants::cado::PATH_PREFIX_PINBOARD;
use eld_common::error::{EldError, ErrorBuilder};
use eld_common::tx::PostMessageUserRequestInput;
use std::fs;
use tracing::{info, warn};

/// Submit a pinboard message: signs user commitment locally, POSTs to node's upload API.
pub(crate) async fn post_pinboard_message(
    client: &ChainClient,
    input: PinboardMessageParams,
) -> Result<(), EldError> {
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

    let wallet = super::util::require_wallet(client, &wallet_name).await?;

    let message_bytes = fs::read(&file_path)
        .map_err(|e| ErrorBuilder::file_system_error("read", &file_path, &e.to_string()))?;

    let user_request = wallet
        .sign_post_message_user_commitment(
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
        )
        .map_err(|e| ErrorBuilder::wallet_error("sign_post_message", &wallet_name, &e))?;

    let submit = PostMessageSubmitRequest {
        user: user_request,
        message_b64: BASE64_STANDARD.encode(&message_bytes),
        idempotency_key: None,
    };

    let app_api = AppApi::new(client.config.get_app_base_url()?)?;
    let resp = app_api.submit_pinboard_message(submit).await?;
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
    Ok(())
}

pub(crate) async fn get_content(client: &ChainClient, content_id: String) -> Result<(), EldError> {
    info!("Getting content with ID: {}", content_id);

    let http = reqwest::Client::new();

    let base_url = client.config.get_app_base_url()?;
    let url = format!("{base_url}content/{content_id}");
    info!("Requesting content from: {}", url);

    let response = http
        .get(&url)
        .send()
        .await
        .map_err(|e| ErrorBuilder::network_error("get content", &e.to_string()))?;

    if !response.status().is_success() {
        return Err(ErrorBuilder::network_error(
            "get content",
            &format!("HTTP status {}", response.status()),
        ));
    }

    let text = response
        .text()
        .await
        .map_err(|e| ErrorBuilder::network_error("read content body", &e.to_string()))?;
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
                    match serde_json::from_str::<serde_json::Value>(&content_string) {
                        Ok(json) => {
                            info!("Content is valid JSON:");
                            info!(
                                "{}",
                                serde_json::to_string_pretty(&json)
                                    .unwrap_or_else(|_| json.to_string())
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
                info!(
                    "{}",
                    serde_json::to_string_pretty(&json).unwrap_or_else(|_| json.to_string())
                );
            }
            Err(e) => {
                info!("Response is not valid JSON: {}", e);
                info!("Raw response:");
                info!("{}", text);
            }
        },
    }
    Ok(())
}

pub(crate) async fn pinboard_get_post(
    client: &ChainClient,
    wallet: String,
    message_id: String,
) -> Result<(), EldError> {
    let path = format!(
        "{}{}/{}/{}",
        PATH_PREFIX_PINBOARD,
        abci_query::PINBOARD_SEGMENT_POST,
        wallet,
        message_id
    );

    let base_url = client
        .config
        .get_app_base_url()?
        .trim_end_matches('/')
        .to_string();
    let url = format!(
        "{}/v1/pinboard/post?path={}",
        base_url,
        urlencoding::encode(&path)
    );

    let response = reqwest::Client::new().get(&url).send().await.map_err(|e| {
        ErrorBuilder::network_error(
            "pinboard get post",
            &format!("request failed for {path} ({url}): {e}"),
        )
    })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|e| format!("failed reading body: {e}"));
        return Err(ErrorBuilder::network_error(
            "pinboard get post",
            &format!("HTTP {status} for {path} body={body}"),
        ));
    }

    let v: serde_json::Value = response
        .json()
        .await
        .map_err(|e| ErrorBuilder::validation_error("pinboard_post", &path, &e.to_string()))?;
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
    Ok(())
}

pub(crate) async fn pinboard_list_by_wallet(
    client: &ChainClient,
    wallet: String,
    page: usize,
    page_size: usize,
) -> Result<(), EldError> {
    let api = crate::api::abci::AbciHttpApi::new(client.config.get_node_url()?)?;
    let path = format!(
        "{}{}/{}/{}/{}",
        PATH_PREFIX_PINBOARD,
        abci_query::PINBOARD_SEGMENT_WALLET,
        wallet,
        page,
        page_size
    );
    let v = api.pinboard_query(path.clone()).await?;
    info!("Pinboard response for {}:\n{}", path, v);
    Ok(())
}

pub(crate) async fn pinboard_list_by_tag(
    client: &ChainClient,
    tag: String,
    page: usize,
    page_size: usize,
) -> Result<(), EldError> {
    let api = crate::api::abci::AbciHttpApi::new(client.config.get_node_url()?)?;
    let path = format!(
        "{}{}/{}/{}/{}",
        PATH_PREFIX_PINBOARD,
        abci_query::PINBOARD_SEGMENT_TAG,
        tag,
        page,
        page_size
    );
    let v = api.pinboard_query(path.clone()).await?;
    info!("Pinboard response for {}:\n{}", path, v);
    Ok(())
}
