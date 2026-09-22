use rocksdb::{ColumnFamily, ColumnFamilyDescriptor, Options};

use super::RocksDBStorage;
use eld_common::error::EldError;

/// Column families whose SST bytes are counted as pinboard payload storage in admin stats.
pub(super) const PINBOARD_BLOB_CF_NAMES: &[&str] = &["pinboard_temp_blobs"];

/// Every column family name after open (including `default`, which RocksDB always adds).
pub(super) const ELD_CF_NAMES_FOR_ADMIN_STATS: &[&str] = &[
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

impl RocksDBStorage {
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
}
