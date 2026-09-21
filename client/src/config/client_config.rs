//! Node/CLI endpoint configuration: [`CliConfig`], [`ConsensusConfig`], and file loading helpers.

use crate::endpoint::{resolve_app_base_url, resolve_faucet_request_url, resolve_node_base_url};
use serde::{Deserialize, Serialize};

pub const DEFAULT_CONFIG_PATH: &str = "config/config.json";
pub const CONSENSUS_CONFIG_PATH: &str = "config/consensus_config.json";
pub const HEX_PREFIX: &str = "0x";
pub const BROADCAST_TX_COMMIT_WITH_BODY_URL_PATH: &str = "broadcast_tx_commit";
pub const WALLETS_PATH: &str = "wallets/wallets.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsensusConfig {
    pub chain_id: String,
    #[serde(default)]
    pub fee_config: eld_common::fee::FeeConfig,
}

pub use eld_common::fee::FeeConfig;

impl ConsensusConfig {
    pub fn from_file(file: &str) -> Result<Self, eld_common::error::EldError> {
        <Self as crate::config::config_loader::ConfigLoadable>::from_file(file)
    }

    #[cfg(test)]
    pub fn from_file_for_test(file: &str) -> Self {
        <Self as crate::config::config_loader::ConfigLoadable>::from_file_for_test(file)
    }

    pub fn validate(&self) -> Result<(), eld_common::error::EldError> {
        if self.chain_id.trim().is_empty() {
            return eld_common::error::EldError::validation_error(
                "chain_id",
                "empty",
                "Chain ID cannot be empty in consensus config",
            );
        }
        self.fee_config.validate()?;
        Ok(())
    }
}

impl crate::config::config_loader::ConfigValidator for ConsensusConfig {
    fn validate(&self) -> Result<(), eld_common::error::EldError> {
        self.validate()
    }
}

impl crate::config::config_loader::ConfigLoadable for ConsensusConfig {}

#[derive(Deserialize, Debug, Clone)]
pub struct CliConfig {
    pub node_host: String,
    pub node_port: String,
    pub faucet_host: String,
    pub faucet_port: String,
    pub faucet_end_point: String,
    /// Full faucet base URL (e.g. `https://faucet.eld.network`). When set, overrides
    /// `faucet_host`/`faucet_port` for scheme and host; `faucet_end_point` is appended
    /// unless `faucet_url` already includes a path.
    #[serde(default)]
    pub faucet_url: Option<String>,
    #[serde(rename = "app_port", alias = "upload_port")]
    pub app_port: String,
    /// Full RPC base URL (e.g. `https://node-rpc.eld.network`). When set, overrides `node_host`/`node_port`.
    #[serde(default)]
    pub node_url: Option<String>,
    /// Full app REST API base URL (e.g. `https://node-api.eld.network`). When set, overrides app port derivation.
    #[serde(default, rename = "app_url", alias = "upload_url")]
    pub app_url: Option<String>,
    #[serde(default)]
    pub chain_id: String,
    #[serde(default)]
    pub p2p_tcp_port: Option<String>,
    #[serde(default)]
    pub p2p_udp_port: Option<String>,
    #[serde(default)]
    pub single_node: Option<bool>,
    #[serde(default)]
    pub capacity_size_mb: Option<u64>,
    #[serde(default)]
    pub capacity_storage_path: Option<String>,
    #[serde(default)]
    pub indexer: bool,
}

impl CliConfig {
    pub fn from_file(file: &str) -> Result<Self, eld_common::error::EldError> {
        <Self as crate::config::config_loader::ConfigLoadable>::from_file(file)
    }

    #[cfg(test)]
    pub fn from_file_for_test(file: &str) -> Self {
        <Self as crate::config::config_loader::ConfigLoadable>::from_file_for_test(file)
    }

