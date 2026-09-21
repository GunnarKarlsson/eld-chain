//! Node process runtime settings: P2P, capacity storage, and indexer.

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
    pub fn from_file(file: &str) -> Result<Self, eld_common::error::EldError> {
        <Self as crate::config::loader::ConfigLoadable>::from_file(file)
    }
}

impl crate::config::loader::ConfigValidator for NodeRuntimeConfig {
    fn validate(&self) -> Result<(), eld_common::error::EldError> {
        Ok(())
    }
}

impl crate::config::loader::ConfigLoadable for NodeRuntimeConfig {}
