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
        <Self as eld_client::config::config_loader::ConfigLoadable>::from_file(file)
    }
}

impl eld_client::config::config_loader::ConfigValidator for AppConfig {
    fn validate(&self) -> Result<(), eld_common::error::EldError> {
        self.client.validate()
    }
}

impl eld_client::config::config_loader::ConfigLoadable for AppConfig {}
