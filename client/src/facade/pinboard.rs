//! Pinboard POST, content fetch, and pinboard query flows.

use super::ChainClient;
use crate::api::rest::AppApi;
use crate::api::rest::{
    PinboardMessageParams, PostMessageSubmitRequest, PostMessageSubmitResponse,
};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use eld_common::constants::abci_query;
use eld_common::constants::cado::PATH_PREFIX_PINBOARD;
use eld_common::error::{EldError, ErrorBuilder};
use eld_common::tx::PostMessageUserRequestInput;
use std::fs;

/// Submit a pinboard message: signs user commitment locally, POSTs to node's upload API.
pub(crate) async fn post_pinboard_message(
    client: &ChainClient,
    input: PinboardMessageParams,
) -> Result<PostMessageSubmitResponse, EldError> {
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
    app_api.submit_pinboard_message(submit).await
}

pub(crate) async fn get_content(
    client: &ChainClient,
    content_id: String,
) -> Result<String, EldError> {
    let http = reqwest::Client::new();

    let base_url = client.config.get_app_base_url()?;
    let url = format!("{base_url}content/{content_id}");

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

    response
        .text()
        .await
        .map_err(|e| ErrorBuilder::network_error("read content body", &e.to_string()))
}

pub(crate) async fn pinboard_get_post(
    client: &ChainClient,
    wallet: String,
    message_id: String,
) -> Result<serde_json::Value, EldError> {
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
        let body = response.text().await.map_err(|e| {
            ErrorBuilder::network_error(
                "pinboard get post",
                &format!("HTTP {status} for {path}; failed reading body: {e}"),
            )
        })?;
        return Err(ErrorBuilder::network_error(
            "pinboard get post",
            &format!("HTTP {status} for {path} body={body}"),
        ));
    }

    response
        .json()
        .await
        .map_err(|e| ErrorBuilder::validation_error("pinboard_post", &path, &e.to_string()))
}

pub(crate) async fn pinboard_list_by_wallet(
    client: &ChainClient,
    wallet: String,
    page: usize,
    page_size: usize,
) -> Result<serde_json::Value, EldError> {
    let api = crate::api::abci::AbciHttpApi::new(client.config.get_node_url()?)?;
    let path = format!(
        "{}{}/{}/{}/{}",
        PATH_PREFIX_PINBOARD,
        abci_query::PINBOARD_SEGMENT_WALLET,
        wallet,
        page,
        page_size
    );
    api.pinboard_query(path).await
}

pub(crate) async fn pinboard_list_by_tag(
    client: &ChainClient,
    tag: String,
    page: usize,
    page_size: usize,
) -> Result<serde_json::Value, EldError> {
    let api = crate::api::abci::AbciHttpApi::new(client.config.get_node_url()?)?;
    let path = format!(
        "{}{}/{}/{}/{}",
        PATH_PREFIX_PINBOARD,
        abci_query::PINBOARD_SEGMENT_TAG,
        tag,
        page,
        page_size
    );
    api.pinboard_query(path).await
}
