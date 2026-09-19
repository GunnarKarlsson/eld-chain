use crate::address::Address;
use crate::constants::cado::{
    FOUR_PART_PATH_TYPES, SCOPE_ELD, SCOPE_ELD_ROOT, SCOPE_OTHER, SCOPE_PUBLIC, SCOPE_TEST,
    SCOPE_USER, TYPE_ACCOUNT, TYPE_APP_STATE_SNAPSHOT, TYPE_APP_STATE_TIP, TYPE_CADO_MAP,
    TYPE_CHUNK_REFERENCE, TYPE_EPOCH_RECORD, TYPE_NAMESPACE, TYPE_SNAPSHOT_CHUNK,
    TYPE_SNAPSHOT_METADATA, TYPE_STAKING_ACCOUNT, TYPE_STORAGE_STAKING_ACCOUNT, VALID_TYPES,
};
use crate::error::EldError;
use crate::logging::{LogSanitizer, SanitizedLoggable};
use crate::namespace::validate_namespace_slug;
use bincode;
use hex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::str::FromStr;

/// Path-kind for a CADO (account, namespace, epoch record, and so on).
///
/// This is the `type` segment of a [`CadoPath`]. It is not the stored object;
/// that envelope is [`CadoBody`] (`Immutable` / `Mutable`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CadoType {
    Account,
    StakingAccount,
    StorageStakingAccount,
    CadoMap,
    AppStateTip,
    SnapshotMetadata,
    SnapshotChunk,
    ChunkReference,
    AppStateSnapshot,
    EpochRecord,
    Namespace,
}

/// CADO scope enum for type-safe operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CadoScope {
    EldRoot,
    User,
    Public,
    Test,
    Other,
    Eld,
}

impl CadoType {
    /// Get the string representation of the CADO type
    pub fn as_str(&self) -> &'static str {
        match self {
            CadoType::Account => TYPE_ACCOUNT,
            CadoType::StakingAccount => TYPE_STAKING_ACCOUNT,
            CadoType::StorageStakingAccount => TYPE_STORAGE_STAKING_ACCOUNT,
            CadoType::CadoMap => TYPE_CADO_MAP,
            CadoType::AppStateTip => TYPE_APP_STATE_TIP,
            CadoType::SnapshotMetadata => TYPE_SNAPSHOT_METADATA,
            CadoType::SnapshotChunk => TYPE_SNAPSHOT_CHUNK,
            CadoType::ChunkReference => TYPE_CHUNK_REFERENCE,
            CadoType::AppStateSnapshot => TYPE_APP_STATE_SNAPSHOT,
            CadoType::EpochRecord => TYPE_EPOCH_RECORD,
            CadoType::Namespace => TYPE_NAMESPACE,
        }
    }

    /// Check if this type requires a 20-byte address
    pub fn is_address_type(&self) -> bool {
        matches!(
            self,
            CadoType::Account | CadoType::StakingAccount | CadoType::StorageStakingAccount
        )
    }

    /// Check if this type requires a 32-byte hash
    pub fn is_hash_type(&self) -> bool {
        !self.is_address_type()
    }

    /// Get the expected hex length for this type
    pub fn expected_hex_length(&self) -> usize {
        if self.is_address_type() {
            20
        } else {
            32
        }
    }

    /// Try to parse a string into a CadoType
    pub fn parse_type(s: &str) -> Option<Self> {
        s.parse().ok()
    }

    /// Check if this type is allowed to be deleted by users (requires signature verification)
    pub fn is_user_deletable(&self) -> bool {
        false
    }

    /// Check if this type is allowed to be deleted by the system (requires system authentication)
    pub fn is_system_deletable(&self) -> bool {
        matches!(
            self,
            CadoType::ChunkReference | CadoType::SnapshotChunk | CadoType::SnapshotMetadata
        )
    }

    /// Check if this type is allowed to be deleted at all (either by users or system)
    pub fn is_deletable(&self) -> bool {
        self.is_user_deletable() || self.is_system_deletable()
    }

    /// Check if this type is protected from deletion (critical system data)
    pub fn is_protected(&self) -> bool {
        !self.is_deletable()
    }
}

