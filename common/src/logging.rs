//! # Logging Module
//!
//! This module provides centralized logging configuration for the Eld blockchain system
//! using the `tracing` framework for structured logging.

use crate::error::EldError;
use std::fmt;
use std::path::PathBuf;
use tracing::Level;
use tracing_subscriber::{fmt::time::UtcTime, prelude::*, EnvFilter};

/// Logging format options
#[derive(Debug, Clone, Copy)]
pub enum LogFormat {
    /// Human-readable format for development
    Human,
    /// JSON format for production and log aggregation
    Json,
}

/// Log rotation configuration
#[derive(Debug, Clone)]
pub struct LogRotation {
    /// Maximum file size in bytes before rotation
    pub max_size: usize,
    /// Maximum number of files to keep
    pub max_files: usize,
    /// Whether to compress old log files
    pub compress: bool,
}

impl Default for LogRotation {
    fn default() -> Self {
        Self {
            max_size: 100 * 1024 * 1024, // 100MB
            max_files: 10,
            compress: true,
        }
    }
}

/// Centralized logging configuration
#[derive(Debug, Clone)]
pub struct LoggingConfig {
    /// Log level for the application
    pub level: Level,
    /// Optional log file path for file-based logging
    pub log_file: Option<PathBuf>,
    /// Log rotation configuration
    pub rotation: LogRotation,
    /// Log format (human-readable or JSON)
    pub format: LogFormat,
    /// Whether to enable console output
    pub console_output: bool,
    /// Whether to show file paths in logs
    pub show_file: bool,
    /// Whether to show line numbers in logs
    pub show_line_number: bool,
    /// Whether to show target (module path) in logs
    pub show_target: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: Level::INFO,
            log_file: None,
            rotation: LogRotation::default(),
            console_output: true,
            format: LogFormat::Human,
            show_file: false,
            show_line_number: false,
            show_target: false,
        }
    }
}

/// Structured logging categories for different components
#[derive(Debug, Clone, Copy)]
pub enum LogCategory {
    Consensus,
    Storage,
    Network,
    Validation,
    Transaction,
    Device,
    Contract,
    CLI,
    API,
    General,
}

impl std::fmt::Display for LogCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogCategory::Consensus => write!(f, "consensus"),
            LogCategory::Storage => write!(f, "storage"),
            LogCategory::Network => write!(f, "network"),
            LogCategory::Validation => write!(f, "validation"),
            LogCategory::Transaction => write!(f, "transaction"),
            LogCategory::Device => write!(f, "device"),
            LogCategory::Contract => write!(f, "contract"),
            LogCategory::CLI => write!(f, "cli"),
            LogCategory::API => write!(f, "api"),
            LogCategory::General => write!(f, "general"),
        }
    }
}

/// Initialize the logging system with the given configuration
pub fn init_logging(config: &LoggingConfig) -> Result<(), EldError> {
    // Create environment filter
    // Create environment filter
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("eld={}", config.level)));

    // Create the subscriber builder
    let subscriber = tracing_subscriber::registry().with(env_filter).with(
        tracing_subscriber::fmt::layer()
            .with_timer(UtcTime::rfc_3339())
            .with_target(config.show_target)
            .with_thread_ids(true)
            .with_thread_names(true)
            .with_file(config.show_file)
            .with_line_number(config.show_line_number),
    );

    // Set the global default subscriber
    tracing::subscriber::set_global_default(subscriber).map_err(|e| {
        EldError::InitializationError {
            component: "logging system".to_string(),
            details: format!("Failed to set global default subscriber: {e}"),
        }
    })?;

    tracing::info!(
        level = %config.level,
        format = ?config.format,
        console_output = config.console_output,
        log_file = ?config.log_file,
        "Logging system initialized"
    );

    Ok(())
}

/// Initialize logging with default configuration
pub fn init_default_logging() -> Result<(), EldError> {
    let config = LoggingConfig::default();
    init_logging(&config)
}

