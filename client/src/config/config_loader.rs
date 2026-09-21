//! # Configuration Loader Module
//!
//! This module provides a shared utility for loading and validating configuration files
//! with consistent error handling patterns across the Eld blockchain system.

use eld_common::error::EldError;
use serde::de::DeserializeOwned;
use std::fs;
use std::path::Path;

/// Configuration loading options
#[derive(Debug, Clone)]
pub struct ConfigLoadOptions {
    /// Whether to validate the configuration after loading
    pub validate: bool,
    /// Custom error context for better error messages
    pub error_context: Option<String>,
}

impl Default for ConfigLoadOptions {
    fn default() -> Self {
        Self {
            validate: true,
            error_context: None,
        }
    }
}

/// Shared configuration loader utility
pub struct ConfigLoader;

impl ConfigLoader {
    /// Load configuration from file with the given options
    pub fn load<T>(file_path: &str, options: ConfigLoadOptions) -> Result<T, EldError>
    where
        T: DeserializeOwned + ConfigValidator,
    {
        let context = options.error_context.as_deref().unwrap_or("configuration");

        // Read file content
        let data = match fs::read_to_string(file_path) {
            Ok(data) => data,
            Err(e) => {
                let error_msg = format!("Failed to read {context} file '{file_path}': {e}");
                return Err(EldError::ConfigError {
                    file: file_path.to_string(),
                    details: error_msg,
                });
            }
        };

        // Parse JSON
        let config: T = match serde_json::from_str(&data) {
            Ok(config) => config,
            Err(e) => {
                let error_msg = format!("Failed to parse {context} JSON from '{file_path}': {e}");
                return Err(EldError::ConfigError {
                    file: file_path.to_string(),
                    details: error_msg,
                });
            }
        };

        // Validate configuration if requested
        if options.validate {
            if let Err(e) = config.validate() {
                let error_msg = format!("{context} validation failed for '{file_path}': {e}");
                return Err(EldError::ConfigError {
                    file: file_path.to_string(),
                    details: error_msg,
                });
            }
        }

        Ok(config)
    }

    /// Load configuration from a file. Callers (including the CLI) handle the error.
    pub fn load_for_cli<T>(file_path: &str) -> Result<T, EldError>
    where
        T: DeserializeOwned + ConfigValidator,
    {
        let options = ConfigLoadOptions {
            validate: true,
            error_context: Some("CLI configuration".to_string()),
        };

        Self::load(file_path, options)
    }

    /// Load configuration with library-specific error handling (returns Result)
    pub fn load_for_library<T>(file_path: &str) -> Result<T, EldError>
    where
        T: DeserializeOwned + ConfigValidator,
    {
        let options = ConfigLoadOptions {
            validate: true,
            error_context: Some("library configuration".to_string()),
        };

        Self::load(file_path, options)
    }

    /// Load configuration for testing. Panics on error; available only in tests.
    #[cfg(test)]
    pub fn load_for_test<T>(file_path: &str) -> T
    where
        T: DeserializeOwned + ConfigValidator,
    {
        let options = ConfigLoadOptions {
            validate: true,
            error_context: Some("test configuration".to_string()),
        };

        Self::load(file_path, options).unwrap_or_else(|e| {
            panic!("Test configuration loading failed: {e}");
        })
    }

    /// Load configuration with secure file checks
    pub fn load_secure<T>(file_path: &str) -> Result<T, EldError>
    where
        T: DeserializeOwned + ConfigValidator,
    {
        // Check if file exists
        if !Path::new(file_path).exists() {
            return Err(EldError::ConfigError {
                file: file_path.to_string(),
                details: format!("Configuration file does not exist: {file_path}"),
            });
        }

        let options = ConfigLoadOptions {
            validate: true,
            error_context: Some("secure configuration".to_string()),
        };

        Self::load(file_path, options)
    }
}

/// Trait for configuration types that can be validated
pub trait ConfigValidator {
    /// Validate the configuration
    fn validate(&self) -> Result<(), EldError>;
}

/// Helper trait for configuration types that can be loaded from file
pub trait ConfigLoadable: DeserializeOwned + ConfigValidator {
    /// Load configuration from file
    fn from_file(file: &str) -> Result<Self, EldError> {
        ConfigLoader::load_for_cli(file)
    }

    /// Load configuration from file with library error handling
    fn from_file_result(file: &str) -> Result<Self, EldError> {
        ConfigLoader::load_for_library(file)
    }

    /// Load configuration from file for testing
    #[cfg(test)]
    fn from_file_for_test(file: &str) -> Self {
        ConfigLoader::load_for_test(file)
    }

    /// Load configuration from file with secure checks
    fn from_file_secure(file: &str) -> Result<Self, EldError> {
        ConfigLoader::load_secure(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use tempfile::NamedTempFile;

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestConfig {
        name: String,
        value: u32,
    }

    impl ConfigValidator for TestConfig {
        fn validate(&self) -> Result<(), EldError> {
            if self.name.is_empty() {
                return EldError::validation_error("name", "empty", "Name cannot be empty");
            }
            if self.value == 0 {
                return EldError::validation_error("value", "0", "Value cannot be zero");
            }
            Ok(())
        }
    }

    impl ConfigLoadable for TestConfig {}

    #[test]
    fn test_load_valid_config() {
        let temp_file = NamedTempFile::new().unwrap();
        let config_data = r#"{"name": "test", "value": 42}"#;
        fs::write(temp_file.path(), config_data).unwrap();

        let config: TestConfig =
            ConfigLoader::load_for_library(temp_file.path().to_str().unwrap()).unwrap();
        assert_eq!(config.name, "test");
        assert_eq!(config.value, 42);
    }

    #[test]
    fn test_load_invalid_json() {
        let temp_file = NamedTempFile::new().unwrap();
        let config_data = r#"{"name": "test", "value": "invalid"}"#;
        fs::write(temp_file.path(), config_data).unwrap();

        let result: Result<TestConfig, EldError> =
            ConfigLoader::load_for_library(temp_file.path().to_str().unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn test_load_invalid_config() {
        let temp_file = NamedTempFile::new().unwrap();
        let config_data = r#"{"name": "", "value": 0}"#;
        fs::write(temp_file.path(), config_data).unwrap();

        let result: Result<TestConfig, EldError> =
            ConfigLoader::load_for_library(temp_file.path().to_str().unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn test_load_nonexistent_file() {
        let result: Result<TestConfig, EldError> = ConfigLoader::load_secure("nonexistent.json");
        assert!(result.is_err());
    }

    #[test]
    fn test_load_for_cli_returns_err_when_file_missing() {
        let result: Result<TestConfig, EldError> = ConfigLoader::load_for_cli("nonexistent.json");
        assert!(result.is_err());
    }

    #[test]
    fn test_config_loadable_trait() {
        let temp_file = NamedTempFile::new().unwrap();
        let config_data = r#"{"name": "test", "value": 42}"#;
        fs::write(temp_file.path(), config_data).unwrap();

        let config = TestConfig::from_file_for_test(temp_file.path().to_str().unwrap());
        assert_eq!(config.name, "test");
        assert_eq!(config.value, 42);
    }
}
