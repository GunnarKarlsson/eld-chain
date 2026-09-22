use std::path::{Path, PathBuf};

use rocksdb::{Options, DB};
use tracing::warn;

use super::column_families::{
    eld_column_family_descriptors, ELD_CF_NAMES_FOR_ADMIN_STATS, PINBOARD_BLOB_CF_NAMES,
};
use super::RocksDBStorage;
use crate::sys_disk::directory_tree_size_bytes;
use eld_common::error::EldError;

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

impl RocksDBStorage {
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
}
