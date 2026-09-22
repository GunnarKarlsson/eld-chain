use crate::storage::cado_key_generator::CADOKeyGenerator;
use eld_common::cado::{CADOKeys, CadoBody, CadoPath, CadoPathKey, CadoType};
use eld_common::error::EldError;
use rocksdb::{Transaction, TransactionDB};
use tracing::{error, info, warn};

use super::super::RocksDBStorage;

impl RocksDBStorage {
    pub fn delete_cado_with_tx(
        &self,
        path: CadoPath,
        owner: &str,
        signature: &str,
        public_key: &str,
        chain_id: &str,
        tx: &Transaction<'_, TransactionDB>,
    ) -> Result<(), EldError> {
        // Validate path for security issues
        Self::validate_cado_path_security_enhanced(&path)?;

        // Validate that this CADO type is allowed for user deletion
        path.validate_user_deletion()?;

        // Log the deletion attempt for audit trail
        info!(
            "CADO deletion attempt (with tx) - path: {}, owner: {}, public_key: {}",
            path.as_str(),
            owner,
            public_key
        );

        let cado_type = self.get_cado_by_path(path.clone())?;
        let cado = match cado_type {
            Some(cado) => cado,
            None => {
                error!(
                    "CADO deletion failed (with tx) - CADO not found: {}",
                    path.as_str()
                );
                return Err(EldError::StorageError {
                    operation: "get_cado_by_path".to_string(),
                    details: format!("CADO not found: {}", path.as_str()),
                });
            }
        };
        let metadata = match &cado {
            CadoBody::Immutable(c) => c.metadata(),
            CadoBody::Mutable(c) => c.metadata(),
        };
        if metadata.owner() != owner {
            error!(
                "CADO deletion failed (with tx) - unauthorized: caller {} is not the owner {}",
                owner,
                metadata.owner()
            );
            return Err(EldError::StorageError {
                operation: "ownership_verification".to_string(),
                details: format!(
                    "Unauthorized: caller {} is not the owner {}",
                    owner,
                    metadata.owner()
                ),
            });
        }

        // Verify cryptographic signature
        if let Err(e) = eld_common::validation::verify_cado_deletion_signature(
            path.as_str(),
            owner,
            signature,
            public_key,
            chain_id,
        ) {
            error!(
                "CADO deletion failed (with tx) - signature verification failed: {}",
                e
            );
            return Err(EldError::StorageError {
                operation: "signature_verification".to_string(),
                details: format!("Signature verification failed: {e}"),
            });
        }

        let keys = CADOKeyGenerator::generate(path.clone(), &cado)?;
        match tx.delete_cf(self.cado_cf()?, keys.original_key.as_bytes()) {
            Ok(_) => (),
            Err(e) => warn!("no original key deleted: {}", e.to_string()),
        };
        if let Some(latest_key) = keys.latest_key {
            match tx.delete_cf(self.cado_cf()?, latest_key.as_bytes()) {
                Ok(_) => (),
                Err(e) => warn!("no latest key deleted: {}", e.to_string()),
            };
        }
        // Create a proper CADO map path instead of appending to the existing path
        let map_path = CadoPath::new(CadoType::CadoMap, CadoPathKey::Name(path.name()))?;
        match tx.delete_cf(self.cado_map_cf()?, map_path.as_str().as_bytes()) {
            Ok(_) => (),
            Err(e) => warn!("no cado_map deleted: {}", e.to_string()),
        };
        match tx.delete_cf(self.path_index_cf()?, path.as_str().as_bytes()) {
            Ok(_) => (),
            Err(e) => warn!("no path_index deleted: {}", e.to_string()),
        };

        info!(
            "CADO deletion successful (with tx) - path: {}, owner: {}",
            path.as_str(),
            owner
        );

        Ok(())
    }

