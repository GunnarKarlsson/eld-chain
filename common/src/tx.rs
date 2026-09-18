use crate::capacity_proof::ChunkProof;
use crate::coin::Coin;
use crate::error::EldError;
use crate::namespace::{
    is_reserved_namespace_slug, normalize_namespace_slug, resolve_optional_namespace,
};
use crate::nonce::Nonce;
use crate::Address;
use abci::types::EventAttribute;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use hex;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json;
use sha2::{Digest, Sha256};
use tracing::warn;

/// Serialization helper for u128 as string in JSON (for cross-language compatibility)
mod u128_string {
    use super::*;

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

/// Transaction-boundary amount wrapper.
///
/// We intentionally keep tx payload fields as `TxAmount` instead of `Coin`:
/// - `TxAmount` represents wire payload intent only and keeps tx schema stable.
/// - `Coin` is a richer state/economics type with validation and arithmetic semantics.
/// - Conversion to `Coin` is done during tx validation/processing.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct TxAmount(pub u128);

impl TxAmount {
    pub fn as_u128(self) -> u128 {
        self.0
    }

    pub fn checked_add(self, rhs: u128) -> Option<Self> {
        self.0.checked_add(rhs).map(Self)
    }

    pub fn saturating_sub(self, rhs: Self) -> Self {
        Self(self.0.saturating_sub(rhs.0))
    }

    pub fn to_coin(self) -> Result<Coin, EldError> {
        Coin::new(self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TxPublicKey(String);

impl TxPublicKey {
    pub const LEN: usize = 32;

    pub fn new(value: String) -> Result<Self, String> {
        let bytes = hex::decode(&value).map_err(|e| format!("invalid public_key hex: {e}"))?;
        if bytes.len() != Self::LEN {
            return Err(format!(
                "public_key must be {} bytes, got {} bytes",
                Self::LEN,
                bytes.len()
            ));
        }
        let array: [u8; Self::LEN] = bytes
            .try_into()
            .map_err(|_| format!("public_key must be {} bytes", Self::LEN))?;
        VerifyingKey::from_bytes(&array).map_err(|e| format!("invalid ed25519 public_key: {e}"))?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn to_verifying_key(&self) -> Result<VerifyingKey, String> {
        let bytes = hex::decode(&self.0).map_err(|e| format!("invalid public_key hex: {e}"))?;
        let array: [u8; Self::LEN] = bytes
            .try_into()
            .map_err(|_| format!("public_key must be {} bytes", Self::LEN))?;
        VerifyingKey::from_bytes(&array).map_err(|e| format!("invalid ed25519 public_key: {e}"))
    }
}

impl std::fmt::Display for TxPublicKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Serialize for TxPublicKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for TxPublicKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        TxPublicKey::new(value).map_err(serde::de::Error::custom)
    }
}

impl From<VerifyingKey> for TxPublicKey {
    fn from(value: VerifyingKey) -> Self {
        Self(hex::encode(value.to_bytes()))
    }
}

impl From<&VerifyingKey> for TxPublicKey {
    fn from(value: &VerifyingKey) -> Self {
        Self(hex::encode(value.to_bytes()))
    }
}

impl From<TxPublicKey> for String {
    fn from(value: TxPublicKey) -> Self {
        value.0
    }
}

impl From<String> for TxPublicKey {
    fn from(value: String) -> Self {
        TxPublicKey::new(value).expect("TxPublicKey::from received invalid public key")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct TxSig(String);

impl TxSig {
    pub const LEN: usize = 64;

    pub fn new(value: String) -> Result<Self, String> {
        let bytes = hex::decode(&value).map_err(|e| format!("invalid signature hex: {e}"))?;
        if bytes.len() != Self::LEN {
            return Err(format!(
                "signature must be {} bytes, got {} bytes",
                Self::LEN,
                bytes.len()
            ));
        }
        let array: [u8; Self::LEN] = bytes
            .try_into()
            .map_err(|_| format!("signature must be {} bytes", Self::LEN))?;
        let _ = Signature::from_bytes(&array);
        Ok(Self(value))
    }

    pub fn empty() -> Self {
        Self(String::new())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn to_signature(&self) -> Result<Signature, String> {
        let bytes = hex::decode(&self.0).map_err(|e| format!("invalid signature hex: {e}"))?;
        let array: [u8; Self::LEN] = bytes
            .try_into()
            .map_err(|_| format!("signature must be {} bytes", Self::LEN))?;
        Ok(Signature::from_bytes(&array))
    }
}

impl std::fmt::Display for TxSig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Serialize for TxSig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for TxSig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if value.is_empty() {
            return Ok(TxSig::empty());
        }
        TxSig::new(value).map_err(serde::de::Error::custom)
    }
}

impl From<Signature> for TxSig {
    fn from(value: Signature) -> Self {
        Self(hex::encode(value.to_bytes()))
    }
}

impl From<TxSig> for String {
    fn from(value: TxSig) -> Self {
        value.0
    }
}

impl From<String> for TxSig {
    fn from(value: String) -> Self {
        if value.is_empty() {
            return TxSig::empty();
        }
        TxSig::new(value).expect("TxSig::from received invalid signature")
    }
}

impl std::fmt::Display for TxAmount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u128> for TxAmount {
    fn from(value: u128) -> Self {
        Self(value)
    }
}

impl From<TxAmount> for u128 {
    fn from(value: TxAmount) -> Self {
        value.0
    }
}

impl From<Coin> for TxAmount {
    fn from(value: Coin) -> Self {
        Self(value.amount())
    }
}

impl PartialEq<u128> for TxAmount {
    fn eq(&self, other: &u128) -> bool {
        self.0 == *other
    }
}

impl Serialize for TxAmount {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u128(self.0)
    }
}

impl<'de> Deserialize<'de> for TxAmount {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self(u128::deserialize(deserializer)?))
    }
}

/// Serialization helper for TxAmount as string in JSON (for cross-language compatibility)
mod tx_amount_string {
    use super::*;

    pub(super) fn serialize<S>(value: &TxAmount, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.0.to_string())
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<TxAmount, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let value = s.parse::<u128>().map_err(serde::de::Error::custom)?;
        Ok(TxAmount(value))
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
#[serde(try_from = "TransferTxUnchecked")]
pub struct TransferTx {
    pub sender: Address,
    pub recipient: Address,
    #[serde(with = "tx_amount_string")]
    pub amount: TxAmount,
}

/// Wire/decoding shape for [`TransferTx`]; validation runs in [`TransferTx::new`].
#[derive(Deserialize)]
struct TransferTxUnchecked {
    sender: Address,
    recipient: Address,
    #[serde(with = "tx_amount_string")]
    amount: TxAmount,
}

impl TryFrom<TransferTxUnchecked> for TransferTx {
    type Error = EldError;

