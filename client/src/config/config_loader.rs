//! Shared utility for loading and validating JSON configuration files.

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

/// Shared configuration loader utility.
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

        let config: TestConfig = ConfigLoader::from_file(
            temp_file.path().to_str().unwrap(),
            ConfigLoadOptions::default(),
        )
        .unwrap();
        assert_eq!(config.name, "test");
        assert_eq!(config.value, 42);
    }

    #[test]
    fn test_load_invalid_json() {
        let temp_file = NamedTempFile::new().unwrap();
        let config_data = r#"{"name": "test", "value": "invalid"}"#;
        fs::write(temp_file.path(), config_data).unwrap();

        let result: Result<TestConfig, EldError> = ConfigLoader::from_file(
            temp_file.path().to_str().unwrap(),
            ConfigLoadOptions::default(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_load_invalid_config() {
        let temp_file = NamedTempFile::new().unwrap();
        let config_data = r#"{"name": "", "value": 0}"#;
        fs::write(temp_file.path(), config_data).unwrap();

        let result: Result<TestConfig, EldError> = ConfigLoader::from_file(
            temp_file.path().to_str().unwrap(),
            ConfigLoadOptions::default(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_load_nonexistent_file() {
        let result: Result<TestConfig, EldError> =
            ConfigLoader::from_file("nonexistent.json", ConfigLoadOptions::default());
        assert!(result.is_err());
    }

    #[test]
    fn test_load_skips_validation_when_disabled() {
        let temp_file = NamedTempFile::new().unwrap();
        let config_data = r#"{"name": "", "value": 0}"#;
        fs::write(temp_file.path(), config_data).unwrap();

        let config: TestConfig = ConfigLoader::from_file(
            temp_file.path().to_str().unwrap(),
            ConfigLoadOptions { validate: false },
        )
        .unwrap();
        assert_eq!(config.value, 0);
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
