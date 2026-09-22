use crate::storage::traits::{CADOStorage, EpochRecordListOrder};
use eld_common::cado::{epoch_record_path_name, CadoPath};
use eld_common::constants::cado::PATH_PREFIX_EPOCH_RECORD;
use eld_common::error::EldError;
use eld_common::validator::EpochRecord;
use rocksdb::{Direction, IteratorMode};

use super::super::keys::EPOCH_RECORD_PATH_INDEX_UPPER_EXCLUSIVE;
use super::super::RocksDBStorage;

impl RocksDBStorage {
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
}
