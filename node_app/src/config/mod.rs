pub mod app_config;
pub mod loader;
pub mod node_runtime_config;

pub use app_config::AppConfig;
pub use node_runtime_config::NodeRuntimeConfig;

use eld_common::validation::{
    validate_address, validate_chain_id, validate_ip_or_hostname, validate_port,
    validate_positive_integer,
};
use hex;
use std::{collections::HashMap, path::PathBuf, str};

use eld_common::account::Account;
use eld_common::error::EldError;
use serde::{Deserialize, Serialize};
use serde_json;

// Use the shared FeeConfig from eld_common::fee
pub use eld_common::fee::FeeConfig;

// Default paths and environment variables for configuration files
const DEFAULT_CONSENSUS_CONFIG_PATH: &str = "./config/consensus_config.json";
const ENV_CONSENSUS_CONFIG_PATH: &str = "ELD_CONSENSUS_CONFIG_PATH";

const DEFAULT_DB_PATH: &str = "./data/rocksdb";
const ENV_DB_PATH: &str = "ELD_DB_PATH";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsensusConfig {
    pub chain_id: String,
    pub app_host: String,
    pub app_port: String,
    pub accounts: HashMap<String, Account>,
    /// Maximum transaction size in bytes (default: 10MB)
    #[serde(default = "default_max_tx_bytes")]
    pub max_tx_bytes: usize,
    /// Fee configuration for dynamic fee calculation
    #[serde(default)]
    pub fee_config: FeeConfig,
    /// Storage limits for preventing resource exhaustion
    #[serde(default)]
    pub storage_limits: StorageLimits,
}

fn default_max_tx_bytes() -> usize {
    10 * 1024 * 1024 // 10MB
}

impl ConsensusConfig {
    // Legacy methods for backward compatibility
    pub fn from_file(file: &str) -> Self {
        <Self as crate::config::loader::ConfigLoadable>::from_file(file)
            .expect("Failed to load consensus config")
    }

    pub fn from_file_result(file: &str) -> Result<Self, EldError> {
        <Self as crate::config::loader::ConfigLoadable>::from_file(file)
    }

    /// Resolve the consensus configuration file path with fallback priority:
    /// 1) CLI argument, 2) environment variable, 3) default path
    pub fn resolve_path(cli_path: Option<String>) -> String {
        cli_path
            .or_else(|| std::env::var(ENV_CONSENSUS_CONFIG_PATH).ok())
            .unwrap_or_else(|| DEFAULT_CONSENSUS_CONFIG_PATH.to_string())
    }

    /// Load consensus configuration for node startup:
    /// - validate file permissions
    /// - validate optional checksum
    /// - load securely
    /// - apply optional chain_id override
    /// - run semantic validation
    pub fn load_for_node(
        config_path: &str,
        override_chain_id: Option<String>,
    ) -> Result<Self, EldError> {
        // Validate file permissions and integrity before loading
        Self::validate_file_permissions(config_path)?;
        Self::validate_integrity(config_path)?;

        let mut cfg = Self::from_file_result(config_path)?;

        // Apply CLI override for chain_id if provided
        if let Some(id) = override_chain_id {
            cfg.chain_id = id;
        }

        // Run semantic validation
        cfg.validate()?;

        Ok(cfg)
    }

    /// Validate and secure file permissions for configuration file
    fn validate_file_permissions(config_path: &str) -> Result<(), EldError> {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let path = PathBuf::from(config_path);

        // Check if file exists
        if !path.exists() {
            return Err(EldError::FileSystemError {
                operation: "validate_config_file_permissions".to_string(),
                path: config_path.to_string(),
                details: format!("Configuration file does not exist: {config_path}"),
            });
        }

        // Get file metadata
        let metadata = fs::metadata(&path).map_err(|e| EldError::FileSystemError {
            operation: "get_file_metadata".to_string(),
            path: config_path.to_string(),
            details: format!("Failed to get file metadata: {e}"),
        })?;

        // Check file permissions (should be read-only for owner, no access for others)
        let permissions = metadata.permissions();
        let mode = permissions.mode();

        // Check if file is readable by owner and not writable by others
        if mode & 0o777 != 0o600 && mode & 0o777 != 0o644 {
            tracing::warn!(
                "Configuration file has insecure permissions: {:o}. Recommended: 600 (owner read/write only) or 644 (owner read/write, others read only)",
                mode & 0o777
            );
        }

        // Check if file is owned by current user (basic security check)
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let current_uid = unsafe { libc::getuid() };
            if metadata.uid() != current_uid {
                tracing::warn!("Configuration file is not owned by current user");
            }
        }

