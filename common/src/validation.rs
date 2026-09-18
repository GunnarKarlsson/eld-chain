//! # Validation Module
//!
//! This module provides validation logic for the Eld blockchain system,
//! focusing on amount validation using the Coin type.

use crate::address::Address;
use crate::coin::Coin;
use crate::constants::{
    cado::MAX_APP_STATE_SNAPSHOT_CADO_SIZE_BYTES, protocol::MIN_STAKE_AMOUNT, token::MAX_COIN,
    tx_type,
};
use crate::error::EldError;
#[cfg(test)]
use crate::tx::{StakeTx, TransferTx, UnstakeTx};
use bincode;
use ed25519_dalek;
use hex;
use serde_json;
use tracing::{debug, error, warn};

/// Validate that a transfer amount is positive and within bounds
#[cfg(test)]
pub(crate) fn validate_transfer_amount(amount: &Coin) -> Result<(), EldError> {
    let value: u128 = (*amount).into();

    if value == 0 {
        return EldError::validation_error(
            "transfer amount",
            &value.to_string(),
            "Transfer amount cannot be zero",
        );
    }

    if value > MAX_COIN {
        return EldError::validation_error(
            "transfer amount",
            &value.to_string(),
            "Transfer amount exceeds maximum coin value",
        );
    }

    Ok(())
}

/// Validate that a stake amount meets minimum requirements
#[cfg(test)]
pub(crate) fn validate_stake_amount(amount: &Coin) -> Result<(), EldError> {
    let value: u128 = (*amount).into();

    if value < MIN_STAKE_AMOUNT {
        return EldError::validation_error(
            "stake amount",
            &value.to_string(),
            &format!("Stake amount {value} is below minimum required {MIN_STAKE_AMOUNT}"),
        );
    }

    if value > MAX_COIN {
        return EldError::validation_error(
            "stake amount",
            &value.to_string(),
            "Stake amount exceeds maximum coin value",
        );
    }

    Ok(())
}

/// Validate that a fee amount is reasonable
pub fn validate_fee_amount(amount: &Coin) -> Result<(), EldError> {
    let value: u128 = (*amount).into();

    if value == 0 {
        return EldError::validation_error("fee amount", &value.to_string(), "Fee cannot be zero");
    }

    if value > MAX_COIN {
        return EldError::validation_error(
            "fee amount",
            &value.to_string(),
            "Fee exceeds maximum coin value",
        );
    }

    Ok(())
}

/// Validate that a chunk ID is properly formatted
/// Chunk IDs are now 32 bytes (64 hex characters) using SHA256 hashes for RocksDB storage
#[cfg(test)]
pub(crate) fn validate_chunk_id(chunk_id: &str) -> Result<(), EldError> {
    chunk_id.parse::<crate::chunk_id::ChunkId>()?;
    Ok(())
}

/// Validate that an address is properly formatted
pub fn validate_address(address: &str) -> Result<(), EldError> {
    parse_validated_address(address).map(|_| ())
}

/// Parse an address string that must use canonical `0x`-prefixed form.
fn parse_validated_address(address: &str) -> Result<Address, EldError> {
    if !address.starts_with("0x") {
        return EldError::validation_error("address", address, "Address must start with 0x")
            .map(|_| unreachable!());
    }

    let hex_str = &address[2..];
    if hex_str.is_empty() {
        return EldError::validation_error(
            "address",
            address,
            "Address cannot be empty after 0x prefix",
        )
        .map(|_| unreachable!());
    }

    Address::parse_hex_str(address)
}

/// Validate that a contract ID is properly formatted
/// Contract IDs are 32 bytes (64 hex characters) - different from addresses which are 20 bytes
#[cfg(test)]
pub(crate) fn validate_contract_id(contract_id: &str) -> Result<(), EldError> {
    if !contract_id.starts_with("0x") {
        return EldError::validation_error(
            "contract_id",
            contract_id,
            "Contract ID must start with 0x",
        );
    }

    let hex_str = &contract_id[2..];
    if hex_str.is_empty() {
        return EldError::validation_error(
            "contract_id",
            contract_id,
            "Contract ID cannot be empty after 0x prefix",
        );
    }

    match hex::decode(hex_str) {
        Ok(bytes) => {
            if bytes.len() != 32 {
                return EldError::validation_error(
                    "contract_id",
                    contract_id,
                    &format!(
                        "Contract ID must be 32 bytes (64 hex characters), got {} bytes",
                        bytes.len()
                    ),
                );
            }
        }
        Err(_) => {
            return EldError::validation_error(
                "contract_id",
                contract_id,
                "Contract ID must be valid hex after 0x prefix",
            );
        }
    }

    Ok(())
}

/// Validate that a string is safe for storage (no injection attacks)
pub(crate) fn validate_safe_string(input: &str, max_length: usize) -> Result<(), EldError> {
    if input.is_empty() {
        return Err(EldError::ValidationError {
            field: "string".to_string(),
            value: input.to_string(),
            details: "String cannot be empty".to_string(),
        });
    }

    if input.len() > max_length {
        return Err(EldError::ValidationError {
            field: "string".to_string(),
            value: input.to_string(),
            details: format!("String exceeds maximum length of {max_length}"),
        });
    }

    // Check for potentially dangerous characters
    let dangerous_chars = [
        '<', '>', '"', '\'', '&', ';', '|', '`', '$', '(', ')', '{', '}',
    ];
    for &ch in &dangerous_chars {
        if input.contains(ch) {
            return Err(EldError::ValidationError {
                field: "string".to_string(),
                value: input.to_string(),
                details: format!("String contains potentially dangerous character: {ch}"),
            });
        }
    }

    // Check for control characters
    if input.chars().any(|c| c.is_control()) {
        return Err(EldError::ValidationError {
            field: "string".to_string(),
            value: input.to_string(),
            details: "String contains control characters".to_string(),
        });
    }

    Ok(())
}

/// Validate that a chain ID is non-empty, alphanumeric, and reasonable length
pub fn validate_chain_id(chain_id: &str) -> Result<(), EldError> {
    let len = chain_id.len();
    if !(3..=64).contains(&len) {
        return Err(EldError::ValidationError {
            field: "chain ID".to_string(),
            value: chain_id.to_string(),
            details: format!("Chain ID must be 3-64 characters, got {len} characters"),
        });
    }
    if !chain_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(EldError::ValidationError {
            field: "chain ID".to_string(),
            value: chain_id.to_string(),
            details: "Chain ID must be alphanumeric, dash, or underscore".to_string(),
        });
    }
    Ok(())
}

/// Validate that a port is a valid integer in 1..=65535
pub fn validate_port(port: &str) -> Result<(), EldError> {
    let port_num: u16 = port.parse().map_err(|_| EldError::ValidationError {
        field: "port".to_string(),
        value: port.to_string(),
        details: "Port must be a valid integer".to_string(),
    })?;
    if port_num == 0 {
        return Err(EldError::ValidationError {
            field: "port".to_string(),
            value: port.to_string(),
            details: "Port must be between 1 and 65535".to_string(),
        });
    }
    Ok(())
}

/// Validate that a string is a valid IPv4/IPv6 address or hostname
pub fn validate_ip_or_hostname(host: &str) -> Result<(), EldError> {
    if host.is_empty() {
        return Err(EldError::ValidationError {
            field: "host".to_string(),
            value: host.to_string(),
            details: "Host cannot be empty".to_string(),
        });
    }
    if host.parse::<std::net::IpAddr>().is_ok() {
        return Ok(());
    }
    // Simple hostname check: must be alphanumeric, dash, dot
    if !host
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
    {
        return Err(EldError::ValidationError {
            field: "host".to_string(),
            value: host.to_string(),
            details: "Host must be a valid IP or hostname".to_string(),
        });
    }
    Ok(())
}

/// Validate a positive integer with optional min/max
pub fn validate_positive_integer<
    T: num_traits::Num + num_traits::NumCast + PartialOrd + std::fmt::Display,
>(
    value: T,
    min: Option<T>,
    max: Option<T>,
    field: &str,
) -> Result<(), EldError> {
    let zero = num_traits::zero();
    if value < zero {
        return Err(EldError::ValidationError {
            field: field.to_string(),
            value: value.to_string(),
            details: format!("{field} must be positive"),
        });
    }
    if let Some(min_val) = min {
        if value < min_val {
            return Err(EldError::ValidationError {
                field: field.to_string(),
                value: value.to_string(),
                details: format!("{field} must be >= {min_val}"),
            });
        }
    }
    if let Some(max_val) = max {
        if value > max_val {
            return Err(EldError::ValidationError {
                field: field.to_string(),
                value: value.to_string(),
                details: format!("{field} must be <= {max_val}"),
            });
        }
    }
    Ok(())
}

/// Validate a transfer transaction
#[cfg(test)]
pub(crate) fn validate_transfer_tx(tx: &TransferTx) -> Result<(), EldError> {
    TransferTx::new(tx.sender, tx.recipient, tx.amount).map(|_| ())
}

/// Validate a stake transaction
#[cfg(test)]
pub(crate) fn validate_stake_tx(tx: &StakeTx) -> Result<(), EldError> {
    StakeTx::new(tx.sender, tx.amount, tx.public_key.clone()).map(|_| ())
}

/// Validate an unstake transaction
#[cfg(test)]
pub(crate) fn validate_unstake_tx(tx: &UnstakeTx) -> Result<(), EldError> {
    UnstakeTx::new(tx.sender, tx.amount).map(|_| ())
}

/// Validate an AddNamespace transaction
#[cfg(test)]
pub(crate) fn validate_add_namespace_tx(tx: &crate::tx::AddNamespaceTx) -> Result<(), EldError> {
    crate::tx::AddNamespaceTx::new(tx.sender, tx.namespace_slug.clone(), tx.registration_fee)
        .map(|_| ())
}

