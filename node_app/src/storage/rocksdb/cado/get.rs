use crate::storage::cado_key_generator::CADOKeyGenerator;
use eld_common::cado::{CadoBody, CadoPath};
use eld_common::constants::cado::TYPE_APP_STATE_SNAPSHOT;
use eld_common::error::EldError;
use eld_common::validation::{
    safe_deserialize_app_state_snapshot_cado_data, safe_deserialize_cado_data,
};
use tracing::error;

use super::super::RocksDBStorage;

impl RocksDBStorage {
    pub(super) fn deserialize_stored_cado_type(
        path: &CadoPath,
        data: &[u8],
    ) -> Result<CadoBody, EldError> {
        let context = format!("CadoBody at {}", path.as_str());
        let result = if path.type_() == TYPE_APP_STATE_SNAPSHOT {
            safe_deserialize_app_state_snapshot_cado_data(data, &context)
        } else {
            safe_deserialize_cado_data(data, &context)
        };
        result.map_err(|e| EldError::StorageError {
            operation: "deserialize_cado".to_string(),
            details: format!("Failed to deserialize CadoBody: {e}"),
        })
    }
    pub fn get_cado_by_path(&self, path: CadoPath) -> Result<Option<CadoBody>, EldError> {
        // Validate path for security issues
        Self::validate_cado_path_security(&path)?;

        // Retrieve Primary Storage-Key from path_index_cf
        let original_key_bytes = self
            .db
            .get_cf(self.path_index_cf()?, path.as_str().as_bytes())
            .map_err(|e| EldError::StorageError {
                operation: "get_cf_path_index".to_string(),
                details: format!("Failed to read path_index_cf: {e}"),
            })?;

        let original_key = match original_key_bytes {
            Some(key_bytes) => {
                String::from_utf8(key_bytes).map_err(|e| EldError::StorageError {
                    operation: "from_utf8_original_key".to_string(),
                    details: format!("Invalid UTF-8 in original_key: {e}"),
                })?
            }
            None => return Ok(None),
        };

        // Retrieve CadoBody from cado_cf
        let value = self
            .db
            .get_cf(self.cado_cf()?, original_key.as_bytes())
            .map_err(|e| EldError::StorageError {
                operation: "get_cf_cado".to_string(),
                details: format!("Failed to read cado_cf: {e}"),
            })?;

        match value {
            Some(data) => {
                let cado = Self::deserialize_stored_cado_type(&path, &data)?;
                // Validate key consistency
                let keys = CADOKeyGenerator::generate(path.clone(), &cado)?;
                if keys.original_key != original_key {
                    return Err(EldError::StorageError {
                        operation: "key_validation".to_string(),
                        details: "Stored key does not match generated key".to_string(),
                    });
                }
                Ok(Some(cado))
            }
            None => Ok(None),
        }
    }

    pub fn search_cado_path(&self, prefix: &str) -> Result<Vec<(String, CadoBody)>, EldError> {
        // Validate prefix for security issues
        Self::validate_cado_prefix_security(prefix)?;

        // Check rate limiting with default client ID
        if !self.rate_limiter.is_allowed("default", prefix) {
            return Err(EldError::StorageError {
                operation: "rate_limit_check".to_string(),
                details: format!("Rate limit exceeded for prefix query: {prefix}"),
            });
        }

        let mut results = Vec::new();
        let prefix_bytes = prefix.as_bytes();
        let debug_iter = self
            .db
            .iterator_cf(self.path_index_cf()?, rocksdb::IteratorMode::Start);
        let mut all_paths = Vec::new();
        for item in debug_iter {
            let (path_bytes, key_bytes) = item.map_err(|e| EldError::StorageError {
                operation: "iterator_read".to_string(),
                details: format!("Failed to read path_index_cf: {e}"),
            })?;
            let path =
                String::from_utf8(path_bytes.to_vec()).map_err(|e| EldError::StorageError {
                    operation: "utf8_decode_path".to_string(),
                    details: format!("Invalid UTF-8 in path: {e}"),
                })?;
            let key =
                String::from_utf8(key_bytes.to_vec()).map_err(|e| EldError::StorageError {
                    operation: "utf8_decode_key".to_string(),
                    details: format!("Invalid UTF-8 in key: {e}"),
                })?;
            all_paths.push((path, key));
        }

        let iter = self.db.iterator_cf(
            self.path_index_cf()?,
            rocksdb::IteratorMode::From(prefix_bytes, rocksdb::Direction::Forward),
        );
        for item in iter {
            let (path_bytes, key_bytes) = item.map_err(|e| EldError::StorageError {
                operation: "iterator_read".to_string(),
                details: format!("Failed to read path_index_cf: {e}"),
            })?;
            let path =
                String::from_utf8(path_bytes.to_vec()).map_err(|e| EldError::StorageError {
                    operation: "utf8_decode_path".to_string(),
                    details: format!("Invalid UTF-8 in path: {e}"),
                })?;
            if !path.starts_with(prefix) {
                break;
            }
            if let Some(value) =
                self.db
                    .get_cf(self.cado_cf()?, &key_bytes)
                    .map_err(|e| EldError::StorageError {
                        operation: "get_cf_cado".to_string(),
                        details: format!("Failed to read cado_cf: {e}"),
                    })?
            {
                if let Ok(cado) = safe_deserialize_cado_data::<CadoBody>(
                    &value,
                    &format!("CadoBody in search for path {path}"),
                ) {
                    results.push((path, cado));
                } else {
                    error!("Failed to deserialize CadoBody in search for path {}", path);
                }
            }
        }
        Ok(results)
    }

    pub fn search_cado_hash(&self, prefix: &str) -> Result<Vec<(String, CadoBody)>, EldError> {
        let results = self.search_cado_path(prefix)?;
        Ok(results
            .into_iter()
            .filter(|(path, _)| {
                path.split('/')
                    .next_back()
                    .map(|name| name.starts_with("0x"))
                    .unwrap_or(false)
            })
            .collect())
    }

    pub fn search_cado_name(&self, prefix: &str) -> Result<Vec<(String, CadoBody)>, EldError> {
        let results = self.search_cado_path(prefix)?;
        Ok(results
            .into_iter()
            .filter(|(path, _)| {
                path.split('/')
                    .next_back()
                    .map(|name| !name.starts_with("0x"))
                    .unwrap_or(false)
            })
            .collect())
    }
}
