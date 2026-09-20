use crate::api::pagination::{PaginatedResult, PrefixQueryOptions};
use crate::storage::cado_key_generator::CADOKeyGenerator;
use crate::storage::rate_limiter::PrefixQueryRateLimiter;
use crate::storage::traits::{
    CADOStorage, EpochRecordListOrder, PinboardGcDeletedItem, PinboardGcMetrics,
    PinboardGlobalFeedOrder, PinboardStorage, SnapshotChunk, SnapshotMetadata, SnapshotStorage,
};
use crate::sys_disk::directory_tree_size_bytes;
use eld_client::logging::SanitizedLog;

use eld_common::account::Account;
use eld_common::address::Address;
use eld_common::cado::{
    epoch_from_record_path_name, epoch_record_path_name, CADOKeys, CADOMap, CADOMetadata, CadoBody,
    CadoPath, CadoPathKey, CadoType,
};
use eld_common::coin::Coin;
use eld_common::constants::cado::{
    ALLOWED_SCOPES, MAX_PATH_LENGTH, MAX_PREFIX_LENGTH, PATH_PREFIX_SNAPSHOT_METADATA,
    TYPE_ACCOUNT, TYPE_APP_STATE_SNAPSHOT, TYPE_STAKING_ACCOUNT, TYPE_STORAGE_STAKING_ACCOUNT,
    VALID_TYPES,
};
use eld_common::constants::cado::{LATEST, PATH_PREFIX_EPOCH_RECORD, TYPE_EPOCH_RECORD};
use eld_common::nonce::Nonce;
use eld_common::pinboard::PinboardMessageMetadata;
use eld_common::tx::canonicalize_post_message_tags;
use eld_common::validator::EpochRecord;

use bincode::{deserialize, serialize};
use eld_common::error::EldError;
use eld_common::staking_account::StakingAccount;
use eld_common::validation::{
    safe_deserialize_app_state_snapshot_cado_data, safe_deserialize_cado_data,
};
use hex;
use rocksdb::{
    ColumnFamily, ColumnFamilyDescriptor, Direction, IteratorMode, Options, ReadOptions,
    Transaction, TransactionDB, TransactionDBOptions, DB,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tracing::{debug, error, info, warn};

/// RocksDB key under `indexed_transactions` holding the count of primary indexed rows
/// (`0x…` keys without `:`, i.e. full tx bodies). Serialized as little-endian `u64`.
/// Never overlaps tx IDs (those match `0x` + hex only).
const INDEXED_TRANSACTIONS_PRIMARY_TOTAL_COUNT_KEY: &[u8] =
    b"_meta:indexed_transactions_primary_total";

/// Little-endian `u64` count of successful indexed [`VerifiedProof`](eld_common::tx::PayloadInner::VerifiedProof)
/// network-wide (rollup). Does not overlap `0x` primaries or `vp:` keys.
const VERIFIED_PROOF_GLOBAL_REWARDS_COUNT_KEY: &[u8] = b"_meta:verified_proof_global_rewards_count";

/// Placeholder value for RocksDB keys where only existence matters (dedup markers).
const DUMMY_ROCKSDB_PAYLOAD: &[u8] = b"1";

/// Chronological ordering index for txs: **`block_pos:` + fixed hex height + ':' + hex index**.
/// Stored **in addition to** unpadded legacy `block_height:{h}:tx_index:{i}` rows.
///
/// Iterate between [`BLOCK_POS_CHRON_LOWER_BOUND`] (inclusive) and
/// [`BLOCK_POS_CHRON_UPPER_BOUND_EXCLUSIVE`] (exclusive) — byte-wise order equals chain order `(height, tx_index)`.
const BLOCK_POS_CHRON_LOWER_BOUND: &[u8] = b"block_pos:";
/// Exclusive scan upper bound: same prefix as real keys except the last byte is **semicolon** `;`,
/// which sorts after **colon** `:` — so every actual key (`block_pos:` + digits + `:` + …) stays below this.
const BLOCK_POS_CHRON_UPPER_BOUND_EXCLUSIVE: &[u8] = b"block_pos;";

/// Exclusive upper bound for `path_index` scans under [`PATH_PREFIX_EPOCH_RECORD`].
const EPOCH_RECORD_PATH_INDEX_UPPER_EXCLUSIVE: &str = "/@eld/epoch_record;";

/// Optional filters matching `GET /transactions` query semantics.
#[derive(Clone, Copy)]
pub(crate) struct IndexedTxChronFilters<'a> {
    pub block_height: Option<u64>,
    pub sender: Option<Address>,
    pub payload_type: Option<&'a str>,
}

/// Column families whose SST bytes are counted as pinboard payload storage in admin stats.
const PINBOARD_BLOB_CF_NAMES: &[&str] = &["pinboard_temp_blobs"];

/// Every column family name after open (including `default`, which RocksDB always adds).
const ELD_CF_NAMES_FOR_ADMIN_STATS: &[&str] = &[
    "cado",
    "path_index",
    "cado_map",
    "indexed_transactions",
    "indexed_events",
    "missing_content",
    "pinboard_meta",
    "pinboard_temp_blobs",
    "pinboard_refcounts",
    "pinboard_idx_wallet",
    "pinboard_idx_tag",
    "pinboard_idx_expiry",
    "pinboard_idx_commit",
    "pinboard_gc_meta",
    "pinboard_gc_deleted_log",
    "verified_proof_submissions",
    "default",
];

/// RocksDB property: on-disk size of SST files for a column family.
const ROCKSDB_TOTAL_SST_FILES_SIZE: &str = "rocksdb.total-sst-files-size";

/// Byte accounting for the admin API (directory walk + optional per-CF SST via read-only DB).
#[derive(Debug, Clone)]
pub struct AdminRocksDbStorageStats {
    /// Resolved database directory (same as the open path).
    pub path: PathBuf,
    /// Recursive on-disk size of the database directory (WAL, manifest, SST, etc.).
    pub directory_bytes: u64,
    /// Sum of `rocksdb.total-sst-files-size` for pinboard content column families, when available.
    pub pinboard_blob_cf_sst_bytes: Option<u64>,
    /// Sum of the same property for all other column families (including indexes and `default`).
    pub other_cf_sst_bytes: Option<u64>,
    /// When SST split is unavailable, a short reason (e.g. read-only open failed).
    pub cf_sst_breakdown_error: Option<String>,
}

/// Column family descriptors for the Eld application database (passed to TransactionDB and read-only DB helpers).
pub fn eld_column_family_descriptors() -> Vec<ColumnFamilyDescriptor> {
    vec![
        ColumnFamilyDescriptor::new("cado", Options::default()),
        ColumnFamilyDescriptor::new("path_index", Options::default()),
        ColumnFamilyDescriptor::new("cado_map", Options::default()),
        ColumnFamilyDescriptor::new("indexed_transactions", Options::default()),
        ColumnFamilyDescriptor::new("indexed_events", Options::default()),
        ColumnFamilyDescriptor::new("missing_content", Options::default()),
        ColumnFamilyDescriptor::new("pinboard_meta", Options::default()),
        ColumnFamilyDescriptor::new("pinboard_temp_blobs", Options::default()),
        ColumnFamilyDescriptor::new("pinboard_refcounts", Options::default()),
        ColumnFamilyDescriptor::new("pinboard_idx_wallet", Options::default()),
        ColumnFamilyDescriptor::new("pinboard_idx_tag", Options::default()),
        ColumnFamilyDescriptor::new("pinboard_idx_expiry", Options::default()),
        ColumnFamilyDescriptor::new("pinboard_idx_commit", Options::default()),
        ColumnFamilyDescriptor::new("pinboard_gc_meta", Options::default()),
        ColumnFamilyDescriptor::new("pinboard_gc_deleted_log", Options::default()),
        ColumnFamilyDescriptor::new("verified_proof_submissions", Options::default()),
    ]
}

/// Opens a short-lived read-only `DB` to read per-CF SST size properties while `TransactionDB` holds the primary lock.
fn try_read_only_cf_sst_split(path: &Path) -> Result<(u64, u64), String> {
    let opts = Options::default();
    let cfs = eld_column_family_descriptors();
    let db =
        DB::open_cf_descriptors_read_only(&opts, path, cfs, false).map_err(|e| e.to_string())?;

    let mut blob_total = 0u64;
    let mut other_total = 0u64;

    for &name in ELD_CF_NAMES_FOR_ADMIN_STATS {
        let Some(cf) = db.cf_handle(name) else {
            continue;
        };
        let sz = db
            .property_int_value_cf(cf, ROCKSDB_TOTAL_SST_FILES_SIZE)
            .map_err(|e| e.to_string())?
            .unwrap_or(0);
        if PINBOARD_BLOB_CF_NAMES.contains(&name) {
            blob_total = blob_total.saturating_add(sz);
        } else {
            other_total = other_total.saturating_add(sz);
        }
    }

    Ok((blob_total, other_total))
}

/// RocksDB-based implementation of AccountStorage
pub struct RocksDBStorage {
    pub db: TransactionDB,
    /// Rate limiter for prefix queries
    rate_limiter: PrefixQueryRateLimiter,
    /// Serializes updates to [`INDEXED_TRANSACTIONS_PRIMARY_TOTAL_COUNT_KEY`].
    indexed_transaction_primary_total_mutex: Mutex<()>,
}

impl RocksDBStorage {
    /// Validates CADO path for security issues
    /// This function checks for path traversal attacks and other malicious patterns
    fn validate_cado_path_security(path: &CadoPath) -> Result<(), EldError> {
        Self::validate_cado_path_security_enhanced(path)
    }

    /// Enhanced CADO path validation with comprehensive security checks
    /// This function implements a whitelist approach and checks for various attack vectors
    fn validate_cado_path_security_enhanced(path: &CadoPath) -> Result<(), EldError> {
        match Self::validate_cado_path_internal(path) {
            Ok(()) => {
                debug!("CADO path validation passed: {}", path.as_str());
                Ok(())
            }
            Err(e) => {
                warn!("CADO path validation failed: {} - {}", path.as_str(), e);
                Err(e)
            }
        }
    }

    /// Internal CADO path validation implementation
    fn validate_cado_path_internal(path: &CadoPath) -> Result<(), EldError> {
        let path_str = path.as_str();

        // 1. Path length limits (prevent resource exhaustion)
        if path_str.len() > MAX_PATH_LENGTH {
            return Err(EldError::ValidationError {
                field: "cado_path_length".to_string(),
                value: path_str.to_string(),
                details: format!("Path exceeds maximum length of {MAX_PATH_LENGTH} characters"),
            });
        }

        // 2. Null byte and control character validation
        if path_str.contains('\0') {
            return Err(EldError::ValidationError {
                field: "cado_path_null_bytes".to_string(),
                value: path_str.to_string(),
                details: "Path contains null bytes".to_string(),
            });
        }

        if path_str
            .chars()
            .any(|c| c.is_control() && c != '\t' && c != '\n' && c != '\r')
        {
            return Err(EldError::ValidationError {
                field: "cado_path_control_chars".to_string(),
                value: path_str.to_string(),
                details: "Path contains forbidden control characters".to_string(),
            });
        }

        // 3. Comprehensive path traversal detection
        let traversal_patterns = [
            "..",
            "//",
            "\\",
            "~",
            "..\\",
            "../",
            "\\..",
            "/..",
            "..\\",
            "..//",
            "//..",
            "\\..\\",
            "....",
            "..../",
            "....\\",
            "..%2f",
            "..%5c",
            "%2e%2e",
            "%2e%2e%2f",
            "%2e%2e%5c",
            "..%252f",
            "..%255c",
            "%252e%252e",
            "%252e%252e%252f",
            "%252e%252e%255c",
        ];

        for pattern in traversal_patterns.iter() {
            if path_str.to_lowercase().contains(pattern) {
                return Err(EldError::ValidationError {
                    field: "cado_path_traversal".to_string(),
                    value: path_str.to_string(),
                    details: format!("Path contains forbidden traversal pattern: {pattern}"),
                });
            }
        }

        // 4. Whitelist approach for path structure
        let path_parts: Vec<&str> = path_str.split('/').filter(|s| !s.is_empty()).collect();
        if path_parts.len() < 3 {
            return Err(EldError::ValidationError {
                field: "cado_path_structure".to_string(),
                value: path_str.to_string(),
                details: "Path must have at least 3 parts: /@scope/type/name".to_string(),
            });
        }

        // 5. Scope validation with whitelist
        let scope = path.scope();
        if !ALLOWED_SCOPES.contains(&scope) {
            return Err(EldError::ValidationError {
                field: "cado_path_scope".to_string(),
                value: scope.to_string(),
                details: format!("Scope '{scope}' is not allowed"),
            });
        }

        // 6. Type validation with whitelist
        let type_ = path.type_();
        if !VALID_TYPES.contains(&type_) {
            return Err(EldError::ValidationError {
                field: "cado_path_type".to_string(),
                value: type_.to_string(),
                details: format!("Invalid CADO type: {type_}"),
            });
        }

        // 7. Name format validation based on type
        let name = path.name();
        if name.is_empty() {
            return Err(EldError::ValidationError {
                field: "cado_path_name".to_string(),
                value: name.to_string(),
                details: "Name cannot be empty".to_string(),
            });
        }

        if type_ == TYPE_ACCOUNT
            || type_ == TYPE_STAKING_ACCOUNT
            || type_ == TYPE_STORAGE_STAKING_ACCOUNT
        {
            if !name.starts_with("0x")
                || hex::decode(&name[2..])
                    .map_err(|_| EldError::ValidationError {
                        field: "cado_path_name_hex".to_string(),
                        value: name.to_string(),
                        details: "Invalid hex format".to_string(),
                    })?
                    .len()
                    != 20
            {
                return Err(EldError::ValidationError {
                    field: "cado_path_name_address".to_string(),
                    value: name.to_string(),
                    details: format!("Invalid address format for {type_}"),
                });
            }
        } else if type_ == eld_common::constants::cado::TYPE_NAMESPACE {
            eld_common::namespace::validate_namespace_slug(name)?;
        } else {
            // For other types, validate as hex
            if !name.starts_with("0x")
                || hex::decode(&name[2..])
                    .map_err(|_| EldError::ValidationError {
                        field: "cado_path_name_hex".to_string(),
                        value: name.to_string(),
                        details: "Invalid hex format".to_string(),
                    })?
                    .len()
                    != 32
            {
                return Err(EldError::ValidationError {
                    field: "cado_path_name_hex".to_string(),
                    value: name.to_string(),
                    details: format!("Invalid hex format for {type_}"),
                });
            }
        }

        // 8. Additional security checks
        if path_str.contains("//") || path_str.contains("\\\\") {
            return Err(EldError::ValidationError {
                field: "cado_path_separators".to_string(),
                value: path_str.to_string(),
                details: "Path contains consecutive separators".to_string(),
            });
        }

        if path_str.ends_with('/') || path_str.ends_with('\\') {
            return Err(EldError::ValidationError {
                field: "cado_path_ending".to_string(),
                value: path_str.to_string(),
                details: "Path cannot end with separator".to_string(),
            });
        }

        Ok(())
    }

    /// Validates CADO prefix for security issues in search operations
    fn validate_cado_prefix_security(prefix: &str) -> Result<(), EldError> {
        // Similar validation as path but for prefixes
        if prefix.len() > MAX_PREFIX_LENGTH {
            return Err(EldError::ValidationError {
                field: "cado_prefix_length".to_string(),
                value: prefix.to_string(),
                details: format!("Prefix exceeds maximum length of {MAX_PREFIX_LENGTH} characters"),
            });
        }

        // Check for traversal patterns in prefix
        let traversal_patterns = ["..", "//", "\\", "~"];
        for pattern in traversal_patterns.iter() {
            if prefix.to_lowercase().contains(pattern) {
                return Err(EldError::ValidationError {
                    field: "cado_prefix_traversal".to_string(),
                    value: prefix.to_string(),
                    details: format!("Prefix contains forbidden traversal pattern: {pattern}"),
                });
            }
        }

        // Validate prefix format
        if !prefix.starts_with('/') {
            return Err(EldError::ValidationError {
                field: "cado_prefix_format".to_string(),
                value: prefix.to_string(),
                details: "Prefix must start with '/'".to_string(),
            });
        }

        // Check for control characters
        if prefix
            .chars()
            .any(|c| c.is_control() && c != '\t' && c != '\n' && c != '\r')
        {
            return Err(EldError::ValidationError {
                field: "cado_prefix_control_chars".to_string(),
                value: prefix.to_string(),
                details: "Prefix contains forbidden control characters".to_string(),
            });
        }

        // Check for null bytes
        if prefix.contains('\0') {
            return Err(EldError::ValidationError {
                field: "cado_prefix_null_bytes".to_string(),
                value: prefix.to_string(),
                details: "Prefix contains null bytes".to_string(),
            });
        }

        // Validate scope in prefix (similar to path validation)
        let prefix_parts: Vec<&str> = prefix.split('/').filter(|s| !s.is_empty()).collect();
        if !prefix_parts.is_empty() {
            let scope = prefix_parts[0];
            if scope.starts_with('@') && !ALLOWED_SCOPES.contains(&scope) {
                return Err(EldError::ValidationError {
                    field: "cado_prefix_scope".to_string(),
                    value: scope.to_string(),
                    details: format!("Scope '{scope}' is not allowed in prefix"),
                });
            }
        }

        Ok(())
    }

    pub fn new(path: &Path) -> Result<Self, EldError> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.set_max_open_files(10000);
        opts.set_use_fsync(false);
        opts.set_bytes_per_sync(1024 * 1024);
        opts.set_wal_bytes_per_sync(1024 * 1024);
        opts.set_max_background_jobs(4);
        opts.set_max_subcompactions(2);
        opts.set_level_zero_file_num_compaction_trigger(4);
        opts.set_level_zero_slowdown_writes_trigger(8);
        opts.set_level_zero_stop_writes_trigger(12);
        opts.set_target_file_size_base(64 * 1024 * 1024);
        opts.set_max_bytes_for_level_base(256 * 1024 * 1024);
        opts.set_write_buffer_size(64 * 1024 * 1024);
        opts.set_max_write_buffer_number(2);
        opts.set_min_write_buffer_number_to_merge(1);

        opts.create_missing_column_families(true);

        let cfs = eld_column_family_descriptors();

        let db =
            TransactionDB::open_cf_descriptors(&opts, &TransactionDBOptions::default(), path, cfs)
                .map_err(|e| EldError::StorageError {
                    operation: "open_rocksdb_database".to_string(),
                    details: format!("Failed to open RocksDB database at {path:?}: {e}"),
                })?;

        let rate_limiter = PrefixQueryRateLimiter::new(60); // Default: 60 requests per minute

        info!("RocksDBStorage initialized with rate limit: 60 requests per minute");

