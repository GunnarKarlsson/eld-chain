//! Shared helpers for flow implementations.

use crate::client::ChainClient;
use crate::error::{EldError, ErrorBuilder};
use crate::nonce::Nonce;
use crate::wallet::Wallet;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use tendermint::abci::EventAttribute;

pub(crate) async fn require_wallet(
    client: &ChainClient,
    wallet_name: &str,
) -> Result<Wallet, EldError> {
    client
        .get_wallet_by_name(wallet_name.to_string())
        .await
        .ok_or_else(|| ErrorBuilder::wallet_error("retrieval", wallet_name, "Wallet not found"))
}

pub(crate) fn require_nonce(nonce: Option<Nonce>) -> Result<Nonce, EldError> {
    nonce.ok_or_else(|| {
        ErrorBuilder::network_error("nonce retrieval", "Failed to get account nonce")
    })
}

pub(crate) fn decode_event_attribute(
    attribute: EventAttribute,
) -> Result<(String, String), EldError> {
    let key_str = attribute
        .key_str()
        .map_err(|e| ErrorBuilder::validation_error("event_attribute_key", "", &e.to_string()))?;
    let decoded_key_bytes = BASE64_STANDARD.decode(key_str).map_err(|e| {
        ErrorBuilder::validation_error("event_attribute_key", key_str, &e.to_string())
    })?;
    let key = String::from_utf8(decoded_key_bytes).map_err(|e| {
        ErrorBuilder::validation_error("event_attribute_key", key_str, &e.to_string())
    })?;

    let value_str = attribute
        .value_str()
        .map_err(|e| ErrorBuilder::validation_error("event_attribute_value", "", &e.to_string()))?;
    let decoded_value_bytes = BASE64_STANDARD.decode(value_str).map_err(|e| {
        ErrorBuilder::validation_error("event_attribute_value", value_str, &e.to_string())
    })?;
    let value = String::from_utf8(decoded_value_bytes).map_err(|e| {
        ErrorBuilder::validation_error("event_attribute_value", value_str, &e.to_string())
    })?;
    Ok((key, value))
}