    fn try_from(unchecked: TransferTxUnchecked) -> Result<Self, Self::Error> {
        TransferTx::new(unchecked.sender, unchecked.recipient, unchecked.amount)
    }
}

impl TransferTx {
    /// Builds a transfer after validating sender, recipient, and amount.
    pub fn new(sender: Address, recipient: Address, amount: TxAmount) -> Result<Self, EldError> {
        if sender == recipient {
            return Err(EldError::ValidationError {
                field: "transfer transaction".to_string(),
                value: format!("sender: {sender}, recipient: {recipient}"),
                details: "Sender and recipient cannot be the same".to_string(),
            });
        }

        let amount_coin = Coin::new(amount.into()).map_err(|_| EldError::ValidationError {
            field: "transfer amount".to_string(),
            value: amount.to_string(),
            details: "Invalid transfer amount (must be non-negative and within bounds)".to_string(),
        })?;

        let value: u128 = amount_coin.into();
        if value == 0 {
            EldError::validation_error(
                "transfer amount",
                &value.to_string(),
                "Transfer amount cannot be zero",
            )?;
        }

        Ok(Self {
            sender,
            recipient,
            amount,
        })
    }
}

impl std::fmt::Display for TransferTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "TransferTx {{\n sender: {}\n recipient: {}\n amount: {}\n }}",
            self.sender, self.recipient, self.amount
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(try_from = "StakeTxUnchecked")]
pub struct StakeTx {
    pub sender: Address,
    #[serde(with = "tx_amount_string")]
    pub amount: TxAmount,
    #[serde(default)]
    pub public_key: Option<String>, // Ed25519 public key in hex format
}

#[derive(Deserialize)]
struct StakeTxUnchecked {
    sender: Address,
    #[serde(with = "tx_amount_string")]
    amount: TxAmount,
    #[serde(default)]
    public_key: Option<String>,
}

impl TryFrom<StakeTxUnchecked> for StakeTx {
    type Error = EldError;

    fn try_from(unchecked: StakeTxUnchecked) -> Result<Self, Self::Error> {
        StakeTx::new(unchecked.sender, unchecked.amount, unchecked.public_key)
    }
}

fn stake_validate_public_key(pk: &str, expected_len: usize) -> Result<(), EldError> {
    if pk.len() != expected_len {
        return Err(EldError::ValidationError {
            field: "public key".to_string(),
            value: pk.to_string(),
            details: format!(
                "Public key must be {} hex characters, got {} characters",
                expected_len,
                pk.len()
            ),
        });
    }
    if !pk.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(EldError::ValidationError {
            field: "public key".to_string(),
            value: pk.to_string(),
            details: "Public key must be valid hex".to_string(),
        });
    }
    Ok(())
}

impl StakeTx {
    pub fn new(
        sender: Address,
        amount: TxAmount,
        public_key: Option<String>,
    ) -> Result<Self, EldError> {
        let amount_coin = Coin::new(amount.into())?;
        if amount_coin < Coin::new(crate::constants::protocol::MIN_STAKE_AMOUNT)? {
            EldError::validation_error(
                "stake amount",
                &amount.to_string(),
                &format!(
                    "Stake amount {} is below minimum required {}",
                    amount_coin,
                    Coin::new(crate::constants::protocol::MIN_STAKE_AMOUNT)?
                ),
            )?;
        }

        if let Some(ref pk) = public_key {
            stake_validate_public_key(pk, 64)?;
        }

        Ok(Self {
            sender,
            amount,
            public_key,
        })
    }
}

impl std::fmt::Display for StakeTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "StakeTx {{\n sender: {}\n amount: {}\n public_key: {:?}\n }}",
            self.sender, self.amount, self.public_key
        )
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
#[serde(deny_unknown_fields)]
#[serde(try_from = "UnstakeTxUnchecked")]
pub struct UnstakeTx {
    pub sender: Address,
    #[serde(with = "tx_amount_string")]
    pub amount: TxAmount,
}

/// Wire shape for [`UnstakeTx`]. With `try_from`, unknown-field rejection must be on this struct
/// (not only on [`UnstakeTx`]); otherwise untagged [`PayloadInner`] can match unstake for stake
/// JSON and drop `public_key`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnstakeTxUnchecked {
    sender: Address,
    #[serde(with = "tx_amount_string")]
    amount: TxAmount,
}

impl TryFrom<UnstakeTxUnchecked> for UnstakeTx {
    type Error = EldError;

    fn try_from(unchecked: UnstakeTxUnchecked) -> Result<Self, Self::Error> {
        UnstakeTx::new(unchecked.sender, unchecked.amount)
    }
}

impl UnstakeTx {
    pub fn new(sender: Address, amount: TxAmount) -> Result<Self, EldError> {
        if amount == 0 {
            return Err(EldError::ValidationError {
                field: "unstake amount".to_string(),
                value: amount.to_string(),
                details: "Unstake amount cannot be zero".to_string(),
            });
        }

        let amount_coin = Coin::new(amount.into())?;
        if amount_coin > Coin::max() {
            return Err(EldError::ValidationError {
                field: "unstake amount".to_string(),
                value: amount.to_string(),
                details: "Unstake amount exceeds maximum coin value".to_string(),
            });
        }

        Ok(Self { sender, amount })
    }
}

impl std::fmt::Display for UnstakeTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "UnstakeTx {{\n sender: {}\n amount: {}\n }}",
            self.sender, self.amount
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(try_from = "RegisterCapacityTxUnchecked")]
pub struct RegisterCapacityTx {
    pub sender: Address,     // Address of the sender (capacity provider)
    pub capacity_bytes: u64, // Total capacity in bytes
    #[serde(with = "hex_vec_u8_32")]
    pub merkle_root: [u8; 32], // Merkle root of capacity proof
    #[serde(with = "hex_vec_u8_32")]
    pub seed: [u8; 32], // Seed for capacity proof
    pub chunk_count: u32,    // Number of chunks in the capacity proof
}

#[derive(Deserialize)]
struct RegisterCapacityTxUnchecked {
    sender: Address,
    capacity_bytes: u64,
    #[serde(with = "hex_vec_u8_32")]
    merkle_root: [u8; 32],
    #[serde(with = "hex_vec_u8_32")]
    seed: [u8; 32],
    chunk_count: u32,
}

impl TryFrom<RegisterCapacityTxUnchecked> for RegisterCapacityTx {
    type Error = EldError;

    fn try_from(unchecked: RegisterCapacityTxUnchecked) -> Result<Self, Self::Error> {
        RegisterCapacityTx::new(
            unchecked.sender,
            unchecked.capacity_bytes,
            unchecked.merkle_root,
            unchecked.seed,
            unchecked.chunk_count,
        )
    }
}

impl RegisterCapacityTx {
    pub fn new(
        sender: Address,
        capacity_bytes: u64,
        merkle_root: [u8; 32],
        seed: [u8; 32],
        chunk_count: u32,
    ) -> Result<Self, EldError> {
        if capacity_bytes == 0 {
            return Err(EldError::ValidationError {
                field: "capacity_bytes".to_string(),
                value: capacity_bytes.to_string(),
                details: "RegisterCapacity capacity_bytes must be greater than zero".to_string(),
            });
        }
        // TODO: what invariant enforce for chunk count

        Ok(Self {
            sender,
            capacity_bytes,
            merkle_root,
            seed,
            chunk_count,
        })
    }
}

impl std::fmt::Display for RegisterCapacityTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RegisterCapacityTx {{\n sender: {}\n capacity_bytes: {}\n merkle_root: {}\n seed: {}\n chunk_count: {}\n }}",
            self.sender,
            self.capacity_bytes,
            hex::encode(self.merkle_root),
            hex::encode(self.seed),
            self.chunk_count
        )
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
#[serde(try_from = "UpdateCapacityMerkleRootTxUnchecked")]
pub struct UpdateCapacityMerkleRootTx {
    pub sender: Address, // Address of the capacity provider
    #[serde(with = "hex_vec_u8_32")]
    pub merkle_root: [u8; 32], // New merkle root after content storage
}

#[derive(Deserialize)]
struct UpdateCapacityMerkleRootTxUnchecked {
    sender: Address,
    #[serde(with = "hex_vec_u8_32")]
    merkle_root: [u8; 32],
}