impl FromStr for CadoType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            TYPE_ACCOUNT => Ok(CadoType::Account),
            TYPE_STAKING_ACCOUNT => Ok(CadoType::StakingAccount),
            TYPE_STORAGE_STAKING_ACCOUNT => Ok(CadoType::StorageStakingAccount),
            TYPE_CADO_MAP => Ok(CadoType::CadoMap),
            TYPE_APP_STATE_TIP => Ok(CadoType::AppStateTip),
            TYPE_SNAPSHOT_METADATA => Ok(CadoType::SnapshotMetadata),
            TYPE_SNAPSHOT_CHUNK => Ok(CadoType::SnapshotChunk),
            TYPE_CHUNK_REFERENCE => Ok(CadoType::ChunkReference),
            TYPE_APP_STATE_SNAPSHOT => Ok(CadoType::AppStateSnapshot),
            TYPE_EPOCH_RECORD => Ok(CadoType::EpochRecord),
            TYPE_NAMESPACE => Ok(CadoType::Namespace),
            _ => Err(()),
        }
    }
}

impl std::fmt::Display for CadoType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl CadoScope {
    /// Get the string representation of the CADO scope
    pub fn as_str(&self) -> &'static str {
        match self {
            CadoScope::EldRoot => SCOPE_ELD_ROOT,
            CadoScope::User => SCOPE_USER,
            CadoScope::Public => SCOPE_PUBLIC,
            CadoScope::Test => SCOPE_TEST,
            CadoScope::Other => SCOPE_OTHER,
            CadoScope::Eld => SCOPE_ELD,
        }
    }

    /// Try to parse a string into a CadoScope
    pub fn parse_scope(s: &str) -> Option<Self> {
        s.parse().ok()
    }
}

