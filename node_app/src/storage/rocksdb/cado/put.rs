use crate::storage::cado_key_generator::CADOKeyGenerator;
use eld_client::logging::SanitizedLog;
use eld_common::account::Account;
use eld_common::address::Address;
use eld_common::cado::{CADOMap, CADOMetadata, CadoBody, CadoPath};
use eld_common::coin::Coin;
use eld_common::error::EldError;
use eld_common::nonce::Nonce;
use eld_common::staking_account::StakingAccount;
use rocksdb::{Transaction, TransactionDB};
use sha2::{Digest, Sha256};
use tracing::{error, info};

use super::super::RocksDBStorage;

impl RocksDBStorage {
    pub fn put_cadotype_by_path_with_tx(
        &self,
        path: CadoPath,
        cado_type: CadoBody,
        tx: &Transaction<'_, TransactionDB>,
    ) -> Result<(), EldError> {
        // Validate path for security issues
        Self::validate_cado_path_security_enhanced(&path)?;

        // For mutable CADOs, we need to ensure the original key remains constant
        let original_key = if let Some(existing_value) = tx
            .get_cf(self.path_index_cf()?, path.as_str().as_bytes())
            .map_err(|e| EldError::StorageError {
                operation: "get_cf_path_index".to_string(),
                details: format!("Failed to read path index: {e}"),
            })? {
            // If we have an existing path index, use its original key
            String::from_utf8(existing_value).map_err(|e| EldError::StorageError {
                operation: "utf8_decode_existing_key".to_string(),
                details: format!("Invalid UTF-8 in existing key: {e}"),
            })?
        } else {
            // For new CADOs, generate keys
            match CADOKeyGenerator::generate_key(path.clone(), &cado_type) {
                Ok(key) => key,
                Err(e) => {
                    error!("{}", e.to_string());
                    return Err(EldError::StorageError {
                        operation: "key_generation".to_string(),
                        details: format!("Failed to generate CADO key: {e}"),
                    });
                }
            }
        };

        // Serialize the CADO type
        let value = cado_type.serialize_bin()?;

        // Write using the original key
        tx.put_cf(self.cado_cf()?, original_key.as_bytes(), &value)
            .map_err(|e| EldError::StorageError {
                operation: "put_cf_cado".to_string(),
                details: format!("Failed to write CADO to database: {e}"),
            })?;

        // Update path index
        tx.put_cf(
            self.path_index_cf()?,
            path.as_str().as_bytes(),
            original_key.as_bytes(),
        )
        .map_err(|e| EldError::StorageError {
            operation: "put_cf_path_index".to_string(),
            details: format!("Failed to update path index: {e}"),
        })?;

        Ok(())
    }
    pub fn put_cado_type(&self, path: CadoPath, cado_type: CadoBody) -> Result<(), EldError> {
        // Validate path for security issues
        Self::validate_cado_path_security_enhanced(&path)?;

        // For mutable CADOs, we need to ensure the original key remains constant
        let original_key = if let Some(existing_value) = self
            .db
            .get_cf(self.path_index_cf()?, path.as_str().as_bytes())
            .map_err(|e| EldError::StorageError {
                operation: "get_cf".to_string(),
                details: format!("Failed to get path index: {e}"),
            })? {
            // If we have an existing path index, use its original key
            String::from_utf8(existing_value).map_err(|e| EldError::StorageError {
                operation: "from_utf8".to_string(),
                details: format!("Failed to convert existing value to string: {e}"),
            })?
        } else {
            // For new CADOs, generate keys
            match CADOKeyGenerator::generate_key(path.clone(), &cado_type) {
                Ok(key) => key,
                Err(e) => {
                    error!("{}", e.to_string());
                    return Err(EldError::StorageError {
                        operation: "generate_key".to_string(),
                        details: format!("Failed to generate CADO key: {e}"),
                    });
                }
            }
        };

        // Serialize the CADO type
        let value = cado_type.serialize_bin()?;

        // Write using the original key
        info!("Writing CADO to cado_cf: key={}", original_key);
        self.db
            .put_cf(self.cado_cf()?, original_key.as_bytes(), &value)
            .map_err(|e| EldError::StorageError {
                operation: "put_cf".to_string(),
                details: format!("Failed to write CADO to database: {e}"),
            })?;

        // Update path index
        self.db
            .put_cf(
                self.path_index_cf()?,
                path.as_str().as_bytes(),
                original_key.as_bytes(),
            )
            .map_err(|e| EldError::StorageError {
                operation: "put_cf_path_index".to_string(),
                details: format!("Failed to update path index: {e}"),
            })?;

        Ok(())
    }