impl TryFrom<UpdateCapacityMerkleRootTxUnchecked> for UpdateCapacityMerkleRootTx {
    type Error = EldError;

    fn try_from(unchecked: UpdateCapacityMerkleRootTxUnchecked) -> Result<Self, Self::Error> {
        UpdateCapacityMerkleRootTx::new(unchecked.sender, unchecked.merkle_root)
    }
}

impl UpdateCapacityMerkleRootTx {
    pub fn new(sender: Address, merkle_root: [u8; 32]) -> Result<Self, EldError> {
        Ok(Self {
            sender,
            merkle_root,
        })
    }
}

impl std::fmt::Display for UpdateCapacityMerkleRootTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "UpdateCapacityMerkleRootTx {{\n sender: {}\n merkle_root: {}\n }}",
            self.sender,
            hex::encode(self.merkle_root)
        )
    }
}

// Helper module for serializing [u8; 32] as hex string (non-optional)
mod hex_vec_u8_32 {
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S>(bytes: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(bytes))
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 32], D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: String = String::deserialize(deserializer)?;
        let bytes = hex::decode(s).map_err(serde::de::Error::custom)?;
        if bytes.len() != 32 {
            return Err(serde::de::Error::custom("Expected 32 bytes"));
        }
        let mut array = [0u8; 32];
        array.copy_from_slice(&bytes);
        Ok(array)
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

        // TODO: Not use error as control flow
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

pub fn validate_message_and_build_post_message_tx(
    message_bytes: &[u8],
    user_request: &PostMessageUserRequest,
    validator_signing_key: &SigningKey,
    validator_tx_nonce: u32,
    received_timestamp: u64,
    chain_id: &str,
    tx_fee: u128,
) -> Result<Tx, String> {
    validate_post_message_user_signature(message_bytes, user_request)?;

    let validator_pubkey = validator_signing_key.verifying_key();
    let sender = Address::from_public_key(&validator_pubkey).map_err(|e| e.to_string())?;
    let original_signer =
        Address::parse_hex_str(&user_request.original_signer).map_err(|e| e.to_string())?;

    let post_message = PostMessageTx::new(
        sender,
        original_signer,
        user_request.original_signer_pubkey.clone(),
        user_request.content_key.clone(),
        user_request.message_id.clone(),
        user_request.expires_height,
        user_request.visibility.clone(),
        user_request.topic.clone(),
        user_request.tags.clone(),
        user_request.content_type.clone(),
        user_request.fee_amount,
        received_timestamp,
        user_request.user_signature.clone(),
        user_request.namespace.clone(),
    )
    .map_err(|e| e.to_string())?;

    let mut tx = Tx::new(
        Nonce::new(validator_tx_nonce),
        Payload::new(post_message),
        hex::encode(validator_pubkey.to_bytes()),
    );
    tx.fee = tx_fee.into();
    tx.sign(validator_signing_key, chain_id);

    Ok(tx)
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
#[serde(try_from = "VerifiedProofTxUnchecked")]
pub struct VerifiedProofTx {
    pub sender: Address,            // Storage validator (challenger)
    pub capacity_provider: Address, // Capacity provider that passed
    pub challenge_id: String,       // Challenge ID that was verified
    pub block_height: i64,          // Block height when challenge was issued
    pub verified_at_block: i64,     // Current block height
    pub verified_at_timestamp: u64, // Unix timestamp
    /// Chunk proofs from the CP response (same as P2P `CapacityChallengeResponse.proofs`).
    pub proofs: Vec<ChunkProof>,
    /// When the CP generated/signed the response (part of the CP signature preimage).
    pub generated_at: u64,
    /// Hex Ed25519 pubkey of the capacity provider (must derive to `capacity_provider`).
    pub provider_pubkey: String,
    /// Hex Ed25519 signature over the capacity challenge response signing preimage.
    pub provider_signature: String,
}

#[derive(Deserialize)]
struct VerifiedProofTxUnchecked {
    sender: Address,
    capacity_provider: Address,
    challenge_id: String,
    block_height: i64,
    verified_at_block: i64,
    verified_at_timestamp: u64,
    proofs: Vec<ChunkProof>,
    generated_at: u64,
    provider_pubkey: String,
    provider_signature: String,
}

impl TryFrom<VerifiedProofTxUnchecked> for VerifiedProofTx {
    type Error = EldError;

    fn try_from(unchecked: VerifiedProofTxUnchecked) -> Result<Self, Self::Error> {
        VerifiedProofTx::new(
            unchecked.sender,
            unchecked.capacity_provider,
            unchecked.challenge_id,
            unchecked.block_height,
            unchecked.verified_at_block,
            unchecked.verified_at_timestamp,
            unchecked.proofs,
            unchecked.generated_at,
            unchecked.provider_pubkey,
            unchecked.provider_signature,
        )
    }
}

/// Same rules as [`crate::validation::validate_safe_string`] for `challenge_id` (max 100).
fn validate_verified_proof_challenge_id(challenge_id: &str) -> Result<(), EldError> {
    if challenge_id.is_empty() {
        return Err(EldError::ValidationError {
            field: "string".to_string(),
            value: challenge_id.to_string(),
            details: "String cannot be empty".to_string(),
        });
    }

    const MAX_LEN: usize = 100;
    if challenge_id.len() > MAX_LEN {
        return Err(EldError::ValidationError {
            field: "string".to_string(),
            value: challenge_id.to_string(),
            details: format!("String exceeds maximum length of {MAX_LEN}"),
        });
    }

    let dangerous_chars = [
        '<', '>', '"', '\'', '&', ';', '|', '`', '$', '(', ')', '{', '}',
    ];
    for &ch in &dangerous_chars {
        if challenge_id.contains(ch) {
            return Err(EldError::ValidationError {
                field: "string".to_string(),
                value: challenge_id.to_string(),
                details: format!("String contains potentially dangerous character: {ch}"),
            });
        }
    }

    if challenge_id.chars().any(|c| c.is_control()) {
        return Err(EldError::ValidationError {
            field: "string".to_string(),
            value: challenge_id.to_string(),
            details: "String contains control characters".to_string(),
        });
    }

    Ok(())
}

impl VerifiedProofTx {
    /// Builds a verified-proof payload after validating challenge id, block heights, proof fields,
    /// and timestamp bounds.
    ///
    /// `proofs` / `provider_pubkey` / `provider_signature` / `generated_at` are the same fields the
    /// CP puts on a signed P2P `CapacityChallengeResponse` (verified again in consensus).
    /// Field list matches that wire payload.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        sender: Address,
        capacity_provider: Address,
        challenge_id: String,
        block_height: i64,
        verified_at_block: i64,
        verified_at_timestamp: u64,
        proofs: Vec<ChunkProof>,
        generated_at: u64,
        provider_pubkey: String,
        provider_signature: String,
    ) -> Result<Self, EldError> {
        validate_verified_proof_challenge_id(&challenge_id)?;

        if block_height <= 0 {
            EldError::validation_error(
                "block_height",
                &block_height.to_string(),
                "Block height must be positive",
            )?;
        }

        if verified_at_block <= 0 {
            EldError::validation_error(
                "verified_at_block",
                &verified_at_block.to_string(),
                "Verified at block must be positive",
            )?;
        }

        if verified_at_block < block_height {
            EldError::validation_error(
                "verified_at_block",
                &verified_at_block.to_string(),
                "Verified at block must be >= challenge block height",
            )?;
        }

        const MIN_TIMESTAMP: u64 = 1577836800;
        const MAX_TIMESTAMP: u64 = 4102444800;
        if !(MIN_TIMESTAMP..=MAX_TIMESTAMP).contains(&verified_at_timestamp) {
            EldError::validation_error(
                "verified_at_timestamp",
                &verified_at_timestamp.to_string(),
                &format!(
                    "Verified at timestamp must be between {MIN_TIMESTAMP} and {MAX_TIMESTAMP}"
                ),
            )?;
        }

        if proofs.is_empty() {
            EldError::validation_error("proofs", "[]", "VerifiedProof proofs must be non-empty")?;
        }

        if !(MIN_TIMESTAMP..=MAX_TIMESTAMP).contains(&generated_at) {
            EldError::validation_error(
                "generated_at",
                &generated_at.to_string(),
                &format!("generated_at must be between {MIN_TIMESTAMP} and {MAX_TIMESTAMP}"),
            )?;
        }

        if provider_pubkey.is_empty() {
            EldError::validation_error(
                "provider_pubkey",
                &provider_pubkey,
                "provider_pubkey must be non-empty hex",
            )?;
        }
        if provider_signature.is_empty() {
            EldError::validation_error(
                "provider_signature",
                &provider_signature,
                "provider_signature must be non-empty hex",
            )?;
        }

        Ok(Self {
            sender,
            capacity_provider,
            challenge_id,
            block_height,
            verified_at_block,
            verified_at_timestamp,
            proofs,
            generated_at,
            provider_pubkey,
            provider_signature,
        })
    }
}