impl FromStr for CadoScope {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            s if s == SCOPE_ELD_ROOT || s == SCOPE_ELD => Ok(CadoScope::EldRoot),
            SCOPE_USER => Ok(CadoScope::User),
            SCOPE_PUBLIC => Ok(CadoScope::Public),
            SCOPE_TEST => Ok(CadoScope::Test),
            SCOPE_OTHER => Ok(CadoScope::Other),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct CADOMarkedForDeletion {
    /// Remains `String` (not [`Address`]): deletion auth compares against
    /// [`CADOMetadata::owner`], which is not always a 20-byte account address.
    pub owner: String,
    pub cado_path: CadoPath,
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct CADOMetadata {
    type_: String,
    /// Remains `String` (not [`Address`]): path/name key for this CADO, not always an
    /// account. Examples: account hex, namespace slug, epoch id, `"system"`.
    /// Persisted in RocksDB via bincode; must stay compatible with existing blobs.
    owner: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct CADO {
    hash: [u8; 32], // Sha256(data)
    data: Vec<u8>,
    metadata: CADOMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub struct CADOMut {
    hash: [u8; 32], // Sha256(original_data)
    data: Vec<u8>,  // Current data
    metadata: CADOMetadata,
    latest_hash: [u8; 32], // Sha256(current_data)
}

/// Stored CADO envelope: an immutable or mutable body.
///
/// Distinct from [`CadoType`], which is the path kind (account, namespace, …).
#[derive(Debug, Clone, Serialize, Deserialize, Hash)]
pub enum CadoBody {
    Immutable(CADO),
    Mutable(CADOMut),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Hash)]
pub struct CadoPath {
    path: String,
    scope: String,
    type_: String,
    name: String,
}

/// Typed key input for constructing a [`CadoPath`] in the [`SCOPE_ELD_ROOT`] scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CadoPathKey<'a> {
    /// 3-part path: `/{SCOPE_ELD_ROOT}/{type_}/{name}`
    Name(&'a str),
    /// 4-part path: `/{SCOPE_ELD_ROOT}/{type_}/{name}/{id}`
    NameAndId(&'a str, &'a str),
    /// 3-part path with a validated 20-byte account address as the name segment.
    ///
    /// Only valid for CADO types whose name is a 20-byte hex address (account, staking, etc.).
    Address(Address),
    /// 4-part path with a validated account address as the first segment after the type.
    AddressAndId(Address, &'a str),
    /// Registry path `/@eld/namespace/{namespace_slug}` — slug, not 32-byte hex.
    NamespaceSlug(&'a str),
}

impl std::fmt::Display for CadoPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.path)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CADOKeys {
    pub original_key: String,
    pub latest_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CADOMap {
    pub from: String, // Secondary Storage-Key
    pub to: String,   // Primary Storage-Key
}

impl CADOMap {
    /// Serializes this mapping for the `cado_map` column family.
    pub fn serialize_bin(&self) -> Result<Vec<u8>, EldError> {
        bincode::serialize(self).map_err(|e| EldError::StorageError {
            operation: "serialize_cado_map".to_string(),
            details: format!("Failed to serialize CADOMap: {e}"),
        })
    }

    /// Deserializes a mapping from the `cado_map` column family.
    pub fn deserialize_bin(data: &[u8]) -> Result<Self, EldError> {
        bincode::deserialize(data).map_err(|e| EldError::StorageError {
            operation: "deserialize_cado_map".to_string(),
            details: format!("Failed to deserialize CADOMap: {e}"),
        })
    }
}

impl CADOMetadata {
    /// Simple constructor for metadata. Type is a CadoType enum for type safety.
    pub fn new(type_: CadoType, owner: impl Into<String>) -> Self {
        Self {
            type_: type_.to_string(),
            owner: owner.into(),
        }
    }

    pub fn type_(&self) -> &str {
        &self.type_
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub fn with_owner(&self, owner: impl Into<String>) -> Self {
        Self {
            type_: self.type_.clone(),
            owner: owner.into(),
        }
    }
}

impl CADO {
    /// Builds an immutable CADO; hash = Sha256(data).
    pub fn new(data: Vec<u8>, metadata: CADOMetadata) -> Self {
        let hash = Sha256::digest(&data).into();
        Self {
            hash,
            data,
            metadata,
        }
    }

    pub fn hash_bytes(&self) -> [u8; 32] {
        self.hash
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn metadata(&self) -> &CADOMetadata {
        &self.metadata
    }
}

impl CADOMut {
    /// New mutable CADO; original and latest both = Sha256(data).
    pub fn new(data: Vec<u8>, metadata: CADOMetadata) -> Self {
        let hash = Sha256::digest(&data).into();
        Self {
            hash,
            data,
            metadata,
            latest_hash: hash,
        }
    }

    /// Updated mutable CADO; keeps original hash, latest_hash = Sha256(data).
    pub fn updated(original_hash: [u8; 32], data: Vec<u8>, metadata: CADOMetadata) -> Self {
        let latest_hash = Sha256::digest(&data).into();
        Self {
            hash: original_hash,
            data,
            metadata,
            latest_hash,
        }
    }

    pub fn hash_bytes(&self) -> [u8; 32] {
        self.hash
    }

    pub fn latest_hash(&self) -> [u8; 32] {
        self.latest_hash
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn metadata(&self) -> &CADOMetadata {
        &self.metadata
    }
}

impl CadoBody {
    /// Immutable CADO from data + metadata.
    pub fn immutable(data: Vec<u8>, metadata: CADOMetadata) -> Self {
        Self::Immutable(CADO::new(data, metadata))
    }

    /// New mutable CADO (no previous version).
    pub fn mutable_new(data: Vec<u8>, metadata: CADOMetadata) -> Self {
        Self::Mutable(CADOMut::new(data, metadata))
    }

    /// Updated mutable CADO (existing original hash).
    pub fn mutable_updated(original_hash: [u8; 32], data: Vec<u8>, metadata: CADOMetadata) -> Self {
        Self::Mutable(CADOMut::updated(original_hash, data, metadata))
    }

    pub fn serialize_bin(&self) -> Result<Vec<u8>, EldError> {
        bincode::serialize(self).map_err(|e| EldError::StorageError {
            operation: "serialize_cado_type".to_string(),
            details: format!("Failed to serialize CadoBody: {e}"),
        })
    }

    pub fn deserialize_bin(data: &[u8]) -> Result<Self, EldError> {
        bincode::deserialize(data).map_err(|e| EldError::StorageError {
            operation: "deserialize_cado_type".to_string(),
            details: format!("Failed to deserialize CadoBody: {e}"),
        })
    }

    pub fn metadata(&self) -> &CADOMetadata {
        match self {
            CadoBody::Immutable(c) => c.metadata(),
            CadoBody::Mutable(m) => m.metadata(),
        }
    }

    pub fn data(&self) -> &[u8] {
        match self {
            CadoBody::Immutable(c) => c.data(),
            CadoBody::Mutable(m) => m.data(),
        }
    }

    pub fn hash_bytes(&self) -> [u8; 32] {
        match self {
            CadoBody::Immutable(c) => c.hash_bytes(),
            CadoBody::Mutable(m) => m.hash_bytes(),
        }
    }

    /// Content digest stored in the Merkle state trie (immutable: data hash; mutable: original hash).
    pub fn content_hash(&self) -> [u8; 32] {
        self.hash_bytes()
    }

    pub fn latest_hash(&self) -> Option<[u8; 32]> {
        match self {
            CadoBody::Immutable(_) => None,
            CadoBody::Mutable(m) => Some(m.latest_hash()),
        }
    }

    pub fn into_data(self) -> Vec<u8> {
        match self {
            CadoBody::Immutable(c) => c.data,
            CadoBody::Mutable(m) => m.data,
        }
    }
}

/// CADO payload bytes decode to `Self` using the same scheme as each type's inherent
/// `deserialize_bin` (typically bincode).
pub trait DeserializableBin: Sized {
    /// Deserializes `data` as `Self`.
    fn deserialize_bin(data: &[u8]) -> Result<Self, EldError>;
}

/// Decode a byte slice as a [`DeserializableBin`] type (e.g. `cado.data().deserialize_bin::<T>()`).
pub trait DeserializeBinSliceExt {
    /// Decodes `self` as `T`.
    fn deserialize_bin<T: DeserializableBin>(&self) -> Result<T, EldError>;
}

impl DeserializeBinSliceExt for [u8] {
    fn deserialize_bin<T: DeserializableBin>(&self) -> Result<T, EldError> {
        T::deserialize_bin(self)
    }
}

impl DeserializableBin for CADOMap {
    fn deserialize_bin(data: &[u8]) -> Result<Self, EldError> {
        CADOMap::deserialize_bin(data)
    }
}

impl DeserializableBin for Vec<String> {
    fn deserialize_bin(data: &[u8]) -> Result<Self, EldError> {
        bincode::deserialize(data).map_err(|e| EldError::StorageError {
            operation: "deserialize_string_vec".to_string(),
            details: format!("Failed to deserialize Vec<String>: {e}"),
        })
    }
}

impl DeserializableBin for CadoBody {
    fn deserialize_bin(data: &[u8]) -> Result<Self, EldError> {
        CadoBody::deserialize_bin(data)
    }
}

/// Builds a 32-byte hex path name for [`CadoType::EpochRecord`] keys.
///
/// `CadoPath` requires 3-part non-account names to be `0x` + 64 hex digits; human-readable
/// keys like `epoch:1` fail validation and must not be used.
pub fn epoch_record_path_name(epoch: i64) -> Result<String, EldError> {
    if epoch < 0 {
        return Err(EldError::ValidationError {
            field: "epoch".to_string(),
            value: epoch.to_string(),
            details: "Epoch number must be non-negative".to_string(),
        });
    }
    Ok(format!("0x{:064x}", epoch as u64))
}

/// Parses the epoch number from an [`epoch_record_path_name`] path key.
///
/// Returns `None` for the `LATEST` alias key or any name that is not `0x` + 64 hex digits.
pub fn epoch_from_record_path_name(name: &str) -> Option<i64> {
    if !name.starts_with("0x") || name.len() != 66 {
        return None;
    }
    let hex_part = &name[2..];
    if !hex_part.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let epoch_u64 = u64::from_str_radix(hex_part, 16).ok()?;
    i64::try_from(epoch_u64).ok()
}

impl CadoPath {
    fn validate_hex(s: &str, expected_len: usize, field_name: &str) -> Result<(), EldError> {
        if !s.starts_with("0x") {
            return Err(EldError::ValidationError {
                field: field_name.to_string(),
                value: s.to_string(),
                details: format!("{field_name} must start with 0x"),
            });
        }
        let bytes = hex::decode(&s[2..]).map_err(|e| EldError::ValidationError {
            field: field_name.to_string(),
            value: s.to_string(),
            details: format!("Invalid hex in {field_name}: {e}"),
        })?;
        if bytes.len() != expected_len {
            return Err(EldError::ValidationError {
                field: field_name.to_string(),
                value: s.to_string(),
                details: format!("{field_name} must be {expected_len} bytes"),
            });
        }
        Ok(())
    }

    /// Typed constructor for `@eld` paths.
    ///
    /// This is intentionally separate from [`CadoPath::parse`], which parses and validates a full
    /// path string. `new` performs validation directly so the string parser can be deleted later.
    pub fn new(type_: CadoType, key: CadoPathKey<'_>) -> Result<Self, EldError> {
        // Important: keep new's validation behavior aligned with the current string parser `parse`.
        // `CadoType` is already an allow-list, so we don't need a `VALID_TYPES` check here.

        let scope = SCOPE_ELD_ROOT;
        let type_str = type_.as_str();

        // Mirror the legacy string parser's definition of 4-part types.
        let requires_four_part = FOUR_PART_PATH_TYPES.contains(&type_str);

        let uses_twenty_byte_name = matches!(
            type_,
            CadoType::Account | CadoType::StakingAccount | CadoType::StorageStakingAccount
        );

        if matches!(type_, CadoType::Namespace) {
            let slug = match key {
                CadoPathKey::NamespaceSlug(slug) => slug.to_string(),
                _ => {
                    return Err(EldError::ValidationError {
                        field: "path".to_string(),
                        value: format!("{key:?}"),
                        details: format!(
                            "Type '{type_str}' requires NamespaceSlug(namespace_slug)"
                        ),
                    });
                }
            };
            validate_namespace_slug(&slug)?;
            let path = format!("/{scope}/{type_str}/{slug}");
            return Ok(CadoPath {
                path,
                scope: scope.to_string(),
                type_: type_str.to_string(),
                name: slug,
            });
        }

        let (path, name_field) = if requires_four_part {
            let (name, id) = match key {
                CadoPathKey::NameAndId(name, id) => (name.to_string(), id),
                CadoPathKey::AddressAndId(addr, id) => (addr.to_string(), id),
                CadoPathKey::Name(_) | CadoPathKey::Address(_) | CadoPathKey::NamespaceSlug(_) => {
                    return Err(EldError::ValidationError {
                        field: "path".to_string(),
                        value: format!("{key:?}"),
                        details: format!(
                            "{type_str} path must have format: /{scope}/{type_str}/sender_address/chunk_or_record_id",
                        ),
                    });
                }
            };

            Self::validate_hex(&name, 20, "sender address")?;
            Self::validate_hex(id, 32, "chunk or record ID")?;

            (
                format!("/{scope}/{type_str}/{name}/{id}"),
                format!("{name}/{id}"),
            )
        } else {
            let name = match key {
                CadoPathKey::Name(name) => name.to_string(),
                CadoPathKey::Address(addr) => {
                    if !uses_twenty_byte_name {
                        return Err(EldError::ValidationError {
                            field: "path".to_string(),
                            value: addr.to_string(),
                            details: format!(
                                "CadoPathKey::Address is only valid for account-like CADO types (20-byte name); type is '{type_str}'"
                            ),
                        });
                    }
                    addr.to_string()
                }
                CadoPathKey::NameAndId(_, _) | CadoPathKey::AddressAndId(_, _) => {
                    return Err(EldError::ValidationError {
                        field: "path".to_string(),
                        value: format!("{key:?}"),
                        details: format!(
                            "Type '{type_str}' requires a 3-part path key: Name(name) or Address(addr)"
                        ),
                    });
                }
                CadoPathKey::NamespaceSlug(_) => {
                    return Err(EldError::ValidationError {
                        field: "path".to_string(),
                        value: format!("{key:?}"),
                        details: format!("NamespaceSlug is only valid for type '{TYPE_NAMESPACE}'"),
                    });
                }
            };

            let expected_len = if uses_twenty_byte_name { 20 } else { 32 };

            Self::validate_hex(&name, expected_len, "name")?;

            (format!("/{scope}/{type_str}/{name}"), name)
        };

        Ok(CadoPath {
            path,
            scope: scope.to_string(),
            type_: type_str.to_string(),
            name: name_field,
        })
    }

    pub fn parse(path: &str) -> Result<Self, EldError> {
        if !path.starts_with('/') {
            return Err(EldError::ValidationError {
                field: "path".to_string(),
                value: path.to_string(),
                details: "Path must start with '/'".to_string(),
            });
        }

        let parts: Vec<&str> = path[1..].splitn(4, '/').collect();
        if parts.len() < 3 {
            return Err(EldError::ValidationError {
                field: "path".to_string(),
                value: path.to_string(),
                details: "Path must have at least 3 parts: /@scope/type_/name".to_string(),
            });
        }

        let scope = parts[0];
        if !scope.starts_with('@') || scope.len() <= 1 {
            return Err(EldError::ValidationError {
                field: "scope".to_string(),
                value: scope.to_string(),
                details: "Scope must start with '@' and be non-empty".to_string(),
            });
        }

        let type_ = parts[1];
        if !VALID_TYPES.contains(&type_) {
            return Err(EldError::ValidationError {
                field: "type".to_string(),
                value: type_.to_string(),
                details: format!("Invalid type: {type_}"),
            });
        }

        let name = if FOUR_PART_PATH_TYPES.contains(&type_) {
            if parts.len() != 4 {
                return Err(EldError::ValidationError {
                    field: "path".to_string(),
                    value: path.to_string(),
                    details: format!(
                        "{type_} path must have format: /@scope/{type_}/sender_address/chunk_or_record_id",
                    ),
                });
            }
            Self::validate_hex(parts[2], 20, "sender address")?;
            Self::validate_hex(parts[3], 32, "chunk or record ID")?;
            format!("{}/{}", parts[2], parts[3])
        } else {
            if parts.len() != 3 {
                return Err(EldError::ValidationError {
                    field: "path".to_string(),
                    value: path.to_string(),
                    details: "Path must have exactly 3 parts for non-device paths".to_string(),
                });
            }
            let expected_len = if type_ == TYPE_ACCOUNT
                || type_ == TYPE_STAKING_ACCOUNT
                || type_ == TYPE_STORAGE_STAKING_ACCOUNT
            {
                20
            } else {
                32
            };
            if type_ == TYPE_NAMESPACE {
                validate_namespace_slug(parts[2])?;
                parts[2].to_string()
            } else {
                Self::validate_hex(parts[2], expected_len, "name")?;
                parts[2].to_string()
            }
        };

        Ok(CadoPath {
            path: path.to_string(),
            scope: scope.to_string(),
            type_: type_.to_string(),
            name,
        })
    }
    pub fn as_str(&self) -> &str {
        &self.path
    }
    pub fn scope(&self) -> &str {
        &self.scope
    }
    pub fn type_(&self) -> &str {
        &self.type_
    }
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Check if this path is allowed to be deleted by users (requires signature verification)
    pub fn is_user_deletable(&self) -> bool {
        use crate::constants::cado::USER_DELETABLE_TYPES;
        USER_DELETABLE_TYPES.contains(&self.type_.as_str())
    }

    /// Check if this path is allowed to be deleted by the system (requires system authentication)
    pub fn is_system_deletable(&self) -> bool {
        use crate::constants::cado::SYSTEM_DELETABLE_TYPES;
        SYSTEM_DELETABLE_TYPES.contains(&self.type_.as_str())
    }

    /// Check if this path is allowed to be deleted at all (either by users or system)
    pub fn is_deletable(&self) -> bool {
        self.is_user_deletable() || self.is_system_deletable()
    }

    /// Check if this path is protected from deletion (critical system data)
    pub fn is_protected(&self) -> bool {
        !self.is_deletable()
    }

    /// True when this CADO is node infrastructure (DB/sync only), not chain-state MPT membership.
    pub fn is_infrastructure(&self) -> bool {
        use crate::constants::cado::INFRASTRUCTURE_CADO_TYPES;
        INFRASTRUCTURE_CADO_TYPES.contains(&self.type_.as_str())
    }

    /// Validate that this path is allowed for user deletion
    pub fn validate_user_deletion(&self) -> Result<(), EldError> {
        if !self.is_user_deletable() {
            return Err(EldError::ValidationError {
                field: "cado_type".to_string(),
                value: self.type_.clone(),
                details: format!("CADO type '{}' is not allowed for user deletion. Only user-controlled data can be deleted.", self.type_),
            });
        }
        Ok(())
    }

    /// Validate that this path is allowed for system deletion
    pub fn validate_system_deletion(&self) -> Result<(), EldError> {
        if !self.is_system_deletable() {
            return Err(EldError::ValidationError {
                field: "cado_type".to_string(),
                value: self.type_.clone(),
                details: format!("CADO type '{}' is not allowed for system deletion. Only system-managed cleanup data can be deleted.", self.type_),
            });
        }

        // Additional validation: system deletions should only be in ELD root scope
        if self.scope() != SCOPE_ELD_ROOT {
            return Err(EldError::ValidationError {
                field: "cado_scope".to_string(),
                value: self.scope().to_string(),
                details: format!(
                    "System deletions are only allowed in {} scope, got scope '{}'",
                    SCOPE_ELD_ROOT,
                    self.scope()
                ),
            });
        }

        Ok(())
    }

    /// Validate that this path is allowed for deletion (either user or system)
    pub fn validate_deletion(&self, is_system_operation: bool) -> Result<(), EldError> {
        if is_system_operation {
            self.validate_system_deletion()
        } else {
            self.validate_user_deletion()
        }
    }
}

impl FromStr for CadoPath {
    type Err = EldError;

    fn from_str(s: &str) -> Result<Self, EldError> {
        CadoPath::parse(s)
    }
}

impl std::fmt::Display for CADOMarkedForDeletion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "MarkedForDeletion {{ owner: {}, path: {} }}",
            self.owner, self.cado_path
        )
    }
}

impl std::fmt::Display for CADOMetadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Metadata {{ type: {}, owner: {} }}",
            self.type_, self.owner
        )
    }
}

impl std::fmt::Display for CADO {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "CADO {{ hash: 0x{}, size: {} bytes, {} }}",
            hex::encode(self.hash),
            self.data.len(),
            self.metadata
        )
    }
}

