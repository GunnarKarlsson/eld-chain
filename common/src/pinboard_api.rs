//! Pinboard (post message) HTTP types and client helpers.

use crate::tx::PostMessageUserRequest;
use serde::{Deserialize, Serialize};

/// Parameters for submitting a pinboard message via the CLI or [`crate::client::ChainClient`].
#[derive(Debug, Clone)]
pub struct PinboardMessageParams {
    pub wallet_name: String,
    pub file_path: String,
    pub content_type: String,
    pub expires_height: u64,
    pub visibility: String,
    pub topic: Option<String>,
    pub tags: Vec<String>,
    pub user_fee_amount: u128,
    pub namespace: Option<String>,
}

/// JSON body for `POST /v1/pinboard/messages:submit`.
#[derive(Debug, Serialize, Deserialize)]
pub struct PostMessageSubmitRequest {
    #[serde(flatten)]
    pub user: PostMessageUserRequest,
    pub message_b64: String,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

/// Success response from the pinboard submit endpoint.
#[derive(Debug, Serialize, Deserialize)]
pub struct PostMessageSubmitResponse {
    pub status: String,
    pub message_id: String,
    /// `hex(SHA-256(message_bytes))` — blob identity for shared storage.
    pub content_key: String,
    pub tx_hash: String,
    pub origin_validator: String,
    pub received_timestamp: u64,
    /// Set when upload used a custom namespace: `/@{namespace}/{message_id}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_path: Option<String>,
}

impl PostMessageSubmitResponse {
    /// Creates the standard success payload returned after mempool accept.
    pub fn submitted(
        message_id: String,
        content_key: String,
        tx_hash: String,
        origin_validator: String,
        received_timestamp: u64,
        content_path: Option<String>,
    ) -> Self {
        Self {
            status: "submitted".to_string(),
            message_id,
            content_key,
            tx_hash,
            origin_validator,
            received_timestamp,
            content_path,
        }
    }
}