/// Initialize logging for development environment
pub fn init_dev_logging() -> Result<(), EldError> {
    let config = LoggingConfig {
        level: Level::DEBUG,
        format: LogFormat::Human,
        console_output: true,
        log_file: None,
        rotation: LogRotation::default(),
        show_file: true,
        show_line_number: true,
        show_target: true,
    };
    init_logging(&config)
}

/// Initialize logging for production environment
pub fn init_prod_logging(log_file: Option<PathBuf>) -> Result<(), EldError> {
    let config = LoggingConfig {
        level: Level::INFO,
        format: LogFormat::Json,
        console_output: false,
        log_file,
        rotation: LogRotation::default(),
        show_file: false,
        show_line_number: false,
        show_target: false,
    };
    init_logging(&config)
}

/// Sanitizes sensitive data for logging by truncating or masking sensitive information
pub struct LogSanitizer;

impl LogSanitizer {
    /// Sanitizes a CADO key by showing only the first 8 and last 4 characters
    /// Example: "abc123def456" -> "abc123...def456"
    pub fn sanitize_cado_key(key: &str) -> String {
        if key.len() <= 12 {
            return key.to_string();
        }
        format!("{}...{}", &key[..8], &key[key.len() - 4..])
    }

    /// Sanitizes a file path by showing only the directory structure and filename
    /// Example: "/home/user/.eld/config.json" -> "/.../config.json"
    pub fn sanitize_path(path: &str) -> String {
        if path.is_empty() {
            return path.to_string();
        }

        let path_components: Vec<&str> = path.split('/').collect();
        if path_components.len() <= 2 {
            return path.to_string();
        }

        // Show only the last component (filename) and indicate directory structure
        if let Some(filename) = path_components.last() {
            format!("/.../{filename}")
        } else {
            path.to_string()
        }
    }

    /// Sanitizes an address by showing only the first 6 and last 4 characters
    /// Example: "0x1234567890abcdef1234567890abcdef12345678" -> "0x1234...5678"
    pub fn sanitize_address(address: &str) -> String {
        if !address.starts_with("0x") || address.len() <= 10 {
            return address.to_string();
        }

        let hex_part = &address[2..]; // Remove 0x prefix
        if hex_part.len() <= 8 {
            return address.to_string();
        }

        format!("0x{}...{}", &hex_part[..4], &hex_part[hex_part.len() - 4..])
    }

    /// Sanitizes a hash by showing only the first 8 and last 4 characters
    /// Example: "a1b2c3d4e5f6789012345678901234567890abcdef" -> "a1b2c3d4...cdef"
    pub fn sanitize_hash(hash: &str) -> String {
        if hash.len() <= 12 {
            return hash.to_string();
        }
        format!("{}...{}", &hash[..8], &hash[hash.len() - 4..])
    }

    /// Sanitizes a public key by showing only the first 8 and last 4 characters
    /// Example: "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef" -> "12345678...cdef"
    pub fn sanitize_public_key(pubkey: &str) -> String {
        if pubkey.len() <= 12 {
            return pubkey.to_string();
        }
        format!("{}...{}", &pubkey[..8], &pubkey[pubkey.len() - 4..])
    }

    /// Sanitizes a signature by showing only the first 8 and last 4 characters
    /// Example: "sig1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef" -> "sig12345...cdef"
    pub fn sanitize_signature(signature: &str) -> String {
        if signature.len() <= 12 {
            return signature.to_string();
        }
        format!(
            "{}...{}",
            &signature[..8],
            &signature[signature.len() - 4..]
        )
    }

    /// Sanitizes a manifest ID by showing only the first 8 and last 4 characters
    /// Example: "manifest_1234567890abcdef1234567890abcdef" -> "manifest...cdef"
    pub fn sanitize_manifest_id(manifest_id: &str) -> String {
        if manifest_id.len() <= 12 {
            return manifest_id.to_string();
        }
        format!(
            "{}...{}",
            &manifest_id[..8],
            &manifest_id[manifest_id.len() - 4..]
        )
    }