impl std::fmt::Display for CADOMut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "CADOMut {{ original_hash: 0x{}, latest_hash: 0x{}, size: {} bytes, {} }}",
            hex::encode(self.hash),
            hex::encode(self.latest_hash),
            self.data.len(),
            self.metadata
        )
    }
}

impl std::fmt::Display for CadoBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CadoBody::Immutable(cado) => write!(f, "{cado}"),
            CadoBody::Mutable(cado) => write!(f, "{cado}"),
        }
    }
}

impl std::fmt::Display for CADOKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.latest_key {
            Some(latest) => write!(
                f,
                "CADOKeys {{ original_key: {}, latest_key: Some({}) }}",
                self.original_key, latest
            ),
            None => write!(
                f,
                "CADOKeys {{ original_key: {}, latest_key: None }}",
                self.original_key
            ),
        }
    }
}

impl std::fmt::Display for CADOMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CADOMap {{ from: {}, to: {} }}", self.from, self.to)
    }
}

impl SanitizedLoggable for CADOMetadata {
    fn sanitized_log(&self) -> String {
        format!(
            "Metadata {{ type: {}, owner: {} }}",
            self.type_,
            LogSanitizer::sanitize_address(&self.owner)
        )
    }
}

impl SanitizedLoggable for CADO {
    fn sanitized_log(&self) -> String {
        format!(
            "CADO {{ hash: {}, size: {} bytes, {} }}",
            LogSanitizer::sanitize_hash(&hex::encode(self.hash)),
            self.data.len(),
            self.metadata.sanitized_log()
        )
    }
}

