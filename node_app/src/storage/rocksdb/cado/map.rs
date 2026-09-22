use bincode::{deserialize, serialize};
use eld_common::cado::{CADOMap, CadoPath};
use eld_common::error::EldError;
use rocksdb::{Transaction, TransactionDB};

use super::super::RocksDBStorage;

impl RocksDBStorage {
    pub fn put_cado_map(&self, path: CadoPath, mapping: CADOMap) -> Result<(), EldError> {
        // Validate path for security issues
        Self::validate_cado_path_security_enhanced(&path)?;

        let serialized = serialize(&mapping).map_err(|e| EldError::StorageError {
            operation: "serialize_mapping".to_string(),
            details: format!("Failed to serialize CADOMap: {e}"),
        })?;

        self.db
            .put_cf(self.cado_map_cf()?, path.as_str().as_bytes(), &serialized)
            .map_err(|e| EldError::StorageError {
                operation: "put_cf_mapping".to_string(),
                details: format!("Failed to store CADO mapping: {e}"),
            })?;
        Ok(())
    }

    pub fn get_cado_map(&self, path: CadoPath) -> Result<Option<CADOMap>, EldError> {
        // Validate path for security issues
        Self::validate_cado_path_security_enhanced(&path)?;

        if let Some(data) = self
            .db
            .get_cf(self.cado_map_cf()?, path.as_str().as_bytes())
            .map_err(|e| EldError::StorageError {
                operation: "get_cf_cado_map".to_string(),
                details: format!("Failed to get CADO mapping: {e}"),
            })?
        {
            let mapping = deserialize(&data).map_err(|e| EldError::StorageError {
                operation: "deserialize_cado_map".to_string(),
                details: format!("Failed to deserialize CADO mapping: {e}"),
            })?;
            Ok(Some(mapping))
        } else {
            Ok(None)
        }
    }

    pub fn put_cado_map_with_tx(
        &self,
        path: CadoPath,
        mapping: CADOMap,
        tx: &Transaction<'_, TransactionDB>,
    ) -> Result<(), EldError> {
        // Validate path for security issues
        Self::validate_cado_path_security_enhanced(&path)?;

        let value = bincode::serialize(&mapping).map_err(|e| EldError::StorageError {
            operation: "serialize_cado_map".to_string(),
            details: format!("Failed to serialize CADOMap: {e}"),
        })?;
        tx.put_cf(self.cado_map_cf()?, path.as_str().as_bytes(), value)
            .map_err(|e| EldError::StorageError {
                operation: "put_cf_cado_map".to_string(),
                details: format!("Failed to write CADO map to database: {e}"),
            })?;
        Ok(())
    }
}