    /// Sanitizes a contract address by showing only the first 6 and last 4 characters
    /// Example: "contract_1234567890abcdef1234567890abcdef" -> "contra...cdef"
    pub fn sanitize_contract_address(contract_addr: &str) -> String {
        if contract_addr.len() <= 10 {
            return contract_addr.to_string();
        }
        format!(
            "{}...{}",
            &contract_addr[..6],
            &contract_addr[contract_addr.len() - 4..]
        )
    }

    /// Generic sanitization that attempts to identify the type of data and sanitize accordingly
    pub fn sanitize_generic(data: &str) -> String {
        if data.starts_with("0x") && data.len() > 10 {
            // Likely an address or hash
            Self::sanitize_address(data)
        } else if data.contains('/') && data.len() > 20 {
            // Likely a path
            Self::sanitize_path(data)
        } else if data.len() > 12 {
            // Generic truncation for other sensitive data
            format!("{}...{}", &data[..8], &data[data.len() - 4..])
        } else {
            data.to_string()
        }
    }
}

/// Wrapper for logging sensitive data with automatic sanitization
pub struct SanitizedLog {
    sanitized: String,
}

impl SanitizedLog {
    pub fn new<T: ToString>(data: T) -> Self {
        let sanitized = LogSanitizer::sanitize_generic(&data.to_string());
        Self { sanitized }
    }

    pub fn as_cado_key<T: ToString>(data: T) -> Self {
        let sanitized = LogSanitizer::sanitize_cado_key(&data.to_string());
        Self { sanitized }
    }

    pub fn as_path<T: ToString>(data: T) -> Self {
        let sanitized = LogSanitizer::sanitize_path(&data.to_string());
        Self { sanitized }
    }

    pub fn as_address<T: ToString>(data: T) -> Self {
        let sanitized = LogSanitizer::sanitize_address(&data.to_string());
        Self { sanitized }
    }

    pub fn as_hash<T: ToString>(data: T) -> Self {
        let sanitized = LogSanitizer::sanitize_hash(&data.to_string());
        Self { sanitized }
    }

    pub fn as_public_key<T: ToString>(data: T) -> Self {
        let sanitized = LogSanitizer::sanitize_public_key(&data.to_string());
        Self { sanitized }
    }

    pub fn as_signature<T: ToString>(data: T) -> Self {
        let sanitized = LogSanitizer::sanitize_signature(&data.to_string());
        Self { sanitized }
    }

    pub fn as_manifest_id<T: ToString>(data: T) -> Self {
        let sanitized = LogSanitizer::sanitize_manifest_id(&data.to_string());
        Self { sanitized }
    }

    pub fn as_contract_address<T: ToString>(data: T) -> Self {
        let sanitized = LogSanitizer::sanitize_contract_address(&data.to_string());
        Self { sanitized }
    }
}

impl fmt::Display for SanitizedLog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.sanitized)
    }
}

impl fmt::Debug for SanitizedLog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.sanitized)
    }
}

/// Trait for types that can provide sanitized logging
pub trait SanitizedLoggable {
    fn sanitized_log(&self) -> String;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Example home config path for path-sanitizer tests (not a real on-disk location).
    const EXAMPLE_USER_CONFIG_PATH: &str = "/home/user/.eld/config.json";

    #[test]
    fn test_log_category_display() {
        assert_eq!(LogCategory::Consensus.to_string(), "consensus");
        assert_eq!(LogCategory::Storage.to_string(), "storage");
        assert_eq!(LogCategory::Transaction.to_string(), "transaction");
    }

    #[test]
    fn test_default_logging_config() {
        let config = LoggingConfig::default();
        assert_eq!(config.level, Level::INFO);
        assert!(config.console_output);
        assert!(config.log_file.is_none());
        assert!(!config.show_file);
        assert!(!config.show_line_number);
        assert!(!config.show_target);
    }