impl SanitizedLoggable for CADOMut {
    fn sanitized_log(&self) -> String {
        format!(
            "CADOMut {{ original_hash: {}, latest_hash: {}, size: {} bytes, {} }}",
            LogSanitizer::sanitize_hash(&hex::encode(self.hash)),
            LogSanitizer::sanitize_hash(&hex::encode(self.latest_hash)),
            self.data.len(),
            self.metadata.sanitized_log()
        )
    }
}

impl SanitizedLoggable for CadoBody {
    fn sanitized_log(&self) -> String {
        match self {
            CadoBody::Immutable(cado) => cado.sanitized_log(),
            CadoBody::Mutable(cado_mut) => cado_mut.sanitized_log(),
        }
    }
}

impl SanitizedLoggable for CadoPath {
    fn sanitized_log(&self) -> String {
        format!(
            "CadoPath {{ path: {}, scope: {}, type: {}, name: {} }}",
            LogSanitizer::sanitize_path(&self.path),
            self.scope,
            self.type_,
            LogSanitizer::sanitize_generic(&self.name)
        )
    }
}

/// True when `path` parses as infrastructure CADO (excluded from chain-state cache / MPT).
pub fn is_infrastructure_cado_path(path: &str) -> bool {
    CadoPath::parse(path)
        .map(|p| p.is_infrastructure())
        .unwrap_or(false)
}