    pub(super) fn prepare_system_cado_deletion(
        &self,
        path: CadoPath,
        owner: &str,
    ) -> Result<(CadoPath, CADOKeys), EldError> {
        Self::validate_cado_path_security_enhanced(&path)?;
        path.validate_system_deletion()?;

        let cado = match self.get_cado_by_path(path.clone())? {
            Some(cado) => cado,
            None => {
                error!(
                    "System CADO deletion failed - CADO not found: {}",
                    path.as_str()
                );
                return Err(EldError::StorageError {
                    operation: "get_cado_by_path".to_string(),
                    details: format!("CADO not found: {}", path.as_str()),
                });
            }
        };

        let metadata = match &cado {
            CadoBody::Immutable(c) => c.metadata(),
            CadoBody::Mutable(c) => c.metadata(),
        };

        if metadata.owner() != owner {
            error!(
                "System CADO deletion failed - unauthorized: caller {} is not the owner {}",
                owner,
                metadata.owner()
            );
            return Err(EldError::StorageError {
                operation: "ownership_verification".to_string(),
                details: format!(
                    "Unauthorized: caller {} is not the owner {}",
                    owner,
                    metadata.owner()
                ),
            });
        }

        let keys = CADOKeyGenerator::generate(path.clone(), &cado)?;
        Ok((path, keys))
    }

    pub(super) fn delete_cado_associated_keys_db(
        &self,
        path: &CadoPath,
        keys: &CADOKeys,
    ) -> Result<(), EldError> {
        let cado_cf = self.cado_cf()?;
        self.db
            .delete_cf(cado_cf, keys.original_key.as_bytes())
            .map_err(|e| EldError::StorageError {
                operation: "delete_cado_original_key".to_string(),
                details: format!("Failed to delete original key for {}: {e}", path.as_str()),
            })?;

        if let Some(latest_key) = &keys.latest_key {
            self.db
                .delete_cf(cado_cf, latest_key.as_bytes())
                .map_err(|e| EldError::StorageError {
                    operation: "delete_cado_latest_key".to_string(),
                    details: format!("Failed to delete latest key for {}: {e}", path.as_str()),
                })?;
        }

        let map_path = CadoPath::new(CadoType::CadoMap, CadoPathKey::Name(path.name()))?;
        self.db
            .delete_cf(self.cado_map_cf()?, map_path.as_str().as_bytes())
            .map_err(|e| EldError::StorageError {
                operation: "delete_cado_map".to_string(),
                details: format!("Failed to delete cado_map for {}: {e}", path.as_str()),
            })?;

        self.delete_path_index(path.as_str())
    }

    pub(super) fn delete_cado_associated_keys_with_tx(
        &self,
        path: &CadoPath,
        keys: &CADOKeys,
        tx: &Transaction<'_, TransactionDB>,
    ) -> Result<(), EldError> {
        let cado_cf = self.cado_cf()?;
        tx.delete_cf(cado_cf, keys.original_key.as_bytes())
            .map_err(|e| EldError::StorageError {
                operation: "delete_cado_original_key_with_tx".to_string(),
                details: format!("Failed to delete original key for {}: {e}", path.as_str()),
            })?;

        if let Some(latest_key) = &keys.latest_key {
            tx.delete_cf(cado_cf, latest_key.as_bytes())
                .map_err(|e| EldError::StorageError {
                    operation: "delete_cado_latest_key_with_tx".to_string(),
                    details: format!("Failed to delete latest key for {}: {e}", path.as_str()),
                })?;
        }

        let map_path = CadoPath::new(CadoType::CadoMap, CadoPathKey::Name(path.name()))?;
        tx.delete_cf(self.cado_map_cf()?, map_path.as_str().as_bytes())
            .map_err(|e| EldError::StorageError {
                operation: "delete_cado_map_with_tx".to_string(),
                details: format!("Failed to delete cado_map for {}: {e}", path.as_str()),
            })?;

        tx.delete_cf(self.path_index_cf()?, path.as_str().as_bytes())
            .map_err(|e| EldError::StorageError {
                operation: "delete_path_index_with_tx".to_string(),
                details: format!("Failed to delete path_index for {}: {e}", path.as_str()),
            })?;

        Ok(())
    }

    /// Internal system function for deleting CADOs without signature verification
    /// This should only be used for system-level operations, not user-initiated deletions
    pub fn system_delete_cado(&self, path: CadoPath, owner: &str) -> Result<(), EldError> {
        info!(
            "System CADO deletion - path: {}, owner: {}",
            path.as_str(),
            owner
        );

        let (path, keys) = self.prepare_system_cado_deletion(path, owner)?;
        self.delete_cado_associated_keys_db(&path, &keys)?;

        info!(
            "System CADO deletion successful - path: {}, owner: {}",
            path.as_str(),
            owner
        );

        Ok(())
    }

