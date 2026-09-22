//! Node process runtime settings: P2P, capacity storage, and indexer.

use eld_common::error::EldError;
use eld_common::validation::validate_port;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Debug, Clone, Serialize)]
pub struct NodeRuntimeConfig {
    #[serde(default)]
    pub p2p_tcp_port: Option<String>,
    #[serde(default)]
    pub p2p_udp_port: Option<String>,
    #[serde(default)]
    pub single_node: Option<bool>,
    pub capacity_size_mb: Option<u64>,
    pub capacity_storage_path: Option<String>,
    #[serde(default)]
    pub indexer: bool,
}

impl NodeRuntimeConfig {
    pub fn from_file(file: &str) -> Result<Self, EldError> {
        <Self as crate::config::loader::ConfigLoadable>::from_file(file)
    }

    pub fn validate(&self) -> Result<(), EldError> {
        if let Some(port) = &self.p2p_tcp_port {
            validate_port(port)?;
        }
        if let Some(port) = &self.p2p_udp_port {
            validate_port(port)?;
        }

        match self.capacity_size_mb {
            Some(mb) if mb > 0 => {}
            Some(mb) => {
                EldError::validation_error(
                    "capacity_size_mb",
                    &mb.to_string(),
                    "must be greater than 0",
                )?;
            }
            None => {
                EldError::validation_error("capacity_size_mb", "", "must be set in config.json")?;
            }
        }

        match self.capacity_storage_path.as_deref().map(str::trim) {
            Some(path) if !path.is_empty() => {}
            Some(_) => {
                EldError::validation_error("capacity_storage_path", "", "cannot be empty")?;
            }
            None => {
                EldError::validation_error(
                    "capacity_storage_path",
                    "",
                    "must be set in config.json",
                )?;
            }
        }

        Ok(())
    }
}

impl crate::config::loader::ConfigValidator for NodeRuntimeConfig {
    fn validate(&self) -> Result<(), EldError> {
        self.validate()
    }
}

impl crate::config::loader::ConfigLoadable for NodeRuntimeConfig {}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_config() -> NodeRuntimeConfig {
        NodeRuntimeConfig {
            p2p_tcp_port: Some("4001".to_string()),
            p2p_udp_port: Some("4002".to_string()),
            single_node: Some(false),
            capacity_size_mb: Some(50),
            capacity_storage_path: Some("./data/capacity".to_string()),
            indexer: true,
        }
    }

    #[test]
    fn validate_accepts_sample_runtime_fields() {
        assert!(valid_config().validate().is_ok());
    }

    #[test]
    fn validate_rejects_invalid_p2p_port() {
        let mut cfg = valid_config();
        cfg.p2p_tcp_port = Some("0".to_string());
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_missing_capacity() {
        let mut cfg = valid_config();
        cfg.capacity_size_mb = None;
        assert!(cfg.validate().is_err());
    }
}
