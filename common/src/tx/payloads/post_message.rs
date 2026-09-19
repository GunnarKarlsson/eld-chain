use crate::error::EldError;
use crate::namespace::resolve_optional_namespace;
use crate::tx::parts::{HasAmount, HasSender};
use crate::Address;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Serialization helper for u128 as string in JSON (for cross-language compatibility)
mod u128_string {
    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S>(value: &u128, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.to_string())
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<u128, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse::<u128>().map_err(serde::de::Error::custom)
    }
}

/// Maximum number of pinboard tags per post (`PostMessageUserRequest` / `PostMessageTx`).
pub const POST_MESSAGE_MAX_TAGS: usize = 4;
/// Maximum UTF-8 byte length of each pinboard tag string.
pub const POST_MESSAGE_MAX_TAG_UTF8_BYTES: usize = 64;

/// Validates pinboard tag count and per-tag byte length; normalizes values that parse as
/// [`Address`] to `0x` plus lowercase hex (20 bytes).
pub fn canonicalize_post_message_tags(tags: &[String]) -> Result<Vec<String>, EldError> {
    if tags.len() > POST_MESSAGE_MAX_TAGS {
        return Err(EldError::ValidationError {
            field: "tags".to_string(),
            value: format!("{} tags", tags.len()),
            details: format!("at most {POST_MESSAGE_MAX_TAGS} tags allowed"),
        });
    }

    let mut out = Vec::with_capacity(tags.len());
    for tag in tags {
        if tag.len() > POST_MESSAGE_MAX_TAG_UTF8_BYTES {
            return Err(EldError::ValidationError {
                field: "tags".to_string(),
                value: tag.clone(),
                details: format!(
                    "tag exceeds maximum length of {POST_MESSAGE_MAX_TAG_UTF8_BYTES} bytes"
                ),
            });
        }

        // Tags that parse as addresses are canonicalized; parse failure means a normal tag.
        let normalized = match Address::parse_hex_str(tag) {
            Ok(addr) => addr.hex_with_prefix(),
            Err(_) => tag.clone(),
        };

        if normalized.len() > POST_MESSAGE_MAX_TAG_UTF8_BYTES {
            return Err(EldError::ValidationError {
                field: "tags".to_string(),
                value: normalized,
                details: format!(
                    "normalized tag exceeds maximum length of {POST_MESSAGE_MAX_TAG_UTF8_BYTES} bytes"
                ),
            });
        }

        out.push(normalized);
    }

    Ok(out)
}

/// Allowed `content_type` values for pinboard posts (`PostMessageUserRequest` / `PostMessageTx`).
pub const POST_MESSAGE_ALLOWED_CONTENT_TYPES: &[&str] =
    &["text/plain", "application/json", "image/png"];