    /// Transactional variant of [`Self::system_delete_cado`] for consensus `commit()`.
    pub fn system_delete_cado_with_tx(
        &self,
        path: CadoPath,
        owner: &str,
        tx: &Transaction<'_, TransactionDB>,
    ) -> Result<(), EldError> {
        info!(
            "System CADO deletion (with tx) - path: {}, owner: {}",
            path.as_str(),
            owner
        );

        let (path, keys) = self.prepare_system_cado_deletion(path, owner)?;
        self.delete_cado_associated_keys_with_tx(&path, &keys, tx)?;

        info!(
            "System CADO deletion successful (with tx) - path: {}, owner: {}",
            path.as_str(),
            owner
        );

        Ok(())
    }
    pub fn delete_cado(
        &self,
        path: CadoPath,
        owner: &str,
        signature: &str,
        public_key: &str,
        chain_id: &str,
    ) -> Result<(), EldError> {
        // Validate path for security issues
        Self::validate_cado_path_security_enhanced(&path)?;

        // Validate that this CADO type is allowed for user deletion
        path.validate_user_deletion()?;

        // Log the deletion attempt for audit trail
        info!(
            "CADO deletion attempt - path: {}, owner: {}, public_key: {}",
            path.as_str(),
            owner,
            public_key
        );

        // Get the CADO object to verify ownership
        let cado_type = self.get_cado_by_path(path.clone())?;
        let cado = match cado_type {
            Some(cado) => cado,
            None => {
                error!("CADO deletion failed - CADO not found: {}", path.as_str());
                return Err(EldError::StorageError {
                    operation: "get_cado_by_path".to_string(),
                    details: format!("CADO not found: {}", path.as_str()),
                });
            }
        };

        // Verify ownership
        let metadata = match &cado {
            CadoBody::Immutable(c) => c.metadata(),
            CadoBody::Mutable(c) => c.metadata(),
        };

        if metadata.owner() != owner {
            error!(
                "CADO deletion failed - unauthorized: caller {} is not the owner {}",
                owner,
                metadata.owner()
            );
            return Err(EldError::StorageError {
                operation: "ownership_verification".to_string(),
                details: format!(
                    "Unauthorized: caller {} is not the owner {}",
                    owner,
                    metadata.owner()
                ),
            });
        }

        // Verify cryptographic signature
        if let Err(e) = eld_common::validation::verify_cado_deletion_signature(
            path.as_str(),
            owner,
            signature,
            public_key,
            chain_id,
        ) {
            error!(
                "CADO deletion failed - signature verification failed: {}",
                e
            );
            return Err(EldError::StorageError {
                operation: "signature_verification".to_string(),
                details: format!("Signature verification failed: {e}"),
            });
        }

        // Get the keys for deletion
        let keys = CADOKeyGenerator::generate(path.clone(), &cado)?;

        // Delete from cado_cf using original key
        match self
            .db
            .delete_cf(self.cado_cf()?, keys.original_key.as_bytes())
        {
            Ok(_) => info!("original key deleted"),
            Err(e) => warn!("no original key deleted: {}", e.to_string()),
        };

        // If it's a mutable CADO, also delete the latest key and CADO map
        if let Some(latest_key) = keys.latest_key {
            match self.db.delete_cf(self.cado_cf()?, latest_key.as_bytes()) {
                Ok(_e) => info!("latest key deleted"),
                Err(e) => warn!("no latest key deleted: {}", e.to_string()),
            };
        }

        // Delete CADO map entry
        let map_path = CadoPath::new(CadoType::CadoMap, CadoPathKey::Name(path.name()))?;
        match self
            .db
            .delete_cf(self.cado_map_cf()?, map_path.as_str().as_bytes())
        {
            Ok(_) => info!("map deleted"),
            Err(e) => warn!("no map deleted: {}", e.to_string()),
        };

        // Delete path index
        self.delete_path_index(path.as_str())?;

        info!(
            "CADO deletion successful - path: {}, owner: {}",
            path.as_str(),
            owner
        );

        Ok(())
    }
    // Optional: Delete path mapping (for renaming or cleanup)
    pub fn delete_path_index(&self, path: &str) -> Result<(), EldError> {
        self.db
            .delete_cf(self.path_index_cf()?, path.as_bytes())
            .map_err(|e| EldError::StorageError {
                operation: "delete_path_index".to_string(),
                details: format!("Failed to delete path index: {e}"),
            })?;
        Ok(())
    }
}