    pub fn validate(&self) -> Result<(), eld_common::error::EldError> {
        resolve_node_base_url(self.node_url.as_deref(), &self.node_host, &self.node_port)?;
        resolve_app_base_url(
            self.app_url.as_deref(),
            self.node_url.as_deref(),
            &self.node_host,
            &self.node_port,
            &self.app_port,
        )?;
        resolve_faucet_request_url(
            self.faucet_url.as_deref(),
            &self.faucet_host,
            &self.faucet_port,
            &self.faucet_end_point,
        )?;
        Ok(())
    }

    pub fn get_node_url(&self) -> Result<String, eld_common::error::EldError> {
        resolve_node_base_url(self.node_url.as_deref(), &self.node_host, &self.node_port)
    }

    pub fn get_app_base_url(&self) -> Result<String, eld_common::error::EldError> {
        resolve_app_base_url(
            self.app_url.as_deref(),
            self.node_url.as_deref(),
            &self.node_host,
            &self.node_port,
            &self.app_port,
        )
    }

    pub fn get_faucet_request_url(&self) -> Result<String, eld_common::error::EldError> {
        resolve_faucet_request_url(
            self.faucet_url.as_deref(),
            &self.faucet_host,
            &self.faucet_port,
            &self.faucet_end_point,
        )
    }
}

impl crate::config::config_loader::ConfigValidator for CliConfig {
    fn validate(&self) -> Result<(), eld_common::error::EldError> {
        self.validate()
    }
}

impl crate::config::config_loader::ConfigLoadable for CliConfig {}

/// CLI / node endpoint config plus fee settings loaded from a consensus JSON file.
#[derive(Debug, Clone)]
pub struct ClientSetup {
    pub config: CliConfig,
    pub fee_config: FeeConfig,
}

fn client_setup_from_paths(
    cli_config_path: &str,
    consensus_config_path: &str,
) -> Result<ClientSetup, eld_common::error::EldError> {
    let mut config = CliConfig::from_file(cli_config_path)?;
    let consensus_config = ConsensusConfig::from_file(consensus_config_path)?;
    config.chain_id = consensus_config.chain_id;
    if config.chain_id.is_empty() {
        return Err(eld_common::error::EldError::make_validation_error(
            "chain_id",
            "empty",
            "Chain ID cannot be empty",
        ));
    }
    Ok(ClientSetup {
        config,
        fee_config: consensus_config.fee_config,
    })
}

/// Load `config/config.json` and `config/consensus_config.json` from the process CWD.
pub fn get_client_setup() -> Result<ClientSetup, eld_common::error::EldError> {
    get_client_setup_from_arg(DEFAULT_CONFIG_PATH)
}

/// Load a CLI config file and the default consensus config from the process CWD.
pub fn get_client_setup_from_arg(
    cli_config_path: &str,
) -> Result<ClientSetup, eld_common::error::EldError> {
    client_setup_from_paths(cli_config_path, CONSENSUS_CONFIG_PATH)
}

/// Load CLI and consensus config from explicit paths (for binaries that resolve paths themselves).
pub fn load_client_setup(
    cli_config_path: &str,
    consensus_config_path: &str,
) -> Result<ClientSetup, eld_common::error::EldError> {
    client_setup_from_paths(cli_config_path, consensus_config_path)
}

pub fn get_config() -> Result<CliConfig, eld_common::error::EldError> {
    Ok(get_client_setup()?.config)
}