/// Validates pinboard `content_type`: trim, require non-empty after trim, allowlist match on trimmed value.
pub fn validate_post_message_content_type(content_type: &str) -> Result<(), EldError> {
    let trimmed = content_type.trim();
    if trimmed.is_empty() {
        return Err(EldError::ValidationError {
            field: "content_type".to_string(),
            value: content_type.to_string(),
            details: "content_type must not be empty".to_string(),
        });
    }
    if !POST_MESSAGE_ALLOWED_CONTENT_TYPES.contains(&trimmed) {
        return Err(EldError::ValidationError {
            field: "content_type".to_string(),
            value: content_type.to_string(),
            details: "unsupported mime type".to_string(),
        });
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
#[serde(try_from = "PostMessageTxUnchecked")]
pub struct PostMessageTx {
    pub sender: Address,
    pub original_signer: Address,
    pub original_signer_pubkey: String,
    /// `SHA-256(message_bytes)` hex — shared blob identity (see pinboard blob model).
    pub content_key: String,
    /// `SHA-256` hex of the full user signing preimage (JSON commitment + raw message bytes).
    pub message_id: String,
    pub expires_height: u64,
    pub visibility: String,
    pub topic: Option<String>,
    /// Application-level filter tags (max [`POST_MESSAGE_MAX_TAGS`], max
    /// [`POST_MESSAGE_MAX_TAG_UTF8_BYTES`] UTF-8 bytes each); address-shaped tags are canonicalized
    /// to `0x` + lowercase hex.
    #[serde(default)]
    pub tags: Vec<String>,
    pub content_type: String,
    #[serde(with = "u128_string")]
    pub fee_amount: u128,
    pub received_timestamp: u64,
    pub user_signature: String,
    /// Optional custom namespace (letter-only canonical slug).
    #[serde(default)]
    pub namespace: Option<String>,
}

#[derive(Deserialize)]
struct PostMessageTxUnchecked {
    sender: Address,
    original_signer: Address,
    original_signer_pubkey: String,
    content_key: String,
    message_id: String,
    expires_height: u64,
    visibility: String,
    topic: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    content_type: String,
    #[serde(with = "u128_string")]
    fee_amount: u128,
    received_timestamp: u64,
    user_signature: String,
    #[serde(default)]
    namespace: Option<String>,
}

impl TryFrom<PostMessageTxUnchecked> for PostMessageTx {
    type Error = EldError;

    fn try_from(unchecked: PostMessageTxUnchecked) -> Result<Self, Self::Error> {
        PostMessageTx::new(
            unchecked.sender,
            unchecked.original_signer,
            unchecked.original_signer_pubkey,
            unchecked.content_key,
            unchecked.message_id,
            unchecked.expires_height,
            unchecked.visibility,
            unchecked.topic,
            unchecked.tags,
            unchecked.content_type,
            unchecked.fee_amount,
            unchecked.received_timestamp,
            unchecked.user_signature,
            unchecked.namespace,
        )
    }
}

impl PostMessageTx {
    /// Field list matches the wire payload; a params struct would be an extra public type.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        sender: Address,
        original_signer: Address,
        original_signer_pubkey: String,
        content_key: String,
        message_id: String,
        expires_height: u64,
        visibility: String,
        topic: Option<String>,
        tags: Vec<String>,
        content_type: String,
        fee_amount: u128,
        received_timestamp: u64,
        user_signature: String,
        namespace: Option<String>,
    ) -> Result<Self, EldError> {
        validate_post_message_content_type(&content_type)?;
        let tags = canonicalize_post_message_tags(&tags)?;
        let namespace = resolve_optional_namespace(namespace)?;
        Ok(Self {
            sender,
            original_signer,
            original_signer_pubkey,
            content_key,
            message_id,
            expires_height,
            visibility,
            topic,
            tags,
            content_type,
            fee_amount,
            received_timestamp,
            user_signature,
            namespace,
        })
    }
}

impl std::fmt::Display for PostMessageTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PostMessageTx {{\n sender: {}\n original_signer: {}\n content_key: {}\n message_id: {}\n expires_height: {}\n visibility: {}\n topic: {:?}\n tags: {:?}\n content_type: {}\n fee_amount: {}\n received_timestamp: {}\n }}",
            self.sender,
            self.original_signer,
            self.content_key,
            self.message_id,
            self.expires_height,
            self.visibility,
            self.topic,
            self.tags,
            self.content_type,
            self.fee_amount,
            self.received_timestamp
        )
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct PostMessageUserRequest {
    pub original_signer: String,
    pub original_signer_pubkey: String,
    pub content_key: String,
    pub message_id: String,
    pub expires_height: u64,
    pub visibility: String,
    pub topic: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub content_type: String,
    #[serde(with = "u128_string")]
    pub fee_amount: u128,
    pub user_signature: String,
    /// Optional custom namespace (letter-only canonical slug).
    #[serde(default)]
    pub namespace: Option<String>,
}

/// User-signed pinboard commitment fields (excludes derived signer identity and signature).
#[derive(Debug, Clone)]
pub struct PostMessageUserRequestInput {
    pub expires_height: u64,
    pub visibility: String,
    pub topic: Option<String>,
    pub tags: Vec<String>,
    pub content_type: String,
    pub fee_amount: u128,
    pub namespace: Option<String>,
}

#[derive(Debug, Serialize)]
struct PostMessageUserSigningPayload<'a> {
    original_signer: &'a str,
    original_signer_pubkey: &'a str,
    content_key: &'a str,
    expires_height: u64,
    visibility: &'a str,
    topic: &'a Option<String>,
    tags: &'a [String],
    content_type: &'a str,
    #[serde(with = "u128_string")]
    fee_amount: u128,
    /// Omitted when `None` so pre-namespace nodes and clients compute the same commitment hash.
    #[serde(skip_serializing_if = "Option::is_none")]
    namespace: &'a Option<String>,
}

fn post_message_user_signing_bytes_with_tags(
    message_bytes: &[u8],
    request: &PostMessageUserRequest,
    tags: &[String],
) -> Result<Vec<u8>, String> {
    let payload = PostMessageUserSigningPayload {
        original_signer: &request.original_signer,
        original_signer_pubkey: &request.original_signer_pubkey,
        content_key: &request.content_key,
        expires_height: request.expires_height,
        visibility: &request.visibility,
        topic: &request.topic,
        tags,
        content_type: &request.content_type,
        fee_amount: request.fee_amount,
        namespace: &request.namespace,
    };

    let canonical_payload = serde_json::to_vec(&payload)
        .map_err(|e| format!("failed to serialize user signing payload: {e}"))?;

    let mut bytes = canonical_payload;
    bytes.extend_from_slice(message_bytes);
    Ok(bytes)
}