        Ok(())
    }

    /// Calculate SHA256 checksum of a file
    fn calculate_file_checksum(file_path: &str) -> Result<String, EldError> {
        use sha2::{Digest, Sha256};
        use std::fs::File;
        use std::io::Read;

        let mut file = File::open(file_path).map_err(|e| EldError::FileSystemError {
            operation: "open_file_for_checksum".to_string(),
            path: file_path.to_string(),
            details: format!("Failed to open file for checksum calculation: {e}"),
        })?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)
            .map_err(|e| EldError::FileSystemError {
                operation: "read_file_for_checksum".to_string(),
                path: file_path.to_string(),
                details: format!("Failed to read file for checksum calculation: {e}"),
            })?;

        let mut hasher = Sha256::new();
        hasher.update(&buffer);
        let result = hasher.finalize();

        Ok(hex::encode(result))
    }

    /// Validate configuration file integrity (optional checksum validation)
    fn validate_integrity(config_path: &str) -> Result<(), EldError> {
        use std::fs;

        // Check if there's a checksum file
        let checksum_path = format!("{config_path}.sha256");
        let checksum_file = PathBuf::from(&checksum_path);

        if checksum_file.exists() {
            let expected_checksum = fs::read_to_string(&checksum_path)
                .map_err(|e| EldError::FileSystemError {
                    operation: "read_checksum_file".to_string(),
                    path: checksum_path.clone(),
                    details: format!("Failed to read checksum file: {e}"),
                })?
                .trim()
                .to_string();
            let actual_checksum = Self::calculate_file_checksum(config_path)?;

            if expected_checksum != actual_checksum {
                return Err(EldError::ValidationError {
                    field: "config_file_integrity".to_string(),
                    value: actual_checksum.clone(),
                    details: format!(
                        "Configuration file integrity check failed. Expected: {expected_checksum}, Got: {actual_checksum}"
                    ),
                });
            }

            tracing::info!("Configuration file integrity validated successfully");
        } else {
            tracing::info!("No checksum file found, skipping integrity validation");
        }

        Ok(())
    }

    /// Save configuration to file with secure permissions
    pub fn save_to_file(&self, file: &str) -> Result<(), EldError> {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        // Serialize to JSON
        let json_data = serde_json::to_string_pretty(self).map_err(|e| EldError::ConfigError {
            file: file.to_string(),
            details: format!("Failed to serialize configuration: {e}"),
        })?;

        // Write to file
        fs::write(file, json_data).map_err(|e| EldError::FileSystemError {
            operation: "write_config_file".to_string(),
            path: file.to_string(),
            details: format!("Failed to write configuration file: {e}"),
        })?;

        // Set secure permissions (600: owner read/write only)
        #[cfg(unix)]
        {
            let mut permissions = fs::metadata(file)
                .map_err(|e| EldError::FileSystemError {
                    operation: "get_file_metadata".to_string(),
                    path: file.to_string(),
                    details: format!("Failed to get file metadata: {e}"),
                })?
                .permissions();
            permissions.set_mode(0o600);
            fs::set_permissions(file, permissions).map_err(|e| EldError::FileSystemError {
                operation: "set_file_permissions".to_string(),
                path: file.to_string(),
                details: format!("Failed to set secure permissions: {e}"),
            })?;
        }

        Ok(())
    }

    /// Generate checksum for configuration file
    pub fn generate_checksum(file: &str) -> Result<String, EldError> {
        use sha2::{Digest, Sha256};
        use std::fs::File;
        use std::io::Read;

        let mut file_handle = File::open(file).map_err(|e| EldError::FileSystemError {
            operation: "open_file_for_checksum".to_string(),
            path: file.to_string(),
            details: format!("Failed to open file for checksum calculation: {e}"),
        })?;
        let mut buffer = Vec::new();
        file_handle
            .read_to_end(&mut buffer)
            .map_err(|e| EldError::FileSystemError {
                operation: "read_file_for_checksum".to_string(),
                path: file.to_string(),
                details: format!("Failed to read file for checksum calculation: {e}"),
            })?;

        let mut hasher = Sha256::new();
        hasher.update(&buffer);
        let result = hasher.finalize();

        Ok(hex::encode(result))
    }

    /// Save checksum to file
    pub fn save_checksum(file: &str) -> Result<(), EldError> {
        let checksum = Self::generate_checksum(file)?;
        let checksum_path = format!("{file}.sha256");
        std::fs::write(&checksum_path, checksum).map_err(|e| EldError::FileSystemError {
            operation: "write_checksum_file".to_string(),
            path: checksum_path,
            details: format!("Failed to write checksum file: {e}"),
        })?;
        Ok(())
    }

    /// Validate all fields in the consensus config
    pub fn validate(&self) -> Result<(), EldError> {
        // Validate chain_id
        validate_chain_id(&self.chain_id)?;

        // Validate app_host
        validate_ip_or_hostname(&self.app_host)?;

        // Validate app_port
        validate_port(&self.app_port)?;

        // Validate max_tx_bytes
        validate_positive_integer(
            self.max_tx_bytes,
            Some(1024),              // Minimum 1KB
            Some(100 * 1024 * 1024), // Maximum 100MB
            "max_tx_bytes",
        )?;

        // Validate fee_config
        self.fee_config.validate()?;

        // Validate storage_limits
        self.storage_limits.validate()?;

        // Validate accounts
        let mut account_names_by_address: HashMap<String, Vec<String>> = HashMap::new();
        for (account_name, account) in &self.accounts {
            validate_address(&account.address().hex_with_prefix())?;
            account_names_by_address
                .entry(account.address().hex_with_prefix())
                .or_default()
                .push(account_name.clone());

            // Validate account balance (non-negative)
            let balance_u128: u128 = account.balance().amount();
            validate_positive_integer(
                balance_u128,
                Some(0u128),
                None,
                &format!("account {account_name} balance"),
            )?;

            // Validate account nonce (non-negative)
            validate_positive_integer(
                account.nonce().value(),
                Some(0u32),
                None,
                &format!("account {account_name} nonce"),
            )?;
        }

        let mut duplicate_account_entries: Vec<(String, Vec<String>)> = account_names_by_address
            .into_iter()
            .filter_map(|(address, mut names)| {
                if names.len() > 1 {
                    names.sort();
                    Some((address, names))
                } else {
                    None
                }
            })
            .collect();

        if !duplicate_account_entries.is_empty() {
            duplicate_account_entries.sort_by(|a, b| a.0.cmp(&b.0));
            let details = duplicate_account_entries
                .into_iter()
                .map(|(address, names)| format!("{address} => [{}]", names.join(", ")))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(EldError::ConfigError {
                file: "consensus_config.accounts".to_string(),
                details: format!("Duplicate account addresses found: {details}"),
            });
        }

        Ok(())
    }

    pub fn ip_address(&self) -> String {
        format!("{}:{}", self.app_host, self.app_port)
    }
}