/// Validate hex string format and reasonable size limits
pub fn validate_hex_string(hex_str: &str, max_bytes: usize) -> Result<(), EldError> {
    if hex_str.is_empty() {
        return Err(EldError::ValidationError {
            field: "hex string".to_string(),
            value: hex_str.to_string(),
            details: "Hex string cannot be empty".to_string(),
        });
    }

    // Check if hex string length is reasonable (2 hex chars = 1 byte)
    if hex_str.len() > max_bytes * 2 {
        return Err(EldError::ValidationError {
            field: "hex string".to_string(),
            value: hex_str.to_string(),
            details: format!(
                "Hex string too long: {} characters (max {} bytes)",
                hex_str.len(),
                max_bytes
            ),
        });
    }

    // Check if hex string has even length (required for valid hex)
    if hex_str.len() % 2 != 0 {
        return Err(EldError::ValidationError {
            field: "hex string".to_string(),
            value: hex_str.to_string(),
            details: "Hex string must have even length".to_string(),
        });
    }

    // Validate hex characters
    if !hex_str.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(EldError::ValidationError {
            field: "hex string".to_string(),
            value: hex_str.to_string(),
            details: "Hex string contains invalid characters".to_string(),
        });
    }

    Ok(())
}

/// Validate JSON string for safe deserialization
pub fn validate_json_string(
    json_str: &str,
    max_size: usize,
    max_depth: usize,
) -> Result<(), EldError> {
    if json_str.is_empty() {
        return Err(EldError::ValidationError {
            field: "JSON string".to_string(),
            value: json_str.to_string(),
            details: "JSON string cannot be empty".to_string(),
        });
    }

    if json_str.len() > max_size {
        return Err(EldError::ValidationError {
            field: "JSON string".to_string(),
            value: json_str.to_string(),
            details: format!(
                "JSON string too large: {} bytes (max {})",
                json_str.len(),
                max_size
            ),
        });
    }

    // Check for excessive nesting (DoS protection)
    let mut depth = 0;
    for ch in json_str.chars() {
        match ch {
            '{' | '[' => {
                depth += 1;
                if depth > max_depth {
                    return Err(EldError::ValidationError {
                        field: "JSON string".to_string(),
                        value: json_str.to_string(),
                        details: format!("JSON nesting too deep: {depth} levels (max {max_depth})"),
                    });
                }
            }
            '}' | ']' => {
                if depth == 0 {
                    return Err(EldError::ValidationError {
                        field: "JSON string".to_string(),
                        value: json_str.to_string(),
                        details: "JSON has mismatched brackets".to_string(),
                    });
                }
                depth -= 1;
            }
            _ => {}
        }
    }

    if depth != 0 {
        return Err(EldError::ValidationError {
            field: "JSON string".to_string(),
            value: json_str.to_string(),
            details: "JSON has mismatched brackets".to_string(),
        });
    }

    // Validate JSON structure without deserializing to specific type
    serde_json::from_str::<serde_json::Value>(json_str).map_err(|e| EldError::ValidationError {
        field: "JSON string".to_string(),
        value: json_str.to_string(),
        details: format!("Invalid JSON format: {e}"),
    })?;

    Ok(())
}

/// Validate transaction structure before deserialization
pub fn validate_transaction_structure(json_value: &serde_json::Value) -> Result<(), EldError> {
    // Validate transaction size limits first
    validate_transaction_size(json_value)?;
    let obj = json_value
        .as_object()
        .ok_or_else(|| EldError::ValidationError {
            field: "transaction".to_string(),
            value: json_value.to_string(),
            details: "Transaction must be a JSON object".to_string(),
        })?;

    // Check required fields
    let required_fields = ["sig", "nonce", "payload", "public_key", "fee"];
    for field in &required_fields {
        if !obj.contains_key(*field) {
            return Err(EldError::ValidationError {
                field: "transaction".to_string(),
                value: json_value.to_string(),
                details: format!("Transaction missing required field: {field}"),
            });
        }
    }

    // Validate field types and basic constraints
    if !obj["sig"].is_string() {
        return Err(EldError::ValidationError {
            field: "transaction sig".to_string(),
            value: obj["sig"].to_string(),
            details: "Transaction 'sig' field must be a string".to_string(),
        });
    }

    // Validate signature is valid hex and reasonable size
    let sig = obj["sig"].as_str().unwrap();
    validate_hex_string(sig, 64)?; // 64 bytes = 128 hex chars for ed25519 signature

    if !obj["nonce"].is_number() {
        return Err(EldError::ValidationError {
            field: "transaction nonce".to_string(),
            value: obj["nonce"].to_string(),
            details: "Transaction 'nonce' field must be a number".to_string(),
        });
    }

    // Validate nonce is positive and reasonable
    let nonce = obj["nonce"]
        .as_u64()
        .ok_or_else(|| EldError::ValidationError {
            field: "transaction nonce".to_string(),
            value: obj["nonce"].to_string(),
            details: "Transaction 'nonce' field must be a positive integer".to_string(),
        })?;
    validate_positive_integer(nonce, Some(0u64), Some(u32::MAX as u64), "nonce")?;

    if !obj["payload"].is_object() {
        return Err(EldError::ValidationError {
            field: "transaction payload".to_string(),
            value: obj["payload"].to_string(),
            details: "Transaction 'payload' field must be an object".to_string(),
        });
    }

    if !obj["public_key"].is_string() {
        return Err(EldError::ValidationError {
            field: "transaction public_key".to_string(),
            value: obj["public_key"].to_string(),
            details: "Transaction 'public_key' field must be a string".to_string(),
        });
    }

    // Validate public key is valid hex and correct size
    let public_key = obj["public_key"].as_str().unwrap();
    validate_hex_string(public_key, 32)?; // 32 bytes = 64 hex chars for ed25519 public key

    if !obj["fee"].is_number() {
        return Err(EldError::ValidationError {
            field: "transaction fee".to_string(),
            value: obj["fee"].to_string(),
            details: "Transaction 'fee' field must be a number".to_string(),
        });
    }

    // Validate fee is positive and reasonable
    let fee = obj["fee"]
        .as_u64()
        .ok_or_else(|| EldError::ValidationError {
            field: "transaction fee".to_string(),
            value: obj["fee"].to_string(),
            details: "Transaction 'fee' field must be a positive integer".to_string(),
        })?;
    validate_positive_integer(fee, Some(0u64), Some(u32::MAX as u64), "fee")?;

    // Validate payload structure
    let payload = &obj["payload"];
    let payload_obj = payload
        .as_object()
        .ok_or_else(|| EldError::ValidationError {
            field: "transaction payload".to_string(),
            value: payload.to_string(),
            details: "Transaction payload must be a JSON object".to_string(),
        })?;

    if !payload_obj.contains_key("type") {
        return Err(EldError::ValidationError {
            field: "transaction payload".to_string(),
            value: payload.to_string(),
            details: "Transaction payload missing 'type' field".to_string(),
        });
    }

    if !payload_obj["type"].is_string() {
        return Err(EldError::ValidationError {
            field: "transaction payload type".to_string(),
            value: payload_obj["type"].to_string(),
            details: "Transaction payload 'type' field must be a string".to_string(),
        });
    }

    // Validate payload type is one of the allowed types
    let payload_type = payload_obj["type"].as_str().unwrap();
    let allowed_types = [
        tx_type::TX_TYPE_TRANSFER,
        tx_type::TX_TYPE_UNSTAKE,
        tx_type::TX_TYPE_STAKE,
        tx_type::TX_TYPE_VERIFIED_PROOF,
        tx_type::TX_TYPE_REGISTER_CAPACITY,
        tx_type::TX_TYPE_UNREGISTER_CAPACITY,
        tx_type::TX_TYPE_UPDATE_CAPACITY_MERKLE_ROOT,
        tx_type::TX_TYPE_POST_MESSAGE,
        tx_type::TX_TYPE_ADD_NAMESPACE,
    ];

    if !allowed_types.contains(&payload_type) {
        return Err(EldError::ValidationError {
            field: "transaction payload type".to_string(),
            value: payload_type.to_string(),
            details: format!(
                "Invalid transaction payload type: {payload_type} (allowed: {allowed_types:?})"
            ),
        });
    }

    // Validate payload content based on type
    validate_payload_content(payload_obj, payload_type)?;

    Ok(())
}

/// Validate transaction size limits
fn validate_transaction_size(json_value: &serde_json::Value) -> Result<(), EldError> {
    // Convert to string to measure size
    let json_string = serde_json::to_string(json_value).map_err(|e| EldError::ValidationError {
        field: "transaction".to_string(),
        value: json_value.to_string(),
        details: format!("Failed to serialize transaction for size validation: {e}"),
    })?;

    // Check against maximum transaction size (10MB default, aligned with max_tx_bytes)
    const MAX_TRANSACTION_SIZE_BYTES: usize = 10 * 1024 * 1024; // 10MB

    if json_string.len() > MAX_TRANSACTION_SIZE_BYTES {
        return Err(EldError::ValidationError {
            field: "transaction".to_string(),
            value: json_value.to_string(),
            details: format!(
                "Transaction size {} bytes exceeds maximum allowed size {} bytes",
                json_string.len(),
                MAX_TRANSACTION_SIZE_BYTES
            ),
        });
    }

    Ok(())
}