impl std::fmt::Display for VerifiedProofTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "VerifiedProofTx {{\n sender: {}\n capacity_provider: {}\n challenge_id: {}\n block_height: {}\n verified_at_block: {}\n verified_at_timestamp: {}\n proofs: {}\n generated_at: {}\n }}",
            self.sender,
            self.capacity_provider,
            self.challenge_id,
            self.block_height,
            self.verified_at_block,
            self.verified_at_timestamp,
            self.proofs.len(),
            self.generated_at
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
#[serde(try_from = "AddNamespaceTxUnchecked")]
pub struct AddNamespaceTx {
    pub sender: Address,
    pub namespace_slug: String,
    #[serde(with = "tx_amount_string")]
    pub registration_fee: TxAmount,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddNamespaceTxUnchecked {
    sender: Address,
    namespace_slug: String,
    #[serde(with = "tx_amount_string")]
    registration_fee: TxAmount,
}

impl TryFrom<AddNamespaceTxUnchecked> for AddNamespaceTx {
    type Error = EldError;

    fn try_from(unchecked: AddNamespaceTxUnchecked) -> Result<Self, Self::Error> {
        AddNamespaceTx::new(
            unchecked.sender,
            unchecked.namespace_slug,
            unchecked.registration_fee,
        )
    }
}

impl AddNamespaceTx {
    /// Validates slug rules and fee; stores canonical lowercase `namespace_slug`.
    pub fn new(
        sender: Address,
        namespace_slug: String,
        registration_fee: TxAmount,
    ) -> Result<Self, EldError> {
        let canonical = normalize_namespace_slug(&namespace_slug)?;
        if is_reserved_namespace_slug(&canonical) {
            return Err(EldError::ValidationError {
                field: "namespace_slug".to_string(),
                value: canonical,
                details: "Namespace slug is reserved".to_string(),
            });
        }
        if registration_fee.as_u128() == 0 {
            return Err(EldError::ValidationError {
                field: "registration_fee".to_string(),
                value: registration_fee.to_string(),
                details: "registration_fee must be greater than zero".to_string(),
            });
        }
        Coin::new(registration_fee.into())?;
        Ok(Self {
            sender,
            namespace_slug: canonical,
            registration_fee,
        })
    }
}

impl std::fmt::Display for AddNamespaceTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "AddNamespaceTx {{\n sender: {}\n namespace_slug: {}\n registration_fee: {}\n }}",
            self.sender, self.namespace_slug, self.registration_fee
        )
    }
}

pub trait HasSender {
    fn sender(&self) -> Address;
}

pub trait HasAmount {
    fn amount(&self) -> u128;
}

impl HasSender for TransferTx {
    fn sender(&self) -> Address {
        self.sender
    }
}

impl HasAmount for TransferTx {
    fn amount(&self) -> u128 {
        self.amount.as_u128()
    }
}

impl HasSender for StakeTx {
    fn sender(&self) -> Address {
        self.sender
    }
}

impl HasAmount for StakeTx {
    fn amount(&self) -> u128 {
        self.amount.as_u128()
    }
}

impl HasSender for UnstakeTx {
    fn sender(&self) -> Address {
        self.sender
    }
}

impl HasAmount for UnstakeTx {
    fn amount(&self) -> u128 {
        self.amount.as_u128()
    }
}

impl HasSender for RegisterCapacityTx {
    fn sender(&self) -> Address {
        self.sender
    }
}

impl HasAmount for RegisterCapacityTx {
    fn amount(&self) -> u128 {
        0 // Capacity registration doesn't require an amount
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
#[serde(try_from = "UnregisterCapacityTxUnchecked")]
pub struct UnregisterCapacityTx {
    pub sender: Address,  // Address of the sender (capacity provider)
    pub unregister: bool, // Distinguishing field to avoid mixup with other tx types
}

#[derive(Deserialize)]
struct UnregisterCapacityTxUnchecked {
    sender: Address,
    unregister: bool,
}

impl TryFrom<UnregisterCapacityTxUnchecked> for UnregisterCapacityTx {
    type Error = EldError;

    fn try_from(unchecked: UnregisterCapacityTxUnchecked) -> Result<Self, Self::Error> {
        UnregisterCapacityTx::new(unchecked.sender, unchecked.unregister)
    }
}

impl UnregisterCapacityTx {
    pub fn new(sender: Address, unregister: bool) -> Result<Self, EldError> {
        // TODO: this is weird but the field exists to make the serde serialization
        // recognize difference between register and unregister
        if !unregister {
            return Err(EldError::ValidationError {
                field: "unregister".to_string(),
                value: unregister.to_string(),
                details: "UnregisterCapacity transaction 'unregister' field must be true"
                    .to_string(),
            });
        }

        Ok(Self { sender, unregister })
    }
}

impl std::fmt::Display for UnregisterCapacityTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "UnregisterCapacityTx {{\n sender: {}\n unregister: {}\n }}",
            self.sender, self.unregister
        )
    }
}

impl HasSender for UnregisterCapacityTx {
    fn sender(&self) -> Address {
        self.sender
    }
}

impl HasAmount for UnregisterCapacityTx {
    fn amount(&self) -> u128 {
        0 // Capacity unregistration doesn't require an amount
    }
}

impl HasSender for UpdateCapacityMerkleRootTx {
    fn sender(&self) -> Address {
        self.sender
    }
}