pub fn get_config_from_arg(config_path: &str) -> Result<CliConfig, eld_common::error::EldError> {
    Ok(get_client_setup_from_arg(config_path)?.config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use eld_common::constants::test::MOCK_CHAIN_ID;
    use std::fs;
    use tempfile::NamedTempFile;

    #[test]
    fn test_consensus_config_validation() {
        let valid_config = format!(r#"{{"chain_id": "{MOCK_CHAIN_ID}"}}"#);
        let consensus_config: ConsensusConfig = serde_json::from_str(&valid_config).unwrap();
        assert_eq!(consensus_config.chain_id, MOCK_CHAIN_ID);
    }

    #[test]
    #[should_panic(expected = "Configuration validation failed")]
    fn test_consensus_config_empty_chain_id_panics() {
        let invalid_config = r#"{
            "chain_id": ""
        }"#;
        let temp_file = NamedTempFile::new().unwrap();
        fs::write(&temp_file, invalid_config).unwrap();
        ConsensusConfig::from_file_for_test(temp_file.path().to_str().unwrap());
    }

    #[test]
    fn test_cli_config_validation_valid() {
        let config = CliConfig {
            node_host: "127.0.0.1".to_string(),
            node_port: "26657".to_string(),
            chain_id: "".to_string(),
            faucet_host: "127.0.0.1".to_string(),
            faucet_port: "8080".to_string(),
            faucet_end_point: "/faucet/request".to_string(),
            faucet_url: None,
            app_port: "9001".to_string(),
            node_url: None,
            app_url: None,
            p2p_tcp_port: None,
            p2p_udp_port: None,
            single_node: Some(true),
            capacity_size_mb: None,
            capacity_storage_path: None,
            indexer: false,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_cli_config_validation_with_hostname() {
        let config = CliConfig {
            node_host: "localhost".to_string(),
            node_port: "26657".to_string(),
            chain_id: "".to_string(),
            faucet_host: "127.0.0.1".to_string(),
            faucet_port: "8080".to_string(),
            faucet_end_point: "/faucet/request".to_string(),
            faucet_url: None,
            app_port: "9001".to_string(),
            node_url: None,
            app_url: None,
            p2p_tcp_port: None,
            p2p_udp_port: None,
            single_node: Some(true),
            capacity_size_mb: None,
            capacity_storage_path: None,
            indexer: false,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    #[should_panic(expected = "Configuration validation failed")]
    fn test_cli_config_validation_invalid_host() {
        let invalid_config = r#"{
            "node_host": "127.0.0.1; rm -rf /",
            "node_port": "26657",
            "faucet_host": "127.0.0.1",
            "faucet_port": "8080",
            "faucet_end_point": "/faucet/request",
            "app_port":"9001"
        }"#;
        let temp_file = NamedTempFile::new().unwrap();
        fs::write(&temp_file, invalid_config).unwrap();
        CliConfig::from_file_for_test(temp_file.path().to_str().unwrap());
    }

    #[test]
    #[should_panic(expected = "Configuration validation failed")]
    fn test_cli_config_validation_empty_host() {
        let invalid_config = r#"{
            "node_host": "",
            "node_port": "26657",
            "faucet_host": "127.0.0.1",
            "faucet_port": "8080",
            "faucet_end_point": "/faucet/request",
            "app_port":"9001"
        }"#;
        let temp_file = NamedTempFile::new().unwrap();
        fs::write(&temp_file, invalid_config).unwrap();
        CliConfig::from_file_for_test(temp_file.path().to_str().unwrap());
    }

    #[test]
    #[should_panic(expected = "Configuration validation failed")]
    fn test_cli_config_validation_invalid_port() {
        let invalid_config = r#"{
            "node_host": "127.0.0.1",
            "node_port": "0",
            "faucet_host": "127.0.0.1",
            "faucet_port": "8080",
            "faucet_end_point": "/faucet/request",
            "app_port":"9001"
        }"#;
        let temp_file = NamedTempFile::new().unwrap();
        fs::write(&temp_file, invalid_config).unwrap();
        CliConfig::from_file_for_test(temp_file.path().to_str().unwrap());
    }

    #[test]
    #[should_panic(expected = "Configuration validation failed")]
    fn test_cli_config_validation_port_not_integer() {
        let invalid_config = r#"{
            "node_host": "127.0.0.1",
            "node_port": "not-a-port",
            "faucet_host": "127.0.0.1",
            "faucet_port": "8080",
            "faucet_end_point": "/faucet/request",
            "app_port": "9001"
        }"#;
        let temp_file = NamedTempFile::new().unwrap();
        fs::write(&temp_file, invalid_config).unwrap();
        CliConfig::from_file_for_test(temp_file.path().to_str().unwrap());
    }

    #[test]
    fn test_cli_config_validate_method() {
        let mut config = CliConfig {
            node_host: "127.0.0.1".to_string(),
            node_port: "26657".to_string(),
            chain_id: "".to_string(),
            faucet_host: "127.0.0.1".to_string(),
            faucet_port: "8080".to_string(),
            faucet_end_point: "/faucet/request".to_string(),
            faucet_url: None,
            app_port: "9001".to_string(),
            node_url: None,
            app_url: None,
            p2p_tcp_port: None,
            p2p_udp_port: None,
            single_node: Some(true),
            capacity_size_mb: None,
            capacity_storage_path: None,
            indexer: false,
        };
        assert!(config.validate().is_ok());
        config.node_url = Some("https://node-rpc.eld.network".to_string());
        config.app_url = Some("https://node-api.eld.network".to_string());
        assert!(config.validate().is_ok());
        assert_eq!(
            config.get_node_url().unwrap(),
            "https://node-rpc.eld.network/"
        );
        assert_eq!(
            config.get_app_base_url().unwrap(),
            "https://node-api.eld.network/"
        );
        config.node_url = None;
        config.app_url = None;
        config.node_host = "".to_string();
        assert!(config.validate().is_err());
        assert!(config.get_node_url().is_err());
        config.node_host = "127.0.0.1".to_string();
        config.node_port = "0".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_cli_config_get_node_url() {
        let config = CliConfig {
            node_host: "127.0.0.1".to_string(),
            node_port: "26657".to_string(),
            chain_id: "".to_string(),
            faucet_host: "127.0.0.1".to_string(),
            faucet_port: "8080".to_string(),
            faucet_end_point: "/faucet/request".to_string(),
            faucet_url: None,
            app_port: "9001".to_string(),
            node_url: None,
            app_url: None,
            p2p_tcp_port: None,
            p2p_udp_port: None,
            single_node: Some(true),
            capacity_size_mb: None,
            capacity_storage_path: None,
            indexer: false,
        };
        let url = config.get_node_url().unwrap();
        assert_eq!(url, "http://127.0.0.1:26657/");
        assert_eq!(config.get_app_base_url().unwrap(), "http://127.0.0.1:9001/");
        assert_eq!(
            config.get_faucet_request_url().unwrap(),
            "http://127.0.0.1:8080/faucet/request"
        );
    }

    #[test]
    fn test_cli_config_get_faucet_request_url_https() {
        let config = CliConfig {
            node_host: "127.0.0.1".to_string(),
            node_port: "26657".to_string(),
            chain_id: "".to_string(),
            faucet_host: "127.0.0.1".to_string(),
            faucet_port: "8080".to_string(),
            faucet_end_point: "/request".to_string(),
            faucet_url: Some("https://faucet.eld.network".to_string()),
            app_port: "9001".to_string(),
            node_url: None,
            app_url: None,
            p2p_tcp_port: None,
            p2p_udp_port: None,
            single_node: Some(true),
            capacity_size_mb: None,
            capacity_storage_path: None,
            indexer: false,
        };
        assert!(config.validate().is_ok());
        assert_eq!(
            config.get_faucet_request_url().unwrap(),
            "https://faucet.eld.network/request"
        );
    }

    #[test]
    fn test_cli_config_from_file_with_urls() {
        let config_json = r#"{
            "node_host": "",
            "node_port": "0",
            "faucet_host": "127.0.0.1",
            "faucet_port": "8080",
            "faucet_end_point": "/faucet/request",
            "app_port": "9001",
            "node_url": "https://node-rpc.eld.network",
            "app_url": "https://node-api.eld.network"
        }"#;
        let temp_file = NamedTempFile::new().unwrap();
        fs::write(&temp_file, config_json).unwrap();
        let config = CliConfig::from_file_for_test(temp_file.path().to_str().unwrap());
        assert_eq!(
            config.get_node_url().unwrap(),
            "https://node-rpc.eld.network/"
        );
        assert_eq!(
            config.get_app_base_url().unwrap(),
            "https://node-api.eld.network/"
        );
    }

    #[test]
    fn test_consensus_config_validation_with_fee_config() {
        let valid_config = format!(
            r#"{{
            "chain_id": "{MOCK_CHAIN_ID}",
            "fee_config": {{
                "base_fee": 1000,
                "size_fee_per_kb": 100,
                "gas_price": 1,
                "transfer_multiplier": 1.0,
                "stake_multiplier": 1.5,
                "content_manifest_multiplier": 2.5,
                "verified_proof_multiplier": 2.0,
                "device_operation_multiplier": 1.8
            }}
        }}"#
        );
        let temp_file = NamedTempFile::new().unwrap();
        fs::write(&temp_file, valid_config).unwrap();
        let config = ConsensusConfig::from_file(temp_file.path().to_str().unwrap()).unwrap();
        assert_eq!(config.chain_id, MOCK_CHAIN_ID);
        assert_eq!(config.fee_config.base_fee, 1000);
        assert_eq!(config.fee_config.transfer_multiplier, 1.0);
    }

    #[test]
    #[should_panic(expected = "Configuration validation failed")]
    fn test_consensus_config_validation_invalid_fee_config() {
        let invalid_config = format!(
            r#"{{
            "chain_id": "{MOCK_CHAIN_ID}",
            "fee_config": {{
                "base_fee": 1000,
                "size_fee_per_kb": 100,
                "gas_price": 1,
                "transfer_multiplier": -1.0,
                "stake_multiplier": 1.5,
                "content_manifest_multiplier": 2.5,
                "verified_proof_multiplier": 2.0,
                "device_operation_multiplier": 1.8
            }}
        }}"#
        );
        let temp_file = NamedTempFile::new().unwrap();
        fs::write(&temp_file, invalid_config).unwrap();
        ConsensusConfig::from_file_for_test(temp_file.path().to_str().unwrap());
    }

    #[test]
    #[should_panic(expected = "Configuration validation failed")]
    fn test_consensus_config_validation_zero_base_fee() {
        let invalid_config = format!(
            r#"{{
            "chain_id": "{MOCK_CHAIN_ID}",
            "fee_config": {{
                "base_fee": 0,
                "size_fee_per_kb": 100,
                "gas_price": 1,
                "transfer_multiplier": 1.0,
                "stake_multiplier": 1.5,
                "content_manifest_multiplier": 2.5,
                "verified_proof_multiplier": 2.0,
                "device_operation_multiplier": 1.8
            }}
        }}"#
        );
        let temp_file = NamedTempFile::new().unwrap();
        fs::write(&temp_file, invalid_config).unwrap();
        ConsensusConfig::from_file_for_test(temp_file.path().to_str().unwrap());
    }

    #[test]
    fn test_cli_config_from_file_returns_err_when_missing() {
        let result = CliConfig::from_file("/nonexistent/eld-chain-config.json");
        assert!(result.is_err());
    }

    #[test]
    fn test_get_config_from_arg_returns_err_when_missing() {
        let result = get_config_from_arg("/nonexistent/eld-chain-config.json");
        assert!(result.is_err());
    }

    #[test]
    fn test_fee_config_validate_method() {
        let valid_fee_config = FeeConfig {
            base_fee: 1000,
            size_fee_per_kb: 100,
            gas_price: 1,
            transfer_multiplier: 1.0,
            stake_multiplier: 1.5,
            content_manifest_multiplier: 2.5,
            verified_proof_multiplier: 2.0,
            device_operation_multiplier: 1.8,
        };
        assert!(valid_fee_config.validate().is_ok());
        let mut invalid_config = valid_fee_config.clone();
        invalid_config.base_fee = 0;
        assert!(invalid_config.validate().is_err());
        invalid_config = valid_fee_config.clone();
        invalid_config.transfer_multiplier = -1.0;
        assert!(invalid_config.validate().is_err());
    }
}