/// Validate payload content based on payload type
fn validate_payload_content(
    payload_obj: &serde_json::Map<String, serde_json::Value>,
    payload_type: &str,
) -> Result<(), EldError> {
    match payload_type {
        tx_type::TX_TYPE_TRANSFER => validate_transfer_payload(payload_obj)?,
        tx_type::TX_TYPE_STAKE => validate_stake_payload(payload_obj)?,
        tx_type::TX_TYPE_UNSTAKE => validate_unstake_payload(payload_obj)?,
        tx_type::TX_TYPE_REGISTER_CAPACITY => validate_register_capacity_payload(payload_obj)?,
        tx_type::TX_TYPE_UNREGISTER_CAPACITY => validate_unregister_capacity_payload(payload_obj)?,
        tx_type::TX_TYPE_UPDATE_CAPACITY_MERKLE_ROOT => {
            validate_update_capacity_merkle_root_payload(payload_obj)?
        }
        tx_type::TX_TYPE_VERIFIED_PROOF => validate_verified_proof_payload(payload_obj)?,
        tx_type::TX_TYPE_POST_MESSAGE => validate_post_message_payload(payload_obj)?,
        tx_type::TX_TYPE_ADD_NAMESPACE => validate_add_namespace_payload(payload_obj)?,
        _ => {
            return Err(EldError::ValidationError {
                field: "transaction payload type".to_string(),
                value: payload_type.to_string(),
                details: format!("Unknown payload type: {payload_type}"),
            })
        }
    }
    Ok(())
}

/// Validate PostMessage payload fields
fn validate_post_message_payload(
    payload_obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), EldError> {
    let required_fields = [
        "sender",
        "original_signer",
        "original_signer_pubkey",
        "content_key",
        "message_id",
        "expires_height",
        "visibility",
        "fee_amount",
        "received_timestamp",
        "user_signature",
    ];
    for field in &required_fields {
        if !payload_obj.contains_key(*field) {
            return Err(EldError::ValidationError {
                field: "post_message payload".to_string(),
                value: "missing".to_string(),
                details: format!("PostMessage payload missing required field: {field}"),
            });
        }
    }

    Ok(())
}

/// Validate Transfer payload fields
fn validate_transfer_payload(
    payload_obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), EldError> {
    // Check required fields
    let required_fields = ["sender", "recipient", "amount"];
    for field in &required_fields {
        if !payload_obj.contains_key(*field) {
            return Err(EldError::ValidationError {
                field: "transfer payload".to_string(),
                value: "missing".to_string(),
                details: format!("Transfer payload missing required field: {field}"),
            });
        }
    }

    // Validate sender
    let sender = payload_obj["sender"]
        .as_str()
        .ok_or_else(|| EldError::ValidationError {
            field: "transfer payload sender".to_string(),
            value: payload_obj["sender"].to_string(),
            details: "Transfer payload 'sender' field must be a string".to_string(),
        })?;
    let sender = parse_validated_address(sender)?;

    // Validate recipient
    let recipient = payload_obj["recipient"]
        .as_str()
        .ok_or_else(|| EldError::ValidationError {
            field: "transfer payload recipient".to_string(),
            value: payload_obj["recipient"].to_string(),
            details: "Transfer payload 'recipient' field must be a string".to_string(),
        })?;
    let recipient = parse_validated_address(recipient)?;

    // Validate sender and recipient are different
    if sender == recipient {
        return Err(EldError::ValidationError {
            field: "transfer addresses".to_string(),
            value: format!("sender: {sender}, recipient: {recipient}"),
            details: "Transfer sender and recipient cannot be the same".to_string(),
        });
    }

    // Validate amount (can be string or number for backward compatibility)
    let amount = match payload_obj["amount"] {
        serde_json::Value::String(ref s) => {
            s.parse::<u128>().map_err(|_| EldError::ValidationError {
                field: "transfer payload amount".to_string(),
                value: s.clone(),
                details: "Transfer payload 'amount' field must be a valid positive integer string"
                    .to_string(),
            })?
        }
        serde_json::Value::Number(ref n) => n.as_u64().ok_or_else(|| EldError::ValidationError {
            field: "transfer payload amount".to_string(),
            value: n.to_string(),
            details: "Transfer payload 'amount' field must be a valid positive integer".to_string(),
        })? as u128,
        _ => {
            return Err(EldError::ValidationError {
                field: "transfer payload amount".to_string(),
                value: payload_obj["amount"].to_string(),
                details: "Transfer payload 'amount' field must be a string or number".to_string(),
            });
        }
    };
    if amount == 0 {
        return Err(EldError::ValidationError {
            field: "transfer payload amount".to_string(),
            value: amount.to_string(),
            details: "Transfer payload 'amount' must be greater than zero".to_string(),
        });
    }
    if amount > MAX_COIN {
        return Err(EldError::ValidationError {
            field: "transfer payload amount".to_string(),
            value: amount.to_string(),
            details: format!("Transfer payload 'amount' exceeds maximum coin value {MAX_COIN}"),
        });
    }

    Ok(())
}

/// Validate Stake payload fields
fn validate_stake_payload(
    payload_obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), EldError> {
    // Check required fields
    let required_fields = ["sender", "amount"];
    for field in &required_fields {
        if !payload_obj.contains_key(*field) {
            return Err(EldError::ValidationError {
                field: "stake payload".to_string(),
                value: "missing".to_string(),
                details: format!("Stake payload missing required field: {field}"),
            });
        }
    }

    // Validate sender
    let sender = payload_obj["sender"]
        .as_str()
        .ok_or_else(|| EldError::ValidationError {
            field: "stake payload sender".to_string(),
            value: payload_obj["sender"].to_string(),
            details: "Stake payload 'sender' field must be a string".to_string(),
        })?;
    parse_validated_address(sender)?;

    // Validate amount (can be string or number for backward compatibility)
    let amount = match payload_obj["amount"] {
        serde_json::Value::String(ref s) => {
            s.parse::<u128>().map_err(|_| EldError::ValidationError {
                field: "stake payload amount".to_string(),
                value: s.clone(),
                details: "Stake payload 'amount' field must be a valid positive integer string"
                    .to_string(),
            })?
        }
        serde_json::Value::Number(ref n) => n.as_u64().ok_or_else(|| EldError::ValidationError {
            field: "stake payload amount".to_string(),
            value: n.to_string(),
            details: "Stake payload 'amount' field must be a valid positive integer".to_string(),
        })? as u128,
        _ => {
            return Err(EldError::ValidationError {
                field: "stake payload amount".to_string(),
                value: payload_obj["amount"].to_string(),
                details: "Stake payload 'amount' field must be a string or number".to_string(),
            });
        }
    };
    if amount < MIN_STAKE_AMOUNT {
        return Err(EldError::ValidationError {
            field: "stake payload amount".to_string(),
            value: amount.to_string(),
            details: format!(
                "Stake payload 'amount' {amount} is below minimum required {MIN_STAKE_AMOUNT}"
            ),
        });
    }
    if amount > MAX_COIN {
        return Err(EldError::ValidationError {
            field: "stake payload amount".to_string(),
            value: amount.to_string(),
            details: format!("Stake payload 'amount' exceeds maximum coin value {MAX_COIN}"),
        });
    }

    // Validate public_key if present (optional field)
    if let Some(public_key) = payload_obj.get("public_key") {
        if !public_key.is_string() {
            return Err(EldError::ValidationError {
                field: "stake payload public_key".to_string(),
                value: public_key.to_string(),
                details: "Stake payload 'public_key' field must be a string".to_string(),
            });
        }
        let public_key_str = public_key.as_str().unwrap();
        validate_hex_string(public_key_str, 32)?; // 32 bytes = 64 hex chars
    }

    Ok(())
}

/// Validate AddNamespace payload fields
fn validate_add_namespace_payload(
    payload_obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), EldError> {
    use crate::tx::{AddNamespaceTx, TxAmount};

    let required_fields = ["sender", "namespace_slug", "registration_fee"];
    for field in &required_fields {
        if !payload_obj.contains_key(*field) {
            return Err(EldError::ValidationError {
                field: "add_namespace payload".to_string(),
                value: "missing".to_string(),
                details: format!("AddNamespace payload missing required field: {field}"),
            });
        }
    }

    let sender = payload_obj["sender"]
        .as_str()
        .ok_or_else(|| EldError::ValidationError {
            field: "add_namespace payload sender".to_string(),
            value: payload_obj["sender"].to_string(),
            details: "AddNamespace payload 'sender' field must be a string".to_string(),
        })?;
    let sender = parse_validated_address(sender)?;

    let namespace_slug =
        payload_obj["namespace_slug"]
            .as_str()
            .ok_or_else(|| EldError::ValidationError {
                field: "add_namespace payload namespace_slug".to_string(),
                value: payload_obj["namespace_slug"].to_string(),
                details: "AddNamespace payload 'namespace_slug' field must be a string".to_string(),
            })?;
    validate_safe_string(namespace_slug, 64)?;

    let registration_fee = match &payload_obj["registration_fee"] {
        serde_json::Value::String(s) => s.parse::<u128>().map_err(|_| EldError::ValidationError {
            field: "add_namespace payload registration_fee".to_string(),
            value: s.clone(),
            details:
                "AddNamespace payload 'registration_fee' must be a valid non-negative integer string"
                    .to_string(),
        })?,
        serde_json::Value::Number(n) => n.as_u64().ok_or_else(|| EldError::ValidationError {
            field: "add_namespace payload registration_fee".to_string(),
            value: n.to_string(),
            details:
                "AddNamespace payload 'registration_fee' must be a valid non-negative integer"
                    .to_string(),
        })? as u128,
        _ => {
            return Err(EldError::ValidationError {
                field: "add_namespace payload registration_fee".to_string(),
                value: payload_obj["registration_fee"].to_string(),
                details: "AddNamespace payload 'registration_fee' must be a string or number"
                    .to_string(),
            });
        }
    };
    if registration_fee > MAX_COIN {
        return Err(EldError::ValidationError {
            field: "add_namespace payload registration_fee".to_string(),
            value: registration_fee.to_string(),
            details: format!(
                "AddNamespace payload 'registration_fee' exceeds maximum coin value {MAX_COIN}"
            ),
        });
    }

    AddNamespaceTx::new(
        sender,
        namespace_slug.to_string(),
        TxAmount(registration_fee),
    )?;
    Ok(())
}

