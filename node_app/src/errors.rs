use std::process;

use abci::{types::ResponseCheckTx, types::ResponseDeliverTx};
use eld_common::{constants::ELD_CODESPACE, error::EldError};
use tracing::error;

/// Handles fatal errors by logging the error and exiting the process with code 1.
/// This function never returns (terminates the program).
pub fn handle_fatal_eld_error(e: EldError) -> ! {
    error!("Fatal error: {}", e);
    process::exit(1);
}

/// Handles recoverable errors by logging the error and returning it.
/// This allows the calling code to decide how to proceed.
pub fn handle_recoverable_eld_error(e: EldError) -> EldError {
    error!("Recoverable error: {}", e);
    e
}

/// Log detailed IO-related server startup error messages, used by main.rs.
pub fn log_server_io_error_details(
    server_name: &str,
    err: &(dyn std::error::Error + Send + Sync + 'static),
) {
    if let Some(io_error) = err.downcast_ref::<std::io::Error>() {
        match io_error.kind() {
            std::io::ErrorKind::AddrInUse => {
                error!(
                    "{}: Port is already in use. Check if another instance is running.",
                    server_name
                );
            }
            std::io::ErrorKind::PermissionDenied => {
                error!(
                    "{}: Permission denied. Check if you have sufficient privileges to bind to the port.",
                    server_name
                );
            }
            std::io::ErrorKind::ConnectionRefused => {
                error!(
                    "{}: Connection refused. Check network configuration.",
                    server_name
                );
            }
            _ => {
                error!(
                    "{}: Unexpected error occurred: {:?}",
                    server_name,
                    io_error.kind()
                );
            }
        }
    } else {
        error!("{}: Unexpected error occurred: {}", server_name, err);
    }
}

// Core transaction errors (1-10)
pub fn response_deliver_tx_error_insufficient_stake() -> ResponseDeliverTx {
    ResponseDeliverTx {
        code: 1,
        codespace: ELD_CODESPACE.to_string(),
        log: "Insufficient stake".to_owned(),
        info: "Insufficient stake".to_owned(),
        ..Default::default()
    }
}

pub fn response_deliver_tx_error_minimum_stake() -> ResponseDeliverTx {
    ResponseDeliverTx {
        code: 2,
        codespace: ELD_CODESPACE.to_string(),
        log: "Minimum stake not met".to_owned(),
        info: "Minimum stake not met".to_owned(),
        ..Default::default()
    }
}

pub fn response_deliver_tx_error_nonce_not_sequential(
    expected_nonce: u32,
    actual_nonce: u32,
    account_address: String,
) -> ResponseDeliverTx {
    let log_message = format!(
        "Nonce is not sequential for account {account_address}: expected {expected_nonce}, but got {actual_nonce}"
    );
    ResponseDeliverTx {
        code: 3,
        codespace: ELD_CODESPACE.to_string(),
        log: log_message.clone(),
        info: log_message,
        ..Default::default()
    }
}

pub fn response_deliver_tx_error_nonce_overflow(account_address: String) -> ResponseDeliverTx {
    let log_message =
        format!("Account nonce overflow for {account_address}: cannot increment past u32::MAX");
    ResponseDeliverTx {
        code: 6,
        codespace: ELD_CODESPACE.to_string(),
        log: log_message.clone(),
        info: log_message,
        ..Default::default()
    }
}

pub fn response_check_tx_error_verification_failed() -> ResponseCheckTx {
    ResponseCheckTx {
        code: 4,
        codespace: ELD_CODESPACE.to_string(),
        log: "Tx verification failed".to_owned(),
        info: "Tx verification failed".to_owned(),
        ..Default::default()
    }
}

pub fn response_check_tx_error_nonce_not_sequential(
    expected_nonce: u32,
    actual_nonce: u32,
    account_address: String,
) -> ResponseCheckTx {
    let log_message = format!(
        "Nonce is not sequential for account {account_address}: expected {expected_nonce}, but got {actual_nonce}"
    );
    ResponseCheckTx {
        code: 3,
        codespace: ELD_CODESPACE.to_string(),
        log: log_message.clone(),
        info: log_message,
        ..Default::default()
    }
}

pub fn response_check_tx_error_nonce_overflow(account_address: String) -> ResponseCheckTx {
    let log_message =
        format!("Account nonce overflow for {account_address}: cannot increment past u32::MAX");
    ResponseCheckTx {
        code: 6,
        codespace: ELD_CODESPACE.to_string(),
        log: log_message.clone(),
        info: log_message,
        ..Default::default()
    }
}

pub fn response_check_tx_error_sender_doesnt_exist(account_address: String) -> ResponseCheckTx {
    ResponseCheckTx {
        code: 9,
        codespace: ELD_CODESPACE.to_string(),
        log: format!("Sender account not found: {account_address}"),
        info: "Sender doesn't exist".to_owned(),
        ..Default::default()
    }
}

pub fn response_deliver_tx_error_insufficient_funds() -> ResponseDeliverTx {
    ResponseDeliverTx {
        code: 5,
        codespace: ELD_CODESPACE.to_string(),
        log: "Sender has insufficient funds".to_owned(),
        info: "Sender has insufficient funds".to_owned(),
        ..Default::default()
    }
}

pub fn response_check_tx_error_tx_too_large(
    max_size: usize,
    actual_size: usize,
) -> ResponseCheckTx {
    ResponseCheckTx {
        code: 7,
        codespace: ELD_CODESPACE.to_string(),
        log: format!(
            "Transaction too large. Max size is {max_size} bytes, but got {actual_size} bytes"
        ),
        info: format!(
            "Transaction too large. Max size is {max_size} bytes, but got {actual_size} bytes"
        ),
        ..Default::default()
    }
}

pub fn response_deliver_tx_error_tx_too_large(
    max_size: usize,
    actual_size: usize,
) -> ResponseDeliverTx {
    ResponseDeliverTx {
        code: 8,
        codespace: ELD_CODESPACE.to_string(),
        log: format!(
            "Transaction too large. Max size is {max_size} bytes, but got {actual_size} bytes"
        ),
        info: format!(
            "Transaction too large. Max size is {max_size} bytes, but got {actual_size} bytes"
        ),
        ..Default::default()
    }
}

pub fn response_deliver_tx_error_sender_doesnt_exist(error_log: String) -> ResponseDeliverTx {
    ResponseDeliverTx {
        code: 9,
        codespace: ELD_CODESPACE.to_string(),
        log: format!("Sender doesn't exist: {error_log}"),
        info: "Sender doesn't exist".to_owned(),
        ..Default::default()
    }
}

// Fee-related errors (11-15)
pub fn response_check_tx_error_insufficient_fee(
    required_fee: u128,
    provided_fee: u128,
) -> ResponseCheckTx {
    ResponseCheckTx {
        code: 11,
        codespace: ELD_CODESPACE.to_string(),
        log: format!("Insufficient fee. Required: {required_fee}, provided: {provided_fee}"),
        info: format!("Insufficient fee. Required: {required_fee}, provided: {provided_fee}"),
        ..Default::default()
    }
}

pub fn response_deliver_tx_error_insufficient_fee(
    required_fee: u128,
    provided_fee: u128,
) -> ResponseDeliverTx {
    ResponseDeliverTx {
        code: 12,
        codespace: ELD_CODESPACE.to_string(),
        log: format!("Insufficient fee. Required: {required_fee}, provided: {provided_fee}"),
        info: format!("Insufficient fee. Required: {required_fee}, provided: {provided_fee}"),
        ..Default::default()
    }
}

// Transaction processing errors (13-20)
pub fn response_deliver_tx_error_verification_failed() -> ResponseDeliverTx {
    ResponseDeliverTx {
        code: 13,
        codespace: ELD_CODESPACE.to_string(),
        log: "Tx verification failed".to_owned(),
        info: "Tx verification failed".to_owned(),
        ..Default::default()
    }
}

pub fn response_deliver_tx_error_unsupported_tx_type() -> ResponseDeliverTx {
    ResponseDeliverTx {
        code: 14,
        codespace: ELD_CODESPACE.to_string(),
        log: "Unsupported tx type".to_owned(),
        info: "Unsupported tx type".to_owned(),
        ..Default::default()
    }
}

// Validation and parsing errors (51-60)
pub fn response_deliver_tx_error_validation_failed(error_details: String) -> ResponseDeliverTx {
    ResponseDeliverTx {
        code: 51,
        codespace: ELD_CODESPACE.to_string(),
        log: format!("Transaction validation failed: {error_details}"),
        info: "Transaction validation failed".to_owned(),
        ..Default::default()
    }
}

pub fn response_deliver_tx_error_json_parsing_failed(error_details: String) -> ResponseDeliverTx {
    ResponseDeliverTx {
        code: 52,
        codespace: ELD_CODESPACE.to_string(),
        log: format!("JSON parsing failed: {error_details}"),
        info: "JSON parsing failed".to_owned(),
        ..Default::default()
    }
}

pub fn response_deliver_tx_error_invalid_public_key_format() -> ResponseDeliverTx {
    ResponseDeliverTx {
        code: 56,
        codespace: ELD_CODESPACE.to_string(),
        log: "Invalid public key format".to_owned(),
        info: "Public key must be hex encoded".to_owned(),
        ..Default::default()
    }
}

pub fn response_deliver_tx_error_invalid_public_key_length() -> ResponseDeliverTx {
    ResponseDeliverTx {
        code: 57,
        codespace: ELD_CODESPACE.to_string(),
        log: "Invalid public key length".to_owned(),
        info: "Public key must be 32 bytes".to_owned(),
        ..Default::default()
    }
}

// Internal system errors (61-70)
pub fn response_deliver_tx_error_internal_lock_failed(error_details: String) -> ResponseDeliverTx {
    ResponseDeliverTx {
        code: 61,
        codespace: ELD_CODESPACE.to_string(),
        log: format!("Internal lock acquisition failed: {error_details}"),
        info: "Internal system error".to_owned(),
        ..Default::default()
    }
}

pub fn response_deliver_tx_error_invalid_utf8_encoding(error_details: String) -> ResponseDeliverTx {
    ResponseDeliverTx {
        code: 62,
        codespace: ELD_CODESPACE.to_string(),
        log: format!("Invalid UTF-8 encoding: {error_details}"),
        info: "Transaction contains invalid UTF-8 bytes".to_owned(),
        ..Default::default()
    }
}

pub fn response_deliver_tx_error_invalid_hex_encoding(error_details: String) -> ResponseDeliverTx {
    ResponseDeliverTx {
        code: 63,
        codespace: ELD_CODESPACE.to_string(),
        log: format!("Invalid hex encoding: {error_details}"),
        info: "Transaction contains invalid hex encoding".to_owned(),
        ..Default::default()
    }
}

pub fn response_deliver_tx_error_invalid_path_format(error_details: String) -> ResponseDeliverTx {
    ResponseDeliverTx {
        code: 64,
        codespace: ELD_CODESPACE.to_string(),
        log: format!("Invalid path format: {error_details}"),
        info: "Invalid CADO path format".to_owned(),
        ..Default::default()
    }
}

pub fn response_deliver_tx_error_invalid_coin_amount(error_details: String) -> ResponseDeliverTx {
    ResponseDeliverTx {
        code: 65,
        codespace: ELD_CODESPACE.to_string(),
        log: format!("Invalid coin amount: {error_details}"),
        info: "Invalid coin amount specified".to_owned(),
        ..Default::default()
    }
}

pub fn response_deliver_tx_error_fee_validation_failed(error_details: String) -> ResponseDeliverTx {
    ResponseDeliverTx {
        code: 66,
        codespace: ELD_CODESPACE.to_string(),
        log: format!("Fee validation failed: {error_details}"),
        info: "Transaction fee validation failed".to_owned(),
        ..Default::default()
    }
}

// Input validation errors (71-80)
pub fn response_deliver_tx_error_hex_validation_failed(error_details: String) -> ResponseDeliverTx {
    ResponseDeliverTx {
        code: 71,
        codespace: ELD_CODESPACE.to_string(),
        log: format!("Hex validation failed: {error_details}"),
        info: "Hex validation failed".to_owned(),
        ..Default::default()
    }
}

pub fn response_deliver_tx_error_json_validation_failed(
    error_details: String,
) -> ResponseDeliverTx {
    ResponseDeliverTx {
        code: 72,
        codespace: ELD_CODESPACE.to_string(),
        log: format!("JSON validation failed: {error_details}"),
        info: "JSON validation failed".to_owned(),
        ..Default::default()
    }
}

pub fn response_deliver_tx_error_transaction_structure_invalid(
    error_details: String,
) -> ResponseDeliverTx {
    ResponseDeliverTx {
        code: 73,
        codespace: ELD_CODESPACE.to_string(),
        log: format!("Transaction structure invalid: {error_details}"),
        info: "Transaction structure invalid".to_owned(),
        ..Default::default()
    }
}

// Fee calculation errors (74-75)
pub fn response_deliver_tx_error_fee_calculation_failed(
    error_details: String,
) -> ResponseDeliverTx {
    ResponseDeliverTx {
        code: 74,
        codespace: ELD_CODESPACE.to_string(),
        log: format!("Fee calculation failed: {error_details}"),
        info: "Dynamic fee calculation failed due to overflow or invalid parameters".to_owned(),
        ..Default::default()
    }
}

pub fn response_check_tx_error_fee_calculation_failed(error_details: String) -> ResponseCheckTx {
    ResponseCheckTx {
        code: 75,
        codespace: ELD_CODESPACE.to_string(),
        log: format!("Fee calculation failed: {error_details}"),
        info: "Dynamic fee calculation failed due to overflow or invalid parameters".to_owned(),
        ..Default::default()
    }
}

pub fn response_check_tx_error_invalid_utf8_encoding(error_details: String) -> ResponseCheckTx {
    ResponseCheckTx {
        code: 76,
        codespace: ELD_CODESPACE.to_string(),
        log: format!("Invalid UTF-8 encoding: {error_details}"),
        info: "Transaction contains invalid UTF-8 bytes".to_owned(),
        ..Default::default()
    }
}

pub fn response_check_tx_error_invalid_hex_encoding(error_details: String) -> ResponseCheckTx {
    ResponseCheckTx {
        code: 77,
        codespace: ELD_CODESPACE.to_string(),
        log: format!("Invalid hex encoding: {error_details}"),
        info: "Transaction contains invalid hex encoding".to_owned(),
        ..Default::default()
    }
}

pub fn response_check_tx_error_json_parsing_failed(error_details: String) -> ResponseCheckTx {
    ResponseCheckTx {
        code: 78,
        codespace: ELD_CODESPACE.to_string(),
        log: format!("JSON parsing failed: {error_details}"),
        info: "Failed to parse transaction JSON".to_owned(),
        ..Default::default()
    }
}