/// Blob identity: `hex(SHA-256(message_bytes))`. Same octets → same key for every originator.
pub fn calculate_message_content_key(message_bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(message_bytes);
    hex::encode(hasher.finalize())
}

/// Post commitment id: `hex(SHA-256(signing_bytes))` where `signing_bytes` is from
/// [`post_message_user_signing_bytes`].
pub fn post_message_id_from_signing_bytes(signing_bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(signing_bytes);
    hex::encode(hasher.finalize())
}

pub fn validate_post_message_user_signature(
    message_bytes: &[u8],
    request: &PostMessageUserRequest,
) -> Result<(), String> {
    validate_post_message_content_type(&request.content_type).map_err(|e| e.to_string())?;
    resolve_optional_namespace(request.namespace.clone()).map_err(|e| e.to_string())?;
    let expected_content_key = calculate_message_content_key(message_bytes);
    if expected_content_key != request.content_key {
        return Err("content_key does not match SHA-256(message_bytes)".to_string());
    }

    let pubkey_bytes = hex::decode(&request.original_signer_pubkey)
        .map_err(|e| format!("invalid original_signer_pubkey hex: {e}"))?;
    let verifying_key = VerifyingKey::from_bytes(
        &pubkey_bytes
            .try_into()
            .map_err(|_| "original_signer_pubkey must be 32 bytes".to_string())?,
    )
    .map_err(|e| format!("invalid original_signer_pubkey: {e}"))?;

    Address::parse_hex_str(&request.original_signer)
        .map_err(|e| e.to_string())?
        .verify_derives_from_pubkey_hex(&request.original_signer_pubkey)
        .map_err(|_| "original_signer does not match original_signer_pubkey".to_string())?;

    let tags = canonicalize_post_message_tags(&request.tags).map_err(|e| e.to_string())?;
    let signing_bytes = post_message_user_signing_bytes_with_tags(message_bytes, request, &tags)?;
    let expected_message_id = post_message_id_from_signing_bytes(&signing_bytes);
    if expected_message_id != request.message_id {
        return Err("message_id does not match commitment hash of signing preimage".to_string());
    }

    let signature_bytes = hex::decode(&request.user_signature)
        .map_err(|e| format!("invalid user_signature hex: {e}"))?;
    let signature = Signature::from_bytes(
        &signature_bytes
            .try_into()
            .map_err(|_| "user_signature must be 64 bytes".to_string())?,
    );

    verifying_key
        .verify(&signing_bytes, &signature)
        .map_err(|e| format!("user signature verification failed: {e}"))
}

/// Builds a [`PostMessageUserRequest`] with `user_signature` set, using the same preimage as
/// [`validate_post_message_user_signature`].
pub fn build_signed_post_message_user_request(
    signing_key: &SigningKey,
    message_bytes: &[u8],
    input: PostMessageUserRequestInput,
) -> Result<PostMessageUserRequest, String> {
    let PostMessageUserRequestInput {
        expires_height,
        visibility,
        topic,
        tags,
        content_type,
        fee_amount,
        namespace,
    } = input;
    validate_post_message_content_type(&content_type).map_err(|e| e.to_string())?;
    let tags = canonicalize_post_message_tags(&tags).map_err(|e| e.to_string())?;
    let namespace = resolve_optional_namespace(namespace).map_err(|e| e.to_string())?;
    let verifying_key = signing_key.verifying_key();
    let original_signer = Address::from_public_key(&verifying_key)
        .map_err(|e| e.to_string())?
        .hex_with_prefix();
    let original_signer_pubkey = hex::encode(verifying_key.to_bytes());
    let content_key = calculate_message_content_key(message_bytes);
    let mut req = PostMessageUserRequest {
        original_signer,
        original_signer_pubkey,
        content_key,
        message_id: String::new(),
        expires_height,
        visibility,
        topic,
        tags: tags.clone(),
        content_type,
        fee_amount,
        user_signature: String::new(),
        namespace,
    };
    let signing_bytes = post_message_user_signing_bytes_with_tags(message_bytes, &req, &tags)?;
    req.message_id = post_message_id_from_signing_bytes(&signing_bytes);
    req.user_signature = hex::encode(signing_key.sign(&signing_bytes).to_bytes());
    Ok(req)
}

impl HasSender for PostMessageTx {
    fn sender(&self) -> Address {
        self.sender
    }
}

impl HasAmount for PostMessageTx {
    fn amount(&self) -> u128 {
        0
    }
}