/// Validate Unstake payload fields
fn validate_unstake_payload(
    payload_obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), EldError> {
    // Check required fields
    let required_fields = ["sender", "amount"];
    for field in &required_fields {
        if !payload_obj.contains_key(*field) {
            return Err(EldError::ValidationError {
                field: "unstake payload".to_string(),
                value: "missing".to_string(),
                details: format!("Unstake payload missing required field: {field}"),
            });
        }
    }

    // Validate sender
    let sender = payload_obj["sender"]
        .as_str()
        .ok_or_else(|| EldError::ValidationError {
            field: "unstake payload sender".to_string(),
            value: payload_obj["sender"].to_string(),
            details: "Unstake payload 'sender' field must be a string".to_string(),
        })?;
    parse_validated_address(sender)?;

    // Validate amount (can be string or number for backward compatibility)
    let amount = match payload_obj["amount"] {
        serde_json::Value::String(ref s) => {
            s.parse::<u128>().map_err(|_| EldError::ValidationError {
                field: "unstake payload amount".to_string(),
                value: s.clone(),
                details: "Unstake payload 'amount' field must be a valid positive integer string"
                    .to_string(),
            })?
        }
        serde_json::Value::Number(ref n) => n.as_u64().ok_or_else(|| EldError::ValidationError {
            field: "unstake payload amount".to_string(),
            value: n.to_string(),
            details: "Unstake payload 'amount' field must be a valid positive integer".to_string(),
        })? as u128,
        _ => {
            return Err(EldError::ValidationError {
                field: "unstake payload amount".to_string(),
                value: payload_obj["amount"].to_string(),
                details: "Unstake payload 'amount' field must be a string or number".to_string(),
            });
        }
    };
    if amount == 0 {
        return Err(EldError::ValidationError {
            field: "unstake payload amount".to_string(),
            value: amount.to_string(),
            details: "Unstake payload 'amount' must be greater than zero".to_string(),
        });
    }
    if amount > MAX_COIN {
        return Err(EldError::ValidationError {
            field: "unstake payload amount".to_string(),
            value: amount.to_string(),
            details: format!("Unstake payload 'amount' exceeds maximum coin value {MAX_COIN}"),
        });
    }

    Ok(())
}

/// Validate RegisterCapacity payload fields
fn validate_register_capacity_payload(
    payload_obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), EldError> {
    // Check required fields
    let required_fields = [
        "sender",
        "capacity_bytes",
        "merkle_root",
        "seed",
        "chunk_count",
    ];
    for field in &required_fields {
        if !payload_obj.contains_key(*field) {
            return Err(EldError::ValidationError {
                field: "register capacity payload".to_string(),
                value: "missing".to_string(),
                details: format!("RegisterCapacity payload missing required field: {field}"),
            });
        }
    }

    // Validate sender
    let sender = payload_obj["sender"]
        .as_str()
        .ok_or_else(|| EldError::ValidationError {
            field: "register capacity payload sender".to_string(),
            value: payload_obj["sender"].to_string(),
            details: "RegisterCapacity payload 'sender' field must be a string".to_string(),
        })?;
    validate_address(sender)?;
    validate_safe_string(sender, 100)?; // Max 100 chars for address

    // Validate capacity_bytes
    let capacity_bytes =
        payload_obj["capacity_bytes"]
            .as_u64()
            .ok_or_else(|| EldError::ValidationError {
                field: "register capacity payload capacity_bytes".to_string(),
                value: payload_obj["capacity_bytes"].to_string(),
                details:
                    "RegisterCapacity payload 'capacity_bytes' field must be a positive integer"
                        .to_string(),
            })?;
    validate_positive_integer(capacity_bytes, Some(1u64), None, "capacity_bytes")?;

    // Validate merkle_root (32 bytes = 64 hex chars)
    let merkle_root =
        payload_obj["merkle_root"]
            .as_str()
            .ok_or_else(|| EldError::ValidationError {
                field: "register capacity payload merkle_root".to_string(),
                value: payload_obj["merkle_root"].to_string(),
                details: "RegisterCapacity payload 'merkle_root' field must be a hex string"
                    .to_string(),
            })?;
    validate_hex_string(merkle_root, 32)?; // 32 bytes = 64 hex chars

    // Validate seed (32 bytes = 64 hex chars)
    let seed = payload_obj["seed"]
        .as_str()
        .ok_or_else(|| EldError::ValidationError {
            field: "register capacity payload seed".to_string(),
            value: payload_obj["seed"].to_string(),
            details: "RegisterCapacity payload 'seed' field must be a hex string".to_string(),
        })?;
    validate_hex_string(seed, 32)?; // 32 bytes = 64 hex chars

    // Validate chunk_count
    let chunk_count =
        payload_obj["chunk_count"]
            .as_u64()
            .ok_or_else(|| EldError::ValidationError {
                field: "register capacity payload chunk_count".to_string(),
                value: payload_obj["chunk_count"].to_string(),
                details: "RegisterCapacity payload 'chunk_count' field must be a positive integer"
                    .to_string(),
            })?;
    validate_positive_integer(chunk_count, Some(1u64), None, "chunk_count")?;

    Ok(())
}

/// Validate UnregisterCapacity payload fields
fn validate_unregister_capacity_payload(
    payload_obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), EldError> {
    // Check required fields
    let required_fields = ["sender", "unregister"];
    for field in &required_fields {
        if !payload_obj.contains_key(*field) {
            return Err(EldError::ValidationError {
                field: "unregister capacity payload".to_string(),
                value: "missing".to_string(),
                details: format!("UnregisterCapacity payload missing required field: {field}"),
            });
        }
    }

    // Validate sender
    let sender = payload_obj["sender"]
        .as_str()
        .ok_or_else(|| EldError::ValidationError {
            field: "unregister capacity payload sender".to_string(),
            value: payload_obj["sender"].to_string(),
            details: "UnregisterCapacity payload 'sender' field must be a string".to_string(),
        })?;
    validate_address(sender)?;
    validate_safe_string(sender, 100)?; // Max 100 chars for address

    // Validate unregister field (must be true)
    let unregister =
        payload_obj["unregister"]
            .as_bool()
            .ok_or_else(|| EldError::ValidationError {
                field: "unregister capacity payload unregister".to_string(),
                value: payload_obj["unregister"].to_string(),
                details: "UnregisterCapacity payload 'unregister' field must be a boolean"
                    .to_string(),
            })?;
    if !unregister {
        return Err(EldError::ValidationError {
            field: "unregister".to_string(),
            value: unregister.to_string(),
            details: "UnregisterCapacity payload 'unregister' field must be true".to_string(),
        });
    }

    Ok(())
}

/// Validate UpdateCapacityMerkleRoot payload fields
fn validate_update_capacity_merkle_root_payload(
    payload_obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), EldError> {
    // Check required fields
    let required_fields = ["sender", "merkle_root"];
    for field in &required_fields {
        if !payload_obj.contains_key(*field) {
            return Err(EldError::ValidationError {
                field: "update capacity merkle root payload".to_string(),
                value: "missing".to_string(),
                details: format!(
                    "UpdateCapacityMerkleRoot payload missing required field: {field}"
                ),
            });
        }
    }

    // Validate sender
    let sender = payload_obj["sender"]
        .as_str()
        .ok_or_else(|| EldError::ValidationError {
            field: "update capacity merkle root payload sender".to_string(),
            value: payload_obj["sender"].to_string(),
            details: "UpdateCapacityMerkleRoot payload 'sender' field must be a string".to_string(),
        })?;
    validate_address(sender)?;
    validate_safe_string(sender, 100)?; // Max 100 chars for address

    // Validate merkle_root (32 bytes = 64 hex chars)
    let merkle_root =
        payload_obj["merkle_root"]
            .as_str()
            .ok_or_else(|| EldError::ValidationError {
                field: "update capacity merkle root payload merkle_root".to_string(),
                value: payload_obj["merkle_root"].to_string(),
                details:
                    "UpdateCapacityMerkleRoot payload 'merkle_root' field must be a hex string"
                        .to_string(),
            })?;
    validate_hex_string(merkle_root, 32)?; // 32 bytes = 64 hex chars

    Ok(())
}

