use eld_common::cado::{CADOKeys, CadoBody, CadoPath};
use eld_common::error::EldError;
use sha2::{Digest, Sha256};
use tracing::info;
/*
CadoPath: /@scope/type_/name
- scope: Starts with '@', non-empty (e.g., @eld).
- type_: Valid types: account, staking_account, content_manifest, cado_map.
- name:
  - account, staking_account: 20-byte Ed25519-derived address (hex-encoded, e.g., 0x<40 chars>).
  - Others: 32-byte Sha256(data) or Sha256(original_data) (hex-encoded, e.g., 0x<64 chars>) if 0x-prefixed, else arbitrary.

Key Derivation:
- Primary Storage-Key: Sha256(scope|type_|hash), stored in cado_cf.
- hash:
  - CADO: Sha256(data) (excluding metadata).
  - CADOMut (general): Sha256(original_data) (initial state, excluding metadata).
  - CADOMut (account, staking_account): Sha256(path.name()) (address).
- Secondary Storage-Key (CADOMut only): Sha256(scope|type_|latest_hash), stored in cado_cf.
- latest_hash: Sha256(current_data) (excluding metadata).
- CADOMap (CADOMut only): Maps Secondary Storage-Key to Primary Storage-Key in cado_map_cf.
- path_index_cf: Maps path to Primary Storage-Key for all types.

CADO (Immutable):
- hash: Sha256(data).
- name: Matches hash if 0x-prefixed (0x<hex(hash)>).
- Storage: Primary Storage-Key only, no CADOMap.
- Query: Clients compute Sha256(data) to construct /@scope/type_/0x<hex(hash)>.

CADOMut (All Types):
- hash: Sha256(original_data) (e.g., {address, balance: 0, nonce: 0} for accounts).
- name:
  - account, staking_account: Address (matches metadata.owner).
  - Others: 0x<hex(hash)> or arbitrary.
- Storage: Primary and Secondary Storage-Keys, CADOMap in cado_map_cf.
*/
pub trait KeyGenerator {
    fn generate_original_key(&self, path: &CadoPath, cado: &CadoBody) -> Result<String, EldError>;
    fn generate_latest_key(
        &self,
        path: &CadoPath,
        cado: &CadoBody,
    ) -> Result<Option<String>, EldError>;
}

pub struct CADOKeyGenerator;

impl CADOKeyGenerator {
    pub fn generate_key(path: CadoPath, cado: &CadoBody) -> Result<String, EldError> {
        let generator: Box<dyn KeyGenerator> = match path.type_() {
            "account" | "staking_account" | "storage_staking_account" => {
                Box::new(AccountKeyGenerator)
            }
            "app_state_tip" | "app_state_snapshot" | "epoch_record" => {
                Box::new(AppStateTipKeyGenerator)
            }
            "namespace" => Box::new(NamespaceRegistryKeyGenerator),
            "cado_map" | "snapshot_metadata" | "snapshot_chunk" | "chunk_reference" => {
                Box::new(GeneralKeyGenerator)
            }
            _ => {
                return Err(EldError::ValidationError {
                    field: "cado_type".to_string(),
                    value: path.type_().to_string(),
                    details: format!("Unsupported CADO type: {}", path.type_()),
                })
            }
        };

        generator.generate_original_key(&path, cado)
    }

    pub fn generate(path: CadoPath, cado: &CadoBody) -> Result<CADOKeys, EldError> {
        info!("path: {}", path);
        let generator: Box<dyn KeyGenerator> = match path.type_() {
            "account" | "staking_account" | "storage_staking_account" => {
                Box::new(AccountKeyGenerator)
            }
            "app_state_tip" | "app_state_snapshot" | "epoch_record" => {
                Box::new(AppStateTipKeyGenerator)
            }
            "namespace" => Box::new(NamespaceRegistryKeyGenerator),
            "cado_map" | "snapshot_metadata" | "snapshot_chunk" | "chunk_reference" => {
                Box::new(GeneralKeyGenerator)
            }
            _ => {
                return Err(EldError::ValidationError {
                    field: "cado_type".to_string(),
                    value: path.type_().to_string(),
                    details: format!("Unsupported CADO type: {}", path.type_()),
                })
            }
        };

        let original_key = generator.generate_original_key(&path, cado)?;
        let latest_key = generator.generate_latest_key(&path, cado)?;
        Ok(CADOKeys {
            original_key,
            latest_key,
        })
    }
}

struct AccountKeyGenerator;