impl SanitizedLoggable for CADOMarkedForDeletion {
    fn sanitized_log(&self) -> String {
        format!(
            "MarkedForDeletion {{ owner: {}, path: {} }}",
            LogSanitizer::sanitize_address(&self.owner),
            self.cado_path.sanitized_log()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_record_path_name_is_valid_cado_path_key() {
        let name = epoch_record_path_name(1).expect("name");
        let path = CadoPath::new(CadoType::EpochRecord, CadoPathKey::Name(&name)).expect("path");
        assert!(path.as_str().ends_with(&name));
    }

    #[test]
    fn epoch_from_record_path_name_round_trip() {
        for epoch in [0i64, 1, 47, 999] {
            let name = epoch_record_path_name(epoch).expect("name");
            assert_eq!(epoch_from_record_path_name(&name), Some(epoch));
        }
    }

    #[test]
    fn epoch_from_record_path_name_rejects_latest_alias() {
        use crate::constants::cado::LATEST;
        assert_eq!(epoch_from_record_path_name(LATEST), None);
    }

    #[test]
    fn epoch_from_record_path_name_rejects_invalid() {
        assert_eq!(epoch_from_record_path_name("epoch:1"), None);
        assert_eq!(epoch_from_record_path_name("0x01"), None);
    }

    #[test]
    fn cado_path_new_account_address_matches_name() {
        let addr =
            Address::parse_hex_str("0xe17404c417fa10cc04fdf73604fcacca8d0a687c").expect("address");
        let via_addr = CadoPath::new(CadoType::Account, CadoPathKey::Address(addr)).expect("path");
        let via_name = CadoPath::new(
            CadoType::Account,
            CadoPathKey::Name("0xe17404c417fa10cc04fdf73604fcacca8d0a687c"),
        )
        .expect("path");
        assert_eq!(via_addr.as_str(), via_name.as_str());
    }

    #[test]
    fn cado_path_account_address_rejects_too_short_hex_name() {
        assert!(
            CadoPath::new(CadoType::Account, CadoPathKey::Name("0x1234")).is_err(),
            "too-short account address should fail"
        );
    }

    #[test]
    fn infrastructure_cado_paths_are_detected() {
        use crate::constants::cado::LATEST;

        let account_path = CadoPath::new(
            CadoType::Account,
            CadoPathKey::Name("0x1234567890123456789012345678901234567890"),
        )
        .expect("account path");
        assert!(!account_path.is_infrastructure());
        assert!(!is_infrastructure_cado_path(account_path.as_str()));

        let app_state_snapshot_path = CadoPath::new(
            CadoType::AppStateSnapshot,
            CadoPathKey::Name("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        )
        .expect("app state snapshot path");
        assert!(app_state_snapshot_path.is_infrastructure());
        assert!(is_infrastructure_cado_path(
            app_state_snapshot_path.as_str()
        ));

        let latest_app_state_tip = CadoPath::new(CadoType::AppStateTip, CadoPathKey::Name(LATEST))
            .expect("app state tip path");
        assert!(latest_app_state_tip.is_infrastructure());

        let epoch_name = epoch_record_path_name(1).expect("epoch name");
        let epoch_path =
            CadoPath::new(CadoType::EpochRecord, CadoPathKey::Name(&epoch_name)).expect("epoch");
        assert!(!epoch_path.is_infrastructure());
    }
}