/// Validate VerifiedProof payload fields (JSON)
fn validate_verified_proof_payload(
    payload_obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), EldError> {
    // Check required fields
    let required_fields = [
        "sender",
        "capacity_provider",
        "challenge_id",
        "block_height",
        "verified_at_block",
        "verified_at_timestamp",
        "proofs",
        "generated_at",
        "provider_pubkey",
        "provider_signature",
    ];
    for field in &required_fields {
        if !payload_obj.contains_key(*field) {
            return Err(EldError::ValidationError {
                field: "verified proof payload".to_string(),
                value: "missing".to_string(),
                details: format!("VerifiedProof payload missing required field: {field}"),
            });
        }
    }

    // Validate sender
    let sender = payload_obj["sender"]
        .as_str()
        .ok_or_else(|| EldError::ValidationError {
            field: "verified proof payload sender".to_string(),
            value: payload_obj["sender"].to_string(),
            details: "VerifiedProof payload 'sender' field must be a string".to_string(),
        })?;
    parse_validated_address(sender)?;

    // Validate capacity_provider
    let capacity_provider =
        payload_obj["capacity_provider"]
            .as_str()
            .ok_or_else(|| EldError::ValidationError {
                field: "verified proof payload capacity_provider".to_string(),
                value: payload_obj["capacity_provider"].to_string(),
                details: "VerifiedProof payload 'capacity_provider' field must be a string"
                    .to_string(),
            })?;
    parse_validated_address(capacity_provider)?;

    // Validate challenge_id
    let challenge_id =
        payload_obj["challenge_id"]
            .as_str()
            .ok_or_else(|| EldError::ValidationError {
                field: "verified proof payload challenge_id".to_string(),
                value: payload_obj["challenge_id"].to_string(),
                details: "VerifiedProof payload 'challenge_id' field must be a string".to_string(),
            })?;
    validate_safe_string(challenge_id, 100)?; // Max 100 chars for challenge_id

    // Validate block_height
    let block_height =
        payload_obj["block_height"]
            .as_i64()
            .ok_or_else(|| EldError::ValidationError {
                field: "verified proof payload block_height".to_string(),
                value: payload_obj["block_height"].to_string(),
                details: "VerifiedProof payload 'block_height' field must be an integer"
                    .to_string(),
            })?;
    if block_height <= 0 {
        return Err(EldError::ValidationError {
            field: "verified proof payload block_height".to_string(),
            value: block_height.to_string(),
            details: "VerifiedProof payload 'block_height' must be positive".to_string(),
        });
    }

    // Validate verified_at_block
    let verified_at_block =
        payload_obj["verified_at_block"]
            .as_i64()
            .ok_or_else(|| EldError::ValidationError {
                field: "verified proof payload verified_at_block".to_string(),
                value: payload_obj["verified_at_block"].to_string(),
                details: "VerifiedProof payload 'verified_at_block' field must be an integer"
                    .to_string(),
            })?;
    if verified_at_block <= 0 {
        return Err(EldError::ValidationError {
            field: "verified proof payload verified_at_block".to_string(),
            value: verified_at_block.to_string(),
            details: "VerifiedProof payload 'verified_at_block' must be positive".to_string(),
        });
    }
    if verified_at_block < block_height {
        return Err(EldError::ValidationError {
            field: "verified proof payload verified_at_block".to_string(),
            value: verified_at_block.to_string(),
            details: "VerifiedProof payload 'verified_at_block' must be >= block_height"
                .to_string(),
        });
    }

    // Validate verified_at_timestamp
    let verified_at_timestamp = payload_obj["verified_at_timestamp"]
        .as_u64()
        .ok_or_else(|| EldError::ValidationError {
            field: "verified proof payload verified_at_timestamp".to_string(),
            value: payload_obj["verified_at_timestamp"].to_string(),
            details:
                "VerifiedProof payload 'verified_at_timestamp' field must be a positive integer"
                    .to_string(),
        })?;
    // Allow timestamps from 2020-01-01 (1577836800) to 2100-01-01 (4102444800)
    const MIN_TIMESTAMP: u64 = 1577836800;
    const MAX_TIMESTAMP: u64 = 4102444800;
    if !(MIN_TIMESTAMP..=MAX_TIMESTAMP).contains(&verified_at_timestamp) {
        return Err(EldError::ValidationError {
            field: "verified proof payload verified_at_timestamp".to_string(),
            value: verified_at_timestamp.to_string(),
            details: format!(
                "VerifiedProof payload 'verified_at_timestamp' must be between {MIN_TIMESTAMP} and {MAX_TIMESTAMP}"
            ),
        });
    }

    let proofs = payload_obj["proofs"]
        .as_array()
        .ok_or_else(|| EldError::ValidationError {
            field: "verified proof payload proofs".to_string(),
            value: payload_obj["proofs"].to_string(),
            details: "VerifiedProof payload 'proofs' field must be an array".to_string(),
        })?;
    if proofs.is_empty() {
        return Err(EldError::ValidationError {
            field: "verified proof payload proofs".to_string(),
            value: "[]".to_string(),
            details: "VerifiedProof payload 'proofs' must be non-empty".to_string(),
        });
    }

    let generated_at =
        payload_obj["generated_at"]
            .as_u64()
            .ok_or_else(|| EldError::ValidationError {
                field: "verified proof payload generated_at".to_string(),
                value: payload_obj["generated_at"].to_string(),
                details: "VerifiedProof payload 'generated_at' field must be a positive integer"
                    .to_string(),
            })?;
    if !(MIN_TIMESTAMP..=MAX_TIMESTAMP).contains(&generated_at) {
        return Err(EldError::ValidationError {
            field: "verified proof payload generated_at".to_string(),
            value: generated_at.to_string(),
            details: format!(
                "VerifiedProof payload 'generated_at' must be between {MIN_TIMESTAMP} and {MAX_TIMESTAMP}"
            ),
        });
    }

    let provider_pubkey =
        payload_obj["provider_pubkey"]
            .as_str()
            .ok_or_else(|| EldError::ValidationError {
                field: "verified proof payload provider_pubkey".to_string(),
                value: payload_obj["provider_pubkey"].to_string(),
                details: "VerifiedProof payload 'provider_pubkey' field must be a string"
                    .to_string(),
            })?;
    if provider_pubkey.is_empty() {
        return Err(EldError::ValidationError {
            field: "verified proof payload provider_pubkey".to_string(),
            value: provider_pubkey.to_string(),
            details: "VerifiedProof payload 'provider_pubkey' must be non-empty".to_string(),
        });
    }
    validate_hex_string(provider_pubkey, 32)?;

    let provider_signature =
        payload_obj["provider_signature"]
            .as_str()
            .ok_or_else(|| EldError::ValidationError {
                field: "verified proof payload provider_signature".to_string(),
                value: payload_obj["provider_signature"].to_string(),
                details: "VerifiedProof payload 'provider_signature' field must be a string"
                    .to_string(),
            })?;
    if provider_signature.is_empty() {
        return Err(EldError::ValidationError {
            field: "verified proof payload provider_signature".to_string(),
            value: provider_signature.to_string(),
            details: "VerifiedProof payload 'provider_signature' must be non-empty".to_string(),
        });
    }
    validate_hex_string(provider_signature, 64)?;

    Ok(())
}

/// Verify CADO deletion signature
pub fn verify_cado_deletion_signature(
    path: &str,
    owner: &str,
    signature: &str,
    public_key: &str,
    chain_id: &str,
) -> Result<(), EldError> {
    use ed25519_dalek::Verifier;

    // Create the message to verify: path + owner + chain_id
    let message = format!("{path}:{owner}:{chain_id}");
    let message_bytes = message.as_bytes();

    // Decode the public key
    let public_key_bytes = hex::decode(public_key).map_err(|e| EldError::ValidationError {
        field: "public key".to_string(),
        value: public_key.to_string(),
        details: format!("Invalid public key hex format: {e}"),
    })?;

    if public_key_bytes.len() != 32 {
        return Err(EldError::ValidationError {
            field: "public key".to_string(),
            value: public_key_bytes.len().to_string(),
            details: "Public key must be 32 bytes".to_string(),
        });
    }

    // Create verifying key
    let verifying_key =
        ed25519_dalek::VerifyingKey::from_bytes(&public_key_bytes.try_into().map_err(|_| {
            EldError::ValidationError {
                field: "public key".to_string(),
                value: "invalid".to_string(),
                details: "Failed to convert public key bytes to array".to_string(),
            }
        })?)
        .map_err(|e| EldError::ValidationError {
            field: "public key".to_string(),
            value: public_key.to_string(),
            details: format!("Failed to create verifying key: {e}"),
        })?;

    // Decode the signature
    let signature_bytes = hex::decode(signature).map_err(|e| EldError::ValidationError {
        field: "signature".to_string(),
        value: signature.to_string(),
        details: format!("Invalid signature hex format: {e}"),
    })?;

    if signature_bytes.len() != 64 {
        return Err(EldError::ValidationError {
            field: "signature".to_string(),
            value: signature_bytes.len().to_string(),
            details: "Signature must be 64 bytes".to_string(),
        });
    }

    // Create signature
    let signature =
        ed25519_dalek::Signature::from_bytes(&signature_bytes.try_into().map_err(|_| {
            EldError::ValidationError {
                field: "signature".to_string(),
                value: "invalid".to_string(),
                details: "Failed to convert signature bytes to array".to_string(),
            }
        })?);

    // Verify the signature
    verifying_key
        .verify(message_bytes, &signature)
        .map_err(|e| EldError::ValidationError {
            field: "signature verification".to_string(),
            value: signature.to_string(),
            details: format!("Signature verification failed: {e}"),
        })?;

    Ok(())
}

/// Safe deserialization with comprehensive validation and error handling
/// This function validates input data before attempting deserialization to prevent
/// attacks through malformed data while maintaining compatibility with existing format
pub(crate) fn safe_bincode_deserialize<T: for<'de> serde::Deserialize<'de>>(
    data: &[u8],
    max_size: usize,
    context: &str,
) -> Result<T, EldError> {
    // Step 1: Input validation
    if data.is_empty() {
        return Err(EldError::ValidationError {
            field: "data".to_string(),
            value: "empty".to_string(),
            details: format!("{context}: Empty data cannot be deserialized"),
        });
    }

    if data.len() > max_size {
        return Err(EldError::ValidationError {
            field: "data size".to_string(),
            value: data.len().to_string(),
            details: format!(
                "{}: Data size {} exceeds maximum allowed size {}",
                context,
                data.len(),
                max_size
            ),
        });
    }

    // Step 2: Check for null bytes or suspicious patterns
    if data.contains(&0) {
        warn!(
            "{}: Data contains null bytes, which may indicate corruption",
            context
        );
    }

    // Step 3: Attempt deserialization with detailed error context
    match bincode::deserialize::<T>(data) {
        Ok(result) => {
            debug!(
                "{}: Successfully deserialized {} bytes",
                context,
                data.len()
            );
            Ok(result)
        }
        Err(e) => {
            error!(
                "{}: Deserialization failed: {} (data size: {} bytes)",
                context,
                e,
                data.len()
            );
            Err(EldError::ValidationError {
                field: "deserialization".to_string(),
                value: format!("{} bytes", data.len()),
                details: format!("{context}: Deserialization failed: {e}"),
            })
        }
    }
}