impl HasAmount for UpdateCapacityMerkleRootTx {
    fn amount(&self) -> u128 {
        0 // Merkle root update doesn't require an amount
    }
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

impl HasSender for AddNamespaceTx {
    fn sender(&self) -> Address {
        self.sender
    }
}

impl HasAmount for AddNamespaceTx {
    fn amount(&self) -> u128 {
        self.registration_fee.as_u128()
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct Payload {
    pub r#type: String, // Renamed to "type" with serde rename
    #[serde(flatten)]
    pub inner: PayloadInner,
}

impl std::fmt::Display for Payload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Payload {{\n type: {}\n inner: {}\n }}",
            self.r#type, self.inner
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxType {
    Transfer,
    Unstake,
    Stake,
    VerifiedProof,
    RegisterCapacity,
    UnregisterCapacity,
    UpdateCapacityMerkleRoot,
    PostMessage,
    AddNamespace,
}

impl TxType {
    pub fn as_str(self) -> &'static str {
        match self {
            TxType::Transfer => crate::constants::tx_type::TX_TYPE_TRANSFER,
            TxType::Unstake => crate::constants::tx_type::TX_TYPE_UNSTAKE,
            TxType::Stake => crate::constants::tx_type::TX_TYPE_STAKE,
            TxType::VerifiedProof => crate::constants::tx_type::TX_TYPE_VERIFIED_PROOF,
            TxType::RegisterCapacity => crate::constants::tx_type::TX_TYPE_REGISTER_CAPACITY,
            TxType::UnregisterCapacity => crate::constants::tx_type::TX_TYPE_UNREGISTER_CAPACITY,
            TxType::UpdateCapacityMerkleRoot => {
                crate::constants::tx_type::TX_TYPE_UPDATE_CAPACITY_MERKLE_ROOT
            }
            TxType::PostMessage => crate::constants::tx_type::TX_TYPE_POST_MESSAGE,
            TxType::AddNamespace => crate::constants::tx_type::TX_TYPE_ADD_NAMESPACE,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
#[serde(untagged)] // No extra "type" tag in JSON
pub enum PayloadInner {
    Transfer(TransferTx),
    Unstake(UnstakeTx), // needs to fo before stake
    Stake(StakeTx),
    VerifiedProof(VerifiedProofTx), // Capacity challenge proof verification
    RegisterCapacity(RegisterCapacityTx), // Capacity proof registration
    UnregisterCapacity(UnregisterCapacityTx), // Capacity proof unregistration
    UpdateCapacityMerkleRoot(UpdateCapacityMerkleRootTx), // Update merkle root after content storage
    PostMessage(PostMessageTx),
    AddNamespace(AddNamespaceTx),
}

impl From<TransferTx> for PayloadInner {
    fn from(tx: TransferTx) -> Self {
        PayloadInner::Transfer(tx)
    }
}

impl From<UnstakeTx> for PayloadInner {
    fn from(tx: UnstakeTx) -> Self {
        PayloadInner::Unstake(tx)
    }
}

impl From<StakeTx> for PayloadInner {
    fn from(tx: StakeTx) -> Self {
        PayloadInner::Stake(tx)
    }
}

impl From<VerifiedProofTx> for PayloadInner {
    fn from(tx: VerifiedProofTx) -> Self {
        PayloadInner::VerifiedProof(tx)
    }
}

impl From<RegisterCapacityTx> for PayloadInner {
    fn from(tx: RegisterCapacityTx) -> Self {
        PayloadInner::RegisterCapacity(tx)
    }
}

impl From<UnregisterCapacityTx> for PayloadInner {
    fn from(tx: UnregisterCapacityTx) -> Self {
        PayloadInner::UnregisterCapacity(tx)
    }
}

impl From<UpdateCapacityMerkleRootTx> for PayloadInner {
    fn from(tx: UpdateCapacityMerkleRootTx) -> Self {
        PayloadInner::UpdateCapacityMerkleRoot(tx)
    }
}

impl From<PostMessageTx> for PayloadInner {
    fn from(tx: PostMessageTx) -> Self {
        PayloadInner::PostMessage(tx)
    }
}

impl From<AddNamespaceTx> for PayloadInner {
    fn from(tx: AddNamespaceTx) -> Self {
        PayloadInner::AddNamespace(tx)
    }
}

impl PayloadInner {
    pub fn tx_type(&self) -> TxType {
        match self {
            PayloadInner::Transfer(_) => TxType::Transfer,
            PayloadInner::Unstake(_) => TxType::Unstake,
            PayloadInner::Stake(_) => TxType::Stake,
            PayloadInner::VerifiedProof(_) => TxType::VerifiedProof,
            PayloadInner::RegisterCapacity(_) => TxType::RegisterCapacity,
            PayloadInner::UnregisterCapacity(_) => TxType::UnregisterCapacity,
            PayloadInner::UpdateCapacityMerkleRoot(_) => TxType::UpdateCapacityMerkleRoot,
            PayloadInner::PostMessage(_) => TxType::PostMessage,
            PayloadInner::AddNamespace(_) => TxType::AddNamespace,
        }
    }
}

impl Payload {
    pub fn new<T>(payload_tx: T) -> Self
    where
        T: Into<PayloadInner>,
    {
        let inner = payload_tx.into();
        Self {
            r#type: inner.tx_type().as_str().to_string(),
            inner,
        }
    }
}

impl std::fmt::Display for PayloadInner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PayloadInner::Transfer(tx) => write!(f, "Transfer({tx})"),
            PayloadInner::Unstake(tx) => write!(f, "Unstake({tx})"),
            PayloadInner::Stake(tx) => write!(f, "Stake({tx})"),
            PayloadInner::VerifiedProof(tx) => write!(f, "VerifiedProof({tx})"),
            PayloadInner::RegisterCapacity(tx) => write!(f, "RegisterCapacity({tx})"),
            PayloadInner::UnregisterCapacity(tx) => write!(f, "UnregisterCapacity({tx})"),
            PayloadInner::UpdateCapacityMerkleRoot(tx) => {
                write!(f, "UpdateCapacityMerkleRoot({tx})")
            }
            PayloadInner::PostMessage(tx) => write!(f, "PostMessage({tx})"),
            PayloadInner::AddNamespace(tx) => write!(f, "AddNamespace({tx})"),
        }
    }
}

impl HasSender for PayloadInner {
    fn sender(&self) -> Address {
        match self {
            PayloadInner::Transfer(tx) => tx.sender(),
            PayloadInner::Stake(tx) => tx.sender(),
            PayloadInner::Unstake(tx) => tx.sender(),
            PayloadInner::RegisterCapacity(tx) => tx.sender(),
            PayloadInner::UnregisterCapacity(tx) => tx.sender(),
            PayloadInner::UpdateCapacityMerkleRoot(tx) => tx.sender(),
            PayloadInner::PostMessage(tx) => tx.sender(),
            PayloadInner::VerifiedProof(tx) => tx.sender,
            PayloadInner::AddNamespace(tx) => tx.sender(),
        }
    }
}

impl HasAmount for PayloadInner {
    fn amount(&self) -> u128 {
        match self {
            PayloadInner::Transfer(tx) => tx.amount(),
            PayloadInner::Stake(tx) => tx.amount(),
            PayloadInner::Unstake(tx) => tx.amount(),
            PayloadInner::RegisterCapacity(tx) => tx.amount(),
            PayloadInner::UnregisterCapacity(tx) => tx.amount(),
            PayloadInner::UpdateCapacityMerkleRoot(tx) => tx.amount(),
            PayloadInner::PostMessage(tx) => tx.amount(),
            PayloadInner::VerifiedProof(_tx) => 0, // VerifiedProof has no amount
            PayloadInner::AddNamespace(tx) => tx.amount(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct Tx {
    pub sig: TxSig,
    pub nonce: Nonce,
    pub payload: Payload,
    pub public_key: TxPublicKey,
    pub fee: TxAmount,
}

impl Tx {
    pub fn new(
        nonce: impl Into<Nonce>,
        payload: Payload,
        public_key: impl Into<TxPublicKey>,
    ) -> Self {
        Self {
            sig: TxSig::empty(),
            nonce: nonce.into(),
            payload,
            public_key: public_key.into(),
            fee: 0.into(),
        }
    }

    pub fn sign(&mut self, signing_key: &SigningKey, chain_id: &str) -> String {
        self.sig = TxSig::empty();
        let json = serde_json::to_string(self)
            .expect("Failed to serialize transaction to JSON for signing");

        // Create the signing bytes: JSON bytes + chain_id bytes
        let mut all_bytes = json.as_bytes().to_vec();
        all_bytes.extend_from_slice(chain_id.as_bytes());

        let signature = signing_key.sign(&all_bytes);
        self.sig = signature.into();
        self.sig.as_str().to_string()
    }

    pub fn verify(&self, chain_id: &str) -> bool {
        // Step 1: Verify the signature using public key
        let tx_to_verify = Tx {
            sig: TxSig::empty(),
            nonce: self.nonce,
            payload: self.payload.clone(),
            public_key: self.public_key.clone(),
            fee: self.fee,
        };

        let json = serde_json::to_string(&tx_to_verify)
            .expect("Failed to serialize transaction to JSON for verification");

        // Create the verification bytes: JSON bytes + chain_id bytes
        let mut all_bytes = json.as_bytes().to_vec();
        all_bytes.extend_from_slice(chain_id.as_bytes());

        let public_key = self
            .public_key
            .to_verifying_key()
            .expect("Failed to create verifying key from public key");
        let signature = self
            .sig
            .to_signature()
            .expect("Failed to decode signature bytes");

        let sig_valid = public_key.verify(&all_bytes, &signature).is_ok();
        if !sig_valid {
            warn!("signature is not valid");
            return false;
        }

        // Step 2: Verify the sender address is derived from the public key
        let derived_address = Address::from_public_key(&public_key)
            .expect("Failed to derive address from public key");
        let payload_sender = self.payload.inner.sender();

        let addr_valid = payload_sender == derived_address;
        if !addr_valid {
            warn!("address is not valid");
        }
        addr_valid
    }
}

impl std::fmt::Display for Tx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Tx {{\n sig: {}\n nonce: {}\n payload: {}\n public_key: {}\n fee: {}\n }}",
            self.sig, self.nonce, self.payload, self.public_key, self.fee
        )
    }
}

pub fn create_event_attribute(key: String, value: String) -> EventAttribute {
    let e = EventAttribute {
        key: key.as_bytes().to_vec(),
        value: value.as_bytes().to_vec(),
        index: true,
    };
    e
}

#[cfg(test)]
mod tests {
    use crate::constants::{protocol::DEFAULT_TX_FEE, test::MOCK_CHAIN_ID, tx_type};
    use tracing::info;

