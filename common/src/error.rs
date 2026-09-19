//! # Error Module
//!
//! This module provides centralized error handling for the Eld blockchain system.
//! It defines the main `EldError` enum and related error types.

use std::fmt;

const REDACTED_VALIDATION_VALUE: &str = "[redacted]";

/// Returns a display-safe validation `value` (secrets never echoed).
pub(crate) fn sanitize_validation_value(field: &str, value: &str) -> String {
    match field {
        "private_key" | "keypair" | "secret" | "secret_key" | "password" | "mnemonic" | "seed"
        | "sig" | "signature" | "hex_str" => REDACTED_VALIDATION_VALUE.to_string(),
        "wallet_json" | "json_str" => {
            let trimmed = value.trim_start();
            if trimmed.starts_with('[') || trimmed.starts_with('{') {
                REDACTED_VALIDATION_VALUE.to_string()
            } else {
                value.to_string()
            }
        }
        _ => value.to_string(),
    }
}

/// Main error type for the Eld blockchain system
#[derive(Clone)]
pub enum EldError {
    /// Network connectivity issues
    NetworkError { operation: String, details: String },
    /// Authentication and authorization issues
    AuthError { operation: String, details: String },
    /// Data validation issues
    ValidationError {
        field: String,
        value: String,
        details: String,
    },
    /// Basic validation issues with a single details message
    BasicValidationError { details: String },
    /// Resource not found
    NotFoundError {
        resource_type: String,
        identifier: String,
    },
    /// Insufficient resources (balance, stake, etc.)
    InsufficientResourceError {
        resource_type: String,
        required: String,
        available: String,
    },
    /// Configuration issues
    ConfigError { file: String, details: String },
    /// File system operations
    FileSystemError {
        operation: String,
        path: String,
        details: String,
    },
    /// Transaction processing issues
    TransactionError { tx_type: String, details: String },
    /// Wallet management issues
    WalletError {
        operation: String,
        wallet_name: String,
        details: String,
    },
    /// Device-related issues
    DeviceError {
        operation: String,
        device_id: Option<String>,
        details: String,
    },
    /// System initialization issues
    InitializationError { component: String, details: String },
    /// Storage and database operations
    StorageError { operation: String, details: String },
    /// Coin-related operations
    CoinError { details: String },
    /// Fee calculation and validation issues
    FeeError { operation: String, details: String },
}

impl fmt::Display for EldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EldError::NetworkError { operation, details } => {
                writeln!(f, "🌐 Network Error during {operation}")?;
                writeln!(f, "   Details: {details}")?;
            }
            EldError::AuthError { operation, details } => {
                writeln!(f, "🔐 Authentication Error during {operation}")?;
                writeln!(f, "   Details: {details}")?;
            }
            EldError::ValidationError {
                field,
                value,
                details,
            } => {
                writeln!(f, "❌ Validation Error for field '{field}'")?;
                writeln!(f, "   Value: {}", sanitize_validation_value(field, value))?;
                writeln!(f, "   Details: {details}")?;
            }
            EldError::BasicValidationError { details } => {
                writeln!(f, "❌ Validation Error")?;
                writeln!(f, "   Details: {details}")?;
            }
            EldError::NotFoundError {
                resource_type,
                identifier,
            } => {
                writeln!(f, "🔍 {resource_type} not found")?;
                writeln!(f, "   Identifier: {identifier}")?;
            }
            EldError::InsufficientResourceError {
                resource_type,
                required,
                available,
            } => {
                writeln!(f, "💰 Insufficient {resource_type}")?;
                writeln!(f, "   Required: {required}")?;
                writeln!(f, "   Available: {available}")?;
            }
            EldError::ConfigError { file, details } => {
                writeln!(f, "⚙️  Configuration Error in {file}")?;
                writeln!(f, "   Details: {details}")?;
            }
            EldError::FileSystemError {
                operation,
                path,
                details,
            } => {
                writeln!(f, "📁 File System Error during {operation}")?;
                writeln!(f, "   Path: {path}")?;
                writeln!(f, "   Details: {details}")?;
            }
            EldError::TransactionError { tx_type, details } => {
                writeln!(f, "📝 Transaction Error for {tx_type}")?;
                writeln!(f, "   Details: {details}")?;
            }
            EldError::WalletError {
                operation,
                wallet_name,
                details,
            } => {
                writeln!(f, "👛 Wallet Error during {operation} for '{wallet_name}'")?;
                writeln!(f, "   Details: {details}")?;
            }
            EldError::DeviceError {
                operation,
                device_id,
                details,
            } => {
                writeln!(f, "📱 Device Error during {operation}")?;
                if let Some(id) = device_id {
                    writeln!(f, "   Device ID: {id}")?;
                }
                writeln!(f, "   Details: {details}")?;
            }
            EldError::InitializationError { component, details } => {
                writeln!(f, "🚀 Initialization Error for {component}")?;
                writeln!(f, "   Details: {details}")?;
            }
            EldError::StorageError { operation, details } => {
                writeln!(f, "💾 Storage Error during {operation}")?;
                writeln!(f, "   Details: {details}")?;
            }
            EldError::CoinError { details } => {
                writeln!(f, "🪙 Coin Error: {details}")?;
            }
            EldError::FeeError { operation, details } => {
                writeln!(f, "💰 Fee Error during {operation}")?;
                writeln!(f, "   Details: {details}")?;
            }
        }
        Ok(())
    }
}