        Ok(Self {
            db,
            rate_limiter,
            indexed_transaction_primary_total_mutex: Mutex::new(()),
        })
    }

    /// Directory where this database was opened (from the underlying `TransactionDB`).
    pub fn database_path(&self) -> &Path {
        self.db.path()
    }

    /// Storage statistics for the admin HTTP API (directory size and optional per-CF SST breakdown).
    pub fn admin_rocksdb_storage_stats(&self) -> Result<AdminRocksDbStorageStats, EldError> {
        let path = self.database_path().to_path_buf();
        let directory_bytes =
            directory_tree_size_bytes(&path).map_err(|e| EldError::FileSystemError {
                operation: "rocksdb_directory_size".to_string(),
                path: path.display().to_string(),
                details: e.to_string(),
            })?;

        let (pinboard_blob_cf_sst_bytes, other_cf_sst_bytes, cf_sst_breakdown_error) =
            match try_read_only_cf_sst_split(&path) {
                Ok((blob, other)) => (Some(blob), Some(other), None),
                Err(err) => {
                    warn!(
                        path = %path.display(),
                        error = %err,
                        "Admin stats: could not open read-only DB for per-CF SST sizes"
                    );
                    (None, None, Some(err))
                }
            };

        Ok(AdminRocksDbStorageStats {
            path,
            directory_bytes,
            pinboard_blob_cf_sst_bytes,
            other_cf_sst_bytes,
            cf_sst_breakdown_error,
        })
    }

    pub fn begin_transaction(&self) -> Transaction<'_, TransactionDB> {
        self.db.transaction()
    }

    /// Secure version of get_cado_paths_by_prefix with pagination and rate limiting
    pub fn get_cado_paths_by_prefix_secure(
        &self,
        prefix: &str,
        options: PrefixQueryOptions,
    ) -> Result<PaginatedResult<CadoPath>, EldError> {
        // Validate options
        options.validate().map_err(|e| EldError::ValidationError {
            field: "prefix_query_options".to_string(),
            value: "invalid".to_string(),
            details: format!("Failed to validate prefix query options: {e}"),
        })?;

        // Check rate limiting
        if !self.rate_limiter.is_allowed(&options.client_id, prefix) {
            return Err(EldError::StorageError {
                operation: "rate_limited_query".to_string(),
                details: format!("Rate limit exceeded for prefix query: {prefix}"),
            });
        }

        // Log large prefix queries for monitoring
        if prefix.len() < 10 {
            warn!(
                "Large prefix query detected: '{}' (length: {})",
                prefix,
                prefix.len()
            );
        }

        let mut paths = Vec::new();
        let mut total_processed = 0;
        let mut estimated_memory_usage = 0;
        let prefix_bytes = prefix.as_bytes();

        // Use path_index_cf to find all paths with matching prefix
        let iter = self.db.iterator_cf(
            self.path_index_cf()?,
            rocksdb::IteratorMode::From(prefix_bytes, rocksdb::Direction::Forward),
        );

        for item in iter {
            let (path_bytes, _) = item.map_err(|e| EldError::StorageError {
                operation: "read_path_index_cf".to_string(),
                details: format!("Failed to read path_index_cf: {e}"),
            })?;
            let path_str =
                String::from_utf8(path_bytes.to_vec()).map_err(|e| EldError::ValidationError {
                    field: "path_utf8".to_string(),
                    value: format!("{path_bytes:?}"),
                    details: format!("Invalid UTF-8 in path: {e}"),
                })?;

            // Stop if we've moved past our prefix
            if !path_str.starts_with(prefix) {
                break;
            }

            total_processed += 1;

            // Check memory usage limit
            estimated_memory_usage += path_str.len();
            if estimated_memory_usage > options.max_memory_bytes {
                warn!(
                    "Memory limit exceeded for prefix query '{}': {} bytes (limit: {} bytes)",
                    prefix, estimated_memory_usage, options.max_memory_bytes
                );
                break;
            }

            // Check result limit
            if total_processed > options.max_results {
                warn!(
                    "Result limit exceeded for prefix query '{}': {} results (limit: {})",
                    prefix, total_processed, options.max_results
                );
                break;
            }

            // Apply pagination
            if total_processed > options.pagination.offset() {
                if paths.len() < options.pagination.page_size {
                    // Convert path string to CadoPath
                    let cado_path = CadoPath::parse(&path_str)?;
                    paths.push(cado_path);
                } else {
                    // We've filled the page
                    break;
                }
            }
        }

        let has_more = total_processed > options.pagination.offset() + paths.len();

        info!(
            "Prefix query '{}' returned {} results (processed: {}, memory: {} bytes)",
            prefix,
            paths.len(),
            total_processed,
            estimated_memory_usage
        );

        Ok(PaginatedResult::new(
            paths,
            options.pagination.page,
            options.pagination.page_size,
            has_more,
        ))
    }

    /// Secure version of get_cados_by_prefix with pagination and rate limiting
    pub fn get_cados_by_prefix_secure(
        &self,
        prefix: &str,
        options: PrefixQueryOptions,
    ) -> Result<PaginatedResult<CadoBody>, EldError> {
        // Validate options
        options.validate().map_err(|e| EldError::ValidationError {
            field: "prefix_query_options".to_string(),
            value: "invalid".to_string(),
            details: format!("Failed to validate prefix query options: {e}"),
        })?;

        // Check rate limiting
        if !self.rate_limiter.is_allowed(&options.client_id, prefix) {
            return Err(EldError::StorageError {
                operation: "rate_limited_query".to_string(),
                details: format!("Rate limit exceeded for prefix query: {prefix}"),
            });
        }

        // Log large prefix queries for monitoring
        if prefix.len() < 10 {
            warn!(
                "Large prefix query detected: '{}' (length: {})",
                prefix,
                prefix.len()
            );
        }

        let mut results = Vec::new();
        let mut total_processed = 0;
        let mut estimated_memory_usage = 0;
        let prefix_bytes = prefix.as_bytes();

        // Use path_index_cf to find all CADOs with matching prefix
        let iter = self.db.iterator_cf(
            self.path_index_cf()?,
            rocksdb::IteratorMode::From(prefix_bytes, rocksdb::Direction::Forward),
        );

        for item in iter {
            let (path_bytes, key_bytes) = item.map_err(|e| EldError::StorageError {
                operation: "read_path_index_cf".to_string(),
                details: format!("Failed to read path_index_cf: {e}"),
            })?;
            let path =
                String::from_utf8(path_bytes.to_vec()).map_err(|e| EldError::ValidationError {
                    field: "path_utf8".to_string(),
                    value: format!("{path_bytes:?}"),
                    details: format!("Invalid UTF-8 in path: {e}"),
                })?;

            // Stop if we've moved past our prefix
            if !path.starts_with(prefix) {
                break;
            }

            total_processed += 1;

            // Check memory usage limit
            estimated_memory_usage += path.len() + key_bytes.len();
            if estimated_memory_usage > options.max_memory_bytes {
                warn!(
                    "Memory limit exceeded for prefix query '{}': {} bytes (limit: {} bytes)",
                    prefix, estimated_memory_usage, options.max_memory_bytes
                );
                break;
            }

            // Check result limit
            if total_processed > options.max_results {
                warn!(
                    "Result limit exceeded for prefix query '{}': {} results (limit: {})",
                    prefix, total_processed, options.max_results
                );
                break;
            }

            // Apply pagination
            if total_processed > options.pagination.offset() {
                if results.len() < options.pagination.page_size {
                    // Get and deserialize the CADO
                    if let Some(value) =
                        self.db.get_cf(self.cado_cf()?, &key_bytes).map_err(|e| {
                            EldError::StorageError {
                                operation: "read_cado_cf".to_string(),
                                details: format!("Failed to read cado_cf: {e}"),
                            }
                        })?
                    {
                        estimated_memory_usage += value.len();
                        if estimated_memory_usage > options.max_memory_bytes {
                            warn!(
                                "Memory limit exceeded after deserializing CADO for prefix '{}'",
                                prefix
                            );
                            break;
                        }

                        let cado: CadoBody = safe_deserialize_cado_data(
                            &value,
                            &format!("CadoBody in prefix query for {prefix}"),
                        )
                        .map_err(|e| EldError::StorageError {
                            operation: "deserialize_cado_type".to_string(),
                            details: format!("Failed to deserialize CadoBody: {e}"),
                        })?;
                        results.push(cado);
                    }
                } else {
                    // We've filled the page
                    break;
                }
            }
        }

        let has_more = total_processed > options.pagination.offset() + results.len();

        info!(
            "Prefix query '{}' returned {} CADOs (processed: {}, memory: {} bytes)",
            prefix,
            results.len(),
            total_processed,
            estimated_memory_usage
        );

        Ok(PaginatedResult::new(
            results,
            options.pagination.page,
            options.pagination.page_size,
            has_more,
        ))
    }

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

    fn prepare_system_cado_deletion(
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

    fn delete_cado_associated_keys_db(
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

    fn delete_cado_associated_keys_with_tx(
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

    /// Returns a column family handle with proper error handling.
    /// This method replaces the use of .expect() with controlled error handling.
    ///
    /// # Arguments
    /// * `name` - The name of the column family to retrieve
    ///
    /// # Returns
    /// * `Ok(&ColumnFamily)` - The column family handle if it exists
    /// * `Err(EldError)` - An error if the column family doesn't exist
    ///
    pub fn cf_handle(&self, name: &str) -> Result<&ColumnFamily, EldError> {
        self.db
            .cf_handle(name)
            .ok_or_else(|| EldError::StorageError {
                operation: "cf_handle".to_string(),
                details: format!("Column family '{name}' not found in RocksDB"),
            })
    }

    /// Version 2 of cado_cf that returns Result instead of panicking
    pub fn cado_cf(&self) -> Result<&ColumnFamily, EldError> {
        self.cf_handle("cado")
    }

    /// Version 2 of path_index_cf that returns Result instead of panicking
    pub fn path_index_cf(&self) -> Result<&ColumnFamily, EldError> {
        self.cf_handle("path_index")
    }

    /// Version 2 of cado_map_cf that returns Result instead of panicking
    pub fn cado_map_cf(&self) -> Result<&ColumnFamily, EldError> {
        self.cf_handle("cado_map")
    }

    /// Returns the indexed_transactions column family handle
    pub fn indexed_transactions_cf(&self) -> Result<&ColumnFamily, EldError> {
        self.cf_handle("indexed_transactions")
    }

    /// Count primary indexed-transaction rows (same predicate as listing endpoints).
    fn count_primary_indexed_transaction_keys(&self, cf: &ColumnFamily) -> Result<u64, EldError> {
        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::Start);
        let mut n = 0u64;
        for item in iter {
            let (key, _) = item.map_err(|e| EldError::StorageError {
                operation: "count_primary_indexed_transactions".to_string(),
                details: format!("Failed to iterate indexed_transactions column family: {e}"),
            })?;
            let key_str = String::from_utf8_lossy(&key);
            if key_str.starts_with("0x") && !key_str.contains(':') {
                n += 1;
            }
        }
        Ok(n)
    }

    fn decode_u64_meta(bytes: &[u8]) -> Result<u64, EldError> {
        let arr: [u8; 8] = bytes.try_into().map_err(|_| EldError::StorageError {
            operation: "decode_indexed_transaction_total_count_meta".to_string(),
            details: format!(
                "Invalid indexed transaction total meta length (expected 8, got {})",
                bytes.len()
            ),
        })?;
        Ok(u64::from_le_bytes(arr))
    }

    /// Current total count of primary indexed transactions (for API pagination).
    fn indexed_transaction_primary_total_count(&self) -> Result<u64, EldError> {
        let cf = self.indexed_transactions_cf()?;
        match self
            .db
            .get_cf(cf, INDEXED_TRANSACTIONS_PRIMARY_TOTAL_COUNT_KEY)
            .map_err(|e| EldError::StorageError {
                operation: "get_indexed_transaction_total_count_meta".to_string(),
                details: format!("Failed to read indexed transaction total meta: {e}"),
            })? {
            Some(bytes) => Self::decode_u64_meta(&bytes),
            None => {
                let n = self.count_primary_indexed_transaction_keys(cf)?;
                self.db
                    .put_cf(
                        cf,
                        INDEXED_TRANSACTIONS_PRIMARY_TOTAL_COUNT_KEY,
                        n.to_le_bytes(),
                    )
                    .map_err(|e| EldError::StorageError {
                        operation: "put_indexed_transaction_total_count_meta".to_string(),
                        details: format!("Failed to repair indexed transaction total meta: {e}"),
                    })?;
                Ok(n)
            }
        }
    }

    fn bump_indexed_transaction_primary_total_count(&self) -> Result<(), EldError> {
        let _guard = self
            .indexed_transaction_primary_total_mutex
            .lock()
            .map_err(|_| EldError::StorageError {
                operation: "indexed_transaction_primary_total_mutex".to_string(),
                details: "Mutex poisoned while updating indexed transaction total count"
                    .to_string(),
            })?;
        let cf = self.indexed_transactions_cf()?;
        let new_total = match self
            .db
            .get_cf(cf, INDEXED_TRANSACTIONS_PRIMARY_TOTAL_COUNT_KEY)
            .map_err(|e| EldError::StorageError {
                operation: "get_indexed_transaction_total_count_meta".to_string(),
                details: format!("Failed to read indexed transaction total meta: {e}"),
            })? {
            Some(bytes) => {
                let cur = Self::decode_u64_meta(&bytes)?;
                cur.checked_add(1).ok_or_else(|| EldError::StorageError {
                    operation: "bump_indexed_transaction_total_count".to_string(),
                    details: "Indexed transaction total count overflow".to_string(),
                })?
            }
            None => self.count_primary_indexed_transaction_keys(cf)?,
        };

        self.db
            .put_cf(
                cf,
                INDEXED_TRANSACTIONS_PRIMARY_TOTAL_COUNT_KEY,
                new_total.to_le_bytes(),
            )
            .map_err(|e| EldError::StorageError {
                operation: "put_indexed_transaction_total_count_meta".to_string(),
                details: format!("Failed to write indexed transaction total meta: {e}"),
            })?;
        Ok(())
    }

    fn read_verified_proof_global_rewards_count(&self, cf: &ColumnFamily) -> Result<u64, EldError> {
        match self
            .db
            .get_cf(cf, VERIFIED_PROOF_GLOBAL_REWARDS_COUNT_KEY)
            .map_err(|e| EldError::StorageError {
                operation: "get_verified_proof_global_rewards_count".to_string(),
                details: e.to_string(),
            })? {
            None => Ok(0),
            Some(bytes) => {
                if bytes.len() == 8 {
                    let arr: [u8; 8] =
                        bytes
                            .as_slice()
                            .try_into()
                            .map_err(|_| EldError::StorageError {
                                operation: "verified_proof_global_rewards_count_slice".to_string(),
                                details: "invalid length".to_string(),
                            })?;
                    Ok(u64::from_le_bytes(arr))
                } else {
                    Ok(0)
                }
            }
        }
    }

    fn bump_verified_proof_global_rewards_count(&self, cf: &ColumnFamily) -> Result<(), EldError> {
        let next = self
            .read_verified_proof_global_rewards_count(cf)?
            .saturating_add(1);
        self.db
            .put_cf(
                cf,
                VERIFIED_PROOF_GLOBAL_REWARDS_COUNT_KEY,
                next.to_le_bytes(),
            )
            .map_err(|e| EldError::StorageError {
                operation: "put_verified_proof_global_rewards_count".to_string(),
                details: e.to_string(),
            })?;
        Ok(())
    }

    fn block_position_chron_index_key_bytes(block_height: u64, block_index: u32) -> Vec<u8> {
        format!("block_pos:{block_height:016x}:{block_index:08x}").into_bytes()
    }

    fn chron_block_position_read_options() -> ReadOptions {
        let mut ro = ReadOptions::default();
        ro.set_iterate_lower_bound(BLOCK_POS_CHRON_LOWER_BOUND.to_vec());
        ro.set_iterate_upper_bound(BLOCK_POS_CHRON_UPPER_BOUND_EXCLUSIVE.to_vec());
        ro.set_total_order_seek(true);
        ro
    }

    fn parse_block_position_chron_key(key_str: &str) -> Option<(u64, u32)> {
        let rest = key_str.strip_prefix("block_pos:")?;
        let (h_hex, ix_hex) = rest.split_once(':')?;
        let bh = u64::from_str_radix(h_hex, 16).ok()?;
        let bi = u32::from_str_radix(ix_hex, 16).ok()?;
        Some((bh, bi))
    }

    fn epoch_record_path_index_read_options() -> ReadOptions {
        let mut ro = ReadOptions::default();
        ro.set_iterate_lower_bound(PATH_PREFIX_EPOCH_RECORD.as_bytes().to_vec());
        ro.set_iterate_upper_bound(EPOCH_RECORD_PATH_INDEX_UPPER_EXCLUSIVE.as_bytes().to_vec());
        ro.set_total_order_seek(true);
        ro
    }

    fn epoch_number_from_record_path_str(path_str: &str) -> Option<i64> {
        let path = CadoPath::parse(path_str).ok()?;
        if path.type_() != TYPE_EPOCH_RECORD {
            return None;
        }
        let name = path.name();
        if name == LATEST {
            return None;
        }
        epoch_from_record_path_name(name)
    }

    /// Lists persisted epoch records in chronological order via `path_index` under
    /// [`PATH_PREFIX_EPOCH_RECORD`]. Skips the `LATEST` alias path.
    pub fn list_epoch_records_chron(
        &self,
        order: EpochRecordListOrder,
        after_epoch: Option<i64>,
        fetch_limit: usize,
    ) -> Result<Vec<EpochRecord>, EldError> {
        let cf = self.path_index_cf()?;
        let readopts = Self::epoch_record_path_index_read_options();

        let seek_key_owned = match (order, after_epoch) {
            (EpochRecordListOrder::Desc, Some(ae)) => {
                let name = epoch_record_path_name(ae)?;
                Some(format!("{PATH_PREFIX_EPOCH_RECORD}{name}"))
            }
            (EpochRecordListOrder::Asc, Some(ae)) => {
                let name = epoch_record_path_name(ae)?;
                Some(format!("{PATH_PREFIX_EPOCH_RECORD}{name}"))
            }
            _ => None,
        };

        let mode = match order {
            EpochRecordListOrder::Desc => match &seek_key_owned {
                None => IteratorMode::From(
                    EPOCH_RECORD_PATH_INDEX_UPPER_EXCLUSIVE.as_bytes(),
                    Direction::Reverse,
                ),
                Some(k) => IteratorMode::From(k.as_bytes(), Direction::Reverse),
            },
            EpochRecordListOrder::Asc => match &seek_key_owned {
                None => IteratorMode::From(PATH_PREFIX_EPOCH_RECORD.as_bytes(), Direction::Forward),
                Some(k) => IteratorMode::From(k.as_bytes(), Direction::Forward),
            },
        };

        let iter = self.db.iterator_cf_opt(cf, readopts, mode);
        let mut out = Vec::new();

        for res in iter {
            let (path_bytes, _) = res.map_err(|e| EldError::StorageError {
                operation: "iterate_epoch_record_path_index".to_string(),
                details: format!("{e}"),
            })?;
            let path_str =
                std::str::from_utf8(&path_bytes).map_err(|e| EldError::StorageError {
                    operation: "epoch_record_path_index_utf8".to_string(),
                    details: format!("{e}"),
                })?;

            if !path_str.starts_with(PATH_PREFIX_EPOCH_RECORD) {
                break;
            }

            let Some(epoch) = Self::epoch_number_from_record_path_str(path_str) else {
                continue;
            };

            let include = match (order, after_epoch) {
                (EpochRecordListOrder::Desc, Some(ae)) => epoch < ae,
                (EpochRecordListOrder::Desc, None) => true,
                (EpochRecordListOrder::Asc, Some(ae)) => epoch > ae,
                (EpochRecordListOrder::Asc, None) => true,
            };
            if !include {
                continue;
            }

            let cado_path = CadoPath::parse(path_str)?;
            let record: EpochRecord = self.get_deserialized_cado_by_path(cado_path)?;
            out.push(record);
            if out.len() >= fetch_limit {
                break;
            }
        }

        Ok(out)
    }

    /// Counts persisted per-epoch records (excludes the `LATEST` alias).
    pub fn count_epoch_records(&self) -> Result<u64, EldError> {
        let cf = self.path_index_cf()?;
        let readopts = Self::epoch_record_path_index_read_options();
        let iter = self.db.iterator_cf_opt(
            cf,
            readopts,
            IteratorMode::From(PATH_PREFIX_EPOCH_RECORD.as_bytes(), Direction::Forward),
        );

        let mut n = 0u64;
        for res in iter {
            let (path_bytes, _) = res.map_err(|e| EldError::StorageError {
                operation: "count_epoch_record_paths".to_string(),
                details: format!("{e}"),
            })?;
            let path_str =
                std::str::from_utf8(&path_bytes).map_err(|e| EldError::StorageError {
                    operation: "count_epoch_record_path_utf8".to_string(),
                    details: format!("{e}"),
                })?;
            if !path_str.starts_with(PATH_PREFIX_EPOCH_RECORD) {
                break;
            }
            if Self::epoch_number_from_record_path_str(path_str).is_some() {
                n += 1;
            }
        }
        Ok(n)
    }

    /// Secondary index for successful [`VerifiedProof`](eld_common::tx::PayloadInner::VerifiedProof)
    /// rewards: `vp:{provider}:{height:016x}:{block_index:08x}`.
    pub(crate) fn verified_proof_reward_index_key(
        provider_normalized: &str,
        block_height: u64,
        block_index: u32,
    ) -> Vec<u8> {
        format!("vp:{provider_normalized}:{block_height:016x}:{block_index:08x}").into_bytes()
    }

    pub(crate) fn parse_verified_proof_reward_index_key(
        key_str: &str,
    ) -> Option<(String, u64, u32)> {
        let rest = key_str.strip_prefix("vp:")?;
        let parts: Vec<&str> = rest.split(':').collect();
        if parts.len() != 3 {
            return None;
        }
        let addr = parts[0].to_string();
        let h = u64::from_str_radix(parts[1], 16).ok()?;
        let bi = u32::from_str_radix(parts[2], 16).ok()?;
        Some((addr, h, bi))
    }

    fn indexed_transaction_matches_optional_filters(
        ix: &IndexedTransaction,
        block_height_filter: Option<u64>,
        sender_filter: Option<Address>,
        payload_type_filter: Option<&str>,
    ) -> bool {
        use eld_common::tx::HasSender;
        if let Some(h) = block_height_filter {
            if ix.block_height != h {
                return false;
            }
        }
        if let Some(addr) = sender_filter {
            if ix.tx.payload.inner.sender() != addr {
                return false;
            }
        }
        if let Some(pt) = payload_type_filter {
            if ix.tx.payload.r#type != pt {
                return false;
            }
        }
        true
    }

    fn load_primary_indexed_transaction_by_cf(
        cf: &ColumnFamily,
        db: &TransactionDB,
        tx_id_bytes: &[u8],
    ) -> Result<IndexedTransaction, EldError> {
        let normalized = std::str::from_utf8(tx_id_bytes).map_err(|e| EldError::StorageError {
            operation: "indexed_tx_secondary_tx_id_utf8".to_string(),
            details: format!("{e}"),
        })?;
        let normalized = normalized.to_string();

        let data = db
            .get_cf(cf, normalized.as_bytes())
            .map_err(|e| EldError::StorageError {
                operation: "get_indexed_transaction_by_secondary".to_string(),
                details: format!("{e}"),
            })?;
        match data {
            Some(bytes) => serde_json::from_slice(&bytes).map_err(|e| EldError::StorageError {
                operation: "deserialize_indexed_transaction_by_secondary".to_string(),
                details: format!("{e}"),
            }),
            None => Err(EldError::StorageError {
                operation: "missing_primary_for_block_pos_row".to_string(),
                details: format!("Missing primary IndexedTransaction row for {normalized}"),
            }),
        }
    }

    pub(crate) fn list_indexed_transactions_chron_desc_scanned(
        &self,
        cursor_tuple: Option<(u64, u32)>,
        filters: IndexedTxChronFilters<'_>,
        fetch_limit: usize,
    ) -> Result<Vec<IndexedTransaction>, EldError> {
        let cf = self.indexed_transactions_cf()?;
        let readopts = Self::chron_block_position_read_options();

        let seek_bytes_owned =
            cursor_tuple.map(|(ch, ci)| Self::block_position_chron_index_key_bytes(ch, ci));

        let mode = match &seek_bytes_owned {
            None => IteratorMode::End,
            Some(ck_vec) => IteratorMode::From(ck_vec.as_slice(), Direction::Reverse),
        };

        let iter = self.db.iterator_cf_opt(cf, readopts, mode);

        let mut out = Vec::new();

        for res in iter {
            let (key, value) = res.map_err(|e| EldError::StorageError {
                operation: "iterate_block_pos_tx_chron_desc".to_string(),
                details: format!("{e}"),
            })?;
            let key_str = std::str::from_utf8(&key).map_err(|e| EldError::StorageError {
                operation: "block_pos_index_key_utf8".to_string(),
                details: format!("{e}"),
            })?;

            if !key_str.starts_with("block_pos:") {
                continue;
            }

            let (kh, ki) = Self::parse_block_position_chron_key(key_str).ok_or_else(|| {
                EldError::StorageError {
                    operation: "parse_block_pos_key".to_string(),
                    details: format!("Malformed block_pos key: {key_str}"),
                }
            })?;

            if let Some((ch, ci)) = cursor_tuple {
                // Skip txs newer than or equal to the cursor anchor (exclusive next page boundary).
                if kh > ch || (kh == ch && ki >= ci) {
                    continue;
                }
            }

            let indexed_tx = Self::load_primary_indexed_transaction_by_cf(cf, &self.db, &value)?;
            if !Self::indexed_transaction_matches_optional_filters(
                &indexed_tx,
                filters.block_height,
                filters.sender,
                filters.payload_type,
            ) {
                continue;
            }

            out.push(indexed_tx);
            if out.len() >= fetch_limit {
                break;
            }
        }

        Ok(out)
    }

    pub(crate) fn count_indexed_transactions_matching_chron_filters(
        &self,
        filters: IndexedTxChronFilters<'_>,
    ) -> Result<u64, EldError> {
        let cf = self.indexed_transactions_cf()?;
        let readopts = Self::chron_block_position_read_options();
        let iter = self.db.iterator_cf_opt(
            cf,
            readopts,
            IteratorMode::From(BLOCK_POS_CHRON_LOWER_BOUND, Direction::Forward),
        );

        let mut n = 0u64;

        for res in iter {
            let (_, value) = res.map_err(|e| EldError::StorageError {
                operation: "count_filtered_chron_scan".to_string(),
                details: format!("{e}"),
            })?;

            let indexed_tx = Self::load_primary_indexed_transaction_by_cf(cf, &self.db, &value)?;
            if Self::indexed_transaction_matches_optional_filters(
                &indexed_tx,
                filters.block_height,
                filters.sender,
                filters.payload_type,
            ) {
                n += 1;
            }
        }

        Ok(n)
    }

    /// Returns the indexed_events column family handle
    pub fn indexed_events_cf(&self) -> Result<&ColumnFamily, EldError> {
        self.cf_handle("indexed_events")
    }

    pub fn pinboard_meta_cf(&self) -> Result<&ColumnFamily, EldError> {
        self.cf_handle("pinboard_meta")
    }

    pub fn pinboard_refcounts_cf(&self) -> Result<&ColumnFamily, EldError> {
        self.cf_handle("pinboard_refcounts")
    }

    pub fn pinboard_idx_wallet_cf(&self) -> Result<&ColumnFamily, EldError> {
        self.cf_handle("pinboard_idx_wallet")
    }

    pub fn pinboard_idx_tag_cf(&self) -> Result<&ColumnFamily, EldError> {
        self.cf_handle("pinboard_idx_tag")
    }

    pub fn pinboard_idx_expiry_cf(&self) -> Result<&ColumnFamily, EldError> {
        self.cf_handle("pinboard_idx_expiry")
    }

    pub fn pinboard_idx_commit_cf(&self) -> Result<&ColumnFamily, EldError> {
        self.cf_handle("pinboard_idx_commit")
    }

    pub fn pinboard_temp_blobs_cf(&self) -> Result<&ColumnFamily, EldError> {
        self.cf_handle("pinboard_temp_blobs")
    }

    pub fn pinboard_gc_meta_cf(&self) -> Result<&ColumnFamily, EldError> {
        self.cf_handle("pinboard_gc_meta")
    }

    pub fn pinboard_gc_deleted_log_cf(&self) -> Result<&ColumnFamily, EldError> {
        self.cf_handle("pinboard_gc_deleted_log")
    }

    pub fn verified_proof_submissions_cf(&self) -> Result<&ColumnFamily, EldError> {
        self.cf_handle("verified_proof_submissions")
    }

    /// Key for verified-proof submission claim rows (`vp_submit:` prefix avoids collisions).
    pub(crate) fn verified_proof_submission_claim_key(
        sender_address_hex: &str,
        challenge_id: &str,
    ) -> Vec<u8> {
        format!(
            "vp_submit:{}:{}",
            sender_address_hex.to_ascii_lowercase(),
            challenge_id
        )
        .into_bytes()
    }

    /// Persist `(sender, challenge_id)` if absent; return `true` on first claim, `false` if already present.
    /// Used by [`HybridStorage`](crate::storage::hybrid_storage::HybridStorage) via
    /// [`VerifiedProofSubmissionClaimStorage`](crate::storage::traits::VerifiedProofSubmissionClaimStorage).
    pub fn insert_verified_proof_submission_claim(
        &self,
        sender_address_hex: &str,
        challenge_id: &str,
    ) -> Result<bool, EldError> {
        let cf = self.verified_proof_submissions_cf()?;
        let key = Self::verified_proof_submission_claim_key(sender_address_hex, challenge_id);
        if self
            .db
            .get_cf(cf, &key)
            .map_err(|e| EldError::StorageError {
                operation: "verified_proof_submission_get".to_string(),
                details: e.to_string(),
            })?
            .is_some()
        {
            return Ok(false);
        }
        self.db
            .put_cf(cf, &key, DUMMY_ROCKSDB_PAYLOAD)
            .map_err(|e| EldError::StorageError {
                operation: "verified_proof_submission_put".to_string(),
                details: e.to_string(),
            })?;
        Ok(true)
    }

    /// Returns `true` if `(sender, challenge_id)` was already claimed (no write).
    pub fn has_verified_proof_submission_claim(
        &self,
        sender_address_hex: &str,
        challenge_id: &str,
    ) -> Result<bool, EldError> {
        let cf = self.verified_proof_submissions_cf()?;
        let key = Self::verified_proof_submission_claim_key(sender_address_hex, challenge_id);
        Ok(self
            .db
            .get_cf(cf, &key)
            .map_err(|e| EldError::StorageError {
                operation: "verified_proof_submission_has".to_string(),
                details: e.to_string(),
            })?
            .is_some())
    }

    /// Key for on-chain rewarded challenge dedup (`vp_rewarded:` prefix).
    pub(crate) fn verified_proof_challenge_rewarded_key(challenge_id: &str) -> Vec<u8> {
        format!("vp_rewarded:{challenge_id}").into_bytes()
    }

    pub fn is_verified_proof_challenge_rewarded(
        &self,
        challenge_id: &str,
    ) -> Result<bool, EldError> {
        let cf = self.verified_proof_submissions_cf()?;
        let key = Self::verified_proof_challenge_rewarded_key(challenge_id);
        self.db
            .get_cf(cf, &key)
            .map_err(|e| EldError::StorageError {
                operation: "verified_proof_challenge_rewarded_get".to_string(),
                details: e.to_string(),
            })
            .map(|v| v.is_some())
    }

    pub fn put_verified_proof_challenge_rewarded_with_tx(
        &self,
        challenge_id: &str,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        let cf = self.verified_proof_submissions_cf()?;
        let key = Self::verified_proof_challenge_rewarded_key(challenge_id);
        tx.put_cf(cf, &key, DUMMY_ROCKSDB_PAYLOAD)
            .map_err(|e| EldError::StorageError {
                operation: "verified_proof_challenge_rewarded_put".to_string(),
                details: e.to_string(),
            })
    }

    fn u64_hex_fixed(v: u64) -> String {
        format!("{v:016x}")
    }

    fn pinboard_wallet_index_key(wallet: &str, committed_height: u64, message_id: &str) -> String {
        format!(
            "{}:{}:{}",
            wallet,
            Self::u64_hex_fixed(committed_height),
            message_id
        )
    }

    fn pinboard_tag_index_key(tag: &str, committed_height: u64, message_id: &str) -> String {
        format!(
            "{}:{}:{}",
            tag,
            Self::u64_hex_fixed(committed_height),
            message_id
        )
    }

    fn pinboard_expiry_index_key(expires_height: u64, message_id: &str) -> String {
        format!("{}:{}", Self::u64_hex_fixed(expires_height), message_id)
    }

    fn pinboard_commit_index_key(committed_height: u64, message_id: &str) -> String {
        format!("{}:{}", Self::u64_hex_fixed(committed_height), message_id)
    }

    // ============================================================================
    // PINBOARD STORAGE METHODS
    // ============================================================================

    pub fn get_pinboard_metadata(
        &self,
        message_id: &str,
    ) -> Result<Option<PinboardMessageMetadata>, EldError> {
        self.get_pinboard_metadata_from_cf_key(message_id.as_bytes(), "get_pinboard_metadata")
    }

    pub fn get_pinboard_metadata_by_path_key(
        &self,
        path_key: &str,
    ) -> Result<Option<PinboardMessageMetadata>, EldError> {
        self.get_pinboard_metadata_from_cf_key(
            path_key.as_bytes(),
            "get_pinboard_metadata_by_path_key",
        )
    }

    fn get_pinboard_metadata_from_cf_key(
        &self,
        key: &[u8],
        operation: &str,
    ) -> Result<Option<PinboardMessageMetadata>, EldError> {
        match self
            .db
            .get_cf(self.pinboard_meta_cf()?, key)
            .map_err(|e| EldError::StorageError {
                operation: operation.to_string(),
                details: format!("Failed to fetch pinboard metadata: {e}"),
            })? {
            Some(v) => Ok(Some(bincode::deserialize(&v).map_err(|e| {
                EldError::StorageError {
                    operation: "deserialize_pinboard_metadata".to_string(),
                    details: format!("Failed to deserialize PinboardMessageMetadata: {e}"),
                }
            })?)),
            None => Ok(None),
        }
    }

    pub fn increment_pinboard_gc_scanned_count_by(&self, delta: u64) -> Result<u64, EldError> {
        self.increment_pinboard_gc_counter(b"metrics:scanned_count", delta)
    }

    pub fn increment_pinboard_gc_deleted_count_by(&self, delta: u64) -> Result<u64, EldError> {
        self.increment_pinboard_gc_counter(b"metrics:deleted_count", delta)
    }

    fn increment_pinboard_gc_counter(&self, key: &[u8], delta: u64) -> Result<u64, EldError> {
        let current = match self
            .db
            .get_cf(self.pinboard_gc_meta_cf()?, key)
            .map_err(|e| EldError::StorageError {
                operation: "increment_pinboard_gc_counter_get".to_string(),
                details: format!("Failed to read pinboard GC counter: {e}"),
            })? {
            Some(v) => bincode::deserialize::<u64>(&v).map_err(|e| EldError::StorageError {
                operation: "increment_pinboard_gc_counter_deserialize".to_string(),
                details: format!("Failed to deserialize pinboard GC counter: {e}"),
            })?,
            None => 0,
        };
        let next = current.saturating_add(delta);
        let bytes = bincode::serialize(&next).map_err(|e| EldError::StorageError {
            operation: "increment_pinboard_gc_counter_serialize".to_string(),
            details: format!("Failed to serialize pinboard GC counter: {e}"),
        })?;
        self.db
            .put_cf(self.pinboard_gc_meta_cf()?, key, bytes)
            .map_err(|e| EldError::StorageError {
                operation: "increment_pinboard_gc_counter_put".to_string(),
                details: format!("Failed to write pinboard GC counter: {e}"),
            })?;
        Ok(next)
    }

    pub fn set_pinboard_gc_last_seen_height(&self, height: u64) -> Result<(), EldError> {
        self.put_pinboard_gc_u64(b"metrics:last_seen_height", height)
    }

    pub fn set_pinboard_gc_last_deleted_expires_height(&self, height: u64) -> Result<(), EldError> {
        self.put_pinboard_gc_u64(b"metrics:last_deleted_expires_height", height)
    }

    fn put_pinboard_gc_u64(&self, key: &[u8], value: u64) -> Result<(), EldError> {
        let bytes = bincode::serialize(&value).map_err(|e| EldError::StorageError {
            operation: "put_pinboard_gc_u64_serialize".to_string(),
            details: format!("Failed to serialize pinboard GC u64 value: {e}"),
        })?;
        self.db
            .put_cf(self.pinboard_gc_meta_cf()?, key, bytes)
            .map_err(|e| EldError::StorageError {
                operation: "put_pinboard_gc_u64_put".to_string(),
                details: format!("Failed to write pinboard GC u64 value: {e}"),
            })?;
        Ok(())
    }

    fn get_pinboard_gc_u64(&self, key: &[u8]) -> Result<u64, EldError> {
        match self
            .db
            .get_cf(self.pinboard_gc_meta_cf()?, key)
            .map_err(|e| EldError::StorageError {
                operation: "get_pinboard_gc_u64_get".to_string(),
                details: format!("Failed to read pinboard GC u64 value: {e}"),
            })? {
            Some(v) => bincode::deserialize::<u64>(&v).map_err(|e| EldError::StorageError {
                operation: "get_pinboard_gc_u64_deserialize".to_string(),
                details: format!("Failed to deserialize pinboard GC u64 value: {e}"),
            }),
            None => Ok(0),
        }
    }

    pub fn append_pinboard_gc_deleted_item(
        &self,
        wallet: &str,
        message_id: &str,
        deleted_at_height: u64,
    ) -> Result<(), EldError> {
        let seq = self.increment_pinboard_gc_counter(b"metrics:deleted_seq", 1)?;
        let key = format!("{seq:016x}");
        let item = PinboardGcDeletedItem {
            wallet: wallet.to_string(),
            message_id: message_id.to_string(),
            deleted_at_height,
        };
        let bytes = bincode::serialize(&item).map_err(|e| EldError::StorageError {
            operation: "append_pinboard_gc_deleted_item_serialize".to_string(),
            details: format!("Failed to serialize pinboard GC deleted item: {e}"),
        })?;
        self.db
            .put_cf(self.pinboard_gc_deleted_log_cf()?, key.as_bytes(), bytes)
            .map_err(|e| EldError::StorageError {
                operation: "append_pinboard_gc_deleted_item_put".to_string(),
                details: format!("Failed to append pinboard GC deleted item: {e}"),
            })?;
        Ok(())
    }

    pub fn get_pinboard_gc_metrics(&self) -> Result<PinboardGcMetrics, EldError> {
        let deleted_count = self.get_pinboard_gc_u64(b"metrics:deleted_count")?;
        let scanned_count = self.get_pinboard_gc_u64(b"metrics:scanned_count")?;
        let last_seen_height = self.get_pinboard_gc_u64(b"metrics:last_seen_height")?;
        let last_deleted_expires_height =
            self.get_pinboard_gc_u64(b"metrics:last_deleted_expires_height")?;
        let lag_blocks = last_seen_height.saturating_sub(last_deleted_expires_height);
        let last_cursor_key = None;

        let mut deleted_items = Vec::new();
        let iter = self.db.iterator_cf(
            self.pinboard_gc_deleted_log_cf()?,
            rocksdb::IteratorMode::Start,
        );
        for item in iter {
            let (_key, value) = item.map_err(|e| EldError::StorageError {
                operation: "get_pinboard_gc_metrics_iter".to_string(),
                details: format!("Failed to iterate pinboard GC deleted log: {e}"),
            })?;
            let row = bincode::deserialize::<PinboardGcDeletedItem>(&value).map_err(|e| {
                EldError::StorageError {
                    operation: "get_pinboard_gc_metrics_deserialize_deleted_item".to_string(),
                    details: format!("Failed to deserialize pinboard GC deleted item: {e}"),
                }
            })?;
            deleted_items.push(row);
        }

        Ok(PinboardGcMetrics {
            deleted_count,
            scanned_count,
            last_cursor_key,
            lag_blocks,
            deleted_items,
        })
    }

    pub fn find_pinboard_metadata_refs_by_content_key(
        &self,
        content_key: &str,
    ) -> Result<Vec<(String, String)>, EldError> {
        let mut refs = Vec::new();
        let iter = self
            .db
            .iterator_cf(self.pinboard_meta_cf()?, rocksdb::IteratorMode::Start);
        for item in iter {
            let (_k, v) = item.map_err(|e| EldError::StorageError {
                operation: "find_pinboard_metadata_refs_by_content_key_iter".to_string(),
                details: format!("Failed to iterate pinboard metadata CF: {e}"),
            })?;
            let meta: PinboardMessageMetadata =
                bincode::deserialize(&v).map_err(|e| EldError::StorageError {
                    operation: "find_pinboard_metadata_refs_by_content_key_deserialize".to_string(),
                    details: format!("Failed to deserialize pinboard metadata: {e}"),
                })?;
            if meta.content_key == content_key {
                refs.push((meta.original_signer.hex_with_prefix(), meta.message_id));
            }
        }
        Ok(refs)
    }

    pub fn put_pinboard_temp_blob(
        &self,
        content_key: &str,
        bytes: &[u8],
        expires_at_unix: u64,
    ) -> Result<(), EldError> {
        #[derive(Serialize, Deserialize)]
        struct TempBlobRecord {
            blob: Vec<u8>,
            expires_at_unix: u64,
        }

        let record = TempBlobRecord {
            blob: bytes.to_vec(),
            expires_at_unix,
        };
        let encoded = bincode::serialize(&record).map_err(|e| EldError::StorageError {
            operation: "serialize_pinboard_temp_blob".to_string(),
            details: format!("Failed to serialize temp blob record: {e}"),
        })?;

        self.db
            .put_cf(
                self.pinboard_temp_blobs_cf()?,
                content_key.as_bytes(),
                encoded,
            )
            .map_err(|e| EldError::StorageError {
                operation: "put_pinboard_temp_blob".to_string(),
                details: format!("Failed to store pinboard temp blob: {e}"),
            })?;
        Ok(())
    }

    pub fn get_pinboard_temp_blob(&self, content_key: &str) -> Result<Option<Vec<u8>>, EldError> {
        #[derive(Serialize, Deserialize)]
        struct TempBlobRecord {
            blob: Vec<u8>,
            expires_at_unix: u64,
        }

        let raw = self
            .db
            .get_cf(self.pinboard_temp_blobs_cf()?, content_key.as_bytes())
            .map_err(|e| EldError::StorageError {
                operation: "get_pinboard_temp_blob".to_string(),
                details: format!("Failed to fetch pinboard temp blob: {e}"),
            })?;

        match raw {
            Some(v) => {
                let record: TempBlobRecord =
                    bincode::deserialize(&v).map_err(|e| EldError::StorageError {
                        operation: "deserialize_pinboard_temp_blob".to_string(),
                        details: format!("Failed to deserialize temp blob record: {e}"),
                    })?;
                Ok(Some(record.blob))
            }
            None => Ok(None),
        }
    }

    /// Secure/paginated listing of message_ids for a given tag.
    ///
    /// This performs a prefix scan over `pinboard_idx_tag` using the key format:
    /// `{tag}:{committed_height_hex}:{message_id}`.
    pub fn get_pinboard_message_ids_by_tag_secure(
        &self,
        tag: &str,
        options: PrefixQueryOptions,
    ) -> Result<PaginatedResult<String>, EldError> {
        options.validate().map_err(|e| EldError::ValidationError {
            field: "prefix_query_options".to_string(),
            value: "invalid".to_string(),
            details: format!("Failed to validate prefix query options: {e}"),
        })?;

        // Rate limit by tag prefix.
        let prefix_for_rl = format!("pinboard_idx_tag:{tag}");
        if !self
            .rate_limiter
            .is_allowed(&options.client_id, &prefix_for_rl)
        {
            return Err(EldError::StorageError {
                operation: "rate_limited_query".to_string(),
                details: format!("Rate limit exceeded for tag query: {tag}"),
            });
        }

        // Canonicalize address-shaped tags the same way as tx validation does.
        let tag = canonicalize_post_message_tags(&[tag.to_string()])?
            .into_iter()
            .next()
            .unwrap_or_else(|| tag.to_string());

        let prefix = format!("{tag}:");
        let prefix_bytes = prefix.as_bytes();

        let mut items: Vec<String> = Vec::new();
        let mut total_processed = 0usize;
        let mut estimated_memory_usage = 0usize;

        let iter = self.db.iterator_cf(
            self.pinboard_idx_tag_cf()?,
            rocksdb::IteratorMode::From(prefix_bytes, rocksdb::Direction::Forward),
        );

        for item in iter {
            let (key_bytes, _value_bytes) = item.map_err(|e| EldError::StorageError {
                operation: "read_pinboard_idx_tag".to_string(),
                details: format!("Failed to read pinboard_idx_tag: {e}"),
            })?;

            let key_str =
                String::from_utf8(key_bytes.to_vec()).map_err(|e| EldError::ValidationError {
                    field: "pinboard_tag_index_key_utf8".to_string(),
                    value: format!("{key_bytes:?}"),
                    details: format!("Invalid UTF-8 in tag index key: {e}"),
                })?;

            if !key_str.starts_with(&prefix) {
                break;
            }

            total_processed += 1;
            estimated_memory_usage += key_str.len();

            if estimated_memory_usage > options.max_memory_bytes {
                warn!(
                    "Memory limit exceeded for tag query '{}': {} bytes (limit: {} bytes)",
                    tag, estimated_memory_usage, options.max_memory_bytes
                );
                break;
            }

            if total_processed > options.max_results {
                warn!(
                    "Result limit exceeded for tag query '{}': {} results (limit: {})",
                    tag, total_processed, options.max_results
                );
                break;
            }

            if total_processed > options.pagination.offset() {
                if items.len() < options.pagination.page_size {
                    // key format: tag:height_hex:message_id
                    let message_id = key_str
                        .rsplit_once(':')
                        .map(|(_prefix, msg)| msg.to_string())
                        .unwrap_or_else(|| key_str.clone());
                    items.push(message_id);
                } else {
                    break;
                }
            }
        }

        let has_more = total_processed > options.pagination.offset() + items.len();

        Ok(PaginatedResult::new(
            items,
            options.pagination.page,
            options.pagination.page_size,
            has_more,
        ))
    }

    /// Secure/paginated listing of message_ids for a given wallet.
    ///
    /// Key format in `pinboard_idx_wallet`: `{wallet}:{committed_height_hex}:{message_id}`.
    pub fn get_pinboard_message_ids_by_wallet_secure(
        &self,
        wallet: &str,
        options: PrefixQueryOptions,
    ) -> Result<PaginatedResult<String>, EldError> {
        options.validate().map_err(|e| EldError::ValidationError {
            field: "prefix_query_options".to_string(),
            value: "invalid".to_string(),
            details: format!("Failed to validate prefix query options: {e}"),
        })?;

        let prefix_for_rl = format!("pinboard_idx_wallet:{wallet}");
        if !self
            .rate_limiter
            .is_allowed(&options.client_id, &prefix_for_rl)
        {
            return Err(EldError::StorageError {
                operation: "rate_limited_query".to_string(),
                details: format!("Rate limit exceeded for wallet query: {wallet}"),
            });
        }

        let prefix = format!("{wallet}:");
        let prefix_bytes = prefix.as_bytes();

        let mut items: Vec<String> = Vec::new();
        let mut total_processed = 0usize;
        let mut estimated_memory_usage = 0usize;

        let iter = self.db.iterator_cf(
            self.pinboard_idx_wallet_cf()?,
            rocksdb::IteratorMode::From(prefix_bytes, rocksdb::Direction::Forward),
        );

        for item in iter {
            let (key_bytes, _value_bytes) = item.map_err(|e| EldError::StorageError {
                operation: "read_pinboard_idx_wallet".to_string(),
                details: format!("Failed to read pinboard_idx_wallet: {e}"),
            })?;

            let key_str =
                String::from_utf8(key_bytes.to_vec()).map_err(|e| EldError::ValidationError {
                    field: "pinboard_wallet_index_key_utf8".to_string(),
                    value: format!("{key_bytes:?}"),
                    details: format!("Invalid UTF-8 in wallet index key: {e}"),
                })?;

            if !key_str.starts_with(&prefix) {
                break;
            }

            total_processed += 1;
            estimated_memory_usage += key_str.len();

            if estimated_memory_usage > options.max_memory_bytes {
                warn!(
                    "Memory limit exceeded for wallet query '{}': {} bytes (limit: {} bytes)",
                    wallet, estimated_memory_usage, options.max_memory_bytes
                );
                break;
            }

            if total_processed > options.max_results {
                warn!(
                    "Result limit exceeded for wallet query '{}': {} results (limit: {})",
                    wallet, total_processed, options.max_results
                );
                break;
            }

            if total_processed > options.pagination.offset() {
                if items.len() < options.pagination.page_size {
                    let message_id = key_str
                        .rsplit_once(':')
                        .map(|(_prefix, msg)| msg.to_string())
                        .unwrap_or_else(|| key_str.clone());
                    items.push(message_id);
                } else {
                    break;
                }
            }
        }

        let has_more = total_processed > options.pagination.offset() + items.len();
        Ok(PaginatedResult::new(
            items,
            options.pagination.page,
            options.pagination.page_size,
            has_more,
        ))
    }

    /// Secure/paginated listing of message_ids across all posts, ordered by commit height.
    ///
    /// Key format in `pinboard_idx_commit`: `{committed_height_hex}:{message_id}`.
    pub fn get_pinboard_message_ids_global_secure(
        &self,
        order: PinboardGlobalFeedOrder,
        options: PrefixQueryOptions,
    ) -> Result<PaginatedResult<String>, EldError> {
        options.validate().map_err(|e| EldError::ValidationError {
            field: "prefix_query_options".to_string(),
            value: "invalid".to_string(),
            details: format!("Failed to validate prefix query options: {e}"),
        })?;

        let prefix_for_rl = "pinboard_idx_commit:global".to_string();
        if !self
            .rate_limiter
            .is_allowed(&options.client_id, &prefix_for_rl)
        {
            return Err(EldError::StorageError {
                operation: "rate_limited_query".to_string(),
                details: "Rate limit exceeded for global pinboard feed".to_string(),
            });
        }

        let mut items: Vec<String> = Vec::new();
        let mut total_processed = 0usize;
        let mut estimated_memory_usage = 0usize;

        let iter_mode = match order {
            PinboardGlobalFeedOrder::Asc => rocksdb::IteratorMode::Start,
            PinboardGlobalFeedOrder::Desc => rocksdb::IteratorMode::End,
        };
        let iter = self
            .db
            .iterator_cf(self.pinboard_idx_commit_cf()?, iter_mode);

        for item in iter {
            let (key_bytes, _value_bytes) = item.map_err(|e| EldError::StorageError {
                operation: "read_pinboard_idx_commit".to_string(),
                details: format!("Failed to read pinboard_idx_commit: {e}"),
            })?;

            let key_str =
                String::from_utf8(key_bytes.to_vec()).map_err(|e| EldError::ValidationError {
                    field: "pinboard_commit_index_key_utf8".to_string(),
                    value: format!("{key_bytes:?}"),
                    details: format!("Invalid UTF-8 in commit index key: {e}"),
                })?;

            total_processed += 1;
            estimated_memory_usage += key_str.len();

            if estimated_memory_usage > options.max_memory_bytes {
                warn!(
                    "Memory limit exceeded for global feed: {} bytes (limit: {} bytes)",
                    estimated_memory_usage, options.max_memory_bytes
                );
                break;
            }

            if total_processed > options.max_results {
                warn!(
                    "Result limit exceeded for global feed: {} results (limit: {})",
                    total_processed, options.max_results
                );
                break;
            }

            if total_processed > options.pagination.offset() {
                if items.len() < options.pagination.page_size {
                    // key format: height_hex:message_id
                    let message_id = key_str
                        .split_once(':')
                        .map(|(_h, msg)| msg.to_string())
                        .unwrap_or_else(|| key_str.clone());
                    items.push(message_id);
                } else {
                    break;
                }
            }
        }

        let has_more = total_processed > options.pagination.offset() + items.len();
        Ok(PaginatedResult::new(
            items,
            options.pagination.page,
            options.pagination.page_size,
            has_more,
        ))
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

    pub fn get_cado_paths_by_prefix(&self, prefix: &str) -> Result<Vec<CadoPath>, EldError> {
        // Validate prefix for security issues
        Self::validate_cado_prefix_security(prefix)?;

        // Check rate limiting with default client ID
        if !self.rate_limiter.is_allowed("default", prefix) {
            return Err(EldError::StorageError {
                operation: "rate_limit_check".to_string(),
                details: format!("Rate limit exceeded for prefix query: {prefix}"),
            });
        }

        // Use secure version with default options
        let options = PrefixQueryOptions::default();
        let result = self.get_cado_paths_by_prefix_secure(prefix, options)?;
        Ok(result.items)
    }

    fn deserialize_stored_cado_type(path: &CadoPath, data: &[u8]) -> Result<CadoBody, EldError> {
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

    pub fn get_cados_by_prefix(&self, prefix: &str) -> Result<Vec<CadoBody>, EldError> {
        // Validate prefix for security issues
        Self::validate_cado_prefix_security(prefix)?;

        // Check rate limiting with default client ID
        if !self.rate_limiter.is_allowed("default", prefix) {
            return Err(EldError::StorageError {
                operation: "rate_limit_check".to_string(),
                details: format!("Rate limit exceeded for prefix query: {prefix}"),
            });
        }

        // Use secure version with default options
        let options = PrefixQueryOptions::default();
        let result = self.get_cados_by_prefix_secure(prefix, options)?;
        Ok(result.items)
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

    // TODO: Return result
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

impl PinboardStorage for RocksDBStorage {
    fn put_pinboard_metadata_with_tx(
        &self,
        message_id: &str,
        meta: &PinboardMessageMetadata,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        let bytes = bincode::serialize(meta).map_err(|e| EldError::StorageError {
            operation: "serialize_pinboard_metadata".to_string(),
            details: format!("Failed to serialize PinboardMessageMetadata: {e}"),
        })?;
        tx.put_cf(self.pinboard_meta_cf()?, message_id.as_bytes(), &bytes)
            .map_err(|e| EldError::StorageError {
                operation: "put_pinboard_metadata_with_tx".to_string(),
                details: format!("Failed to store pinboard metadata: {e}"),
            })?;

        // Secondary key for custom-namespace posts (same payload; enables GET by /@ns/message_id).
        if let Some(namespace) = meta.namespace.as_ref() {
            let path_key =
                eld_common::pinboard::pinboard_namespace_content_path(namespace, message_id);
            tx.put_cf(self.pinboard_meta_cf()?, path_key.as_bytes(), bytes)
                .map_err(|e| EldError::StorageError {
                    operation: "put_pinboard_metadata_namespace_path_with_tx".to_string(),
                    details: format!("Failed to store pinboard namespace path key: {e}"),
                })?;
        }

        Ok(())
    }

    fn put_pinboard_commit_index_with_tx(
        &self,
        committed_height: u64,
        message_id: &str,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        let key = Self::pinboard_commit_index_key(committed_height, message_id);
        tx.put_cf(self.pinboard_idx_commit_cf()?, key.as_bytes(), [])
            .map_err(|e| EldError::StorageError {
                operation: "put_pinboard_commit_index_with_tx".to_string(),
                details: format!("Failed to store commit index key: {e}"),
            })?;
        Ok(())
    }

    fn put_pinboard_wallet_index_with_tx(
        &self,
        wallet: &str,
        committed_height: u64,
        message_id: &str,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        let key = Self::pinboard_wallet_index_key(wallet, committed_height, message_id);
        tx.put_cf(self.pinboard_idx_wallet_cf()?, key.as_bytes(), [])
            .map_err(|e| EldError::StorageError {
                operation: "put_pinboard_wallet_index_with_tx".to_string(),
                details: format!("Failed to store wallet index key: {e}"),
            })?;
        Ok(())
    }

    fn put_pinboard_tag_index_with_tx(
        &self,
        tag: &str,
        committed_height: u64,
        message_id: &str,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        let key = Self::pinboard_tag_index_key(tag, committed_height, message_id);
        tx.put_cf(self.pinboard_idx_tag_cf()?, key.as_bytes(), [])
            .map_err(|e| EldError::StorageError {
                operation: "put_pinboard_tag_index_with_tx".to_string(),
                details: format!("Failed to store tag index key: {e}"),
            })?;
        Ok(())
    }

    fn put_pinboard_expiry_index_with_tx(
        &self,
        expires_height: u64,
        message_id: &str,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        let key = Self::pinboard_expiry_index_key(expires_height, message_id);
        tx.put_cf(self.pinboard_idx_expiry_cf()?, key.as_bytes(), [])
            .map_err(|e| EldError::StorageError {
                operation: "put_pinboard_expiry_index_with_tx".to_string(),
                details: format!("Failed to store expiry index key: {e}"),
            })?;
        Ok(())
    }

    fn update_pinboard_refcount_with_tx(
        &self,
        content_key: &str,
        delta: i64,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<u64, EldError> {
        // Read current (within transaction snapshot if supported by TransactionDB).
        let current = match tx
            .get_cf(self.pinboard_refcounts_cf()?, content_key.as_bytes())
            .map_err(|e| EldError::StorageError {
                operation: "get_pinboard_refcount_with_tx".to_string(),
                details: format!("Failed to fetch pinboard refcount: {e}"),
            })? {
            Some(v) => bincode::deserialize::<u64>(&v).map_err(|e| EldError::StorageError {
                operation: "deserialize_pinboard_refcount".to_string(),
                details: format!("Failed to deserialize pinboard refcount: {e}"),
            })?,
            None => 0,
        };

        let next_i128 = current as i128 + delta as i128;
        if next_i128 < 0 {
            return Err(EldError::StorageError {
                operation: "update_pinboard_refcount_with_tx".to_string(),
                details: format!(
                    "refcount underflow for content_key={content_key}: current={current}, delta={delta}"
                ),
            });
        }
        let next = next_i128 as u64;
        let bytes = bincode::serialize(&next).map_err(|e| EldError::StorageError {
            operation: "serialize_pinboard_refcount".to_string(),
            details: format!("Failed to serialize pinboard refcount: {e}"),
        })?;
        tx.put_cf(self.pinboard_refcounts_cf()?, content_key.as_bytes(), bytes)
            .map_err(|e| EldError::StorageError {
                operation: "set_pinboard_refcount_with_tx".to_string(),
                details: format!("Failed to store pinboard refcount: {e}"),
            })?;
        Ok(next)
    }

    fn delete_pinboard_temp_blob_with_tx(
        &self,
        content_key: &str,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        tx.delete_cf(self.pinboard_temp_blobs_cf()?, content_key.as_bytes())
            .map_err(|e| EldError::StorageError {
                operation: "delete_pinboard_temp_blob_with_tx".to_string(),
                details: format!("Failed to delete temp pinboard blob: {e}"),
            })?;
        Ok(())
    }
}

impl crate::storage::traits::PinboardQueryStorage for RocksDBStorage {
    fn get_pinboard_metadata(
        &self,
        message_id: &str,
    ) -> Result<Option<PinboardMessageMetadata>, EldError> {
        self.get_pinboard_metadata(message_id)
    }

    fn get_pinboard_metadata_by_path_key(
        &self,
        path_key: &str,
    ) -> Result<Option<PinboardMessageMetadata>, EldError> {
        self.get_pinboard_metadata_by_path_key(path_key)
    }

    fn get_pinboard_temp_blob(&self, content_key: &str) -> Result<Option<Vec<u8>>, EldError> {
        self.get_pinboard_temp_blob(content_key)
    }

    fn get_pinboard_message_ids_global_secure(
        &self,
        order: PinboardGlobalFeedOrder,
        options: PrefixQueryOptions,
    ) -> Result<PaginatedResult<String>, EldError> {
        self.get_pinboard_message_ids_global_secure(order, options)
    }

    fn get_pinboard_message_ids_by_tag_secure(
        &self,
        tag: &str,
        options: PrefixQueryOptions,
    ) -> Result<PaginatedResult<String>, EldError> {
        self.get_pinboard_message_ids_by_tag_secure(tag, options)
    }

    fn get_pinboard_message_ids_by_wallet_secure(
        &self,
        wallet: &str,
        options: PrefixQueryOptions,
    ) -> Result<PaginatedResult<String>, EldError> {
        self.get_pinboard_message_ids_by_wallet_secure(wallet, options)
    }

    fn get_pinboard_gc_metrics(&self) -> Result<PinboardGcMetrics, EldError> {
        self.get_pinboard_gc_metrics()
    }
}

impl SnapshotStorage for RocksDBStorage {
    fn put_snapshot_metadata(&self, metadata: &SnapshotMetadata) -> Result<(), EldError> {
        // Hash the height to create a proper hex-encoded name
        let height_hash = sha2::Sha256::digest(metadata.height.to_string());
        let snapshot_metadata_id = format!("0x{}", hex::encode(height_hash));
        let path = CadoPath::new(
            CadoType::SnapshotMetadata,
            CadoPathKey::Name(&snapshot_metadata_id),
        )?;
        let data = bincode::serialize(metadata).map_err(|e| EldError::StorageError {
            operation: "serialize_snapshot_metadata".to_string(),
            details: format!("Failed to serialize SnapshotMetadata: {e}"),
        })?;
        self.put_cado_data(
            path,
            data,
            CADOMetadata::new(CadoType::SnapshotMetadata, "system"),
        )?;
        Ok(())
    }

    fn get_snapshot_metadata(&self, height: i64) -> Result<Option<SnapshotMetadata>, EldError> {
        // Hash the height to create a proper hex-encoded name
        let height_hash = sha2::Sha256::digest(height.to_string());
        let snapshot_metadata_id = format!("0x{}", hex::encode(height_hash));
        let path = CadoPath::new(
            CadoType::SnapshotMetadata,
            CadoPathKey::Name(&snapshot_metadata_id),
        )?;

        match self.get_deserialized_cado_by_path::<SnapshotMetadata>(path) {
            Ok(m) => Ok(Some(m)),
            Err(EldError::NotFoundError { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn get_latest_snapshot_metadata(&self) -> Result<Option<SnapshotMetadata>, EldError> {
        // Get all snapshot metadata CADOs and find the one with highest height
        let paths = self.get_cado_paths_by_prefix(PATH_PREFIX_SNAPSHOT_METADATA)?;
        let mut latest_metadata = None;
        let mut latest_height = i64::MIN;

        for path in paths {
            match self.get_deserialized_cado_by_path::<SnapshotMetadata>(path.clone()) {
                Ok(metadata) => {
                    if metadata.height > latest_height {
                        latest_height = metadata.height;
                        latest_metadata = Some(metadata);
                    }
                }
                Err(EldError::NotFoundError { .. }) => {
                    tracing::warn!(path = %path, "No CADO found for snapshot metadata path during get_latest_snapshot_metadata");
                }
                Err(e) => {
                    tracing::warn!(?e, path = %path, "Failed to deserialize SnapshotMetadata during get_latest_snapshot_metadata");
                }
            }
        }

        Ok(latest_metadata)
    }

    fn list_snapshots(&self, limit: u32) -> Result<Vec<SnapshotMetadata>, EldError> {
        let paths = self.get_cado_paths_by_prefix(PATH_PREFIX_SNAPSHOT_METADATA)?;
        let mut snapshots = Vec::new();

        for path in paths {
            match self.get_deserialized_cado_by_path::<SnapshotMetadata>(path.clone()) {
                Ok(metadata) => {
                    snapshots.push(metadata);
                }
                Err(EldError::NotFoundError { .. }) => {
                    tracing::warn!(path = %path, "No CADO found for snapshot metadata path during list_snapshots");
                }
                Err(e) => {
                    tracing::warn!(?e, path = %path, "Failed to deserialize SnapshotMetadata during list_snapshots");
                }
            }
            if snapshots.len() >= limit as usize {
                break;
            }
        }

        // Sort by height in descending order
        snapshots.sort_by(|a, b| b.height.cmp(&a.height));
        Ok(snapshots)
    }

    fn put_snapshot_chunk(&self, height: i64, chunk: &SnapshotChunk) -> Result<(), EldError> {
        // Hash the height and chunk index to create a proper hex-encoded name
        let chunk_id = format!("{}_{}", height, chunk.index);
        let chunk_hash = sha2::Sha256::digest(chunk_id);
        let snapshot_chunk_id = format!("0x{}", hex::encode(chunk_hash));
        let path = CadoPath::new(
            CadoType::SnapshotChunk,
            CadoPathKey::Name(&snapshot_chunk_id),
        )?;
        let data = chunk.serialize_bin()?;
        self.put_cado_data(
            path,
            data,
            CADOMetadata::new(CadoType::SnapshotChunk, "system"),
        )?;
        Ok(())
    }

    fn get_snapshot_chunk(
        &self,
        height: i64,
        chunk_index: u32,
    ) -> Result<Option<SnapshotChunk>, EldError> {
        // Hash the height and chunk index to create a proper hex-encoded name
        let chunk_id = format!("{height}_{chunk_index}");
        let chunk_hash = sha2::Sha256::digest(chunk_id);
        let snapshot_chunk_id = format!("0x{}", hex::encode(chunk_hash));
        let path = CadoPath::new(
            CadoType::SnapshotChunk,
            CadoPathKey::Name(&snapshot_chunk_id),
        )?;

        match self.get_deserialized_cado_by_path::<SnapshotChunk>(path) {
            Ok(c) => Ok(Some(c)),
            Err(EldError::NotFoundError { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn delete_snapshot_chunk(&self, height: i64, chunk_index: u32) -> Result<(), EldError> {
        // Hash the height and chunk index to create a proper hex-encoded name
        let chunk_id = format!("{height}_{chunk_index}");
        let chunk_hash = sha2::Sha256::digest(chunk_id);
        let snapshot_chunk_id = format!("0x{}", hex::encode(chunk_hash));
        let path = CadoPath::new(
            CadoType::SnapshotChunk,
            CadoPathKey::Name(&snapshot_chunk_id),
        )?;
        self.system_delete_cado(path, "system")?;
        Ok(())
    }

    fn delete_snapshot_metadata(&self, height: i64) -> Result<(), EldError> {
        // Hash the height to create a proper hex-encoded name
        let height_hash = sha2::Sha256::digest(height.to_string());
        let snapshot_metadata_id = format!("0x{}", hex::encode(height_hash));
        let path = CadoPath::new(
            CadoType::SnapshotMetadata,
            CadoPathKey::Name(&snapshot_metadata_id),
        )?;
        self.system_delete_cado(path, "system")?;
        Ok(())
    }
}

#[cfg(test)]
impl crate::storage::traits::SnapshotStorageTestExt for RocksDBStorage {
    fn verify_snapshot(&self, height: i64) -> Result<bool, EldError> {
        let height_hash = sha2::Sha256::digest(height.to_string());
        let snapshot_metadata_id = format!("0x{}", hex::encode(height_hash));
        let metadata_path = CadoPath::new(
            CadoType::SnapshotMetadata,
            CadoPathKey::Name(&snapshot_metadata_id),
        )?;

        if self.get_cado_by_path(metadata_path)?.is_none() {
            return Ok(false);
        }

        let paths =
            self.get_cado_paths_by_prefix(eld_common::constants::cado::PATH_PREFIX_SNAPSHOT_CHUNK)?;
        for path in paths {
            match self.get_deserialized_cado_by_path::<SnapshotChunk>(path.clone()) {
                Ok(chunk) => {
                    let chunk_id = format!("{}_{}", height, chunk.index);
                    let chunk_hash = sha2::Sha256::digest(chunk_id);
                    let snapshot_chunk_id = format!("0x{}", hex::encode(chunk_hash));
                    let expected_path = CadoPath::new(
                        CadoType::SnapshotChunk,
                        CadoPathKey::Name(&snapshot_chunk_id),
                    )?;
                    if path.as_str() == expected_path.as_str() {
                        return Ok(true);
                    }
                }
                Err(EldError::NotFoundError { .. }) => {
                    tracing::warn!(path = %path, "No CADO found for snapshot chunk path during verify_snapshot");
                }
                Err(e) => {
                    tracing::warn!(?e, path = %path, "Failed to deserialize SnapshotChunk during verify_snapshot");
                }
            }
        }

        Ok(false)
    }

    fn get_snapshot_chunk_count(&self, height: i64) -> Result<u32, EldError> {
        let paths =
            self.get_cado_paths_by_prefix(eld_common::constants::cado::PATH_PREFIX_SNAPSHOT_CHUNK)?;
        let mut count = 0;

        for path in paths {
            match self.get_deserialized_cado_by_path::<SnapshotChunk>(path.clone()) {
                Ok(chunk) => {
                    let chunk_id = format!("{}_{}", height, chunk.index);
                    let chunk_hash = sha2::Sha256::digest(chunk_id);
                    let snapshot_chunk_id = format!("0x{}", hex::encode(chunk_hash));
                    let expected_path = CadoPath::new(
                        CadoType::SnapshotChunk,
                        CadoPathKey::Name(&snapshot_chunk_id),
                    )?;
                    if path.as_str() == expected_path.as_str() {
                        count += 1;
                    }
                }
                Err(EldError::NotFoundError { .. }) => {
                    tracing::warn!(path = %path, "No CADO found for snapshot chunk path during get_snapshot_chunk_count");
                }
                Err(e) => {
                    tracing::warn!(?e, path = %path, "Failed to deserialize SnapshotChunk during get_snapshot_chunk_count");
                }
            }
        }

        Ok(count)
    }

    fn snapshot_exists(&self, height: i64) -> Result<bool, EldError> {
        let height_hash = sha2::Sha256::digest(height.to_string());
        let snapshot_metadata_id = format!("0x{}", hex::encode(height_hash));
        let path = CadoPath::new(
            CadoType::SnapshotMetadata,
            CadoPathKey::Name(&snapshot_metadata_id),
        )?;
        Ok(self.get_cado_by_path(path)?.is_some())
    }

    fn prune_snapshots(&self, keep_last_n: u32) -> Result<(), EldError> {
        let snapshots = self.list_snapshots(u32::MAX)?;

        if snapshots.len() <= keep_last_n as usize {
            return Ok(());
        }

        let snapshots_to_delete = &snapshots[keep_last_n as usize..];

        for snapshot in snapshots_to_delete {
            let height_hash = sha2::Sha256::digest(snapshot.height.to_string());
            let snapshot_metadata_id = format!("0x{}", hex::encode(height_hash));
            let metadata_path = CadoPath::new(
                CadoType::SnapshotMetadata,
                CadoPathKey::Name(&snapshot_metadata_id),
            )?;

            if let Err(e) = self.system_delete_cado(metadata_path, "system") {
                tracing::error!(
                    ?e,
                    height = snapshot.height,
                    "Failed to delete snapshot metadata during prune_snapshots"
                );
            }

            let paths = self.get_cado_paths_by_prefix(
                eld_common::constants::cado::PATH_PREFIX_SNAPSHOT_CHUNK,
            )?;

            for path in paths {
                match self.get_deserialized_cado_by_path::<SnapshotChunk>(path.clone()) {
                    Ok(chunk) => {
                        let chunk_id = format!("{}_{}", snapshot.height, chunk.index);
                        let chunk_hash = sha2::Sha256::digest(chunk_id);
                        let snapshot_chunk_id = format!("0x{}", hex::encode(chunk_hash));
                        let expected_path = CadoPath::new(
                            CadoType::SnapshotChunk,
                            CadoPathKey::Name(&snapshot_chunk_id),
                        )?;
                        if path.as_str() == expected_path.as_str() {
                            if let Err(e) = self.system_delete_cado(path, "system") {
                                tracing::error!(
                                    ?e,
                                    chunk_index = chunk.index,
                                    height = snapshot.height,
                                    "Failed to delete snapshot chunk during prune_snapshots"
                                );
                            }
                        }
                    }
                    Err(EldError::NotFoundError { .. }) => {
                        tracing::warn!(path = %path, "No CADO found for snapshot chunk path during prune_snapshots");
                    }
                    Err(e) => {
                        tracing::warn!(?e, path = %path, "Failed to deserialize SnapshotChunk during prune_snapshots");
                    }
                }
            }
        }

        Ok(())
    }
}

impl CADOStorage for RocksDBStorage {
    fn put_cado_type(&self, path: CadoPath, cado_type: CadoBody) -> Result<(), EldError> {
        self.put_cado_type(path, cado_type)
    }
    fn put_cado_data(
        &self,
        path: CadoPath,
        cado_data: Vec<u8>,
        metadata: CADOMetadata,
    ) -> Result<(), EldError> {
        self.put_cado_data(path, cado_data, metadata)
    }
    fn put_cado_map(&self, path: CadoPath, mapping: CADOMap) -> Result<(), EldError> {
        self.put_cado_map(path, mapping)
    }
    fn get_cado_by_path(&self, path: CadoPath) -> Result<Option<CadoBody>, EldError> {
        self.get_cado_by_path(path)
    }
    fn get_cado_paths_by_prefix(&self, prefix: &str) -> Result<Vec<CadoPath>, EldError> {
        self.get_cado_paths_by_prefix(prefix)
    }
    fn get_cados_by_prefix(&self, prefix: &str) -> Result<Vec<CadoBody>, EldError> {
        self.get_cados_by_prefix(prefix)
    }

    fn list_epoch_records_chron(
        &self,
        order: EpochRecordListOrder,
        after_epoch: Option<i64>,
        fetch_limit: usize,
    ) -> Result<Vec<EpochRecord>, EldError> {
        self.list_epoch_records_chron(order, after_epoch, fetch_limit)
    }

    fn count_epoch_records(&self) -> Result<u64, EldError> {
        self.count_epoch_records()
    }

    fn search_cado_hash(&self, prefix: &str) -> Result<Vec<(String, CadoBody)>, EldError> {
        self.search_cado_hash(prefix)
    }
    fn search_cado_name(&self, prefix: &str) -> Result<Vec<(String, CadoBody)>, EldError> {
        self.search_cado_name(prefix)
    }
    fn search_cado_path(&self, prefix: &str) -> Result<Vec<(String, CadoBody)>, EldError> {
        self.search_cado_path(prefix)
    }
    fn get_cado_map(&self, path: CadoPath) -> Result<Option<CADOMap>, EldError> {
        self.get_cado_map(path)
    }
    fn delete_cado(
        &self,
        path: CadoPath,
        owner: &str,
        signature: &str,
        public_key: &str,
        chain_id: &str,
    ) -> Result<(), EldError> {
        self.delete_cado(path, owner, signature, public_key, chain_id)
    }

    fn system_delete_cado(&self, path: CadoPath, owner: &str) -> Result<(), EldError> {
        self.system_delete_cado(path, owner)
    }

    fn system_delete_cado_with_tx(
        &self,
        path: CadoPath,
        owner: &str,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        self.system_delete_cado_with_tx(path, owner, tx)
    }
    // Transaction-aware methods
    fn begin_transaction(&self) -> Transaction<'_, rocksdb::TransactionDB> {
        self.begin_transaction()
    }
    fn put_cado_type_with_tx(
        &self,
        path: CadoPath,
        cado_type: CadoBody,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        self.put_cadotype_by_path_with_tx(path, cado_type, tx)
    }
    fn put_cado_data_with_tx(
        &self,
        path: CadoPath,
        cado_data: Vec<u8>,
        metadata: CADOMetadata,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        self.put_cado_data_with_tx(path, cado_data, metadata, tx)
    }
    fn delete_cado_with_tx(
        &self,
        path: CadoPath,
        owner: &str,
        signature: &str,
        public_key: &str,
        chain_id: &str,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        self.delete_cado_with_tx(path, owner, signature, public_key, chain_id, tx)
    }
    fn put_cado_map_with_tx(
        &self,
        path: CadoPath,
        mapping: CADOMap,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        self.put_cado_map_with_tx(path, mapping, tx)
    }
}

impl crate::storage::traits::VerifiedProofRewardDedupStorage for RocksDBStorage {
    fn is_verified_proof_challenge_rewarded(&self, challenge_id: &str) -> Result<bool, EldError> {
        RocksDBStorage::is_verified_proof_challenge_rewarded(self, challenge_id)
    }

    fn put_verified_proof_challenge_rewarded_with_tx(
        &self,
        challenge_id: &str,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        RocksDBStorage::put_verified_proof_challenge_rewarded_with_tx(self, challenge_id, tx)
    }
}

#[cfg(test)]
impl RocksDBStorage {
    pub fn update_rate_limiter_config(&mut self, max_requests_per_minute: u32) {
        self.rate_limiter = PrefixQueryRateLimiter::new(max_requests_per_minute);
        info!(
            "Updated rate limiter configuration: {} requests per minute",
            max_requests_per_minute
        );
    }
}

use crate::indexer::{IndexedEvent, IndexedTransaction, TransactionStatus};
use crate::storage::traits::TransactionIndexerStorage;
use abci::types::Event;
use eld_common::tx::{HasSender, PayloadInner, Tx};
use serde_json;
use std::str;

impl TransactionIndexerStorage for RocksDBStorage {
    /// Calculate transaction ID from transaction
    fn calculate_tx_id(&self, tx: &Tx) -> String {
        // Serialize transaction to JSON (canonical form with signature)
        // Using JSON instead of bincode because transactions contain variable-length sequences
        // that bincode's default config can't handle
        let serialized = serde_json::to_vec(tx).expect("Failed to serialize transaction");
        let hash = Sha256::digest(&serialized);
        format!("0x{}", hex::encode(hash))
    }

    /// Index a transaction with all secondary indexes
    fn index_transaction(
        &self,
        tx: &Tx,
        block_height: u64,
        block_index: u32,
        status: TransactionStatus,
    ) -> Result<(), EldError> {
        let tx_id = self.calculate_tx_id(tx);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| EldError::StorageError {
                operation: "get_timestamp".to_string(),
                details: format!("Failed to get timestamp: {e}"),
            })?
            .as_secs();

        let indexed_tx = IndexedTransaction {
            id: tx_id.clone(),
            block_height,
            block_index,
            timestamp,
            tx: tx.clone(),
            status,
            gas_used: None,     // TODO: Extract from response if available
            events: Vec::new(), // TODO: Extract events from response if available
        };

        // Use JSON serialization since IndexedTransaction contains Tx which has variable-length sequences
        let serialized = serde_json::to_vec(&indexed_tx).map_err(|e| EldError::StorageError {
            operation: "serialize_indexed_transaction".to_string(),
            details: format!("Failed to serialize indexed transaction: {e}"),
        })?;

        let cf = self.indexed_transactions_cf()?;

        let primary_already_present = self
            .db
            .get_cf(cf, tx_id.as_bytes())
            .map_err(|e| EldError::StorageError {
                operation: "get_indexed_transaction_primary_exists".to_string(),
                details: format!("Failed to check indexed transaction existence: {e}"),
            })?
            .is_some();

        // Store primary key: tx_id -> IndexedTransaction
        self.db
            .put_cf(cf, tx_id.as_bytes(), &serialized)
            .map_err(|e| EldError::StorageError {
                operation: "put_indexed_transaction".to_string(),
                details: format!("Failed to store indexed transaction: {e}"),
            })?;

        // Store secondary index: block_height:tx_index -> tx_id
        let block_index_key = format!("block_height:{block_height}:tx_index:{block_index}");
        self.db
            .put_cf(cf, block_index_key.as_bytes(), tx_id.as_bytes())
            .map_err(|e| EldError::StorageError {
                operation: "put_block_index_key".to_string(),
                details: format!("Failed to store block index key: {e}"),
            })?;

        let chron_key = Self::block_position_chron_index_key_bytes(block_height, block_index);
        self.db
            .put_cf(cf, &chron_key, tx_id.as_bytes())
            .map_err(|e| EldError::StorageError {
                operation: "put_block_pos_chron_index_key".to_string(),
                details: format!("Failed to store block_pos chronological index: {e}"),
            })?;

        // Store secondary index: sender:nonce -> tx_id
        let sender = tx.payload.inner.sender();
        let sender_index_key = format!("sender:{}:nonce:{}", sender, tx.nonce.value());
        self.db
            .put_cf(cf, sender_index_key.as_bytes(), tx_id.as_bytes())
            .map_err(|e| EldError::StorageError {
                operation: "put_sender_index_key".to_string(),
                details: format!("Failed to store sender index key: {e}"),
            })?;

        // Store secondary index: type:payload_type:tx_id -> tx_id
        let payload_type = &tx.payload.r#type;
        let type_index_key = format!("type:{payload_type}:{tx_id}");
        self.db
            .put_cf(cf, type_index_key.as_bytes(), tx_id.as_bytes())
            .map_err(|e| EldError::StorageError {
                operation: "put_type_index_key".to_string(),
                details: format!("Failed to store type index key: {e}"),
            })?;

        if status == TransactionStatus::Success {
            if let PayloadInner::VerifiedProof(vp_tx) = &tx.payload.inner {
                let provider_key = vp_tx.capacity_provider.to_string().to_lowercase();
                let vp_key =
                    Self::verified_proof_reward_index_key(&provider_key, block_height, block_index);
                self.db
                    .put_cf(cf, &vp_key, [])
                    .map_err(|e| EldError::StorageError {
                        operation: "put_verified_proof_reward_index".to_string(),
                        details: format!("Failed to store verified proof reward index: {e}"),
                    })?;
                self.bump_verified_proof_global_rewards_count(cf)?;
            }
        }

        debug!(
            tx_id = %tx_id,
            block_height = block_height,
            block_index = block_index,
            "Indexed transaction"
        );

        if !primary_already_present {
            self.bump_indexed_transaction_primary_total_count()?;
        }

        Ok(())
    }

    /// Get a transaction by ID
    fn get_indexed_transaction(&self, tx_id: &str) -> Result<Option<IndexedTransaction>, EldError> {
        let cf = self.indexed_transactions_cf()?;

        // Normalize tx_id: ensure it has "0x" prefix to match how it's stored
        // Transactions are stored with the full "0x" prefix from calculate_tx_id
        let normalized_tx_id = if tx_id.starts_with("0x") {
            tx_id.to_string()
        } else {
            // If no prefix, add it (though API should always provide it)
            format!("0x{tx_id}")
        };

        let value = self
            .db
            .get_cf(cf, normalized_tx_id.as_bytes())
            .map_err(|e| EldError::StorageError {
                operation: "get_indexed_transaction".to_string(),
                details: format!("Failed to get indexed transaction: {e}"),
            })?;

        match value {
            Some(data) => {
                // Use JSON deserialization since IndexedTransaction is stored as JSON
                let indexed_tx: IndexedTransaction =
                    serde_json::from_slice(&data).map_err(|e| EldError::StorageError {
                        operation: "deserialize_indexed_transaction".to_string(),
                        details: format!("Failed to deserialize indexed transaction: {e}"),
                    })?;
                Ok(Some(indexed_tx))
            }
            None => Ok(None),
        }
    }

    fn get_indexed_transactions_primary_total(&self) -> Result<u64, EldError> {
        self.indexed_transaction_primary_total_count()
    }

    fn list_indexed_transactions_chron_desc(
        &self,
        cursor_exclusive: Option<(u64, u32)>,
        limit: u32,
        fetch_one_extra_row: bool,
        block_height: Option<u64>,
        sender: Option<Address>,
        payload_type: Option<&str>,
    ) -> Result<Vec<IndexedTransaction>, EldError> {
        let extra = u32::from(fetch_one_extra_row);
        let fetch_limit = (limit.saturating_add(extra) as usize).max(1);
        self.list_indexed_transactions_chron_desc_scanned(
            cursor_exclusive,
            IndexedTxChronFilters {
                block_height,
                sender,
                payload_type,
            },
            fetch_limit,
        )
    }

    fn count_indexed_transactions_matching_filters_chron_scan(
        &self,
        block_height: Option<u64>,
        sender: Option<Address>,
        payload_type: Option<&str>,
    ) -> Result<u64, EldError> {
        self.count_indexed_transactions_matching_chron_filters(IndexedTxChronFilters {
            block_height,
            sender,
            payload_type,
        })
    }

    /// Index an event with all secondary indexes
    fn index_event(
        &self,
        tx_id: &str,
        event_index: u32,
        event: &Event,
        block_height: u64,
        block_index: u32,
    ) -> Result<(), EldError> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| EldError::StorageError {
                operation: "get_timestamp".to_string(),
                details: format!("Failed to get timestamp: {e}"),
            })?
            .as_secs();

        // Convert ABCI Event to IndexedEvent format
        // Event attributes are bytes, convert to String tuples
        let attributes: Vec<(String, String)> = event
            .attributes
            .iter()
            .map(|attr| {
                let key = String::from_utf8_lossy(&attr.key).to_string();
                let value = String::from_utf8_lossy(&attr.value).to_string();
                (key, value)
            })
            .collect();

        let event_type = event.r#type.clone();

        let indexed_event = IndexedEvent {
            tx_id: tx_id.to_string(),
            event_index,
            event_type: event_type.clone(),
            attributes,
            block_height,
            block_index,
            timestamp,
        };

        // Use JSON serialization (same as transactions)
        let serialized =
            serde_json::to_vec(&indexed_event).map_err(|e| EldError::StorageError {
                operation: "serialize_indexed_event".to_string(),
                details: format!("Failed to serialize indexed event: {e}"),
            })?;

        let cf = self.indexed_events_cf()?;

        // Store primary key: tx_id:event_index -> IndexedEvent
        let primary_key = format!("tx_id:{tx_id}:event_index:{event_index}");
        self.db
            .put_cf(cf, primary_key.as_bytes(), &serialized)
            .map_err(|e| EldError::StorageError {
                operation: "put_indexed_event".to_string(),
                details: format!("Failed to store indexed event: {e}"),
            })?;

        // Store secondary index: type:event_type:block_height:block_index:event_index -> tx_id:event_index
        let type_index_key = format!(
            "type:{event_type}:block_height:{block_height}:block_index:{block_index}:event_index:{event_index}"
        );
        self.db
            .put_cf(cf, type_index_key.as_bytes(), primary_key.as_bytes())
            .map_err(|e| EldError::StorageError {
                operation: "put_type_index_key".to_string(),
                details: format!("Failed to store type index key: {e}"),
            })?;

        // Store secondary index: block_height:block_index:event_index -> tx_id:event_index
        let block_index_key = format!(
            "block_height:{block_height}:block_index:{block_index}:event_index:{event_index}"
        );
        self.db
            .put_cf(cf, block_index_key.as_bytes(), primary_key.as_bytes())
            .map_err(|e| EldError::StorageError {
                operation: "put_block_index_key".to_string(),
                details: format!("Failed to store block index key: {e}"),
            })?;

        debug!(
            tx_id = %tx_id,
            event_index = event_index,
            event_type = %event_type,
            block_height = block_height,
            block_index = block_index,
            "Indexed event"
        );

        Ok(())
    }

    /// Get all events for a transaction
    fn get_events_by_tx_id(&self, tx_id: &str) -> Result<Vec<IndexedEvent>, EldError> {
        let cf = self.indexed_events_cf()?;

        // Normalize tx_id: ensure it has "0x" prefix to match how it's stored
        let normalized_tx_id = if tx_id.starts_with("0x") {
            tx_id.to_string()
        } else {
            format!("0x{tx_id}")
        };

        let prefix = format!("tx_id:{normalized_tx_id}:event_index:");
        let mut events = Vec::new();

        // Iterate through all keys starting from the beginning and filter by prefix
        // This is more reliable than IteratorMode::From for prefix matching
        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::Start);

        for item in iter {
            let (key, value) = item.map_err(|e| EldError::StorageError {
                operation: "iterate_events_by_tx_id".to_string(),
                details: format!("Failed to iterate events: {e}"),
            })?;

            let key_str = String::from_utf8_lossy(&key);

            // Only process keys that start with our prefix
            if key_str.starts_with(&prefix) {
                // Deserialize the event
                let indexed_event: IndexedEvent =
                    serde_json::from_slice(&value).map_err(|e| EldError::StorageError {
                        operation: "deserialize_indexed_event".to_string(),
                        details: format!("Failed to deserialize indexed event: {e}"),
                    })?;
                events.push(indexed_event);
            } else if key_str.as_ref() > prefix.as_str() {
                // Since keys are sorted, if we've passed the prefix, we can stop
                // This optimization only works if the prefix is a valid key prefix
                // For our case, keys are sorted lexicographically, so this is safe
                break;
            }
        }

        // Sort by event_index to ensure correct order
        events.sort_by_key(|e| e.event_index);

        Ok(events)
    }

    /// Get all events (paginated, descending chronological order)
    fn get_all_events(&self, page: u32, limit: u32) -> Result<(Vec<IndexedEvent>, u64), EldError> {
        let cf = self.indexed_events_cf()?;
        let offset = ((page - 1) * limit) as u64;

        let prefix = "block_height:";
        let mut events = Vec::new();
        let mut count = 0u64;
        let mut skipped = 0u64;

        // Use reverse iteration to get events in descending chronological order (newest first)
        // Start from the end and iterate backwards through block_height keys
        let iter = self.db.iterator_cf(cf, rocksdb::IteratorMode::End);

        for item in iter {
            let (key, value) = item.map_err(|e| EldError::StorageError {
                operation: "iterate_all_events".to_string(),
                details: format!("Failed to iterate events: {e}"),
            })?;

            let key_str = String::from_utf8_lossy(&key);

            // Only process block_height index keys (secondary index for chronological ordering)
            if key_str.starts_with(prefix)
                && key_str.contains(":block_index:")
                && key_str.contains(":event_index:")
            {
                count += 1;

                if skipped < offset {
                    skipped += 1;
                    continue;
                }

                if events.len() < limit as usize {
                    // Get the primary key reference
                    let primary_key = String::from_utf8_lossy(&value);

                    // Fetch the actual event data using the primary key
                    let event_data = self.db.get_cf(cf, primary_key.as_bytes()).map_err(|e| {
                        EldError::StorageError {
                            operation: "get_event_by_primary_key".to_string(),
                            details: format!("Failed to get event data: {e}"),
                        }
                    })?;

                    if let Some(data) = event_data {
                        let indexed_event: IndexedEvent =
                            serde_json::from_slice(&data).map_err(|e| EldError::StorageError {
                                operation: "deserialize_indexed_event".to_string(),
                                details: format!("Failed to deserialize indexed event: {e}"),
                            })?;
                        events.push(indexed_event);
                    }
                }
            }
        }

        Ok((events, count))
    }

    fn aggregate_verified_proof_rewards(
        &self,
        provider: &str,
        from_height: u64,
        to_height: u64,
    ) -> Result<(u64, u128), EldError> {
        use eld_common::constants::protocol::VERIFIED_PROOF_REWARD_BASE_AMOUNT;

        let addr = Address::parse_hex_str(provider)?;
        let normalized = addr.hex_with_prefix().to_lowercase();
        let lower = Self::verified_proof_reward_index_key(&normalized, from_height, 0);
        let prefix = format!("vp:{normalized}:").into_bytes();
        let cf = self.indexed_transactions_cf()?;
        let iter = self
            .db
            .iterator_cf(cf, IteratorMode::From(lower.as_slice(), Direction::Forward));
        let mut count = 0u64;
        for item in iter {
            let (key, _) = item.map_err(|e| EldError::StorageError {
                operation: "aggregate_verified_proof_rewards_iterate".to_string(),
                details: e.to_string(),
            })?;
            if !key.starts_with(&prefix) {
                break;
            }
            let key_str = String::from_utf8_lossy(&key);
            let Some((_addr_key, h, _bi)) = Self::parse_verified_proof_reward_index_key(&key_str)
            else {
                continue;
            };
            if h < from_height {
                continue;
            }
            if h > to_height {
                break;
            }
            count += 1;
        }
        let total = (count as u128).saturating_mul(VERIFIED_PROOF_REWARD_BASE_AMOUNT);
        Ok((count, total))
    }

    fn global_verified_proof_rewards(&self) -> Result<(u64, u128), EldError> {
        use eld_common::constants::protocol::VERIFIED_PROOF_REWARD_BASE_AMOUNT;

        let cf = self.indexed_transactions_cf()?;
        let count = self.read_verified_proof_global_rewards_count(cf)?;
        let total = (count as u128).saturating_mul(VERIFIED_PROOF_REWARD_BASE_AMOUNT);
        Ok((count, total))
    }

    fn get_indexer_cursor(&self) -> Result<Option<u64>, EldError> {
        const KEY: &[u8] = b"meta:indexer:last_synced_height";
        let cf = self.indexed_transactions_cf()?;
        let Some(bytes) = self
            .db
            .get_cf(cf, KEY)
            .map_err(|e| EldError::StorageError {
                operation: "get_indexer_cursor".to_string(),
                details: format!("Failed to read indexer cursor: {e}"),
            })?
        else {
            return Ok(None);
        };
        let s = std::str::from_utf8(&bytes).map_err(|e| EldError::StorageError {
            operation: "get_indexer_cursor".to_string(),
            details: format!("Invalid UTF-8 in indexer cursor: {e}"),
        })?;
        s.parse::<u64>()
            .map(Some)
            .map_err(|e| EldError::StorageError {
                operation: "get_indexer_cursor".to_string(),
                details: format!("Invalid indexer cursor value {s:?}: {e}"),
            })
    }

    fn set_indexer_cursor(&self, height: u64) -> Result<(), EldError> {
        const KEY: &[u8] = b"meta:indexer:last_synced_height";
        let cf = self.indexed_transactions_cf()?;
        self.db
            .put_cf(cf, KEY, height.to_string().as_bytes())
            .map_err(|e| EldError::StorageError {
                operation: "set_indexer_cursor".to_string(),
                details: format!("Failed to write indexer cursor: {e}"),
            })
    }

    fn block_position_indexed(
        &self,
        block_height: u64,
        block_index: u32,
    ) -> Result<bool, EldError> {
        let cf = self.indexed_transactions_cf()?;
        let key = Self::block_position_chron_index_key_bytes(block_height, block_index);
        self.db
            .get_cf(cf, &key)
            .map(|opt| opt.is_some())
            .map_err(|e| EldError::StorageError {
                operation: "block_position_indexed".to_string(),
                details: format!(
                    "Failed to read block_pos index at {block_height}:{block_index}: {e}"
                ),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::pagination::PaginationParams;
    use crate::storage::traits::PinboardStorage;
    use crate::storage::traits::SnapshotStorageTestExt;
    use crate::storage::traits::{SnapshotChunk, SnapshotChunkMetadata, SnapshotMetadata};
    use eld_common::constants::cado::{
        PATH_PREFIX_ACCOUNT, PATH_PREFIX_APP_STATE_TIP, PATH_PREFIX_CADO_MAP,
        PATH_PREFIX_CHUNK_REFERENCE, PATH_PREFIX_OTHER_SCOPE, PATH_PREFIX_SNAPSHOT_CHUNK,
        PATH_PREFIX_SNAPSHOT_METADATA, PATH_PREFIX_STAKING_ACCOUNT, PATH_PREFIX_TEST_SCOPE,
        SCOPE_PUBLIC, SCOPE_TEST, SCOPE_USER, TYPE_CADO_MAP, TYPE_CONTENT_MANIFEST,
        TYPE_SNAPSHOT_CHUNK,
    };
    use tempfile::TempDir;

    fn create_test_storage() -> RocksDBStorage {
        let temp_dir = TempDir::new().unwrap();
        RocksDBStorage::new(temp_dir.path()).unwrap()
    }

    fn put_test_epoch_record(storage: &RocksDBStorage, epoch: i64) {
        use eld_common::cado::{epoch_record_path_name, CADOMetadata, CadoPathKey, CadoType};
        use eld_common::constants::cado::LATEST;
        use eld_common::validator::EpochRecord;

        let name = epoch_record_path_name(epoch).expect("epoch name");
        let path =
            CadoPath::new(CadoType::EpochRecord, CadoPathKey::Name(&name)).expect("epoch path");
        let record = EpochRecord {
            epoch,
            start_block: epoch * 10,
            active_validators: vec![],
            active_capacity_validator: None,
            challenged_capacity_validators: vec![],
        };
        let bytes = record.serialize_bin().expect("serialize");
        let cado = CadoBody::immutable(
            bytes,
            CADOMetadata::new(CadoType::EpochRecord, epoch.to_string()),
        );
        storage
            .put_cado_type(path, cado.clone())
            .expect("put epoch");

        let latest_path =
            CadoPath::new(CadoType::EpochRecord, CadoPathKey::Name(LATEST)).expect("latest path");
        storage
            .put_cado_type(latest_path, cado)
            .expect("put latest");
    }

    #[test]
    fn list_epoch_records_chron_desc_asc_and_cursor() {
        let storage = create_test_storage();
        for epoch in [0i64, 2, 5, 10] {
            put_test_epoch_record(&storage, epoch);
        }

        assert_eq!(storage.count_epoch_records().unwrap(), 4);

        let desc = storage
            .list_epoch_records_chron(EpochRecordListOrder::Desc, None, 10)
            .unwrap();
        assert_eq!(
            desc.iter().map(|r| r.epoch).collect::<Vec<_>>(),
            vec![10, 5, 2, 0]
        );

        let page1 = storage
            .list_epoch_records_chron(EpochRecordListOrder::Desc, None, 2)
            .unwrap();
        assert_eq!(page1.len(), 2);
        assert_eq!(page1[0].epoch, 10);
        assert_eq!(page1[1].epoch, 5);

        let page2 = storage
            .list_epoch_records_chron(EpochRecordListOrder::Desc, Some(5), 10)
            .unwrap();
        assert_eq!(
            page2.iter().map(|r| r.epoch).collect::<Vec<_>>(),
            vec![2, 0]
        );

        let asc = storage
            .list_epoch_records_chron(EpochRecordListOrder::Asc, None, 10)
            .unwrap();
        assert_eq!(
            asc.iter().map(|r| r.epoch).collect::<Vec<_>>(),
            vec![0, 2, 5, 10]
        );

        let asc_page2 = storage
            .list_epoch_records_chron(EpochRecordListOrder::Asc, Some(2), 10)
            .unwrap();
        assert_eq!(
            asc_page2.iter().map(|r| r.epoch).collect::<Vec<_>>(),
            vec![5, 10]
        );
    }

    #[test]
    fn verified_proof_submission_claim_is_idempotent_per_sender_and_challenge() {
        let s = create_test_storage();
        assert!(!s
            .has_verified_proof_submission_claim("0xAbCd", "challenge-a")
            .unwrap());
        assert!(s
            .insert_verified_proof_submission_claim("0xAbCd", "challenge-a")
            .unwrap());
        assert!(s
            .has_verified_proof_submission_claim("0xabcd", "challenge-a")
            .unwrap());
        assert!(!s
            .insert_verified_proof_submission_claim("0xabcd", "challenge-a")
            .unwrap());
        assert!(s
            .insert_verified_proof_submission_claim("0xabcd", "challenge-b")
            .unwrap());
    }

    #[test]
    fn verified_proof_challenge_rewarded_dedup_persists() {
        let s = create_test_storage();
        assert!(!s
            .is_verified_proof_challenge_rewarded("challenge-x")
            .unwrap());

        let tx = s.begin_transaction();
        s.put_verified_proof_challenge_rewarded_with_tx("challenge-x", &tx)
            .unwrap();
        tx.commit().unwrap();

        assert!(s
            .is_verified_proof_challenge_rewarded("challenge-x")
            .unwrap());
    }

    const TEST_ADDR_20: &str = "0x1234567890123456789012345678901234567890";
    const TEST_HASH_32A: &str =
        "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
    const TEST_HASH_32B: &str =
        "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
    const TEST_HASH_DEAD: &str =
        "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
    /// 32-byte hex id in `0x` + 64 nibbles form (used for multi-scope path fixtures).
    const TEST_HASH_32_CONTRACT_STYLE: &str =
        "0x1234567890123456789012345678901234567890123456789012345678901234";
    /// Prefix using a scope that is not in [`ALLOWED_SCOPES`], for negative tests only.
    const PREFIX_MALICIOUS_ACCOUNT: &str = "/@malicious/account/";
    /// Forbidden scope label used in path-validation tests (not an allowed [`SCOPE_ELD_ROOT`]-family scope).
    const SCOPE_REJECTED_SYSTEM: &str = "@system";

    #[test]
    fn test_pinboard_metadata_roundtrip_and_indexes() {
        let storage = create_test_storage();

        let addr = Address::parse_hex_str("0x0123456789abcdef0123456789abcdef01234567").unwrap();
        let meta = PinboardMessageMetadata {
            message_id: "msg1".to_string(),
            original_signer: addr,
            content_key: "deadbeef".to_string(),
            content_type: "application/json".to_string(),
            expires_height: 50,
            visibility: "Public".to_string(),
            topic: Some("general".to_string()),
            tags: vec!["dapp".to_string()],
            committed_height: 10,
            received_timestamp: 1_710_000_000,
            namespace: None,
        };

        let tx = storage.begin_transaction();
        storage
            .put_pinboard_metadata_with_tx(&meta.message_id, &meta, &tx)
            .unwrap();
        storage
            .put_pinboard_wallet_index_with_tx(
                &addr.hex_with_prefix(),
                meta.committed_height,
                &meta.message_id,
                &tx,
            )
            .unwrap();
        storage
            .put_pinboard_tag_index_with_tx(
                &meta.tags[0],
                meta.committed_height,
                &meta.message_id,
                &tx,
            )
            .unwrap();
        storage
            .put_pinboard_expiry_index_with_tx(meta.expires_height, &meta.message_id, &tx)
            .unwrap();
        tx.commit().unwrap();

        let got = storage
            .get_pinboard_metadata(&meta.message_id)
            .unwrap()
            .unwrap();
        assert_eq!(got, meta);
    }

    #[test]
    fn test_pinboard_namespace_secondary_path_key() {
        use eld_common::pinboard::pinboard_namespace_content_path;

        let storage = create_test_storage();
        let addr = Address::parse_hex_str("0x0123456789abcdef0123456789abcdef01234567").unwrap();
        let meta = PinboardMessageMetadata {
            message_id: "msgcaptain".to_string(),
            original_signer: addr,
            content_key: "cafebabe".to_string(),
            content_type: "text/plain".to_string(),
            expires_height: 50,
            visibility: "Public".to_string(),
            topic: None,
            tags: vec![],
            committed_height: 10,
            received_timestamp: 1_710_000_000,
            namespace: Some("captainhook".to_string()),
        };

        let path_key = pinboard_namespace_content_path("captainhook", "msgcaptain");
        let tx = storage.begin_transaction();
        storage
            .put_pinboard_metadata_with_tx(&meta.message_id, &meta, &tx)
            .unwrap();
        tx.commit().unwrap();

        assert_eq!(
            storage.get_pinboard_metadata("msgcaptain").unwrap(),
            Some(meta.clone())
        );
        assert_eq!(
            storage
                .get_pinboard_metadata_by_path_key(&path_key)
                .unwrap(),
            Some(meta)
        );
        assert!(storage
            .get_pinboard_metadata_by_path_key("/@eld/pinboard/post/0x0/msgcaptain")
            .unwrap()
            .is_none());
    }

    #[test]
    fn test_pinboard_refcount_update() {
        let storage = create_test_storage();
        let ck = "aa";
        let tx = storage.begin_transaction();
        assert_eq!(
            storage
                .update_pinboard_refcount_with_tx(ck, 1, &tx)
                .unwrap(),
            1
        );
        assert_eq!(
            storage
                .update_pinboard_refcount_with_tx(ck, 2, &tx)
                .unwrap(),
            3
        );
        assert_eq!(
            storage
                .update_pinboard_refcount_with_tx(ck, -1, &tx)
                .unwrap(),
            2
        );
        tx.commit().unwrap();

        let raw = storage
            .db
            .get_cf(storage.pinboard_refcounts_cf().unwrap(), ck.as_bytes())
            .unwrap()
            .unwrap();
        let persisted: u64 = bincode::deserialize(&raw).unwrap();
        assert_eq!(persisted, 2);
    }

    #[test]
    fn test_pinboard_list_by_tag_paginated() {
        let storage = create_test_storage();

        let tx = storage.begin_transaction();
        storage
            .put_pinboard_tag_index_with_tx("dapp", 10, "m1", &tx)
            .unwrap();
        storage
            .put_pinboard_tag_index_with_tx("dapp", 11, "m2", &tx)
            .unwrap();
        storage
            .put_pinboard_tag_index_with_tx("dapp", 12, "m3", &tx)
            .unwrap();
        storage
            .put_pinboard_tag_index_with_tx("other", 10, "x1", &tx)
            .unwrap();
        tx.commit().unwrap();

        let page0 = storage
            .get_pinboard_message_ids_by_tag_secure(
                "dapp",
                PrefixQueryOptions::default().with_pagination(PaginationParams {
                    page: 0,
                    page_size: 2,
                    cursor: None,
                }),
            )
            .unwrap();
        assert_eq!(page0.items, vec!["m1".to_string(), "m2".to_string()]);
        assert!(page0.has_more);

        let page1 = storage
            .get_pinboard_message_ids_by_tag_secure(
                "dapp",
                PrefixQueryOptions::default().with_pagination(PaginationParams {
                    page: 1,
                    page_size: 2,
                    cursor: None,
                }),
            )
            .unwrap();
        assert_eq!(page1.items, vec!["m3".to_string()]);
        assert!(!page1.has_more);
    }

    #[test]
    fn test_pinboard_list_global_paginated_ordering() {
        let storage = create_test_storage();

        let tx = storage.begin_transaction();
        storage
            .put_pinboard_commit_index_with_tx(10, "m1", &tx)
            .unwrap();
        storage
            .put_pinboard_commit_index_with_tx(11, "m2", &tx)
            .unwrap();
        storage
            .put_pinboard_commit_index_with_tx(12, "m3", &tx)
            .unwrap();
        tx.commit().unwrap();

        let asc_page0 = storage
            .get_pinboard_message_ids_global_secure(
                PinboardGlobalFeedOrder::Asc,
                PrefixQueryOptions::default().with_pagination(PaginationParams {
                    page: 0,
                    page_size: 2,
                    cursor: None,
                }),
            )
            .unwrap();
        assert_eq!(asc_page0.items, vec!["m1".to_string(), "m2".to_string()]);
        assert!(asc_page0.has_more);

        let asc_page1 = storage
            .get_pinboard_message_ids_global_secure(
                PinboardGlobalFeedOrder::Asc,
                PrefixQueryOptions::default().with_pagination(PaginationParams {
                    page: 1,
                    page_size: 2,
                    cursor: None,
                }),
            )
            .unwrap();
        assert_eq!(asc_page1.items, vec!["m3".to_string()]);
        assert!(!asc_page1.has_more);

        let desc_page0 = storage
            .get_pinboard_message_ids_global_secure(
                PinboardGlobalFeedOrder::Desc,
                PrefixQueryOptions::default().with_pagination(PaginationParams {
                    page: 0,
                    page_size: 2,
                    cursor: None,
                }),
            )
            .unwrap();
        assert_eq!(desc_page0.items, vec!["m3".to_string(), "m2".to_string()]);
        assert!(desc_page0.has_more);
    }

    #[test]
    fn test_snapshot_storage() {
        let storage = create_test_storage();
        let metadata = SnapshotMetadata {
            height: 100,
            epoch: 1,
            format_version: 1,
            chunk_count: 5,
            total_size: 1024,
            compression: "zstd".to_string(),
            created_at: 1234567890,
            app_hash: vec![1, 2, 3, 4],
            chunk_hashes: vec!["hash1".to_string(), "hash2".to_string()],
        };

        // Test put and get
        storage.put_snapshot_metadata(&metadata).unwrap();
        let retrieved = storage.get_snapshot_metadata(100).unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().height, 100);

        // Test chunk storage
        let chunk = SnapshotChunk {
            index: 0,
            data: vec![1, 2, 3, 4, 5],
            hash: "chunk_hash".to_string(),
            size: 5,
            metadata: SnapshotChunkMetadata {
                accounts_count: 10,
                staking_accounts_count: 5,
                devices_count: 3,
                manifests_count: 2,
                chunk_proofs_count: 1,
            },
        };

        storage.put_snapshot_chunk(100, &chunk).unwrap();
        let retrieved_chunk = storage.get_snapshot_chunk(100, 0).unwrap();
        assert!(retrieved_chunk.is_some());
        assert_eq!(retrieved_chunk.unwrap().data, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_secure_prefix_query_paths() {
        let storage = create_test_storage();

        // Create some test CADOs
        let test_paths = vec![
            format!(
                "{}{}/{}",
                PATH_PREFIX_TEST_SCOPE, TYPE_CADO_MAP, TEST_HASH_32A
            ),
            format!(
                "{}{}/{}",
                PATH_PREFIX_TEST_SCOPE, TYPE_CADO_MAP, TEST_HASH_32B
            ),
            format!(
                "{}{}/0x1111111111111111111111111111111111111111111111111111111111111111",
                PATH_PREFIX_TEST_SCOPE, TYPE_CADO_MAP
            ),
            format!(
                "{}{}/0x2222222222222222222222222222222222222222222222222222222222222222",
                PATH_PREFIX_OTHER_SCOPE, TYPE_CADO_MAP
            ),
            format!(
                "{}{}/0x3333333333333333333333333333333333333333333333333333333333333333",
                PATH_PREFIX_TEST_SCOPE, TYPE_CADO_MAP
            ),
        ];

        for path_str in &test_paths {
            let path = CadoPath::parse(path_str.as_str()).unwrap();
            let cado_type = CadoBody::immutable(
                vec![1, 2, 3],
                CADOMetadata::new(CadoType::CadoMap, "test_owner"),
            );
            storage.put_cado_type(path, cado_type).unwrap();
        }

        // Test secure prefix query with default options
        let options = PrefixQueryOptions::default();
        let result = storage
            .get_cado_paths_by_prefix_secure(PATH_PREFIX_TEST_SCOPE.trim_end_matches('/'), options)
            .unwrap();

        assert_eq!(result.items.len(), 4); // Should find 4 paths under test scope prefix
        assert!(!result.has_more); // Should not have more pages
        assert_eq!(result.page, 0);
        assert_eq!(result.page_size, 100);
    }

    #[test]
    fn test_secure_prefix_query_cados() {
        let storage = create_test_storage();

        // Create a single test CADO with unique data
        let path = CadoPath::parse(&format!(
            "{PATH_PREFIX_TEST_SCOPE}{TYPE_CADO_MAP}/{TEST_HASH_32A}"
        ))
        .unwrap();
        let cado_type = CadoBody::immutable(
            vec![1, 2, 3, 4, 5], // Unique data
            CADOMetadata::new(CadoType::CadoMap, "test_owner"),
        );
        storage.put_cado_type(path, cado_type).unwrap();

        // Test secure prefix query
        let options = PrefixQueryOptions::default();
        let result = storage
            .get_cados_by_prefix_secure(PATH_PREFIX_TEST_SCOPE.trim_end_matches('/'), options)
            .unwrap();

        assert_eq!(result.items.len(), 1); // Should find 1 CADO

        // Check the data
        match &result.items[0] {
            CadoBody::Immutable(cado) => assert_eq!(cado.data(), vec![1, 2, 3, 4, 5]),
            _ => panic!("Expected immutable CADO"),
        }
    }

    #[test]
    fn test_pagination() {
        let storage = create_test_storage();

        // Create 25 test CADOs
        for i in 0..25 {
            let path_str = format!("{PATH_PREFIX_TEST_SCOPE}{TYPE_CADO_MAP}/0x{i:064x}");
            let path = CadoPath::parse(&path_str).unwrap();
            let cado_type = CadoBody::immutable(
                vec![i as u8],
                CADOMetadata::new(CadoType::CadoMap, "test_owner"),
            );
            storage.put_cado_type(path, cado_type).unwrap();
        }

        // Test first page (10 items per page)
        let pagination = PaginationParams {
            page: 0,
            page_size: 10,
            cursor: None,
        };
        let options = PrefixQueryOptions::default().with_pagination(pagination);
        let result = storage
            .get_cado_paths_by_prefix_secure(PATH_PREFIX_TEST_SCOPE.trim_end_matches('/'), options)
            .unwrap();

        assert_eq!(result.items.len(), 10);
        assert!(result.has_more); // Should have more pages
        assert_eq!(result.page, 0);

        // Test second page
        let pagination = PaginationParams {
            page: 1,
            page_size: 10,
            cursor: None,
        };
        let options = PrefixQueryOptions::default().with_pagination(pagination);
        let result = storage
            .get_cado_paths_by_prefix_secure(PATH_PREFIX_TEST_SCOPE.trim_end_matches('/'), options)
            .unwrap();

        assert_eq!(result.items.len(), 10);
        assert!(result.has_more); // Should have more pages
        assert_eq!(result.page, 1);

        // Test third page
        let pagination = PaginationParams {
            page: 2,
            page_size: 10,
            cursor: None,
        };
        let options = PrefixQueryOptions::default().with_pagination(pagination);
        let result = storage
            .get_cado_paths_by_prefix_secure(PATH_PREFIX_TEST_SCOPE.trim_end_matches('/'), options)
            .unwrap();

        assert_eq!(result.items.len(), 5); // Should have 5 remaining items
        assert!(!result.has_more); // Should not have more pages
        assert_eq!(result.page, 2);
    }

    #[test]
    fn test_memory_limits() {
        let storage = create_test_storage();

        // Create a large CADO
        let path = CadoPath::parse(&format!(
            "{PATH_PREFIX_TEST_SCOPE}{TYPE_CADO_MAP}/{TEST_HASH_32A}"
        ))
        .unwrap();
        let large_data = vec![0u8; 1024 * 1024]; // 1MB of data
        let cado_type = CadoBody::immutable(
            large_data,
            CADOMetadata::new(CadoType::CadoMap, "test_owner"),
        );
        storage.put_cado_type(path, cado_type).unwrap();

        // Test with very low memory limit
        let options = PrefixQueryOptions::with_limits(
            1000,
            1024, // Only 1KB limit
            "test_client".to_string(),
        );

        let result = storage
            .get_cados_by_prefix_secure(PATH_PREFIX_TEST_SCOPE.trim_end_matches('/'), options);
        // Should succeed but with memory limit warning in logs
        assert!(result.is_ok());
    }

    #[test]
    fn test_result_limits() {
        let storage = create_test_storage();

        // Create many test CADOs
        for i in 0..50 {
            let path_str = format!("{PATH_PREFIX_TEST_SCOPE}{TYPE_CADO_MAP}/0x{i:064x}");
            let path = CadoPath::parse(&path_str).unwrap();
            let cado_type = CadoBody::immutable(
                vec![i as u8],
                CADOMetadata::new(CadoType::CadoMap, "test_owner"),
            );
            storage.put_cado_type(path, cado_type).unwrap();
        }

        // Test with low result limit
        let pagination = PaginationParams {
            page: 0,
            page_size: 10, // Small page size
            cursor: None,
        };
        let options = PrefixQueryOptions::with_limits(
            10, // Only 10 results max
            50 * 1024 * 1024,
            "test_client".to_string(),
        )
        .with_pagination(pagination);

        let result = storage
            .get_cado_paths_by_prefix_secure(PATH_PREFIX_TEST_SCOPE.trim_end_matches('/'), options)
            .unwrap();
        assert_eq!(result.items.len(), 10); // Should be limited to 10 results
    }

    #[test]
    fn test_snapshot_manager_multiple_snapshots() {
        let storage = create_test_storage();

        // Create multiple snapshots
        for height in 100..105 {
            let metadata = SnapshotMetadata {
                height,
                epoch: 1,
                format_version: 1,
                chunk_count: 2,
                total_size: 1024,
                compression: "zstd".to_string(),
                created_at: 1234567890,
                app_hash: vec![height as u8],
                chunk_hashes: vec!["hash1".to_string(), "hash2".to_string()],
            };
            storage.put_snapshot_metadata(&metadata).unwrap();

            // Add chunks for each snapshot
            for chunk_index in 0..2 {
                let chunk = SnapshotChunk {
                    index: chunk_index,
                    data: vec![height as u8, chunk_index as u8],
                    hash: format!("hash_{height}_{chunk_index}"),
                    size: 2,
                    metadata: SnapshotChunkMetadata {
                        accounts_count: 10,
                        staking_accounts_count: 5,
                        devices_count: 3,
                        manifests_count: 2,
                        chunk_proofs_count: 1,
                    },
                };
                storage.put_snapshot_chunk(height, &chunk).unwrap();
            }
        }

        // Test list snapshots
        let snapshots = storage.list_snapshots(10).unwrap();
        assert_eq!(snapshots.len(), 5);

        // Test get latest snapshot
        let latest = storage.get_latest_snapshot_metadata().unwrap();
        assert!(latest.is_some());
        assert_eq!(latest.unwrap().height, 104);

        // Test prune snapshots
        storage.prune_snapshots(2).unwrap();
        let snapshots = storage.list_snapshots(10).unwrap();
        assert_eq!(snapshots.len(), 2); // Should keep only the latest 2
    }

    #[test]
    fn test_snapshot_manager_retrieval_edge_cases() {
        let storage = create_test_storage();

        // Test getting non-existent snapshot
        let result = storage.get_snapshot_metadata(999).unwrap();
        assert!(result.is_none());

        // Test getting non-existent chunk
        let result = storage.get_snapshot_chunk(999, 0).unwrap();
        assert!(result.is_none());

        // Test snapshot exists
        let exists = storage.snapshot_exists(999).unwrap();
        assert!(!exists);

        // Create a snapshot and test
        let metadata = SnapshotMetadata {
            height: 100,
            epoch: 1,
            format_version: 1,
            chunk_count: 1,
            total_size: 512,
            compression: "zstd".to_string(),
            created_at: 1234567890,
            app_hash: vec![1, 2, 3, 4],
            chunk_hashes: vec!["hash1".to_string()],
        };
        storage.put_snapshot_metadata(&metadata).unwrap();

        let exists = storage.snapshot_exists(100).unwrap();
        assert!(exists);

        // Test chunk count
        let count = storage.get_snapshot_chunk_count(100).unwrap();
        assert_eq!(count, 0); // No chunks added yet

        // Add a chunk and test count
        let chunk = SnapshotChunk {
            index: 0,
            data: vec![1, 2, 3],
            hash: "chunk_hash".to_string(),
            size: 3,
            metadata: SnapshotChunkMetadata {
                accounts_count: 5,
                staking_accounts_count: 2,
                devices_count: 1,
                manifests_count: 1,
                chunk_proofs_count: 0,
            },
        };
        storage.put_snapshot_chunk(100, &chunk).unwrap();

        let count = storage.get_snapshot_chunk_count(100).unwrap();
        assert_eq!(count, 1); // Now has one chunk
    }

    #[test]
    fn test_cado_path_validation_security() {
        // Test valid paths
        let valid_path = CadoPath::parse(&format!("{PATH_PREFIX_ACCOUNT}{TEST_ADDR_20}")).unwrap();
        assert!(RocksDBStorage::validate_cado_path_security(&valid_path).is_ok());

        // Test reserved scope names - create a path with reserved scope
        let reserved_scope_path =
            CadoPath::parse(&format!("/{SCOPE_REJECTED_SYSTEM}/account/{TEST_ADDR_20}")).unwrap();
        assert!(RocksDBStorage::validate_cado_path_security(&reserved_scope_path).is_err());
    }

    #[test]
    fn test_cado_path_validation_in_storage_functions() {
        let storage = create_test_storage();

        // Test put_cado_data with reserved scope path
        let malicious_path =
            CadoPath::parse(&format!("/{SCOPE_REJECTED_SYSTEM}/account/{TEST_ADDR_20}")).unwrap();
        let metadata = CADOMetadata::new(
            CadoType::Account,
            "0x1234567890123456789012345678901234567890",
        );
        let result = storage.put_cado_data(malicious_path, vec![1, 2, 3], metadata);
        assert!(result.is_err());

        // Test get_cado_by_path with reserved scope path
        let malicious_path2 =
            CadoPath::parse(&format!("/{SCOPE_REJECTED_SYSTEM}/account/{TEST_ADDR_20}")).unwrap();
        let result = storage.get_cado_by_path(malicious_path2);
        assert!(result.is_err());

        // Test put_cado_data_with_tx with reserved scope path
        let malicious_path3 =
            CadoPath::parse(&format!("/{SCOPE_REJECTED_SYSTEM}/account/{TEST_ADDR_20}")).unwrap();
        let tx = storage.begin_transaction();
        let metadata = CADOMetadata::new(
            CadoType::Account,
            "0x1234567890123456789012345678901234567890",
        );
        let result = storage.put_cado_data_with_tx(malicious_path3, vec![1, 2, 3], metadata, &tx);
        assert!(result.is_err());
    }

    #[test]
    fn test_snapshot_operations_with_corrupted_data() {
        use tracing_subscriber::fmt::Subscriber;
        let _ = Subscriber::builder().with_test_writer().try_init();

        let storage = create_test_storage();
        // Insert a valid snapshot
        let metadata = SnapshotMetadata {
            height: 1,
            epoch: 1,
            format_version: 1,
            chunk_count: 1,
            total_size: 100,
            compression: "zstd".to_string(),
            created_at: 1234567890,
            app_hash: vec![1, 2, 3],
            chunk_hashes: vec!["hash1".to_string()],
        };
        storage.put_snapshot_metadata(&metadata).unwrap();

        // Insert a corrupted snapshot (invalid bincode data)
        let corrupted_path =
            CadoPath::parse(&format!("{PATH_PREFIX_SNAPSHOT_METADATA}{TEST_HASH_DEAD}")).unwrap();
        let corrupted_data = vec![0xde, 0xad, 0xbe, 0xef, 0x00, 0x01]; // Not valid bincode
        let corrupted_meta = CADOMetadata::new(CadoType::SnapshotMetadata, "system");
        storage
            .put_cado_data(corrupted_path, corrupted_data, corrupted_meta)
            .unwrap();

        // list_snapshots should not panic and should return the valid snapshot
        let snapshots = storage.list_snapshots(10).unwrap();
        assert!(snapshots.iter().any(|m| m.height == 1));
        // prune_snapshots should not panic
        storage.prune_snapshots(0).unwrap();
    }

    #[test]
    fn test_enhanced_cado_path_validation() {
        // Test valid paths
        let valid_paths = [
            format!("{PATH_PREFIX_ACCOUNT}{TEST_ADDR_20}"),
            format!("/{SCOPE_USER}/{TYPE_CADO_MAP}/{TEST_HASH_32_CONTRACT_STYLE}"),
            format!("/{SCOPE_PUBLIC}/{TYPE_CADO_MAP}/{TEST_HASH_32_CONTRACT_STYLE}"),
            format!("/{SCOPE_TEST}/{TYPE_CADO_MAP}/{TEST_HASH_32_CONTRACT_STYLE}"),
        ];

        for path_str in &valid_paths {
            let path = CadoPath::parse(path_str).unwrap();
            assert!(
                RocksDBStorage::validate_cado_path_security_enhanced(&path).is_ok(),
                "Valid path should pass: {path_str}"
            );
        }

        // Test forbidden scopes (these should be rejected by enhanced validation)
        let forbidden_scopes = ["@system", "@admin", "@root", "@internal"];
        for scope in forbidden_scopes.iter() {
            let path_str = format!("/{scope}/account/{TEST_ADDR_20}");
            let path = CadoPath::parse(&path_str).unwrap();
            assert!(
                RocksDBStorage::validate_cado_path_security_enhanced(&path).is_err(),
                "Forbidden scope should be rejected: {scope}"
            );
        }

        // Test invalid types (this will be caught by CadoPath::new, so we test the enhanced validation directly)
        // Create a path with a valid type first, then test the enhanced validation logic
        let valid_path = CadoPath::parse(&format!("{PATH_PREFIX_ACCOUNT}{TEST_ADDR_20}")).unwrap();
        assert!(
            RocksDBStorage::validate_cado_path_security_enhanced(&valid_path).is_ok(),
            "Valid path should pass enhanced validation"
        );

        // Test path length limits (create a path that's too long)
        // We need to create a path that passes CadoPath::new but fails our length validation
        // Let's test this by creating a path with a very long scope name
        let long_scope = "@".to_string() + &"a".repeat(500);
        let long_path = format!("/{long_scope}/account/0x1234567890123456789012345678901234567890");
        let path = CadoPath::parse(&long_path).unwrap();
        assert!(
            RocksDBStorage::validate_cado_path_security_enhanced(&path).is_err(),
            "Path exceeding length limit should be rejected"
        );

        // Test that the original validation still works for basic cases
        let reserved_scope_path =
            CadoPath::parse(&format!("/{SCOPE_REJECTED_SYSTEM}/account/{TEST_ADDR_20}")).unwrap();
        assert!(RocksDBStorage::validate_cado_path_security(&reserved_scope_path).is_err());
    }

    #[test]
    fn test_cado_prefix_validation() {
        // Test valid prefixes
        let valid_prefixes = vec![
            PATH_PREFIX_ACCOUNT.to_string(),
            format!("/{}/{}/", SCOPE_USER, TYPE_CONTENT_MANIFEST),
            format!("/{}/{}/", SCOPE_PUBLIC, TYPE_CADO_MAP),
        ];

        for prefix in &valid_prefixes {
            assert!(
                RocksDBStorage::validate_cado_prefix_security(prefix).is_ok(),
                "Valid prefix should pass: {prefix}"
            );
        }

        // Test malicious prefixes
        let malicious_prefixes = vec![
            format!("{}../", PATH_PREFIX_ACCOUNT),
            format!("{}..%2f", PATH_PREFIX_ACCOUNT),
            format!("{}//", PATH_PREFIX_ACCOUNT.trim_end_matches('/')),
            format!("{}~", PATH_PREFIX_ACCOUNT),
            "invalid_prefix".to_string(), // doesn't start with /
        ];

        for prefix in &malicious_prefixes {
            assert!(
                RocksDBStorage::validate_cado_prefix_security(prefix).is_err(),
                "Malicious prefix should be rejected: {prefix}"
            );
        }

        // Test prefix length limits
        let long_prefix = format!("{}{}", PATH_PREFIX_ACCOUNT, "a".repeat(300));
        assert!(
            RocksDBStorage::validate_cado_prefix_security(&long_prefix).is_err(),
            "Prefix exceeding length limit should be rejected"
        );

        // Test control characters in prefix
        let control_char_prefix = format!("{PATH_PREFIX_ACCOUNT}\x00");
        assert!(
            RocksDBStorage::validate_cado_prefix_security(&control_char_prefix).is_err(),
            "Prefix with null bytes should be rejected"
        );
    }

    #[test]
    fn test_rate_limiting() {
        let storage = create_test_storage();

        // Test that we can make requests up to the limit
        for i in 0..60 {
            let result = storage.get_cado_paths_by_prefix(PATH_PREFIX_TEST_SCOPE);
            assert!(result.is_ok(), "Request {i} should succeed");
        }

        // Test that the 61st request is rate limited
        let result = storage.get_cado_paths_by_prefix(PATH_PREFIX_TEST_SCOPE);
        assert!(result.is_err(), "61st request should be rate limited");
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Rate limit exceeded"));

        // Test that requests for different prefixes are tracked separately
        let result = storage.get_cado_paths_by_prefix(PATH_PREFIX_OTHER_SCOPE);
        assert!(
            result.is_ok(),
            "Different prefix should not be rate limited"
        );
    }

    #[test]
    fn test_rate_limiting_with_different_clients() {
        let storage = create_test_storage();

        // Test that different client IDs are tracked separately in secure methods
        let options1 = PrefixQueryOptions {
            client_id: "client1".to_string(),
            ..Default::default()
        };
        let options2 = PrefixQueryOptions {
            client_id: "client2".to_string(),
            ..Default::default()
        };

        // Client 1 should be able to make 60 requests
        for i in 0..60 {
            let result =
                storage.get_cado_paths_by_prefix_secure(PATH_PREFIX_TEST_SCOPE, options1.clone());
            assert!(result.is_ok(), "Client 1 request {i} should succeed");
        }

        // Client 1 should be rate limited on the 61st request
        let result =
            storage.get_cado_paths_by_prefix_secure(PATH_PREFIX_TEST_SCOPE, options1.clone());
        assert!(
            result.is_err(),
            "Client 1 61st request should be rate limited"
        );

        // Client 2 should still be able to make requests
        let result =
            storage.get_cado_paths_by_prefix_secure(PATH_PREFIX_TEST_SCOPE, options2.clone());
        assert!(result.is_ok(), "Client 2 should not be rate limited");
    }

    #[test]
    fn test_rate_limit_configuration() {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut storage = RocksDBStorage::new(temp_dir.path()).unwrap();

        // Test configuration update (default is 60; tighten to 30)
        storage.update_rate_limiter_config(30);

        // Test that the new limit is enforced
        for i in 0..30 {
            let result = storage.get_cado_paths_by_prefix(PATH_PREFIX_TEST_SCOPE);
            assert!(result.is_ok(), "Request {i} should succeed");
        }

        // Test that the 31st request is rate limited
        let result = storage.get_cado_paths_by_prefix(PATH_PREFIX_TEST_SCOPE);
        assert!(result.is_err(), "31st request should be rate limited");
    }

    #[test]
    fn test_rate_limiting_all_prefix_methods() {
        let storage = create_test_storage();

        // Test each prefix-based method individually
        // get_cado_paths_by_prefix
        for i in 0..60 {
            let result = storage.get_cado_paths_by_prefix(PATH_PREFIX_TEST_SCOPE);
            assert!(
                result.is_ok(),
                "get_cado_paths_by_prefix request {i} should succeed"
            );
        }
        let result = storage.get_cado_paths_by_prefix(PATH_PREFIX_TEST_SCOPE);
        assert!(
            result.is_err(),
            "get_cado_paths_by_prefix 61st request should be rate limited"
        );
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Rate limit exceeded"));

        // Reset rate limiter for next test
        let mut storage = create_test_storage();
        storage.update_rate_limiter_config(60);

        // get_cados_by_prefix
        for i in 0..60 {
            let result = storage.get_cados_by_prefix(PATH_PREFIX_TEST_SCOPE);
            assert!(
                result.is_ok(),
                "get_cados_by_prefix request {i} should succeed"
            );
        }
        let result = storage.get_cados_by_prefix(PATH_PREFIX_TEST_SCOPE);
        assert!(
            result.is_err(),
            "get_cados_by_prefix 61st request should be rate limited"
        );
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Rate limit exceeded"));

        // Reset rate limiter for next test
        let mut storage = create_test_storage();
        storage.update_rate_limiter_config(60);

        // search_cado_path
        for i in 0..60 {
            let result = storage.search_cado_path(PATH_PREFIX_TEST_SCOPE);
            assert!(
                result.is_ok(),
                "search_cado_path request {i} should succeed"
            );
        }
        let result = storage.search_cado_path(PATH_PREFIX_TEST_SCOPE);
        assert!(
            result.is_err(),
            "search_cado_path 61st request should be rate limited"
        );
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Rate limit exceeded"));

        // Reset rate limiter for next test
        let mut storage = create_test_storage();
        storage.update_rate_limiter_config(60);

        // search_cado_hash
        for i in 0..60 {
            let result = storage.search_cado_hash(PATH_PREFIX_TEST_SCOPE);
            assert!(
                result.is_ok(),
                "search_cado_hash request {i} should succeed"
            );
        }
        let result = storage.search_cado_hash(PATH_PREFIX_TEST_SCOPE);
        assert!(
            result.is_err(),
            "search_cado_hash 61st request should be rate limited"
        );
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Rate limit exceeded"));

        // Reset rate limiter for next test
        let mut storage = create_test_storage();
        storage.update_rate_limiter_config(60);

        // search_cado_name
        for i in 0..60 {
            let result = storage.search_cado_name(PATH_PREFIX_TEST_SCOPE);
            assert!(
                result.is_ok(),
                "search_cado_name request {i} should succeed"
            );
        }
        let result = storage.search_cado_name(PATH_PREFIX_TEST_SCOPE);
        assert!(
            result.is_err(),
            "search_cado_name 61st request should be rate limited"
        );
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Rate limit exceeded"));
    }

    #[test]
    fn test_rate_limiter_cleanup() {
        let storage = create_test_storage();

        // Make some requests
        for i in 0..10 {
            let result = storage.get_cado_paths_by_prefix(PATH_PREFIX_TEST_SCOPE);
            assert!(result.is_ok(), "Request {i} should succeed");
        }

        // Verify that requests are tracked
        assert_eq!(
            storage
                .rate_limiter
                .get_request_count("default", PATH_PREFIX_TEST_SCOPE),
            10
        );

        // Clean up old entries
        storage.rate_limiter.cleanup();

        // The cleanup should not affect current requests
        assert_eq!(
            storage
                .rate_limiter
                .get_request_count("default", PATH_PREFIX_TEST_SCOPE),
            10
        );
    }

    #[test]
    fn test_all_cado_operations_validation() {
        let storage = create_test_storage();

        // Create a malicious path that should be rejected by all operations
        let malicious_path =
            CadoPath::parse(&format!("/{SCOPE_REJECTED_SYSTEM}/account/{TEST_ADDR_20}")).unwrap();
        let valid_path = CadoPath::parse(&format!("{PATH_PREFIX_ACCOUNT}{TEST_ADDR_20}")).unwrap();

        // Test all CADO operations that should be protected
        let metadata = CADOMetadata::new(
            CadoType::Account,
            "0x1234567890123456789012345678901234567890",
        );
        let mapping = CADOMap {
            from: "from".to_string(),
            to: "to".to_string(),
        };
        let cado_type = CadoBody::mutable_new(vec![1, 2, 3], metadata.clone());

        // Test put_cado_map
        assert!(storage
            .put_cado_map(malicious_path.clone(), mapping.clone())
            .is_err());
        assert!(storage
            .put_cado_map(valid_path.clone(), mapping.clone())
            .is_ok());

        // Test get_cado_map
        assert!(storage.get_cado_map(malicious_path.clone()).is_err());
        assert!(storage.get_cado_map(valid_path.clone()).is_ok());

        // Test put_cado_type
        assert!(storage
            .put_cado_type(malicious_path.clone(), cado_type.clone())
            .is_err());
        assert!(storage
            .put_cado_type(valid_path.clone(), cado_type.clone())
            .is_ok());

        // Test delete_cado (requires valid signature, but path validation should happen first)
        assert!(storage
            .delete_cado(
                malicious_path.clone(),
                "owner",
                "signature",
                "public_key",
                "chain_id"
            )
            .is_err());

        // Test system_delete_cado
        assert!(storage
            .system_delete_cado(malicious_path.clone(), "owner")
            .is_err());

        // Test transaction-based operations
        let tx = storage.begin_transaction();
        assert!(storage
            .put_cado_map_with_tx(malicious_path.clone(), mapping.clone(), &tx)
            .is_err());
        assert!(storage
            .put_cadotype_by_path_with_tx(malicious_path.clone(), cado_type.clone(), &tx)
            .is_err());
        assert!(storage
            .delete_cado_with_tx(
                malicious_path.clone(),
                "owner",
                "signature",
                "public_key",
                "chain_id",
                &tx
            )
            .is_err());

        // Test prefix-based operations with malicious scope
        assert!(storage.search_cado_path(PREFIX_MALICIOUS_ACCOUNT).is_err());
        assert!(storage
            .get_cado_paths_by_prefix(PREFIX_MALICIOUS_ACCOUNT)
            .is_err());
        assert!(storage
            .get_cados_by_prefix(PREFIX_MALICIOUS_ACCOUNT)
            .is_err());

        // Test valid prefix operations
        assert!(storage.search_cado_path(PATH_PREFIX_ACCOUNT).is_ok());
        assert!(storage
            .get_cado_paths_by_prefix(PATH_PREFIX_ACCOUNT)
            .is_ok());
        assert!(storage.get_cados_by_prefix(PATH_PREFIX_ACCOUNT).is_ok());
    }

    #[test]
    fn test_cado_deletion_validation() {
        let storage = create_test_storage();

        // Test system-deletable types (should be allowed for system deletion)
        let system_deletable_paths = vec![
            format!("{}{}", PATH_PREFIX_CHUNK_REFERENCE, TEST_HASH_32A),
            format!("{}{}", PATH_PREFIX_SNAPSHOT_CHUNK, TEST_HASH_32B),
            format!("{}{}", PATH_PREFIX_SNAPSHOT_METADATA, TEST_HASH_32A),
        ];

        for path_str in system_deletable_paths {
            let path = CadoPath::parse(&path_str).unwrap();

            // Should be rejected for user deletion
            assert!(!path.is_user_deletable());
            assert!(path.validate_user_deletion().is_err());

            // Should be allowed for system deletion
            assert!(path.is_system_deletable());
            assert!(path.validate_system_deletion().is_ok());

            // Should be deletable overall
            assert!(path.is_deletable());
            assert!(!path.is_protected());
        }

        // Test protected types (should not be deletable at all)
        let protected_paths = vec![
            format!("{}{}", PATH_PREFIX_ACCOUNT, TEST_ADDR_20),
            format!("{}{}", PATH_PREFIX_STAKING_ACCOUNT, TEST_ADDR_20),
            format!("{}{}", PATH_PREFIX_CADO_MAP, TEST_HASH_32A),
            format!("{}{}", PATH_PREFIX_APP_STATE_TIP, TEST_HASH_32A),
        ];

        for path_str in protected_paths {
            let path = CadoPath::parse(&path_str).unwrap();

            // Should be rejected for both user and system deletion
            assert!(!path.is_user_deletable());
            assert!(!path.is_system_deletable());
            assert!(path.validate_user_deletion().is_err());
            assert!(path.validate_system_deletion().is_err());

            // Should not be deletable overall
            assert!(!path.is_deletable());
            assert!(path.is_protected());
        }

        // Test system deletion scope validation
        let non_chain_system_path = CadoPath::parse(&format!(
            "/{SCOPE_USER}/{TYPE_SNAPSHOT_CHUNK}/{TEST_HASH_32A}"
        ))
        .unwrap();
        assert!(non_chain_system_path.is_system_deletable());
        assert!(non_chain_system_path.validate_system_deletion().is_err()); // Wrong scope

        // Test storage layer validation
        let protected_path =
            CadoPath::parse(&format!("{PATH_PREFIX_ACCOUNT}{TEST_ADDR_20}")).unwrap();

        // Test user deletion validation in storage
        assert!(storage
            .delete_cado(
                protected_path.clone(),
                "owner",
                "signature",
                "public_key",
                "chain_id"
            )
            .is_err());

        // Test system deletion validation in storage
        assert!(storage
            .system_delete_cado(protected_path.clone(), "owner")
            .is_err());
        // Note: system_deletable_path would fail due to CADO not existing, but validation should pass
    }

    #[test]
    fn system_delete_cado_with_tx_commits_atomically() {
        let storage = create_test_storage();
        let height = 100i64;
        let chunk = SnapshotChunk {
            index: 0,
            data: vec![1, 2, 3],
            hash: "0xabc".to_string(),
            size: 3,
            metadata: SnapshotChunkMetadata {
                accounts_count: 0,
                staking_accounts_count: 0,
                devices_count: 0,
                manifests_count: 0,
                chunk_proofs_count: 0,
            },
        };
        storage.put_snapshot_chunk(height, &chunk).unwrap();
        assert!(storage.get_snapshot_chunk(height, 0).unwrap().is_some());

        let chunk_id = format!("{height}_{}", chunk.index);
        let chunk_hash = Sha256::digest(chunk_id);
        let snapshot_chunk_id = format!("0x{}", hex::encode(chunk_hash));
        let path = CadoPath::new(
            CadoType::SnapshotChunk,
            CadoPathKey::Name(&snapshot_chunk_id),
        )
        .unwrap();

        let tx = storage.begin_transaction();
        storage
            .system_delete_cado_with_tx(path, "system", &tx)
            .unwrap();
        tx.commit().unwrap();

        assert!(storage.get_snapshot_chunk(height, 0).unwrap().is_none());
    }

    #[test]
    fn system_delete_cado_with_tx_rollback_leaves_cado_intact() {
        let storage = create_test_storage();
        let height = 200i64;
        let chunk = SnapshotChunk {
            index: 1,
            data: vec![4, 5, 6],
            hash: "0xdef".to_string(),
            size: 3,
            metadata: SnapshotChunkMetadata {
                accounts_count: 0,
                staking_accounts_count: 0,
                devices_count: 0,
                manifests_count: 0,
                chunk_proofs_count: 0,
            },
        };
        storage.put_snapshot_chunk(height, &chunk).unwrap();
        assert!(storage.get_snapshot_chunk(height, 1).unwrap().is_some());

        let chunk_id = format!("{height}_{}", chunk.index);
        let chunk_hash = Sha256::digest(chunk_id);
        let snapshot_chunk_id = format!("0x{}", hex::encode(chunk_hash));
        let path = CadoPath::new(
            CadoType::SnapshotChunk,
            CadoPathKey::Name(&snapshot_chunk_id),
        )
        .unwrap();

        let tx = storage.begin_transaction();
        storage
            .system_delete_cado_with_tx(path, "system", &tx)
            .unwrap();
        drop(tx);

        assert!(storage.get_snapshot_chunk(height, 1).unwrap().is_some());
    }

    #[test]
    fn test_block_pos_chron_desc_order_newest_first_and_continuation_tuple() {
        use crate::indexer::TransactionStatus;
        use crate::storage::traits::TransactionIndexerStorage;
        use ed25519_dalek::SigningKey;
        use eld_common::constants::{protocol::DEFAULT_TX_FEE, test::MOCK_CHAIN_ID};
        use eld_common::tx::{Payload, TransferTx, Tx, TxPublicKey, TxSig};

        fn transfer_tx_fixture(signing_key: &SigningKey, nonce: u32) -> Tx {
            let verifying_key = signing_key.verifying_key();
            let sender = Address::from_public_key(&verifying_key).unwrap();
            let recipient =
                Address::parse_hex_str("0x0987654321098765432109876543210987654321").unwrap();
            let inner = TransferTx::new(sender, recipient, 1.into()).unwrap();
            let mut tx = Tx {
                sig: TxSig::empty(),
                nonce: nonce.into(),
                payload: Payload::new(inner),
                public_key: TxPublicKey::from(&verifying_key),
                fee: DEFAULT_TX_FEE.into(),
            };
            tx.sign(signing_key, MOCK_CHAIN_ID).expect("sign");
            tx
        }

        let storage = create_test_storage();
        let secret = SigningKey::from_bytes(&[11u8; 32]);

        storage
            .index_transaction(
                &transfer_tx_fixture(&secret, 1),
                9,
                0,
                TransactionStatus::Success,
            )
            .unwrap();
        storage
            .index_transaction(
                &transfer_tx_fixture(&secret, 2),
                10,
                0,
                TransactionStatus::Success,
            )
            .unwrap();

        let all = storage
            .list_indexed_transactions_chron_desc(None, 50, false, None, None, None)
            .unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].block_height, 10);
        assert_eq!(all[1].block_height, 9);

        let page_probe = storage
            .list_indexed_transactions_chron_desc(None, 1, true, None, None, None)
            .unwrap();
        assert_eq!(page_probe.len(), 2);

        let page2 = storage
            .list_indexed_transactions_chron_desc(Some((10, 0)), 10, false, None, None, None)
            .unwrap();
        assert_eq!(page2.len(), 1);
        assert_eq!(page2[0].block_height, 9);
    }

    #[test]
    fn test_verified_proof_reward_index_key_parse() {
        let k = RocksDBStorage::verified_proof_reward_index_key(
            "0x1234567890123456789012345678901234567890",
            5,
            3,
        );
        let key_str = String::from_utf8(k).unwrap();
        let parsed = RocksDBStorage::parse_verified_proof_reward_index_key(&key_str).unwrap();
        assert_eq!(
            parsed.0,
            "0x1234567890123456789012345678901234567890".to_string()
        );
        assert_eq!(parsed.1, 5);
        assert_eq!(parsed.2, 3);
    }

    #[test]
    fn test_aggregate_verified_proof_rewards_by_height_range() {
        use crate::storage::traits::TransactionIndexerStorage;
        use eld_common::constants::protocol::VERIFIED_PROOF_REWARD_BASE_AMOUNT;

        let temp_dir = TempDir::new().unwrap();
        let storage = RocksDBStorage::new(temp_dir.path()).unwrap();
        let cf = storage.indexed_transactions_cf().unwrap();
        let addr = "0x1234567890123456789012345678901234567890";
        let norm = addr.to_lowercase();
        for (h, i) in [(5u64, 0u32), (5, 1), (7, 0), (10, 0)] {
            let key = RocksDBStorage::verified_proof_reward_index_key(&norm, h, i);
            storage.db.put_cf(cf, key, []).unwrap();
        }
        let other = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_lowercase();
        storage
            .db
            .put_cf(
                cf,
                RocksDBStorage::verified_proof_reward_index_key(&other, 6, 0),
                [],
            )
            .unwrap();

        let (c, t) = storage
            .aggregate_verified_proof_rewards(addr, 5, 7)
            .unwrap();
        assert_eq!(c, 3);
        assert_eq!(t, 3 * VERIFIED_PROOF_REWARD_BASE_AMOUNT);
    }

    #[test]
    fn test_global_verified_proof_rewards_zero_when_unset() {
        use crate::storage::traits::TransactionIndexerStorage;

        let storage = create_test_storage();
        let (c, t) = storage.global_verified_proof_rewards().unwrap();
        assert_eq!(c, 0);
        assert_eq!(t, 0);
    }

    #[test]
    fn test_index_verified_proof_success_bumps_global_rollup() {
        use crate::indexer::TransactionStatus;
        use crate::storage::traits::TransactionIndexerStorage;
        use ed25519_dalek::SigningKey;
        use eld_common::constants::{
            protocol::VERIFIED_PROOF_REWARD_BASE_AMOUNT, test::MOCK_CHAIN_ID,
        };
        use eld_common::tx::{Payload, Tx, TxPublicKey, TxSig, VerifiedProofTx};

        fn verified_proof_tx_fixture(signing_key: &SigningKey, nonce: u32) -> Tx {
            use eld_common::capacity_proof::{ChunkProof, SlotState};

            let verifying_key = signing_key.verifying_key();
            let sender = Address::from_public_key(&verifying_key).unwrap();
            let provider = Address::parse_hex_str(TEST_ADDR_20).unwrap();
            let inner = VerifiedProofTx::new(
                sender,
                provider,
                "challenge-1".to_string(),
                1,
                2,
                1_600_000_000,
                vec![ChunkProof {
                    chunk_index: 0,
                    chunk_data: vec![1],
                    chunk_hash: [2u8; 32],
                    merkle_proof: vec![],
                    slot_state: SlotState::Proof,
                }],
                1_600_000_000,
                hex::encode([3u8; 32]),
                hex::encode([4u8; 64]),
            )
            .unwrap();
            let mut tx = Tx {
                sig: TxSig::empty(),
                nonce: nonce.into(),
                payload: Payload::new(inner),
                public_key: TxPublicKey::from(&verifying_key),
                fee: 0.into(),
            };
            tx.sign(signing_key, MOCK_CHAIN_ID).expect("sign");
            tx
        }

        let storage = create_test_storage();
        let secret = SigningKey::from_bytes(&[29u8; 32]);

        storage
            .index_transaction(
                &verified_proof_tx_fixture(&secret, 1),
                100,
                0,
                TransactionStatus::Success,
            )
            .unwrap();
        storage
            .index_transaction(
                &verified_proof_tx_fixture(&secret, 2),
                100,
                1,
                TransactionStatus::Success,
            )
            .unwrap();

        let (c, t) = storage.global_verified_proof_rewards().unwrap();
        assert_eq!(c, 2);
        assert_eq!(t, 2 * VERIFIED_PROOF_REWARD_BASE_AMOUNT);
    }

    #[test]
    fn test_index_verified_proof_failed_does_not_bump_global_rollup() {
        use crate::indexer::TransactionStatus;
        use crate::storage::traits::TransactionIndexerStorage;
        use ed25519_dalek::SigningKey;
        use eld_common::constants::test::MOCK_CHAIN_ID;
        use eld_common::tx::{Payload, Tx, TxPublicKey, TxSig, VerifiedProofTx};

        fn verified_proof_tx_fixture(signing_key: &SigningKey, nonce: u32) -> Tx {
            use eld_common::capacity_proof::{ChunkProof, SlotState};

            let verifying_key = signing_key.verifying_key();
            let sender = Address::from_public_key(&verifying_key).unwrap();
            let provider = Address::parse_hex_str(TEST_ADDR_20).unwrap();
            let inner = VerifiedProofTx::new(
                sender,
                provider,
                "challenge-1".to_string(),
                1,
                2,
                1_600_000_000,
                vec![ChunkProof {
                    chunk_index: 0,
                    chunk_data: vec![1],
                    chunk_hash: [2u8; 32],
                    merkle_proof: vec![],
                    slot_state: SlotState::Proof,
                }],
                1_600_000_000,
                hex::encode([3u8; 32]),
                hex::encode([4u8; 64]),
            )
            .unwrap();
            let mut tx = Tx {
                sig: TxSig::empty(),
                nonce: nonce.into(),
                payload: Payload::new(inner),
                public_key: TxPublicKey::from(&verifying_key),
                fee: 0.into(),
            };
            tx.sign(signing_key, MOCK_CHAIN_ID).expect("sign");
            tx
        }

        let storage = create_test_storage();
        let secret = SigningKey::from_bytes(&[31u8; 32]);

        storage
            .index_transaction(
                &verified_proof_tx_fixture(&secret, 1),
                200,
                0,
                TransactionStatus::Failed,
            )
            .unwrap();

        let (c, t) = storage.global_verified_proof_rewards().unwrap();
        assert_eq!(c, 0);
        assert_eq!(t, 0);
    }
}