    pub fn put_cado_data(
        &self,
        path: CadoPath,
        cado_data: Vec<u8>,
        metadata: CADOMetadata,
    ) -> Result<(), EldError> {
        // Validate path for security issues
        Self::validate_cado_path_security(&path)?;

        let type_ = path.type_();
        let is_mutable = type_ == "account" || type_ == "staking_account";

        if is_mutable {
            // Validate address format for mutable CADOs
            if !path.name().starts_with("0x")
                || hex::decode(&path.name()[2..])
                    .map_err(|e| EldError::ValidationError {
                        field: "address".to_string(),
                        value: path.name().to_string(),
                        details: format!("Invalid hex format: {e}"),
                    })?
                    .len()
                    != 20
            {
                return Err(EldError::ValidationError {
                    field: "address".to_string(),
                    value: path.name().to_string(),
                    details: "Address must be exactly 20 bytes (40 hex characters after 0x)"
                        .to_string(),
                });
            }

            // Create original data based on type
            let original_data = if type_ == "account" {
                let address = Address::parse_hex_str(path.name())?;
                let original_account = Account::new(
                    address,
                    Coin::new(0).expect("Can init coin"),
                    Nonce::new(Nonce::ZERO),
                );
                original_account.serialize_bin()?
            } else {
                let address = Address::parse_hex_str(path.name())?;
                let original_staking_account = StakingAccount {
                    address,
                    stake_balance: Coin::zero(),
                    originator: address, // Same as address for initial creation
                };
                original_staking_account.serialize_bin()?
            };

            let original_hash: [u8; 32] = Sha256::digest(&original_data).into();
            let cado = CadoBody::mutable_updated(
                original_hash,
                cado_data.clone(),
                metadata.with_owner(path.name().to_string()),
            );

            let keys = CADOKeyGenerator::generate(path.clone(), &cado)?;

            let value = cado.serialize_bin()?;

            // Handle existing CADO cleanup
            if let Some(existing_value) = self
                .db
                .get_cf(self.cado_cf()?, keys.original_key.as_bytes())
                .map_err(|e| EldError::StorageError {
                    operation: "get_cf".to_string(),
                    details: format!("Failed to get existing CADO: {e}"),
                })?
            {
                let existing_cado: CadoBody = CadoBody::deserialize_bin(&existing_value)?;
                if let CadoBody::Mutable(existing_cadomut) = existing_cado {
                    let old_keys = CADOKeyGenerator::generate(
                        path.clone(),
                        &CadoBody::Mutable(existing_cadomut),
                    )
                    .map_err(|e| EldError::StorageError {
                        operation: "generate_old_keys".to_string(),
                        details: format!("Failed to generate old CADO keys: {e}"),
                    })?;
                    if let Some(old_latest_key) = old_keys.latest_key {
                        self.db
                            .delete_cf(self.cado_cf()?, old_latest_key.as_bytes())
                            .map_err(|e| EldError::StorageError {
                                operation: "delete_cf_cado".to_string(),
                                details: format!("Failed to delete old CADO: {e}"),
                            })?;
                        self.db
                            .delete_cf(self.cado_map_cf()?, old_latest_key.as_bytes())
                            .map_err(|e| EldError::StorageError {
                                operation: "delete_cf_map".to_string(),
                                details: format!("Failed to delete old CADO map: {e}"),
                            })?;
                    }
                }
            }

            // Write new CADO data
            info!(
                "Writing CADO to cado_cf: key={}",
                SanitizedLog::as_cado_key(&keys.original_key)
            );
            self.db
                .put_cf(self.cado_cf()?, keys.original_key.as_bytes(), &value)
                .map_err(|e| EldError::StorageError {
                    operation: "put_cf_cado".to_string(),
                    details: format!("Failed to write CADO to database: {e}"),
                })?;

            // Handle latest key if present
            if let Some(latest_key) = keys.latest_key {
                info!(
                    "Writing latest CADO to cado_cf: key={}",
                    SanitizedLog::as_cado_key(&latest_key)
                );
                self.db
                    .put_cf(self.cado_cf()?, latest_key.as_bytes(), &value)
                    .map_err(|e| EldError::StorageError {
                        operation: "put_cf_latest".to_string(),
                        details: format!("Failed to write latest CADO to database: {e}"),
                    })?;

                let cado_map = CADOMap {
                    from: latest_key.clone(),
                    to: keys.original_key.clone(),
                };
                let map_value =
                    bincode::serialize(&cado_map).map_err(|e| EldError::StorageError {
                        operation: "serialize_map".to_string(),
                        details: format!("Failed to serialize CADO map: {e}"),
                    })?;
                self.db
                    .put_cf(self.cado_map_cf()?, latest_key.as_bytes(), map_value)
                    .map_err(|e| EldError::StorageError {
                        operation: "put_cf_map".to_string(),
                        details: format!("Failed to write CADO map to database: {e}"),
                    })?;
            }

            // Update path index
            info!(
                "Storing path_index_cf: path={}, key={}",
                path.as_str(),
                keys.original_key
            );
            self.db
                .put_cf(
                    self.path_index_cf()?,
                    path.as_str().as_bytes(),
                    keys.original_key.as_bytes(),
                )
                .map_err(|e| EldError::StorageError {
                    operation: "put_cf_path_index".to_string(),
                    details: format!("Failed to update path index: {e}"),
                })?;
        } else {
            // Handle immutable CADOs
            let cado = CadoBody::immutable(cado_data, metadata);
            let keys = CADOKeyGenerator::generate(path.clone(), &cado)?;
            let value = cado.serialize_bin()?;

            info!(
                "Writing CADO to cado_cf: key={}",
                SanitizedLog::as_cado_key(&keys.original_key)
            );
            self.db
                .put_cf(self.cado_cf()?, keys.original_key.as_bytes(), &value)
                .map_err(|e| EldError::StorageError {
                    operation: "put_cf_cado".to_string(),
                    details: format!("Failed to write CADO to database: {e}"),
                })?;

            info!(
                "Storing path_index_cf: path={}, key={}",
                SanitizedLog::as_path(path.as_str()),
                SanitizedLog::as_cado_key(&keys.original_key)
            );
            self.db
                .put_cf(
                    self.path_index_cf()?,
                    path.as_str().as_bytes(),
                    keys.original_key.as_bytes(),
                )
                .map_err(|e| EldError::StorageError {
                    operation: "put_cf_path_index".to_string(),
                    details: format!("Failed to update path index: {e}"),
                })?;
        }
        Ok(())
    }

