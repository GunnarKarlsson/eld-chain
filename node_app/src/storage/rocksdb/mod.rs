use crate::storage::rate_limiter::PrefixQueryRateLimiter;
use eld_common::error::EldError;
use rocksdb::{Options, Transaction, TransactionDB, TransactionDBOptions};
use std::path::Path;
use std::sync::Mutex;
use tracing::info;

mod admin;
mod cado;
mod column_families;
mod indexer;
mod keys;
mod pinboard;
mod pinboard_gc;
mod snapshots;
mod verified_proof;

#[cfg(test)]
mod tests;

#[allow(unused_imports)] // re-exported for `crate::storage::rocksdb::AdminRocksDbStorageStats`
pub use admin::AdminRocksDbStorageStats;
pub use column_families::eld_column_family_descriptors;

const PROTOCOL_CONSTANTS_KEY: &[u8] = b"protocol_constants";

/// RocksDB-based implementation of AccountStorage
pub struct RocksDBStorage {
    pub db: TransactionDB,
    /// Rate limiter for prefix queries
    rate_limiter: PrefixQueryRateLimiter,
    /// Serializes updates to [`INDEXED_TRANSACTIONS_PRIMARY_TOTAL_COUNT_KEY`].
    indexed_transaction_primary_total_mutex: Mutex<()>,
}

impl RocksDBStorage {
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

    pub fn put_protocol_constants(&self, bytes: &[u8]) -> Result<(), EldError> {
        let cf = self.cf_handle("default")?;
        self.db
            .put_cf(cf, PROTOCOL_CONSTANTS_KEY, bytes)
            .map_err(|e| EldError::StorageError {
                operation: "put_protocol_constants".to_string(),
                details: e.to_string(),
            })
    }

    pub fn get_protocol_constants(&self) -> Result<Option<Vec<u8>>, EldError> {
        let cf = self.cf_handle("default")?;
        self.db
            .get_cf(cf, PROTOCOL_CONSTANTS_KEY)
            .map_err(|e| EldError::StorageError {
                operation: "get_protocol_constants".to_string(),
                details: e.to_string(),
            })
    }

    pub fn begin_transaction(&self) -> Transaction<'_, TransactionDB> {
        self.db.transaction()
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