    use super::*;

    /// Documented throwaway seed (all 0x01). Not a live-network key.
    fn throwaway_signing_key() -> SigningKey {
        SigningKey::from_bytes(&[1u8; 32])
    }

    #[test]
    fn test_tx_with_transfer_sign_verify() {
        let signing_key = throwaway_signing_key();
        let verifying_key = signing_key.verifying_key();
        let sender = Address::from_public_key(&verifying_key)
            .expect("Failed to derive address from public key in test");
        let recipient = Address::parse_hex_str("0x0987654321098765432109876543210987654321")
            .expect("valid recipient in test");

        let transfer = TransferTx::new(sender, recipient, 1.into()).expect("valid transfer");
        let mut tx = Tx {
            sig: TxSig::empty(),
            nonce: 1u32.into(),
            payload: Payload::new(transfer),
            public_key: hex::encode(verifying_key.to_bytes()).into(),
            fee: DEFAULT_TX_FEE.into(),
        };

        tx.sign(&signing_key, MOCK_CHAIN_ID);
        let json =
            serde_json::to_string(&tx).expect("Failed to serialize transaction to JSON in test");
        info!("Rust JSON: {}", json);
        let hex = hex::encode(&json);
        info!("Rust Hex: {}", hex);

        assert!(
            tx.verify(MOCK_CHAIN_ID),
            "Verification failed for original tx"
        );

        let decoded_json =
            String::from_utf8(hex::decode(&hex).expect("Failed to decode hex in test"))
                .expect("Failed to convert decoded hex to UTF-8 in test");
        let decoded_tx: Tx = serde_json::from_str(&decoded_json)
            .expect("Failed to deserialize JSON to transaction in test");
        assert!(
            decoded_tx.verify(MOCK_CHAIN_ID),
            "Verification failed for decoded tx"
        );
    }

    #[test]
    fn test_transfer_tx_hex_json_serialization() {
        // Create a signing key for testing using known test bytes
        let signing_key = throwaway_signing_key();
        let verifying_key = signing_key.verifying_key();
        let sender = Address::from_public_key(&verifying_key)
            .expect("Failed to derive address from public key in test");
        let recipient = Address::parse_hex_str("0x0987654321098765432109876543210987654321")
            .expect("valid recipient in test");

        // Create a transfer transaction
        let transfer = TransferTx::new(sender, recipient, 1000.into()).expect("valid transfer");

        // Create the payload
        let payload = Payload::new(transfer);

        // Create and sign the transaction
        let mut tx = Tx {
            sig: TxSig::empty(),
            nonce: 1u32.into(),
            payload,
            public_key: hex::encode(verifying_key.to_bytes()).into(),
            fee: DEFAULT_TX_FEE.into(),
        };

        // Sign the transaction
        tx.sign(&signing_key, MOCK_CHAIN_ID);

        // Verify the original transaction
        assert!(
            tx.verify(MOCK_CHAIN_ID),
            "Original transaction verification failed"
        );

        // Convert to hex-encoded JSON
        let tx_json = serde_json::to_string(&tx).expect("Failed to serialize tx to JSON");
        let tx_hex = hex::encode(tx_json.as_bytes());

        // Convert back from hex-encoded JSON
        let tx_json_bytes = hex::decode(tx_hex).expect("Failed to decode hex");
        let tx_json_str =
            String::from_utf8(tx_json_bytes).expect("Failed to convert bytes to string");
        let decoded_tx: Tx =
            serde_json::from_str(&tx_json_str).expect("Failed to deserialize JSON to tx");

        // Verify the decoded transaction
        assert!(
            decoded_tx.verify(MOCK_CHAIN_ID),
            "Decoded transaction verification failed"
        );

        // Compare original and decoded transactions
        assert_eq!(tx.sig, decoded_tx.sig, "Transaction signature mismatch");
        assert_eq!(tx.nonce, decoded_tx.nonce, "Transaction nonce mismatch");
        assert_eq!(
            tx.public_key, decoded_tx.public_key,
            "Transaction public key mismatch"
        );
        assert_eq!(
            tx.payload, decoded_tx.payload,
            "Transaction payload mismatch"
        );

        // Verify specific transfer details
        match decoded_tx.payload.inner {
            PayloadInner::Transfer(t) => {
                assert_eq!(t.sender, sender);
                assert_eq!(
                    t.recipient,
                    Address::parse_hex_str("0x0987654321098765432109876543210987654321").unwrap()
                );
                assert_eq!(t.amount, 1000);
            }
            _ => panic!("Wrong payload type after deserialization"),
        }
    }