impl KeyGenerator for AccountKeyGenerator {
    fn generate_original_key(&self, path: &CadoPath, cado: &CadoBody) -> Result<String, EldError> {
        if !path.name().starts_with("0x") {
            return Err(EldError::ValidationError {
                field: "account_address".to_string(),
                value: path.name().to_string(),
                details: "Account address must start with '0x'".to_string(),
            });
        }
        let name_bytes = hex::decode(&path.name()[2..]).map_err(|e| EldError::ValidationError {
            field: "account_address".to_string(),
            value: path.name().to_string(),
            details: format!("Invalid hex in address: {e}"),
        })?;
        if name_bytes.len() != 20 {
            return Err(EldError::ValidationError {
                field: "account_address".to_string(),
                value: path.name().to_string(),
                details: "Account address must be 20 bytes".to_string(),
            });
        }

        let metadata = match cado {
            CadoBody::Mutable(cadomut) => cadomut.metadata(),
            _ => {
                return Err(EldError::ValidationError {
                    field: "cado_type".to_string(),
                    value: "immutable".to_string(),
                    details: "Expected mutable CADO for account type".to_string(),
                })
            }
        };
        if metadata.owner() != path.name() {
            return Err(EldError::ValidationError {
                field: "metadata_owner".to_string(),
                value: metadata.owner().to_string(),
                details: "Metadata owner does not match path name".to_string(),
            });
        }

        // Primary Storage-Key: Sha256(scope|type_|Sha256(address))
        let address_hash: [u8; 32] = Sha256::digest(path.name().as_bytes()).into();
        let key_input = format!(
            "{}|{}|{}",
            path.scope(),
            path.type_(),
            hex::encode(address_hash)
        );
        let mut hasher = Sha256::new();
        hasher.update(&key_input);
        Ok(format!("0x{}", hex::encode(hasher.finalize())))
    }

    fn generate_latest_key(
        &self,
        path: &CadoPath,
        cado: &CadoBody,
    ) -> Result<Option<String>, EldError> {
        let latest_hash = match cado {
            CadoBody::Mutable(cadomut) => cadomut.latest_hash(),
            _ => {
                return Err(EldError::ValidationError {
                    field: "cado_type".to_string(),
                    value: "immutable".to_string(),
                    details: "Expected mutable CADO for account type".to_string(),
                })
            }
        };
        // Secondary Storage-Key: Sha256(scope|type_|latest_hash)
        let key_input = format!(
            "{}|{}|{}",
            path.scope(),
            path.type_(),
            hex::encode(latest_hash)
        );
        let mut hasher = Sha256::new();
        hasher.update(&key_input);
        Ok(Some(format!("0x{}", hex::encode(hasher.finalize()))))
    }
}

struct GeneralKeyGenerator;

impl KeyGenerator for GeneralKeyGenerator {
    fn generate_original_key(&self, path: &CadoPath, cado: &CadoBody) -> Result<String, EldError> {
        let hash = match cado {
            CadoBody::Immutable(cado) => cado.hash_bytes(),
            CadoBody::Mutable(cadomut) => cadomut.hash_bytes(),
        };
        if path.name().starts_with("0x") {
            let name_bytes =
                hex::decode(&path.name()[2..]).map_err(|e| EldError::ValidationError {
                    field: "path_name".to_string(),
                    value: path.name().to_string(),
                    details: format!("Invalid hex in name: {e}"),
                })?;
            if name_bytes.len() != 32 {
                return Err(EldError::ValidationError {
                    field: "path_name".to_string(),
                    value: path.name().to_string(),
                    details: "Hash must be 32 bytes".to_string(),
                });
            }
            // Mutable names are the original hash or the current data hash.
            // Updates keep the original hash and change latest_hash, so either
            // match is valid. Immutable names here are identifiers, not payload hashes.
            if let CadoBody::Mutable(cadomut) = cado {
                let matches_original = name_bytes.as_slice() == cadomut.hash_bytes();
                let matches_latest = name_bytes.as_slice() == cadomut.latest_hash();
                if !matches_original && !matches_latest {
                    return Err(EldError::ValidationError {
                        field: "path_name".to_string(),
                        value: path.name().to_string(),
                        details: "Hash does not match path name".to_string(),
                    });
                }
            }
        }
        // Primary Storage-Key: Sha256(scope|type_|hash)
        let key_input = format!("{}|{}|{}", path.scope(), path.type_(), hex::encode(hash));
        let mut hasher = Sha256::new();
        hasher.update(&key_input);
        Ok(format!("0x{}", hex::encode(hasher.finalize())))
    }

    fn generate_latest_key(
        &self,
        path: &CadoPath,
        cado: &CadoBody,
    ) -> Result<Option<String>, EldError> {
        match cado {
            CadoBody::Mutable(cadomut) => {
                let key_input = format!(
                    "{}|{}|{}",
                    path.scope(),
                    path.type_(),
                    hex::encode(cadomut.latest_hash())
                );
                let mut hasher = Sha256::new();
                hasher.update(&key_input);
                Ok(Some(format!("0x{}", hex::encode(hasher.finalize()))))
            }
            CadoBody::Immutable(_) => Ok(None),
        }
    }
}

struct NamespaceRegistryKeyGenerator;

impl KeyGenerator for NamespaceRegistryKeyGenerator {
    fn generate_original_key(&self, path: &CadoPath, _cado: &CadoBody) -> Result<String, EldError> {
        let key_input = format!("{}|{}|{}", path.scope(), path.type_(), path.name());
        let mut hasher = Sha256::new();
        hasher.update(&key_input);
        Ok(format!("0x{}", hex::encode(hasher.finalize())))
    }

    fn generate_latest_key(
        &self,
        _path: &CadoPath,
        _cado: &CadoBody,
    ) -> Result<Option<String>, EldError> {
        Ok(None)
    }
}