    #[test]
    fn test_default_log_rotation() {
        let rotation = LogRotation::default();
        assert_eq!(rotation.max_size, 100 * 1024 * 1024);
        assert_eq!(rotation.max_files, 10);
        assert!(rotation.compress);
    }

    #[test]
    fn test_sanitize_cado_key() {
        assert_eq!(
            LogSanitizer::sanitize_cado_key("abc123def456"),
            "abc123def456"
        );
        assert_eq!(
            LogSanitizer::sanitize_cado_key("abc123def456789"),
            "abc123de...6789"
        );
        assert_eq!(LogSanitizer::sanitize_cado_key("short"), "short");
    }

    #[test]
    fn test_sanitize_path() {
        assert_eq!(
            LogSanitizer::sanitize_path(EXAMPLE_USER_CONFIG_PATH),
            "/.../config.json"
        );
        assert_eq!(LogSanitizer::sanitize_path("/config.json"), "/config.json");
    }

    #[test]
    fn test_sanitize_address() {
        assert_eq!(
            LogSanitizer::sanitize_address("0x1234567890abcdef1234567890abcdef12345678"),
            "0x1234...5678"
        );
        assert_eq!(LogSanitizer::sanitize_address("0x12345678"), "0x12345678");
    }

    #[test]
    fn test_sanitize_hash() {
        assert_eq!(
            LogSanitizer::sanitize_hash("a1b2c3d4e5f6789012345678901234567890abcdef"),
            "a1b2c3d4...cdef"
        );
        assert_eq!(LogSanitizer::sanitize_hash("short"), "short");
    }

    #[test]
    fn test_sanitize_public_key() {
        assert_eq!(
            LogSanitizer::sanitize_public_key(
                "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
            ),
            "12345678...cdef"
        );
        assert_eq!(LogSanitizer::sanitize_public_key("short"), "short");
    }

    #[test]
    fn test_sanitize_signature() {
        assert_eq!(
            LogSanitizer::sanitize_signature(
                "sig1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
            ),
            "sig12345...cdef"
        );
        assert_eq!(LogSanitizer::sanitize_signature("short"), "short");
    }

    #[test]
    fn test_sanitize_manifest_id() {
        assert_eq!(
            LogSanitizer::sanitize_manifest_id("manifest_1234567890abcdef1234567890abcdef"),
            "manifest...cdef"
        );
        assert_eq!(LogSanitizer::sanitize_manifest_id("short"), "short");
    }

    #[test]
    fn test_sanitize_contract_address() {
        assert_eq!(
            LogSanitizer::sanitize_contract_address("contract_1234567890abcdef1234567890abcdef"),
            "contra...cdef"
        );
        assert_eq!(LogSanitizer::sanitize_contract_address("short"), "short");
    }

    #[test]
    fn test_sanitize_generic() {
        assert_eq!(
            LogSanitizer::sanitize_generic("0x1234567890abcdef1234567890abcdef12345678"),
            "0x1234...5678"
        );
        assert_eq!(
            LogSanitizer::sanitize_generic(EXAMPLE_USER_CONFIG_PATH),
            "/.../config.json"
        );
        assert_eq!(
            LogSanitizer::sanitize_generic("very_long_sensitive_data_that_should_be_truncated"),
            "very_lon...ated"
        );
    }

    #[test]
    fn test_sanitized_log_wrapper() {
        let sanitized = SanitizedLog::as_cado_key("abc123def456");
        assert_eq!(sanitized.to_string(), "abc123def456");

        let sanitized = SanitizedLog::as_cado_key("abc123def456789");
        assert_eq!(sanitized.to_string(), "abc123de...6789");

        let sanitized = SanitizedLog::as_address("0x1234567890abcdef1234567890abcdef12345678");
        assert_eq!(sanitized.to_string(), "0x1234...5678");

        let sanitized = SanitizedLog::as_path(EXAMPLE_USER_CONFIG_PATH);
        assert_eq!(sanitized.to_string(), "/.../config.json");
    }
}
