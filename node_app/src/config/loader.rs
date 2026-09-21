//! Node-local JSON config loading and validation (not part of `eld-client` public API).

use eld_common::error::EldError;
use serde::de::DeserializeOwned;
use std::fs;

/// Options for [`ConfigLoader::from_file`].
#[derive(Debug, Clone)]
pub struct ConfigLoadOptions {
    /// Whether to run [`ConfigValidator::validate`] after parsing JSON.
    pub validate: bool,
}

impl Default for ConfigLoadOptions {
    fn default() -> Self {
        Self { validate: true }
    }
}

/// JSON configuration loader for node config types.
pub struct ConfigLoader;

impl ConfigLoader {
    /// Load and optionally validate configuration from a JSON file.
    pub fn from_file<T>(file_path: &str, options: ConfigLoadOptions) -> Result<T, EldError>
    where
        T: DeserializeOwned + ConfigValidator,
    {
        let data = fs::read_to_string(file_path).map_err(|e| EldError::ConfigError {
            file: file_path.to_string(),
            details: format!("Failed to read configuration file '{file_path}': {e}"),
        })?;

        let config: T = serde_json::from_str(&data).map_err(|e| EldError::ConfigError {
            file: file_path.to_string(),
            details: format!("Failed to parse configuration JSON from '{file_path}': {e}"),
        })?;

        if options.validate {
            config.validate().map_err(|e| EldError::ConfigError {
                file: file_path.to_string(),
                details: format!("Configuration validation failed for '{file_path}': {e}"),
            })?;
        }

        Ok(config)
    }

    /// Load configuration for testing. Panics on error; available only in tests.
    #[cfg(test)]
    pub fn from_file_for_test<T>(file_path: &str) -> T
    where
        T: DeserializeOwned + ConfigValidator,
    {
        Self::from_file(file_path, ConfigLoadOptions::default())
            .unwrap_or_else(|e| panic!("Test configuration loading failed: {e}"))
    }
}

/// Trait for configuration types that can be validated after load.
pub trait ConfigValidator {
    fn validate(&self) -> Result<(), EldError>;
}

/// Helper trait for configuration types that can be loaded from file.
pub trait ConfigLoadable: DeserializeOwned + ConfigValidator {
    /// Load configuration from file with validation enabled.
    fn from_file(file: &str) -> Result<Self, EldError> {
        ConfigLoader::from_file(file, ConfigLoadOptions::default())
    }

    /// Load configuration from file for testing.
    #[cfg(test)]
    fn from_file_for_test(file: &str) -> Self {
        ConfigLoader::from_file_for_test(file)
    }
}