/// Safe deserialization for CADO data with appropriate size limits
pub fn safe_deserialize_cado_data<T: for<'de> serde::Deserialize<'de>>(
    data: &[u8],
    context: &str,
) -> Result<T, EldError> {
    // CADO data should be reasonable size - 1MB limit for most CADO types
    const MAX_CADO_DATA_SIZE: usize = 1024 * 1024; // 1MB

    safe_bincode_deserialize::<T>(data, MAX_CADO_DATA_SIZE, context)
}

/// App state snapshots can be large (full committed cache + envelope fields).
pub fn safe_deserialize_app_state_snapshot_cado_data<T: for<'de> serde::Deserialize<'de>>(
    data: &[u8],
    context: &str,
) -> Result<T, EldError> {
    safe_bincode_deserialize::<T>(data, MAX_APP_STATE_SNAPSHOT_CADO_SIZE_BYTES, context)
}

/// Safe deserialization for account data with appropriate size limits
pub fn safe_deserialize_account_data<T: for<'de> serde::Deserialize<'de>>(
    data: &[u8],
    context: &str,
) -> Result<T, EldError> {
    // Account data should be small - 10KB limit
    const MAX_ACCOUNT_DATA_SIZE: usize = 10 * 1024; // 10KB

    safe_bincode_deserialize::<T>(data, MAX_ACCOUNT_DATA_SIZE, context)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::address::Address;
    use crate::tx::StakeTx;
    use crate::tx::TransferTx;

    #[test]
    fn test_validate_transfer_amount() {
        // Valid transfer
        let valid_amount = Coin::new(100).expect("Failed to create valid amount coin in test");
        assert!(validate_transfer_amount(&valid_amount).is_ok());

        // Zero amount should fail
        let zero_amount = Coin::zero();
        assert!(validate_transfer_amount(&zero_amount).is_err());

        // Maximum amount should be valid
        let max_amount = Coin::max();
        assert!(validate_transfer_amount(&max_amount).is_ok());
    }

    #[test]
    fn test_validate_stake_amount() {
        // Valid stake amount
        let valid_stake =
            Coin::new(MIN_STAKE_AMOUNT).expect("Failed to create valid stake coin in test");
        assert!(validate_stake_amount(&valid_stake).is_ok());

        // Below minimum should fail
        let low_stake =
            Coin::new(MIN_STAKE_AMOUNT - 1).expect("Failed to create low stake coin in test");
        assert!(validate_stake_amount(&low_stake).is_err());

        // Maximum amount should be valid
        let max_amount = Coin::max();
        assert!(validate_stake_amount(&max_amount).is_ok());
    }

    #[test]
    fn test_validate_fee_amount() {
        // Valid fee
        let valid_fee = Coin::new(100).expect("Failed to create valid fee coin in test");
        assert!(validate_fee_amount(&valid_fee).is_ok());

        // Zero fee should fail
        let zero_fee = Coin::zero();
        assert!(validate_fee_amount(&zero_fee).is_err());

        // Maximum fee should be valid
        let max_fee = Coin::max();
        assert!(validate_fee_amount(&max_fee).is_ok());
    }

    #[test]
    fn test_validate_chunk_id() {
        // Valid chunk ID (32 bytes = 64 hex chars)
        let valid_chunk_id = "0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert!(validate_chunk_id(valid_chunk_id).is_ok());

        let invalid_prefix = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert!(validate_chunk_id(invalid_prefix).is_err());

        // Short chunk ID (20 bytes = 40 hex chars, should fail)
        let short_chunk_id = "0xabcdef1234567890abcdef1234567890abcdef12";
        assert!(validate_chunk_id(short_chunk_id).is_err());

        // Even shorter chunk ID
        let very_short_chunk_id = "0xabcdef1234567890abcdef1234567890abcdef1";
        assert!(validate_chunk_id(very_short_chunk_id).is_err());

        let invalid_hex = "0x0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdefgh";
        assert!(validate_chunk_id(invalid_hex).is_err());

        let empty_chunk_id = "0x";
        assert!(validate_chunk_id(empty_chunk_id).is_err());
    }

    #[test]
    fn test_validate_address() {
        // Valid address (20 bytes = 40 hex chars)
        let valid_address = "0x1234567890abcdef1234567890abcdef12345678";
        assert!(validate_address(valid_address).is_ok());

        // Missing 0x prefix
        let invalid_prefix = "1234567890abcdef1234567890abcdef12345678";
        assert!(validate_address(invalid_prefix).is_err());

        // Wrong length (19 bytes)
        let short_address = "0x1234567890abcdef1234567890abcdef1234567";
        assert!(validate_address(short_address).is_err());

        // Invalid hex
        let invalid_hex = "0x1234567890abcdef1234567890abcdef1234567g";
        assert!(validate_address(invalid_hex).is_err());

        // Empty
        let empty_address = "0x";
        assert!(validate_address(empty_address).is_err());
    }

    #[test]
    fn test_validate_address_requires_prefix() {
        let valid_address = "0x1234567890abcdef1234567890abcdef12345678";
        assert!(validate_address(valid_address).is_ok());
        assert!(Address::parse_hex_str(valid_address).is_ok());

        let invalid_prefix = "1234567890abcdef1234567890abcdef12345678";
        assert!(validate_address(invalid_prefix).is_err());
        assert!(Address::parse_hex_str(invalid_prefix).is_ok());
    }

    #[test]
    fn test_validate_address_error_messages() {
        let missing_prefix = validate_address("1234567890abcdef1234567890abcdef12345678")
            .expect_err("missing prefix should fail");
        assert!(matches!(
            missing_prefix,
            EldError::ValidationError {
                field,
                details,
                ..
            } if field == "address" && details == "Address must start with 0x"
        ));

        let short = validate_address("0x1234567890abcdef1234567890abcdef123456")
            .expect_err("short address should fail");
        assert!(matches!(
            short,
            EldError::ValidationError {
                field,
                details,
                ..
            } if field == "address" && details.contains("Invalid address length")
        ));

        let invalid_hex = validate_address("0x1234567890abcdef1234567890abcdef1234567g")
            .expect_err("invalid hex should fail");
        assert!(matches!(
            invalid_hex,
            EldError::ValidationError {
                field,
                details,
                ..
            } if field == "address" && details.contains("Invalid hex format")
        ));

        let empty = validate_address("0x").expect_err("empty address should fail");
        assert!(matches!(
            empty,
            EldError::ValidationError {
                field,
                details,
                ..
            } if field == "address" && details == "Address cannot be empty after 0x prefix"
        ));
    }

    #[test]
    fn test_validate_contract_id() {
        // Valid 32-byte contract ID (64 hex characters)
        let valid_contract_id =
            "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        assert!(validate_contract_id(valid_contract_id).is_ok());

        // Invalid: missing 0x prefix
        let invalid_prefix = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        assert!(validate_contract_id(invalid_prefix).is_err());

        // Invalid: too short (20 bytes instead of 32)
        let short_contract_id = "0x1234567890abcdef1234567890abcdef12345678";
        assert!(validate_contract_id(short_contract_id).is_err());

        // Invalid: too long
        let long_contract_id =
            "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef12";
        assert!(validate_contract_id(long_contract_id).is_err());

        // Invalid: invalid hex
        let invalid_hex = "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdefg";
        assert!(validate_contract_id(invalid_hex).is_err());

        // Invalid: empty
        let empty_contract_id = "0x";
        assert!(validate_contract_id(empty_contract_id).is_err());
    }

    #[test]
    fn test_validate_safe_string() {
        // Valid string
        let valid_string = "Hello World";
        assert!(validate_safe_string(valid_string, 100).is_ok());

        // Empty string
        let empty_string = "";
        assert!(validate_safe_string(empty_string, 100).is_err());

        // Too long string
        let long_string = "a".repeat(101);
        assert!(validate_safe_string(&long_string, 100).is_err());

        // Contains dangerous characters
        let dangerous_string = "Hello<script>alert('xss')</script>";
        assert!(validate_safe_string(dangerous_string, 100).is_err());

        let dangerous_string2 = "Hello; rm -rf /";
        assert!(validate_safe_string(dangerous_string2, 100).is_err());

        // Contains control characters
        let control_string = "Hello\x00World";
        assert!(validate_safe_string(control_string, 100).is_err());
    }

    #[test]
    fn test_validate_transfer_tx() {
        let a1 = Address::parse_hex_str("0x1234567890abcdef1234567890abcdef12345678").unwrap();
        let a2 = Address::parse_hex_str("0xabcdef1234567890abcdef1234567890abcdef12").unwrap();

        // Valid transfer
        let valid_tx = TransferTx::new(a1, a2, 1000.into()).expect("valid transfer");
        assert!(validate_transfer_tx(&valid_tx).is_ok());

        // Same sender and recipient
        assert!(TransferTx::new(a1, a1, 1000.into()).is_err());

        // Zero amount
        assert!(TransferTx::new(a1, a2, 0.into()).is_err());
    }

    #[test]
    fn test_transfer_tx_json_rejects_invalid_addresses() {
        let bad_sender = r#"{"sender":"invalid","recipient":"0xabcdef1234567890abcdef1234567890abcdef12","amount":"1000"}"#;
        assert!(serde_json::from_str::<TransferTx>(bad_sender).is_err());

        let bad_recipient = r#"{"sender":"0x1234567890abcdef1234567890abcdef12345678","recipient":"invalid","amount":"1000"}"#;
        assert!(serde_json::from_str::<TransferTx>(bad_recipient).is_err());
    }

    #[test]
    fn test_validate_stake_tx() {
        let sender = Address::parse_hex_str("0x1234567890abcdef1234567890abcdef12345678").unwrap();

        // Valid stake
        let valid_tx = StakeTx::new(
            sender,
            MIN_STAKE_AMOUNT.into(),
            Some("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef".to_string()),
        )
        .expect("valid stake");
        assert!(validate_stake_tx(&valid_tx).is_ok());

        // Valid stake without public key
        let valid_tx_no_pk =
            StakeTx::new(sender, MIN_STAKE_AMOUNT.into(), None).expect("valid stake");
        assert!(validate_stake_tx(&valid_tx_no_pk).is_ok());

        // Invalid public key
        let invalid_pk_tx = StakeTx::new(
            sender,
            MIN_STAKE_AMOUNT.into(),
            Some("invalid_key".to_string()),
        );
        assert!(invalid_pk_tx.is_err());

        // Below minimum stake amount
        let low_amount_tx = StakeTx::new(sender, (MIN_STAKE_AMOUNT - 1).into(), None);
        assert!(low_amount_tx.is_err());
    }

    #[test]
    fn test_stake_tx_json_rejects_invalid_sender() {
        let j = format!(
            r#"{{"sender":"not_an_address","amount":"{MIN_STAKE_AMOUNT}","public_key":null}}"#
        );
        assert!(serde_json::from_str::<StakeTx>(&j).is_err());
    }

    #[test]
    fn test_validate_unstake_tx() {
        use crate::tx::UnstakeTx;

        let sender = Address::parse_hex_str("0x1234567890abcdef1234567890abcdef12345678").unwrap();

        // Valid unstake
        let valid_tx = UnstakeTx::new(sender, 1000.into()).expect("valid unstake");
        assert!(validate_unstake_tx(&valid_tx).is_ok());

        // Zero amount
        assert!(UnstakeTx::new(sender, 0.into()).is_err());
    }

    #[test]
    fn test_unstake_tx_json_rejects_invalid_sender() {
        use crate::tx::UnstakeTx;

        let j = r#"{"sender":"invalid","amount":"1000"}"#;
        assert!(serde_json::from_str::<UnstakeTx>(j).is_err());
    }

    #[test]
    fn test_validate_add_namespace_tx() {
        use crate::tx::AddNamespaceTx;

        let sender = Address::parse_hex_str("0x1234567890abcdef1234567890abcdef12345678").unwrap();

        let valid_tx = AddNamespaceTx::new(sender, "peter".to_string(), 1.into())
            .expect("valid add_namespace");
        assert!(AddNamespaceTx::new(sender, "peter".to_string(), 0.into()).is_err());
        assert!(validate_add_namespace_tx(&valid_tx).is_ok());
        assert_eq!(valid_tx.namespace_slug, "peter");

        assert!(
            AddNamespaceTx::new(sender, "eld".to_string(), 1.into()).is_err(),
            "reserved slug"
        );
        assert!(
            AddNamespaceTx::new(sender, "ab".to_string(), 1.into()).is_err(),
            "slug too short"
        );
    }

    #[test]
    fn test_add_namespace_tx_json_rejects_unknown_field() {
        use crate::tx::AddNamespaceTx;

        let j = r#"{"sender":"0x1234567890abcdef1234567890abcdef12345678","namespace_slug":"peter","registration_fee":"0","extra":true}"#;
        assert!(serde_json::from_str::<AddNamespaceTx>(j).is_err());
    }

    #[test]
    fn test_validate_add_namespace_transaction_structure() {
        let tx = serde_json::json!({
            "sig": "a".repeat(128),
            "nonce": 1,
            "fee": 5000,
            "public_key": "a".repeat(64),
            "payload": {
                "type": tx_type::TX_TYPE_ADD_NAMESPACE,
                "sender": "0x1234567890abcdef1234567890abcdef12345678",
                "namespace_slug": "peter",
                "registration_fee": "1"
            }
        });
        assert!(validate_transaction_structure(&tx).is_ok());

        let reserved = serde_json::json!({
            "sig": "a".repeat(128),
            "nonce": 1,
            "fee": 5000,
            "public_key": "a".repeat(64),
            "payload": {
                "type": tx_type::TX_TYPE_ADD_NAMESPACE,
                "sender": "0x1234567890abcdef1234567890abcdef12345678",
                "namespace_slug": "eld",
                "registration_fee": "1"
            }
        });
        assert!(validate_transaction_structure(&reserved).is_err());
    }

    #[test]
    fn test_validate_hex_string() {
        // Valid hex strings
        assert!(validate_hex_string("1234567890abcdef", 8).is_ok());
        assert!(validate_hex_string("", 0).is_err()); // Empty string
        assert!(validate_hex_string("1234567890abcdef", 4).is_err()); // Too long
        assert!(validate_hex_string("1234567890abcde", 8).is_err()); // Odd length
        assert!(validate_hex_string("1234567890abcdefg", 8).is_err()); // Invalid characters
        assert!(validate_hex_string("1234567890ABCDEF", 8).is_ok()); // Uppercase is valid
    }

    #[test]
    fn test_validate_json_string() {
        // Valid JSON
        assert!(validate_json_string(r#"{"key": "value"}"#, 100, 5).is_ok());
        assert!(validate_json_string(r#"{"nested": {"key": "value"}}"#, 100, 5).is_ok());

        // Invalid cases
        assert!(validate_json_string("", 100, 5).is_err()); // Empty
        assert!(validate_json_string("invalid json", 100, 5).is_err()); // Invalid JSON
        assert!(validate_json_string(&"x".repeat(200), 100, 5).is_err()); // Too large

        // Test nesting depth - simpler test
        let nested_5 = r#"{"a":{"b":{"c":{"d":{"e":"v"}}}}}"#;
        assert!(validate_json_string(nested_5, 1000, 3).is_err()); // Too deep
        assert!(validate_json_string(nested_5, 1000, 10).is_ok()); // Within limit

        // Test mismatched brackets
        assert!(validate_json_string(r#"{"key": "value""#, 100, 5).is_err()); // Missing }
        assert!(validate_json_string(r#"{"key": "value"}"#, 100, 5).is_ok()); // Valid
    }

    #[test]
    fn test_validate_transaction_structure() {
        // Valid transaction structure
        let valid_tx = serde_json::json!({
            "sig": "a".repeat(128), // 64 bytes = 128 hex chars
            "nonce": 1,
            "payload": {
                "type": tx_type::TX_TYPE_TRANSFER,
                "sender": "0x1234567890123456789012345678901234567890",
                "recipient": "0x0987654321098765432109876543210987654321",
                "amount": 1000
            },
            "public_key": "a".repeat(64), // 32 bytes = 64 hex chars
            "fee": 100
        });
        assert!(validate_transaction_structure(&valid_tx).is_ok());

        // Missing required field
        let missing_field = serde_json::json!({
            "sig": "a".repeat(128),
            "nonce": 1,
            "payload": {
                "type": tx_type::TX_TYPE_TRANSFER,
                "sender": "0x1234567890123456789012345678901234567890",
                "recipient": "0x0987654321098765432109876543210987654321",
                "amount": 1000
            },
            "public_key": "a".repeat(64)
            // Missing fee
        });
        assert!(validate_transaction_structure(&missing_field).is_err());

        // Invalid field type
        let invalid_type = serde_json::json!({
            "sig": 123, // Should be string
            "nonce": 1,
            "payload": {
                "type": tx_type::TX_TYPE_TRANSFER,
                "sender": "0x1234567890123456789012345678901234567890",
                "recipient": "0x0987654321098765432109876543210987654321",
                "amount": 1000
            },
            "public_key": "a".repeat(64),
            "fee": 100
        });
        assert!(validate_transaction_structure(&invalid_type).is_err());

        // Invalid payload type
        let invalid_payload_type = serde_json::json!({
            "sig": "a".repeat(128),
            "nonce": 1,
            "payload": {
                "type": "InvalidType"
            },
            "public_key": "a".repeat(64),
            "fee": 100
        });
        assert!(validate_transaction_structure(&invalid_payload_type).is_err());

        // Not an object
        let not_object = serde_json::json!("not an object");
        assert!(validate_transaction_structure(&not_object).is_err());

        // Invalid signature length
        let invalid_sig = serde_json::json!({
            "sig": "short", // Too short
            "nonce": 1,
            "payload": {
                "type": tx_type::TX_TYPE_TRANSFER,
                "sender": "0x1234567890123456789012345678901234567890",
                "recipient": "0x0987654321098765432109876543210987654321",
                "amount": 1000
            },
            "public_key": "a".repeat(64),
            "fee": 100
        });
        assert!(validate_transaction_structure(&invalid_sig).is_err());

        // Invalid public key length
        let invalid_pubkey = serde_json::json!({
            "sig": "a".repeat(128),
            "nonce": 1,
            "payload": {
                "type": tx_type::TX_TYPE_TRANSFER,
                "sender": "0x1234567890123456789012345678901234567890",
                "recipient": "0x0987654321098765432109876543210987654321",
                "amount": 1000
            },
            "public_key": "short", // Too short
            "fee": 100
        });
        assert!(validate_transaction_structure(&invalid_pubkey).is_err());

        // Negative nonce
        let negative_nonce = serde_json::json!({
            "sig": "a".repeat(128),
            "nonce": -1,
            "payload": {
                "type": tx_type::TX_TYPE_TRANSFER,
                "sender": "0x1234567890123456789012345678901234567890",
                "recipient": "0x0987654321098765432109876543210987654321",
                "amount": 1000
            },
            "public_key": "a".repeat(64),
            "fee": 100
        });
        assert!(validate_transaction_structure(&negative_nonce).is_err());

        // Negative fee
        let negative_fee = serde_json::json!({
            "sig": "a".repeat(128),
            "nonce": 1,
            "payload": {
                "type": tx_type::TX_TYPE_TRANSFER,
                "sender": "0x1234567890123456789012345678901234567890",
                "recipient": "0x0987654321098765432109876543210987654321",
                "amount": 1000
            },
            "public_key": "a".repeat(64),
            "fee": -100
        });
        assert!(validate_transaction_structure(&negative_fee).is_err());
    }

    #[test]
    fn test_validate_transfer_payload() {
        // Valid transfer payload
        let valid_tx = serde_json::json!({
            "sig": "a".repeat(128),
            "nonce": 1,
            "payload": {
                "type": tx_type::TX_TYPE_TRANSFER,
                "sender": "0x1234567890123456789012345678901234567890",
                "recipient": "0x0987654321098765432109876543210987654321",
                "amount": 1000
            },
            "public_key": "a".repeat(64),
            "fee": 100
        });
        assert!(validate_transaction_structure(&valid_tx).is_ok());

        // Missing sender
        let missing_sender = serde_json::json!({
            "sig": "a".repeat(128),
            "nonce": 1,
            "payload": {
                "type": tx_type::TX_TYPE_TRANSFER,
                "recipient": "0x0987654321098765432109876543210987654321",
                "amount": 1000
            },
            "public_key": "a".repeat(64),
            "fee": 100
        });
        assert!(validate_transaction_structure(&missing_sender).is_err());

        // Invalid sender address
        let invalid_sender = serde_json::json!({
            "sig": "a".repeat(128),
            "nonce": 1,
            "payload": {
                "type": tx_type::TX_TYPE_TRANSFER,
                "sender": "invalid_address",
                "recipient": "0x0987654321098765432109876543210987654321",
                "amount": 1000
            },
            "public_key": "a".repeat(64),
            "fee": 100
        });
        assert!(validate_transaction_structure(&invalid_sender).is_err());

        // Same sender and recipient
        let same_addresses = serde_json::json!({
            "sig": "a".repeat(128),
            "nonce": 1,
            "payload": {
                "type": tx_type::TX_TYPE_TRANSFER,
                "sender": "0x1234567890123456789012345678901234567890",
                "recipient": "0x1234567890123456789012345678901234567890",
                "amount": 1000
            },
            "public_key": "a".repeat(64),
            "fee": 100
        });
        assert!(validate_transaction_structure(&same_addresses).is_err());

        // Zero amount
        let zero_amount = serde_json::json!({
            "sig": "a".repeat(128),
            "nonce": 1,
            "payload": {
                "type": tx_type::TX_TYPE_TRANSFER,
                "sender": "0x1234567890123456789012345678901234567890",
                "recipient": "0x0987654321098765432109876543210987654321",
                "amount": 0
            },
            "public_key": "a".repeat(64),
            "fee": 100
        });
        assert!(validate_transaction_structure(&zero_amount).is_err());

        // Test string amount (u128 serialization format)
        let string_amount_tx = serde_json::json!({
            "sig": "a".repeat(128),
            "nonce": 1,
            "payload": {
                "type": tx_type::TX_TYPE_TRANSFER,
                "sender": "0x1234567890123456789012345678901234567890",
                "recipient": "0x0987654321098765432109876543210987654321",
                "amount": "77777777777"
            },
            "public_key": "a".repeat(64),
            "fee": 100
        });
        assert!(validate_transaction_structure(&string_amount_tx).is_ok());
    }

    #[test]
    fn test_validate_stake_payload() {
        // Valid stake payload
        let valid_tx = serde_json::json!({
            "sig": "a".repeat(128),
            "nonce": 1,
            "payload": {
                "type": tx_type::TX_TYPE_STAKE,
                "sender": "0x1234567890123456789012345678901234567890",
                "amount": 1000
            },
            "public_key": "a".repeat(64),
            "fee": 100
        });
        assert!(validate_transaction_structure(&valid_tx).is_ok());

        // Valid stake payload with public key
        let valid_tx_with_pk = serde_json::json!({
            "sig": "a".repeat(128),
            "nonce": 1,
            "payload": {
                "type": tx_type::TX_TYPE_STAKE,
                "sender": "0x1234567890123456789012345678901234567890",
                "amount": 1000,
                "public_key": "a".repeat(64)
            },
            "public_key": "a".repeat(64),
            "fee": 100
        });
        assert!(validate_transaction_structure(&valid_tx_with_pk).is_ok());

        // Zero amount
        let zero_amount = serde_json::json!({
            "sig": "a".repeat(128),
            "nonce": 1,
            "payload": {
                "type": tx_type::TX_TYPE_STAKE,
                "sender": "0x1234567890123456789012345678901234567890",
                "amount": 0
            },
            "public_key": "a".repeat(64),
            "fee": 100
        });
        assert!(validate_transaction_structure(&zero_amount).is_err());
    }

    #[test]
    fn test_safe_bincode_deserialize() {
        use serde::{Deserialize, Serialize};

        #[derive(Debug, Serialize, Deserialize, PartialEq)]
        struct TestStruct {
            value: String,
            number: u32,
        }

        let test_data = TestStruct {
            value: "test".to_string(),
            number: 42,
        };

        // Test successful deserialization
        let serialized = bincode::serialize(&test_data).unwrap();
        let deserialized =
            safe_bincode_deserialize::<TestStruct>(&serialized, 1024, "test_struct").unwrap();
        assert_eq!(deserialized, test_data);

        // Test empty data
        assert!(safe_bincode_deserialize::<TestStruct>(&[], 1024, "empty_data").is_err());

        // Test oversized data
        let large_data = vec![0u8; 2048];
        assert!(
            safe_bincode_deserialize::<TestStruct>(&large_data, 1024, "oversized_data").is_err()
        );

        // Test malformed data
        let malformed_data = b"this is not valid bincode data";
        assert!(
            safe_bincode_deserialize::<TestStruct>(malformed_data, 1024, "malformed_data").is_err()
        );
    }

    #[test]
    fn test_safe_deserialize_cado_data() {
        use serde::{Deserialize, Serialize};

        #[derive(Debug, Serialize, Deserialize, PartialEq)]
        struct TestCado {
            id: String,
            data: Vec<u8>,
        }

        let test_cado = TestCado {
            id: "test_id".to_string(),
            data: vec![1, 2, 3, 4, 5],
        };

        // Test successful deserialization
        let serialized = bincode::serialize(&test_cado).unwrap();
        let deserialized =
            safe_deserialize_cado_data::<TestCado>(&serialized, "test_cado").unwrap();
        assert_eq!(deserialized, test_cado);

        // Test oversized data (exceeds 1MB limit)
        let oversized_cado = TestCado {
            id: "oversized".to_string(),
            data: vec![0u8; 2 * 1024 * 1024], // 2MB
        };
        let oversized_serialized = bincode::serialize(&oversized_cado).unwrap();
        assert!(
            safe_deserialize_cado_data::<TestCado>(&oversized_serialized, "oversized_cado")
                .is_err()
        );
    }

    #[test]
    fn test_verify_cado_deletion_signature() {
        use ed25519_dalek::{Signer, SigningKey};

        // Generate a keypair for testing
        let signing_key = SigningKey::from_bytes(&[1u8; 32]);
        let verifying_key = signing_key.verifying_key();

        let path = "/test/cado/path";
        let owner = "test_owner";
        let chain_id = "test_chain";

        // Create the message to sign: path + owner + chain_id
        let message = format!("{path}:{owner}:{chain_id}");
        let signature = signing_key.sign(message.as_bytes());
        let signature_hex = hex::encode(signature.to_bytes());
        let public_key_hex = hex::encode(verifying_key.to_bytes());

        // Test valid signature
        assert!(verify_cado_deletion_signature(
            path,
            owner,
            &signature_hex,
            &public_key_hex,
            chain_id
        )
        .is_ok());

        // Test invalid signature
        let invalid_signature = "a".repeat(128); // 64 bytes hex = 128 chars
        assert!(verify_cado_deletion_signature(
            path,
            owner,
            &invalid_signature,
            &public_key_hex,
            chain_id
        )
        .is_err());

        // Test invalid public key
        let invalid_public_key = "a".repeat(64); // 32 bytes hex = 64 chars
        assert!(verify_cado_deletion_signature(
            path,
            owner,
            &signature_hex,
            &invalid_public_key,
            chain_id
        )
        .is_err());

        // Test wrong owner
        assert!(verify_cado_deletion_signature(
            path,
            "wrong_owner",
            &signature_hex,
            &public_key_hex,
            chain_id
        )
        .is_err());

        // Test wrong path
        assert!(verify_cado_deletion_signature(
            "/wrong/path",
            owner,
            &signature_hex,
            &public_key_hex,
            chain_id
        )
        .is_err());

        // Test wrong chain_id
        assert!(verify_cado_deletion_signature(
            path,
            owner,
            &signature_hex,
            &public_key_hex,
            "wrong_chain"
        )
        .is_err());
    }
}
