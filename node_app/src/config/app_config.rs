//! Combined on-disk `config.json` split into client and node runtime settings.

use eld_client::config::ClientConfig;
use serde::Deserialize;

use super::NodeRuntimeConfig;

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    #[serde(flatten)]
    pub client: ClientConfig,
    #[serde(flatten)]
    pub node: NodeRuntimeConfig,
}

impl AppConfig {
    pub fn from_file(file: &str) -> Result<Self, eld_common::error::EldError> {
        <Self as crate::config::loader::ConfigLoadable>::from_file(file)
    }
}

impl crate::config::loader::ConfigValidator for AppConfig {
    fn validate(&self) -> Result<(), eld_common::error::EldError> {
        self.client.validate()?;
        self.node.validate()
    }
}

impl crate::config::loader::ConfigLoadable for AppConfig {}