struct AppStateTipKeyGenerator;

impl KeyGenerator for AppStateTipKeyGenerator {
    fn generate_original_key(&self, path: &CadoPath, _cado: &CadoBody) -> Result<String, EldError> {
        // For app state tips, we just use the path name directly as the key
        // This allows us to use a fixed path (LATEST) without hash validation
        let key_input = format!("{}|{}|{}", path.scope(), path.type_(), path.name());
        let mut hasher = Sha256::new();
        hasher.update(&key_input);
        Ok(format!("0x{}", hex::encode(hasher.finalize())))
    }

    fn generate_latest_key(
        &self,
        _path: &CadoPath,
        _cado: &CadoBody,
    ) -> Result<Option<String>, EldError> {
        // App state tips don't need latest keys since they're always immutable
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eld_common::address::Address;
    use eld_common::cado::{epoch_record_path_name, CADOMetadata, CadoPathKey, CadoType};
    use eld_common::constants::cado::LATEST;
    use eld_common::error::EldError;
    use eld_common::namespace::{slug_to_namespace_cadopath, NamespaceRecord};
    use eld_common::validator::EpochRecord;

    #[test]
    fn epoch_record_immutable_cado_generates_storage_keys() {
        let epoch_name = epoch_record_path_name(47).expect("epoch name");
        let path =
            CadoPath::new(CadoType::EpochRecord, CadoPathKey::Name(&epoch_name)).expect("path");
        let record = EpochRecord {
            epoch: 47,
            start_block: 940,
            active_validators: vec![],
            active_capacity_validator: None,
            challenged_capacity_validators: vec![],
        };
        let bytes = record.serialize_bin().expect("serialize");
        let cado = CadoBody::immutable(bytes, CADOMetadata::new(CadoType::EpochRecord, "47"));

        let keys = CADOKeyGenerator::generate(path, &cado).expect("keys");
        assert!(keys.latest_key.is_none());
        assert!(keys.original_key.starts_with("0x"));

        let latest_path =
            CadoPath::new(CadoType::EpochRecord, CadoPathKey::Name(LATEST)).expect("latest path");
        let latest_keys = CADOKeyGenerator::generate(latest_path, &cado).expect("latest keys");
        assert_ne!(keys.original_key, latest_keys.original_key);
    }

    #[test]
    fn namespace_registry_immutable_cado_generates_storage_keys() {
        let path = slug_to_namespace_cadopath("peter").expect("path");
        let owner =
            Address::parse_hex_str("0xe17404c417fa10cc04fdf73604fcacca8d0a687c").expect("address");
        let record = NamespaceRecord {
            namespace_slug: "peter".to_string(),
            owner,
            registered_height: 1,
        };
        let bytes = record.serialize_bin().expect("serialize");
        let cado = CadoBody::immutable(bytes, CADOMetadata::new(CadoType::Namespace, "peter"));

        let keys = CADOKeyGenerator::generate(path.clone(), &cado).expect("keys");
        assert!(keys.latest_key.is_none());
        assert!(keys.original_key.starts_with("0x"));

        let key_only = CADOKeyGenerator::generate_key(path, &cado).expect("key");
        assert_eq!(key_only, keys.original_key);
    }

    fn cado_map_path(name: &str) -> CadoPath {
        CadoPath::new(CadoType::CadoMap, CadoPathKey::Name(name)).expect("path")
    }

    #[test]
    fn mutable_path_name_matches_original_or_latest_hash() {
        let original = [0x11u8; 32];
        let cado = CadoBody::mutable_updated(
            original,
            b"current-state".to_vec(),
            CADOMetadata::new(CadoType::CadoMap, "owner"),
        );
        let latest = cado.latest_hash().expect("latest hash");
        assert_ne!(original, latest);

        let original_name = format!("0x{}", hex::encode(original));
        let original_path = cado_map_path(&original_name);
        let original_keys =
            CADOKeyGenerator::generate(original_path, &cado).expect("original name");
        assert!(original_keys.latest_key.is_some());

        let latest_name = format!("0x{}", hex::encode(latest));
        let latest_path = cado_map_path(&latest_name);
        let latest_keys = CADOKeyGenerator::generate(latest_path, &cado).expect("latest name");
        assert_eq!(original_keys.original_key, latest_keys.original_key);

        let other_name = format!("0x{}", hex::encode([0x22u8; 32]));
        let other_path = cado_map_path(&other_name);
        let err = CADOKeyGenerator::generate(other_path, &cado).expect_err("mismatch");
        match err {
            EldError::ValidationError { field, details, .. } => {
                assert_eq!(field, "path_name");
                assert_eq!(details, "Hash does not match path name");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn immutable_identifier_path_name_need_not_match_payload_hash() {
        let name = format!("0x{}", hex::encode([0x33u8; 32]));
        let path = cado_map_path(&name);
        let cado = CadoBody::immutable(
            b"snapshot-like".to_vec(),
            CADOMetadata::new(CadoType::CadoMap, "system"),
        );
        CADOKeyGenerator::generate(path, &cado).expect("identifier name");
    }
}