    #[test]
    fn test_stake_tx_hex_json_serialization() {
        // Create a signing key for testing using known test bytes
        let signing_key = throwaway_signing_key();
        let verifying_key = signing_key.verifying_key();
        let sender = Address::from_public_key(&verifying_key)
            .expect("Failed to derive address from public key in test");

        // Create a stake transaction
        let stake = StakeTx::new(
            sender,
            1000.into(),
            Some(hex::encode(verifying_key.to_bytes())),
        )
        .expect("valid stake");

        // Create the payload
        let payload = Payload::new(stake);

        // Create and sign the transaction
        let mut tx = Tx {
            sig: TxSig::empty(),
            nonce: 1u32.into(),
            payload,
            public_key: hex::encode(verifying_key.to_bytes()).into(),
            fee: DEFAULT_TX_FEE.into(),
        };

        // Sign the transaction
        tx.sign(&signing_key, MOCK_CHAIN_ID);

        // Verify the original transaction
        assert!(
            tx.verify(MOCK_CHAIN_ID),
            "Original transaction verification failed"
        );

        // Convert to hex-encoded JSON
        let tx_json = serde_json::to_string(&tx).expect("Failed to serialize tx to JSON");
        let tx_hex = hex::encode(tx_json.as_bytes());

        // Convert back from hex-encoded JSON
        let tx_json_bytes = hex::decode(tx_hex).expect("Failed to decode hex");
        let tx_json_str =
            String::from_utf8(tx_json_bytes).expect("Failed to convert bytes to string");
        let decoded_tx: Tx =
            serde_json::from_str(&tx_json_str).expect("Failed to deserialize JSON to tx");

        // Verify the decoded transaction
        assert!(
            decoded_tx.verify(MOCK_CHAIN_ID),
            "Decoded transaction verification failed"
        );

        // Compare original and decoded transactions
        assert_eq!(tx.sig, decoded_tx.sig, "Transaction signature mismatch");
        assert_eq!(tx.nonce, decoded_tx.nonce, "Transaction nonce mismatch");
        assert_eq!(
            tx.public_key, decoded_tx.public_key,
            "Transaction public key mismatch"
        );
        assert_eq!(
            tx.payload, decoded_tx.payload,
            "Transaction payload mismatch"
        );

        // Verify specific stake details
        match decoded_tx.payload.inner {
            PayloadInner::Stake(s) => {
                assert_eq!(s.sender, sender);
                assert_eq!(s.amount, 1000);
                assert_eq!(s.public_key, Some(hex::encode(verifying_key.to_bytes())));
            }
            _ => panic!("Wrong payload type after deserialization"),
        }
    }

    #[test]
    fn test_unstake_tx_hex_json_serialization() {
        // Create a signing key for testing using known test bytes
        let signing_key = throwaway_signing_key();
        let verifying_key = signing_key.verifying_key();
        let sender = Address::from_public_key(&verifying_key)
            .expect("Failed to derive address from public key in test");

        // Create an unstake transaction
        let unstake = UnstakeTx::new(sender, 1000.into()).expect("valid unstake");

        // Create and sign the transaction
        let mut tx = Tx {
            sig: TxSig::empty(),
            nonce: 1u32.into(),
            payload: Payload::new(unstake),
            public_key: hex::encode(verifying_key.to_bytes()).into(),
            fee: DEFAULT_TX_FEE.into(),
        };

        // Sign the transaction
        tx.sign(&signing_key, MOCK_CHAIN_ID);

        // Convert to hex-encoded JSON
        let tx_json = serde_json::to_string(&tx).expect("Failed to serialize tx to JSON");
        info!("Original JSON: {}", tx_json);
        let tx_hex = hex::encode(tx_json.as_bytes());

        // Convert back from hex-encoded JSON
        let tx_json_bytes = hex::decode(tx_hex).expect("Failed to decode hex");
        let tx_json_str =
            String::from_utf8(tx_json_bytes).expect("Failed to convert bytes to string");
        info!("Decoded JSON: {}", tx_json_str);
        let decoded_tx: Tx =
            serde_json::from_str(&tx_json_str).expect("Failed to deserialize JSON to tx");
        info!(
            "Re-encoded JSON: {}",
            serde_json::to_string(&decoded_tx)
                .expect("Failed to serialize decoded transaction to JSON in test")
        );

        // Verify the decoded transaction
        assert!(
            decoded_tx.verify(MOCK_CHAIN_ID),
            "Decoded transaction verification failed"
        );

        // Compare original and decoded transactions
        assert_eq!(tx.sig, decoded_tx.sig, "Transaction signature mismatch");
        assert_eq!(tx.nonce, decoded_tx.nonce, "Transaction nonce mismatch");
        assert_eq!(
            tx.public_key, decoded_tx.public_key,
            "Transaction public key mismatch"
        );
        assert_eq!(
            tx.payload, decoded_tx.payload,
            "Transaction payload mismatch"
        );

        // Verify specific unstake details
        match decoded_tx.payload.inner {
            PayloadInner::Unstake(u) => {
                assert_eq!(u.sender, sender);
                assert_eq!(u.amount, 1000);
            }
            _ => panic!("Wrong payload type after deserialization"),
        }
    }

    #[test]
    fn test_chain_id_verification() {
        let signing_key = throwaway_signing_key();
        let verifying_key = signing_key.verifying_key();
        let sender = Address::from_public_key(&verifying_key)
            .expect("Failed to derive address from public key in test");
        let recipient = Address::parse_hex_str("0x4567890123456789012345678901234567890123")
            .expect("valid recipient in test");

        let transfer = TransferTx::new(sender, recipient, 100.into()).expect("valid transfer");

        let mut tx = Tx {
            sig: TxSig::empty(),
            nonce: 1u32.into(),
            payload: Payload::new(transfer),
            public_key: hex::encode(verifying_key.to_bytes()).into(),
            fee: DEFAULT_TX_FEE.into(),
        };

        // Sign with MOCK_CHAIN_ID
        tx.sign(&signing_key, MOCK_CHAIN_ID);

        // Verify with same chain ID should succeed
        assert!(
            tx.verify(MOCK_CHAIN_ID),
            "Transaction should verify with correct chain ID"
        );

        // Verify with different chain ID should fail
        assert!(
            !tx.verify("different-chain"),
            "Transaction should not verify with wrong chain ID"
        );

        // Verify with empty chain ID should fail
        assert!(
            !tx.verify(""),
            "Transaction should not verify with empty chain ID"
        );
    }

    #[test]
    fn test_post_message_validate_signature_and_build_tx() {
        let user_secret_bytes = [7u8; 32];
        let user_signing_key = SigningKey::from_bytes(&user_secret_bytes);
        let user_verifying_key = user_signing_key.verifying_key();
        let original_signer_addr = Address::from_public_key(&user_verifying_key)
            .expect("Failed to derive user address in test");

        let message_bytes = b"hello pinboard";

        let user_request = build_signed_post_message_user_request(
            &user_signing_key,
            message_bytes,
            PostMessageUserRequestInput {
                expires_height: 123_456,
                visibility: "Public".to_string(),
                topic: Some("general".to_string()),
                tags: vec![],
                content_type: "text/plain".to_string(),
                fee_amount: 1000,
                namespace: None,
            },
        )
        .expect("build user request");

        validate_post_message_user_signature(message_bytes, &user_request)
            .expect("User signature should validate");

        assert_eq!(
            user_request.content_key,
            calculate_message_content_key(message_bytes)
        );

        let validator_secret_bytes = [9u8; 32];
        let validator_signing_key = SigningKey::from_bytes(&validator_secret_bytes);
        let tx = validate_message_and_build_post_message_tx(
            message_bytes,
            &user_request,
            &validator_signing_key,
            7,
            1_710_000_000,
            MOCK_CHAIN_ID,
            DEFAULT_TX_FEE,
        )
        .expect("Failed to build PostMessage tx");

        assert_eq!(
            tx.payload.r#type,
            tx_type::TX_TYPE_POST_MESSAGE.to_string(),
            "Unexpected tx payload type"
        );
        assert!(tx.verify(MOCK_CHAIN_ID), "Built tx should verify");

        match tx.payload.inner {
            PayloadInner::PostMessage(post_message_tx) => {
                assert_eq!(post_message_tx.original_signer, original_signer_addr);
                assert_eq!(post_message_tx.content_key, user_request.content_key);
                assert_eq!(post_message_tx.message_id, user_request.message_id);
                assert_eq!(post_message_tx.visibility, "Public");
                assert_eq!(post_message_tx.topic, Some("general".to_string()));
                assert_eq!(post_message_tx.content_type, "text/plain");
                assert_eq!(post_message_tx.fee_amount, 1000);
            }
            _ => panic!("Wrong payload type after building PostMessage tx"),
        }
    }