impl fmt::Debug for EldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

impl std::error::Error for EldError {}

/// Convert PoisonError to EldError for automatic error conversion
impl<T> From<std::sync::PoisonError<T>> for EldError {
    fn from(e: std::sync::PoisonError<T>) -> Self {
        EldError::InitializationError {
            component: "mutex lock".to_string(),
            details: e.to_string(),
        }
    }
}

impl EldError {
    /// Builds a validation error with sensitive `value` fields redacted.
    pub fn make_validation_error(
        field: impl Into<String>,
        value: &str,
        details: impl Into<String>,
    ) -> Self {
        let field = field.into();
        Self::ValidationError {
            field: field.clone(),
            value: sanitize_validation_value(&field, value),
            details: details.into(),
        }
    }

    /// Create a validation error and return it as a Result
    pub fn validation_error(field: &str, value: &str, details: &str) -> Result<(), Self> {
        Err(Self::make_validation_error(field, value, details))
    }

    /// Create a network error and return it as a Result
    pub fn network_error(operation: &str, details: &str) -> Result<(), Self> {
        Err(Self::NetworkError {
            operation: operation.to_string(),
            details: details.to_string(),
        })
    }

    /// Create a basic validation error and return it as a Result
    pub fn basic_validation_error(details: &str) -> Result<(), Self> {
        Err(Self::BasicValidationError {
            details: details.to_string(),
        })
    }

    /// Create a storage error and return it as a Result
    pub fn storage_error(operation: &str, details: &str) -> Result<(), Self> {
        Err(Self::StorageError {
            operation: operation.to_string(),
            details: details.to_string(),
        })
    }

    /// Create an initialization error and return it as a Result
    pub fn initialization_error(component: &str, details: &str) -> Result<(), Self> {
        Err(Self::InitializationError {
            component: component.to_string(),
            details: details.to_string(),
        })
    }

    /// Create a not found error and return it as a Result
    pub fn not_found_error(resource_type: &str, identifier: &str) -> Result<(), Self> {
        Err(Self::NotFoundError {
            resource_type: resource_type.to_string(),
            identifier: identifier.to_string(),
        })
    }

    /// Create an insufficient resource error and return it as a Result
    pub fn insufficient_resource_error(
        resource_type: &str,
        required: &str,
        available: &str,
    ) -> Result<(), Self> {
        Err(Self::InsufficientResourceError {
            resource_type: resource_type.to_string(),
            required: required.to_string(),
            available: available.to_string(),
        })
    }

    /// Create a config error and return it as a Result
    pub fn config_error(file: &str, details: &str) -> Result<(), Self> {
        Err(Self::ConfigError {
            file: file.to_string(),
            details: details.to_string(),
        })
    }

    /// Create a file system error and return it as a Result
    pub fn file_system_error(operation: &str, path: &str, details: &str) -> Result<(), Self> {
        Err(Self::FileSystemError {
            operation: operation.to_string(),
            path: path.to_string(),
            details: details.to_string(),
        })
    }

    /// Create a transaction error and return it as a Result
    pub fn transaction_error(tx_type: &str, details: &str) -> Result<(), Self> {
        Err(Self::TransactionError {
            tx_type: tx_type.to_string(),
            details: details.to_string(),
        })
    }

    /// Create an auth error and return it as a Result
    pub fn auth_error(operation: &str, details: &str) -> Result<(), Self> {
        Err(Self::AuthError {
            operation: operation.to_string(),
            details: details.to_string(),
        })
    }

    /// Create a wallet error and return it as a Result
    pub fn wallet_error(operation: &str, wallet_name: &str, details: &str) -> Result<(), Self> {
        Err(Self::WalletError {
            operation: operation.to_string(),
            wallet_name: wallet_name.to_string(),
            details: details.to_string(),
        })
    }

    /// Create a device error and return it as a Result
    pub fn device_error(
        operation: &str,
        device_id: Option<&str>,
        details: &str,
    ) -> Result<(), Self> {
        Err(Self::DeviceError {
            operation: operation.to_string(),
            device_id: device_id.map(|s| s.to_string()),
            details: details.to_string(),
        })
    }

