use crate::storage::traits::{PinboardGcDeletedItem, PinboardGcMetrics};
use eld_common::error::EldError;

use super::RocksDBStorage;

impl RocksDBStorage {
    pub fn increment_pinboard_gc_scanned_count_by(&self, delta: u64) -> Result<u64, EldError> {
        self.increment_pinboard_gc_counter(b"metrics:scanned_count", delta)
    }

    pub fn increment_pinboard_gc_deleted_count_by(&self, delta: u64) -> Result<u64, EldError> {
        self.increment_pinboard_gc_counter(b"metrics:deleted_count", delta)
    }

    pub(super) fn increment_pinboard_gc_counter(
        &self,
        key: &[u8],
        delta: u64,
    ) -> Result<u64, EldError> {
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

    pub(super) fn put_pinboard_gc_u64(&self, key: &[u8], value: u64) -> Result<(), EldError> {
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

    pub(super) fn get_pinboard_gc_u64(&self, key: &[u8]) -> Result<u64, EldError> {
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
}