    #[test]
    fn test_post_message_validation_rejects_signer_pubkey_mismatch() {
        let user_secret_bytes = [21u8; 32];
        let user_signing_key = SigningKey::from_bytes(&user_secret_bytes);
        let message_bytes = b"hello";

        let mut user_request = build_signed_post_message_user_request(
            &user_signing_key,
            message_bytes,
            PostMessageUserRequestInput {
                expires_height: 42,
                visibility: "Public".to_string(),
                topic: None,
                tags: vec![],
                content_type: "text/plain".to_string(),
                fee_amount: 1000,
                namespace: None,
            },
        )
        .expect("build user request");

        user_request.original_signer = "0x1234567890123456789012345678901234567890".to_string();
        let err = validate_post_message_user_signature(message_bytes, &user_request)
            .expect_err("mismatched signer should fail");
        assert_eq!(err, "original_signer does not match original_signer_pubkey");
    }

    #[test]
    fn test_post_message_validation_rejects_tampered_message() {
        let user_secret_bytes = [17u8; 32];
        let user_signing_key = SigningKey::from_bytes(&user_secret_bytes);

        let original_message = b"original message";
        let user_request = build_signed_post_message_user_request(
            &user_signing_key,
            original_message,
            PostMessageUserRequestInput {
                expires_height: 321_000,
                visibility: "Public".to_string(),
                topic: None,
                tags: vec![],
                content_type: "application/json".to_string(),
                fee_amount: 1000,
                namespace: None,
            },
        )
        .expect("build user request");

        let tampered_message = b"tampered message";
        let validation_result =
            validate_post_message_user_signature(tampered_message, &user_request);
        assert!(
            validation_result.is_err(),
            "Validation should fail when message bytes are changed"
        );
    }

    #[test]
    fn test_post_message_namespace_affects_message_id() {
        let user_secret_bytes = [11u8; 32];
        let user_signing_key = SigningKey::from_bytes(&user_secret_bytes);
        let message_bytes = b"namespace test";

        let without = build_signed_post_message_user_request(
            &user_signing_key,
            message_bytes,
            PostMessageUserRequestInput {
                expires_height: 100,
                visibility: "Public".to_string(),
                topic: None,
                tags: vec![],
                content_type: "text/plain".to_string(),
                fee_amount: 1000,
                namespace: None,
            },
        )
        .expect("without namespace");

        let with_ns = build_signed_post_message_user_request(
            &user_signing_key,
            message_bytes,
            PostMessageUserRequestInput {
                expires_height: 100,
                visibility: "Public".to_string(),
                topic: None,
                tags: vec![],
                content_type: "text/plain".to_string(),
                fee_amount: 1000,
                namespace: Some("peterpan".to_string()),
            },
        )
        .expect("with namespace");

        assert_ne!(without.message_id, with_ns.message_id);
        assert_eq!(with_ns.namespace, Some("peterpan".to_string()));
    }

    #[test]
    fn test_validate_post_message_content_type_allowlist() {
        validate_post_message_content_type("text/plain").expect("text/plain");
        validate_post_message_content_type("application/json").expect("application/json");
        validate_post_message_content_type("image/png").expect("image/png");
        assert!(validate_post_message_content_type("").is_err());
        assert!(validate_post_message_content_type(" ").is_err());
        validate_post_message_content_type(" text/plain").expect("trim spaces");
        validate_post_message_content_type("\timage/png\n").expect("trim ws");
        assert!(validate_post_message_content_type("text/html").is_err());
        assert!(validate_post_message_content_type("image/jpeg").is_err());
    }

    #[test]
    fn test_canonicalize_post_message_tags_normalizes_address_hex_case() {
        let out = canonicalize_post_message_tags(&[
            "0x0123456789ABCDEF0123456789ABCDEF01234567".to_string()
        ])
        .expect("canonical tags");
        assert_eq!(out[0], "0x0123456789abcdef0123456789abcdef01234567");
    }

    #[test]
    fn test_canonicalize_post_message_tags_non_address_unchanged() {
        let out =
            canonicalize_post_message_tags(&["dapp-inbox".to_string()]).expect("canonical tags");
        assert_eq!(out[0], "dapp-inbox");
    }

    #[test]
    fn test_canonicalize_post_message_tags_rejects_too_many() {
        let tags: Vec<String> = (0..POST_MESSAGE_MAX_TAGS + 1)
            .map(|i| format!("t{i}"))
            .collect();
        assert!(canonicalize_post_message_tags(&tags).is_err());
    }

    #[test]
    fn test_canonicalize_post_message_tags_rejects_oversized_tag() {
        let tag = "a".repeat(POST_MESSAGE_MAX_TAG_UTF8_BYTES + 1);
        assert!(canonicalize_post_message_tags(&[tag]).is_err());
    }

    #[test]
    fn test_add_namespace_tx_sign_verify_hex_json() {
        let signing_key = throwaway_signing_key();
        let verifying_key = signing_key.verifying_key();
        let sender = Address::from_public_key(&verifying_key)
            .expect("Failed to derive address from public key in test");

        let add_namespace = AddNamespaceTx::new(sender, "peter".to_string(), 1.into())
            .expect("valid add_namespace");
        let mut tx = Tx {
            sig: TxSig::empty(),
            nonce: 1u32.into(),
            payload: Payload::new(add_namespace),
            public_key: hex::encode(verifying_key.to_bytes()).into(),
            fee: DEFAULT_TX_FEE.into(),
        };

        tx.sign(&signing_key, MOCK_CHAIN_ID);
        assert!(
            tx.verify(MOCK_CHAIN_ID),
            "Original AddNamespace transaction verification failed"
        );
        assert_eq!(
            tx.payload.r#type,
            tx_type::TX_TYPE_ADD_NAMESPACE.to_string()
        );

        let tx_json = serde_json::to_string(&tx).expect("serialize tx");
        let tx_hex = hex::encode(tx_json.as_bytes());
        let decoded_tx: Tx = serde_json::from_str(
            &String::from_utf8(hex::decode(tx_hex).expect("decode hex")).expect("utf8"),
        )
        .expect("deserialize tx");

        assert!(
            decoded_tx.verify(MOCK_CHAIN_ID),
            "Decoded AddNamespace transaction verification failed"
        );

        match decoded_tx.payload.inner {
            PayloadInner::AddNamespace(t) => {
                assert_eq!(t.sender, sender);
                assert_eq!(t.namespace_slug, "peter");
                assert_eq!(t.registration_fee, TxAmount(1));
            }
            _ => panic!("Wrong payload type after AddNamespace deserialization"),
        }
    }
}
