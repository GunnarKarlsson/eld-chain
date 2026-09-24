use crate::api::pagination::{PaginatedResult, PrefixQueryOptions};
use crate::storage::traits::{PinboardGcMetrics, PinboardGlobalFeedOrder, PinboardStorage};
use eld_common::error::EldError;
use eld_common::pinboard::PinboardMessageMetadata;
use eld_common::tx::canonicalize_post_message_tags;
use rocksdb::Transaction;
use serde::{Deserialize, Serialize};
use tracing::warn;

use super::RocksDBStorage;

#[derive(Serialize, Deserialize)]
struct TempBlobRecord {
    blob: Vec<u8>,
    expires_at_unix: u64,
}

impl RocksDBStorage {
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

    pub(super) fn get_pinboard_metadata_from_cf_key(
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

    fn read_pinboard_temp_blob_record(
        &self,
        content_key: &str,
    ) -> Result<Option<TempBlobRecord>, EldError> {
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
                Ok(Some(record))
            }
            None => Ok(None),
        }
    }

    fn write_pinboard_temp_blob_record(
        &self,
        content_key: &str,
        record: &TempBlobRecord,
    ) -> Result<(), EldError> {
        let encoded = bincode::serialize(record).map_err(|e| EldError::StorageError {
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

    pub fn put_pinboard_temp_blob(
        &self,
        content_key: &str,
        bytes: &[u8],
        expires_at_unix: u64,
    ) -> Result<(), EldError> {
        self.write_pinboard_temp_blob_record(
            content_key,
            &TempBlobRecord {
                blob: bytes.to_vec(),
                expires_at_unix,
            },
        )
    }

    /// Bump `expires_at_unix` when a temp blob is already stored.
    ///
    /// Returns `false` when `content_key` is absent. An existing later expiry is left in place.
    pub fn extend_pinboard_temp_blob_ttl(
        &self,
        content_key: &str,
        expires_at_unix: u64,
    ) -> Result<bool, EldError> {
        let Some(mut record) = self.read_pinboard_temp_blob_record(content_key)? else {
            return Ok(false);
        };
        let extended = expires_at_unix.max(record.expires_at_unix);
        if extended == record.expires_at_unix {
            return Ok(true);
        }
        record.expires_at_unix = extended;
        self.write_pinboard_temp_blob_record(content_key, &record)?;
        Ok(true)
    }

    pub fn get_pinboard_temp_blob(&self, content_key: &str) -> Result<Option<Vec<u8>>, EldError> {
        Ok(self
            .read_pinboard_temp_blob_record(content_key)?
            .map(|record| record.blob))
    }

    #[cfg(test)]
    pub(super) fn pinboard_temp_blob_expires_at(
        &self,
        content_key: &str,
    ) -> Result<Option<u64>, EldError> {
        Ok(self
            .read_pinboard_temp_blob_record(content_key)?
            .map(|record| record.expires_at_unix))
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
