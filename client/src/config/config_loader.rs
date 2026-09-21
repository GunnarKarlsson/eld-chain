//! Internal helper for loading client JSON config files (`ClientConfig`, etc.).

use eld_common::error::EldError;
use serde::de::DeserializeOwned;
use std::fs;

/// Configuration types loaded by the client crate must implement this trait.
pub(crate) trait ConfigValidator {
    fn validate(&self) -> Result<(), EldError>;
}

/// Load and validate a client config JSON file from disk.
pub(crate) fn load_config_file<T>(file_path: &str) -> Result<T, EldError>
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

    config.validate().map_err(|e| EldError::ConfigError {
        file: file_path.to_string(),
        details: format!("Configuration validation failed for '{file_path}': {e}"),
    })?;

    Ok(config)
}

/// Load a client config file for tests. Panics on error.
#[cfg(test)]
pub(crate) fn load_config_file_for_test<T>(file_path: &str) -> T
where
    T: DeserializeOwned + ConfigValidator,
{
    load_config_file(file_path).unwrap_or_else(|e| panic!("Test configuration loading failed: {e}"))
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

    #[test]
    fn test_load_valid_config() {
        let temp_file = NamedTempFile::new().unwrap();
        let config_data = r#"{"name": "test", "value": 42}"#;
        fs::write(temp_file.path(), config_data).unwrap();

        let config: TestConfig = load_config_file(temp_file.path().to_str().unwrap()).unwrap();
        assert_eq!(config.name, "test");
        assert_eq!(config.value, 42);
    }

    #[test]
    fn test_load_invalid_json() {
        let temp_file = NamedTempFile::new().unwrap();
        let config_data = r#"{"name": "test", "value": "invalid"}"#;
        fs::write(temp_file.path(), config_data).unwrap();

        let result: Result<TestConfig, EldError> =
            load_config_file(temp_file.path().to_str().unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn test_load_invalid_config() {
        let temp_file = NamedTempFile::new().unwrap();
        let config_data = r#"{"name": "", "value": 0}"#;
        fs::write(temp_file.path(), config_data).unwrap();

        let result: Result<TestConfig, EldError> =
            load_config_file(temp_file.path().to_str().unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn test_load_nonexistent_file() {
        let result: Result<TestConfig, EldError> = load_config_file("nonexistent.json");
        assert!(result.is_err());
    }
}