    pub fn put_cado_data_with_tx(
        &self,
        path: CadoPath,
        cado_data: Vec<u8>,
        metadata: CADOMetadata,
        tx: &Transaction<'_, TransactionDB>,
    ) -> Result<(), EldError> {
        // Validate path for security issues
        Self::validate_cado_path_security(&path)?;

        let type_ = path.type_();
        let is_mutable = type_ == "account" || type_ == "staking_account";

        if is_mutable {
            if !path.name().starts_with("0x")
                || hex::decode(&path.name()[2..])
                    .map_err(|e| EldError::ValidationError {
                        field: "address_format".to_string(),
                        value: path.name().to_string(),
                        details: format!("Invalid hex format: {e}"),
                    })?
                    .len()
                    != 20
            {
                return Err(EldError::ValidationError {
                    field: "address_format".to_string(),
                    value: path.name().to_string(),
                    details: "Invalid address format".to_string(),
                });
            }
            let original_data = if type_ == "account" {
                let address = Address::parse_hex_str(path.name())?;
                let original_account = Account::new(
                    address,
                    Coin::new(0).expect("Can init coin"),
                    Nonce::new(Nonce::ZERO),
                );
                original_account.serialize_bin()?
            } else {
                let address = Address::parse_hex_str(path.name())?;
                let original_staking_account = StakingAccount {
                    address,
                    stake_balance: Coin::zero(),
                    originator: address, // Same as address for initial creation
                };
                original_staking_account.serialize_bin()?
            };
            let original_hash: [u8; 32] = Sha256::digest(&original_data).into();
            let cado = CadoBody::mutable_updated(
                original_hash,
                cado_data.clone(),
                metadata.with_owner(path.name().to_string()),
            );

            let keys = CADOKeyGenerator::generate(path.clone(), &cado)?;
            let value = cado.serialize_bin()?;

            if let Some(existing_value) = tx
                .get_cf(self.cado_cf()?, keys.original_key.as_bytes())
                .map_err(|e| EldError::StorageError {
                operation: "get_cf_existing_cado".to_string(),
                details: format!("Failed to read existing CADO: {e}"),
            })? {
                let existing_cado: CadoBody = CadoBody::deserialize_bin(&existing_value)?;
                if let CadoBody::Mutable(existing_cadomut) = existing_cado {
                    let old_keys = CADOKeyGenerator::generate(
                        path.clone(),
                        &CadoBody::Mutable(existing_cadomut),
                    )?;
                    if let Some(old_latest_key) = old_keys.latest_key {
                        tx.delete_cf(self.cado_cf()?, old_latest_key.as_bytes())
                            .map_err(|e| EldError::StorageError {
                                operation: "delete_cf_old_latest".to_string(),
                                details: format!("Failed to delete old latest key: {e}"),
                            })?;
                        tx.delete_cf(self.cado_map_cf()?, old_latest_key.as_bytes())
                            .map_err(|e| EldError::StorageError {
                                operation: "delete_cf_old_map".to_string(),
                                details: format!("Failed to delete old map entry: {e}"),
                            })?;
                    }
                }
            }

            info!(
                "Writing CADO to cado_cf: key={}",
                SanitizedLog::as_cado_key(&keys.original_key)
            );
            tx.put_cf(self.cado_cf()?, keys.original_key.as_bytes(), &value)
                .map_err(|e| EldError::StorageError {
                    operation: "put_cf_cado".to_string(),
                    details: format!("Failed to write CADO to database: {e}"),
                })?;
            if let Some(latest_key) = keys.latest_key {
                info!(
                    "Writing latest CADO to cado_cf: key={}",
                    SanitizedLog::as_cado_key(&latest_key)
                );
                tx.put_cf(self.cado_cf()?, latest_key.as_bytes(), &value)
                    .map_err(|e| EldError::StorageError {
                        operation: "put_cf_latest_cado".to_string(),
                        details: format!("Failed to write latest CADO to database: {e}"),
                    })?;
                let cado_map = CADOMap {
                    from: latest_key.clone(),
                    to: keys.original_key.clone(),
                };
                let map_value =
                    bincode::serialize(&cado_map).map_err(|e| EldError::StorageError {
                        operation: "serialize_cado_map".to_string(),
                        details: format!("Failed to serialize CADOMap: {e}"),
                    })?;
                tx.put_cf(self.cado_map_cf()?, latest_key.as_bytes(), map_value)
                    .map_err(|e| EldError::StorageError {
                        operation: "put_cf_cado_map".to_string(),
                        details: format!("Failed to write CADO map to database: {e}"),
                    })?;
            }
            info!(
                "Storing path_index_cf: path={}, key={}",
                path.as_str(),
                keys.original_key
            );
            tx.put_cf(
                self.path_index_cf()?,
                path.as_str().as_bytes(),
                keys.original_key.as_bytes(),
            )
            .map_err(|e| EldError::StorageError {
                operation: "put_cf_path_index".to_string(),
                details: format!("Failed to update path index: {e}"),
            })?;
        } else {
            let cado = CadoBody::immutable(cado_data, metadata);
            let keys = CADOKeyGenerator::generate(path.clone(), &cado)?;
            let value = cado.serialize_bin()?;
            info!(
                "Writing CADO to cado_cf: key={}",
                SanitizedLog::as_cado_key(&keys.original_key)
            );
            tx.put_cf(self.cado_cf()?, keys.original_key.as_bytes(), &value)
                .map_err(|e| EldError::StorageError {
                    operation: "put_cf_cado".to_string(),
                    details: format!("Failed to write CADO to database: {e}"),
                })?;
            info!(
                "Storing path_index_cf: path={}, key={}",
                SanitizedLog::as_path(path.as_str()),
                SanitizedLog::as_cado_key(&keys.original_key)
            );
            tx.put_cf(
                self.path_index_cf()?,
                path.as_str().as_bytes(),
                keys.original_key.as_bytes(),
            )
            .map_err(|e| EldError::StorageError {
                operation: "put_cf_path_index".to_string(),
                details: format!("Failed to update path index: {e}"),
            })?;
        }
        Ok(())
    }
}