// Implement ConfigValidator trait for ConsensusConfig
impl crate::config::loader::ConfigValidator for ConsensusConfig {
    fn validate(&self) -> Result<(), eld_common::error::EldError> {
        self.validate()
    }
}

// Implement ConfigLoadable trait for ConsensusConfig
impl crate::config::loader::ConfigLoadable for ConsensusConfig {}

/// Resolve the database path with fallback priority:
/// 1) CLI argument, 2) environment variable, 3) default path
pub fn resolve_db_path(cli_db_path: Option<String>) -> String {
    cli_db_path
        .or_else(|| std::env::var(ENV_DB_PATH).ok())
        .unwrap_or_else(|| DEFAULT_DB_PATH.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageLimits {
    /// Maximum number of results returned by prefix queries (default: 1000)
    #[serde(default = "default_max_prefix_results")]
    pub max_prefix_results: usize,
    /// Maximum number of results per page for paginated queries (default: 100)
    #[serde(default = "default_max_page_size")]
    pub max_page_size: usize,
    /// Rate limit for prefix queries per minute (default: 60)
    #[serde(default = "default_prefix_query_rate_limit")]
    pub prefix_query_rate_limit: u32,
    /// Maximum memory usage for prefix query results in MB (default: 50)
    #[serde(default = "default_max_query_memory_mb")]
    pub max_query_memory_mb: usize,
}

fn default_max_prefix_results() -> usize {
    1000
}

fn default_max_page_size() -> usize {
    100
}

fn default_prefix_query_rate_limit() -> u32 {
    60
}

fn default_max_query_memory_mb() -> usize {
    50
}

impl Default for StorageLimits {
    fn default() -> Self {
        Self {
            max_prefix_results: default_max_prefix_results(),
            max_page_size: default_max_page_size(),
            prefix_query_rate_limit: default_prefix_query_rate_limit(),
            max_query_memory_mb: default_max_query_memory_mb(),
        }
    }
}

impl StorageLimits {
    pub fn validate(&self) -> Result<(), eld_common::error::EldError> {
        use eld_common::validation::validate_positive_integer;

        // Validate max_prefix_results
        validate_positive_integer(
            self.max_prefix_results,
            Some(1),
            Some(100_000), // Maximum 100k results
            "max_prefix_results",
        )?;

        // Validate max_page_size
        validate_positive_integer(
            self.max_page_size,
            Some(1),
            Some(10_000), // Maximum 10k per page
            "max_page_size",
        )?;

        // Validate prefix_query_rate_limit
        validate_positive_integer(
            self.prefix_query_rate_limit as usize,
            Some(1),
            Some(10_000), // Maximum 10k queries per minute
            "prefix_query_rate_limit",
        )?;

        // Validate max_query_memory_mb
        validate_positive_integer(
            self.max_query_memory_mb,
            Some(1),
            Some(1000), // Maximum 1GB
            "max_query_memory_mb",
        )?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eld_common::constants::test::MOCK_CHAIN_ID;
    use std::fs;
    use tempfile::NamedTempFile;

    #[test]
    fn test_consensus_config_chain_id_validation() {
        // Test with valid chain_id
        let valid_config = format!(
            r#"{{
            "chain_id": "{MOCK_CHAIN_ID}",
            "app_host": "0.0.0.0",
            "app_port": "26658",
            "accounts": {{}},
            "validators": [],
            "max_tx_bytes": 10485760
        }}"#
        );

        let temp_file = NamedTempFile::new().expect("Unable to create temp file");
        fs::write(&temp_file, valid_config).expect("Unable to write to temp file");

        let config = ConsensusConfig::from_file(
            temp_file
                .path()
                .to_str()
                .expect("Unable to get temp file path"),
        );
        assert_eq!(config.chain_id, MOCK_CHAIN_ID);
        assert_eq!(config.max_tx_bytes, 10 * 1024 * 1024);
    }

    #[test]
    #[should_panic(expected = "Configuration validation failed")]
    fn test_consensus_config_empty_chain_id_panics() {
        let invalid_config = r#"{
            "chain_id": "",
            "app_host": "0.0.0.0",
            "app_port": "26658",
            "accounts": {},
            "validators": [],
            "max_tx_bytes": 10485760
        }"#;

        let temp_file = NamedTempFile::new().expect("Unable to create temp file");
        fs::write(&temp_file, invalid_config).expect("Unable to write to temp file");

        ConsensusConfig::from_file(
            temp_file
                .path()
                .to_str()
                .expect("Unable to get temp file path"),
        );
    }

    #[test]
    #[should_panic(expected = "Configuration validation failed")]
    fn test_consensus_config_whitespace_chain_id_panics() {
        let invalid_config = r#"{
            "chain_id": "   ",
            "app_host": "0.0.0.0",
            "app_port": "26658",
            "accounts": {},
            "validators": [],
            "max_tx_bytes": 10485760
        }"#;

        let temp_file = NamedTempFile::new().expect("Unable to create temp file");
        fs::write(&temp_file, invalid_config).expect("Unable to write to temp file");

        ConsensusConfig::from_file(
            temp_file
                .path()
                .to_str()
                .expect("Unable to get temp file path"),
        );
    }

    #[test]
    fn test_fee_config_validation() {
        let valid_fee_config = FeeConfig {
            base_fee: 1000u128,
            size_fee_per_kb: 100u128,
            gas_price: 1u128,
            transfer_multiplier: 1.0,
            stake_multiplier: 1.5,
            content_manifest_multiplier: 2.5,
            verified_proof_multiplier: 2.0,
            device_operation_multiplier: 1.8,
        };
        assert!(valid_fee_config.validate().is_ok());
    }

    #[test]
    fn test_fee_config_validation_invalid_multiplier() {
        let invalid_fee_config = FeeConfig {
            base_fee: 1000u128,
            size_fee_per_kb: 100u128,
            gas_price: 1u128,
            transfer_multiplier: -1.0, // Invalid negative multiplier
            stake_multiplier: 1.5,
            content_manifest_multiplier: 2.5,
            verified_proof_multiplier: 2.0,
            device_operation_multiplier: 1.8,
        };
        assert!(invalid_fee_config.validate().is_err());
    }

    #[test]
    fn test_consensus_config_validation_with_accounts() {
        let config_with_accounts = format!(
            r#"{{
            "chain_id": "{MOCK_CHAIN_ID}",
            "app_host": "0.0.0.0",
            "app_port": "26658",
            "accounts": {{
                "0xe17404c417fa10cc04fdf73604fcacca8d0a687c": {{
                    "address": "0xe17404c417fa10cc04fdf73604fcacca8d0a687c",
                    "balance": "100000000000",
                    "nonce": 0
                }}
            }},
            "validators": [
                {{
                    "address": "0x5de5bb98d243d7a7d74ae1d592cef1becb43957c",
                    "stake": 999,
                    "public_key": "dfa389799e01b644175c3fc2077359405fd4182513c95c28a89de3c283e3f7e9"
                }}
            ]
        }}"#
        );

        let temp_file = NamedTempFile::new().expect("Unable to create temp file");
        fs::write(&temp_file, config_with_accounts).expect("Unable to write to temp file");

        let config = ConsensusConfig::from_file(
            temp_file
                .path()
                .to_str()
                .expect("Unable to get temp file path"),
        );
        assert!(config.validate().is_ok());
    }

    #[test]
    #[should_panic(expected = "Configuration validation failed")]
    fn test_consensus_config_rejects_duplicate_account_addresses() {
        let config_with_duplicate_addresses = format!(
            r#"{{
            "chain_id": "{MOCK_CHAIN_ID}",
            "app_host": "0.0.0.0",
            "app_port": "26658",
            "accounts": {{
                "block-reward-pool": {{
                    "address": "0x5ffd39cc9d6ca5003d2414a784242f2fa9baaab3",
                    "balance": "0",
                    "nonce": 0
                }},
                "wallet-capacity-provider-1": {{
                    "address": "0x5ffd39cc9d6ca5003d2414a784242f2fa9baaab3",
                    "balance": "100000000000",
                    "nonce": 0
                }}
            }},
            "validators": []
        }}"#
        );

        let temp_file = NamedTempFile::new().expect("Unable to create temp file");
        fs::write(&temp_file, config_with_duplicate_addresses)
            .expect("Unable to write to temp file");

        ConsensusConfig::from_file(
            temp_file
                .path()
                .to_str()
                .expect("Unable to get temp file path"),
        );
    }
}