    /// Create a coin error and return it as a Result
    pub fn coin_error(details: &str) -> Result<(), Self> {
        Err(Self::CoinError {
            details: details.to_string(),
        })
    }

    /// Create a fee error and return it as a Result
    pub fn fee_error(operation: &str, details: &str) -> Result<(), Self> {
        Err(Self::FeeError {
            operation: operation.to_string(),
            details: details.to_string(),
        })
    }
}

/// Helper functions for creating structured errors
pub struct ErrorBuilder;

impl ErrorBuilder {
    pub fn network_error(operation: &str, details: &str) -> EldError {
        EldError::NetworkError {
            operation: operation.to_string(),
            details: details.to_string(),
        }
    }

    pub fn auth_error(operation: &str, details: &str) -> EldError {
        EldError::AuthError {
            operation: operation.to_string(),
            details: details.to_string(),
        }
    }

    pub fn validation_error(field: &str, value: &str, details: &str) -> EldError {
        EldError::make_validation_error(field, value, details)
    }

    pub fn basic_validation_error(details: &str) -> EldError {
        EldError::BasicValidationError {
            details: details.to_string(),
        }
    }

    pub fn not_found_error(resource_type: &str, identifier: &str) -> EldError {
        EldError::NotFoundError {
            resource_type: resource_type.to_string(),
            identifier: identifier.to_string(),
        }
    }

    pub fn insufficient_resource_error(
        resource_type: &str,
        required: &str,
        available: &str,
    ) -> EldError {
        EldError::InsufficientResourceError {
            resource_type: resource_type.to_string(),
            required: required.to_string(),
            available: available.to_string(),
        }
    }

    pub fn config_error(file: &str, details: &str) -> EldError {
        EldError::ConfigError {
            file: file.to_string(),
            details: details.to_string(),
        }
    }

    pub fn file_system_error(operation: &str, path: &str, details: &str) -> EldError {
        EldError::FileSystemError {
            operation: operation.to_string(),
            path: path.to_string(),
            details: details.to_string(),
        }
    }

    pub fn transaction_error(tx_type: &str, details: &str) -> EldError {
        EldError::TransactionError {
            tx_type: tx_type.to_string(),
            details: details.to_string(),
        }
    }

    pub fn wallet_error(operation: &str, wallet_name: &str, details: &str) -> EldError {
        EldError::WalletError {
            operation: operation.to_string(),
            wallet_name: wallet_name.to_string(),
            details: details.to_string(),
        }
    }

    pub fn device_error(operation: &str, device_id: Option<&str>, details: &str) -> EldError {
        EldError::DeviceError {
            operation: operation.to_string(),
            device_id: device_id.map(|s| s.to_string()),
            details: details.to_string(),
        }
    }

    pub fn initialization_error(component: &str, details: &str) -> EldError {
        EldError::InitializationError {
            component: component.to_string(),
            details: details.to_string(),
        }
    }

    pub fn storage_error(operation: &str, details: &str) -> EldError {
        EldError::StorageError {
            operation: operation.to_string(),
            details: details.to_string(),
        }
    }

    pub fn coin_error(details: &str) -> EldError {
        EldError::CoinError {
            details: details.to_string(),
        }
    }

    pub fn fee_error(operation: &str, details: &str) -> EldError {
        EldError::FeeError {
            operation: operation.to_string(),
            details: details.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_error_redacts_wallet_json_content() {
        let secret = r#"[{"private_key":"deadbeef"}]"#;
        let err = EldError::make_validation_error("wallet_json", secret, "parse failed");
        let msg = err.to_string();
        let debug = format!("{err:?}");

        assert!(!msg.contains("deadbeef"));
        assert!(!debug.contains("deadbeef"));
        assert!(msg.contains(REDACTED_VALIDATION_VALUE));
    }

    #[test]
    fn validation_error_keeps_wallet_file_path() {
        let path = "/app/wallets/wallets.json";
        let err = EldError::make_validation_error("wallet_json", path, "parse failed");
        let msg = err.to_string();

        assert!(msg.contains("wallets.json"));
        assert!(!msg.contains(REDACTED_VALIDATION_VALUE));
    }

    #[test]
    fn validation_error_redacts_private_key_field() {
        // Documented throwaway hex (repeating 0x01). Not a live-network key.
        const THROWAWAY_SEED_HEX: &str =
            "0101010101010101010101010101010101010101010101010101010101010101";
        let err = EldError::make_validation_error("private_key", THROWAWAY_SEED_HEX, "invalid");
        let msg = err.to_string();

        assert!(!msg.contains("01010101"));
        assert!(msg.contains(REDACTED_VALIDATION_VALUE));
    }
}
