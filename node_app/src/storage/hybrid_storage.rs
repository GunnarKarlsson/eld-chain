use crate::storage::rocksdb::RocksDBStorage;
use crate::storage::traits::{
    CADOStorage, ConsensusConnectionStorage, PinboardGcMetrics, PinboardGlobalFeedOrder,
    PinboardQueryStorage, PinboardStorage, ProtocolConstantsStorage, SnapshotChunk,
    SnapshotMetadata, SnapshotStorage, VerifiedProofRewardDedupStorage,
    VerifiedProofSubmissionClaimStorage,
};
use eld_common::account::Account;
use eld_common::cado::{CADOMap, CADOMetadata, CadoBody, CadoPath};
use eld_common::error::EldError;
use eld_common::pinboard::PinboardMessageMetadata;

use eld_common::storage::AccountStorage;
use rocksdb::Transaction;
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[derive(Clone)]
pub struct HybridStorage {
    rocks_db: Arc<RocksDBStorage>,
}

impl HybridStorage {
    /// HybridStorage is a wrapper around RocksDB
    ///  RocksDB is not cloneable, so we need to wrap it in an Arc
    pub fn new(rocks_db: Arc<RocksDBStorage>) -> Self {
        Self { rocks_db }
    }
}

impl std::fmt::Debug for HybridStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HybridStorage")
            .field("rocks_db", &"<redacted>")
            .finish()
    }
}

// CONSENSUS TRAITS
impl ConsensusConnectionStorage for HybridStorage {}

impl ProtocolConstantsStorage for HybridStorage {
    fn put_protocol_constants(&self, bytes: &[u8]) -> Result<(), EldError> {
        self.rocks_db.put_protocol_constants(bytes)
    }

    fn get_protocol_constants(&self) -> Result<Option<Vec<u8>>, EldError> {
        self.rocks_db.get_protocol_constants()
    }
}

impl VerifiedProofRewardDedupStorage for HybridStorage {
    fn is_verified_proof_challenge_rewarded(&self, challenge_id: &str) -> Result<bool, EldError> {
        self.rocks_db
            .is_verified_proof_challenge_rewarded(challenge_id)
    }

    fn put_verified_proof_challenge_rewarded_with_tx(
        &self,
        challenge_id: &str,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        self.rocks_db
            .put_verified_proof_challenge_rewarded_with_tx(challenge_id, tx)
    }

    fn is_verified_proof_challenge_failed(&self, challenge_id: &str) -> Result<bool, EldError> {
        self.rocks_db
            .is_verified_proof_challenge_failed(challenge_id)
    }

    fn put_verified_proof_challenge_failed_with_tx(
        &self,
        challenge_id: &str,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        self.rocks_db
            .put_verified_proof_challenge_failed_with_tx(challenge_id, tx)
    }
}

impl VerifiedProofSubmissionClaimStorage for HybridStorage {
    fn insert_verified_proof_submission_claim(
        &self,
        sender_address_hex: &str,
        challenge_id: &str,
    ) -> Result<bool, EldError> {
        self.rocks_db
            .insert_verified_proof_submission_claim(sender_address_hex, challenge_id)
    }

    fn has_verified_proof_submission_claim(
        &self,
        sender_address_hex: &str,
        challenge_id: &str,
    ) -> Result<bool, EldError> {
        self.rocks_db
            .has_verified_proof_submission_claim(sender_address_hex, challenge_id)
    }
}

impl PinboardStorage for HybridStorage {
    fn put_pinboard_metadata_with_tx(
        &self,
        message_id: &str,
        meta: &PinboardMessageMetadata,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        self.rocks_db
            .put_pinboard_metadata_with_tx(message_id, meta, tx)
    }

    fn put_pinboard_commit_index_with_tx(
        &self,
        committed_height: u64,
        message_id: &str,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        self.rocks_db
            .put_pinboard_commit_index_with_tx(committed_height, message_id, tx)
    }

    fn put_pinboard_wallet_index_with_tx(
        &self,
        wallet: &str,
        committed_height: u64,
        message_id: &str,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        self.rocks_db
            .put_pinboard_wallet_index_with_tx(wallet, committed_height, message_id, tx)
    }

    fn put_pinboard_tag_index_with_tx(
        &self,
        tag: &str,
        committed_height: u64,
        message_id: &str,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        self.rocks_db
            .put_pinboard_tag_index_with_tx(tag, committed_height, message_id, tx)
    }

    fn put_pinboard_expiry_index_with_tx(
        &self,
        expires_height: u64,
        message_id: &str,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        self.rocks_db
            .put_pinboard_expiry_index_with_tx(expires_height, message_id, tx)
    }

    fn update_pinboard_refcount_with_tx(
        &self,
        content_key: &str,
        delta: i64,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<u64, EldError> {
        self.rocks_db
            .update_pinboard_refcount_with_tx(content_key, delta, tx)
    }

    fn delete_pinboard_temp_blob_with_tx(
        &self,
        content_key: &str,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        self.rocks_db
            .delete_pinboard_temp_blob_with_tx(content_key, tx)
    }
}

impl PinboardQueryStorage for HybridStorage {
    fn get_pinboard_metadata(
        &self,
        message_id: &str,
    ) -> Result<Option<PinboardMessageMetadata>, EldError> {
        self.rocks_db.get_pinboard_metadata(message_id)
    }

    fn get_pinboard_metadata_by_path_key(
        &self,
        path_key: &str,
    ) -> Result<Option<PinboardMessageMetadata>, EldError> {
        self.rocks_db.get_pinboard_metadata_by_path_key(path_key)
    }

    fn get_pinboard_temp_blob(&self, content_key: &str) -> Result<Option<Vec<u8>>, EldError> {
        self.rocks_db.get_pinboard_temp_blob(content_key)
    }

    fn get_pinboard_message_ids_global_secure(
        &self,
        order: PinboardGlobalFeedOrder,
        options: crate::api::pagination::PrefixQueryOptions,
    ) -> Result<crate::api::pagination::PaginatedResult<String>, EldError> {
        self.rocks_db
            .get_pinboard_message_ids_global_secure(order, options)
    }

    fn get_pinboard_message_ids_by_tag_secure(
        &self,
        tag: &str,
        options: crate::api::pagination::PrefixQueryOptions,
    ) -> Result<crate::api::pagination::PaginatedResult<String>, EldError> {
        self.rocks_db
            .get_pinboard_message_ids_by_tag_secure(tag, options)
    }

    fn get_pinboard_message_ids_by_wallet_secure(
        &self,
        wallet: &str,
        options: crate::api::pagination::PrefixQueryOptions,
    ) -> Result<crate::api::pagination::PaginatedResult<String>, EldError> {
        self.rocks_db
            .get_pinboard_message_ids_by_wallet_secure(wallet, options)
    }

    fn get_pinboard_gc_metrics(&self) -> Result<PinboardGcMetrics, EldError> {
        self.rocks_db.get_pinboard_gc_metrics()
    }
}

impl AccountStorage for HybridStorage {
    fn get_account_by_path(&self, path: CadoPath) -> Result<Option<Account>, EldError> {
        let path_str = path.to_string();
        let cado_type = self
            .get_cado_by_path(path)?
            .ok_or_else(|| EldError::NotFoundError {
                resource_type: "CADO".to_string(),
                identifier: path_str,
            })?;
        match cado_type {
            CadoBody::Mutable(cado) => {
                if Sha256::digest(cado.data())[..] != cado.latest_hash()[..] {
                    return Err(EldError::StorageError {
                        operation: "verify_latest_hash".to_string(),
                        details: "Latest hash verification failed".to_string(),
                    });
                }
                let account: Account = Account::deserialize_bin(cado.data())?;
                Ok(Some(account))
            }
            _ => Err(EldError::StorageError {
                operation: "get_account_by_path".to_string(),
                details: "Expected CADOMut type".to_string(),
            }),
        }
    }
}

impl SnapshotStorage for HybridStorage {
    fn put_snapshot_metadata(&self, metadata: &SnapshotMetadata) -> Result<(), EldError> {
        // Store in RocksDB
        self.rocks_db.put_snapshot_metadata(metadata)
    }

    fn get_snapshot_metadata(&self, height: i64) -> Result<Option<SnapshotMetadata>, EldError> {
        self.rocks_db.get_snapshot_metadata(height)
    }

    fn get_latest_snapshot_metadata(&self) -> Result<Option<SnapshotMetadata>, EldError> {
        self.rocks_db.get_latest_snapshot_metadata()
    }

    fn list_snapshots(&self, limit: u32) -> Result<Vec<SnapshotMetadata>, EldError> {
        self.rocks_db.list_snapshots(limit)
    }

    fn put_snapshot_chunk(&self, height: i64, chunk: &SnapshotChunk) -> Result<(), EldError> {
        // Store all chunks in RocksDB
        self.rocks_db.put_snapshot_chunk(height, chunk)
    }

    fn get_snapshot_chunk(
        &self,
        height: i64,
        chunk_index: u32,
    ) -> Result<Option<SnapshotChunk>, EldError> {
        // Get chunk from RocksDB
        self.rocks_db.get_snapshot_chunk(height, chunk_index)
    }

    fn delete_snapshot_chunk(&self, height: i64, chunk_index: u32) -> Result<(), EldError> {
        // Delete from RocksDB
        self.rocks_db.delete_snapshot_chunk(height, chunk_index)
    }

    fn delete_snapshot_metadata(&self, height: i64) -> Result<(), EldError> {
        self.rocks_db.delete_snapshot_metadata(height)
    }
}

impl CADOStorage for HybridStorage {
    fn put_cado_type(&self, path: CadoPath, cado_type: CadoBody) -> Result<(), EldError> {
        self.rocks_db.put_cado_type(path, cado_type)
    }

    fn put_cado_data(
        &self,
        path: CadoPath,
        cado_data: Vec<u8>,
        metadata: CADOMetadata,
    ) -> Result<(), EldError> {
        self.rocks_db.put_cado_data(path, cado_data, metadata)
    }

    fn put_cado_map(&self, path: CadoPath, mapping: CADOMap) -> Result<(), EldError> {
        self.rocks_db.put_cado_map(path, mapping)
    }

    fn get_cado_paths_by_prefix(&self, prefix: &str) -> Result<Vec<CadoPath>, EldError> {
        self.rocks_db.get_cado_paths_by_prefix(prefix)
    }

    fn get_cados_by_prefix(&self, prefix: &str) -> Result<Vec<CadoBody>, EldError> {
        self.rocks_db.get_cados_by_prefix(prefix)
    }

    fn list_epoch_records_chron(
        &self,
        order: crate::storage::traits::EpochRecordListOrder,
        after_epoch: Option<i64>,
        fetch_limit: usize,
    ) -> Result<Vec<eld_common::validator::EpochRecord>, EldError> {
        self.rocks_db
            .list_epoch_records_chron(order, after_epoch, fetch_limit)
    }

    fn count_epoch_records(&self) -> Result<u64, EldError> {
        self.rocks_db.count_epoch_records()
    }

    fn get_cado_by_path(&self, path: CadoPath) -> Result<Option<CadoBody>, EldError> {
        self.rocks_db.get_cado_by_path(path)
    }

    fn search_cado_hash(&self, prefix: &str) -> Result<Vec<(String, CadoBody)>, EldError> {
        self.rocks_db.search_cado_hash(prefix)
    }

    fn search_cado_name(&self, prefix: &str) -> Result<Vec<(String, CadoBody)>, EldError> {
        self.rocks_db.search_cado_name(prefix)
    }

    fn search_cado_path(&self, prefix: &str) -> Result<Vec<(String, CadoBody)>, EldError> {
        self.rocks_db.search_cado_path(prefix)
    }

    fn get_cado_map(&self, path: CadoPath) -> Result<Option<CADOMap>, EldError> {
        self.rocks_db.get_cado_map(path)
    }

    fn delete_cado(
        &self,
        path: CadoPath,
        owner: &str,
        signature: &str,
        public_key: &str,
        chain_id: &str,
    ) -> Result<(), EldError> {
        self.rocks_db
            .delete_cado(path, owner, signature, public_key, chain_id)
    }

    fn system_delete_cado(&self, path: CadoPath, owner: &str) -> Result<(), EldError> {
        self.rocks_db.system_delete_cado(path, owner)
    }

    fn system_delete_cado_with_tx(
        &self,
        path: CadoPath,
        owner: &str,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        self.rocks_db.system_delete_cado_with_tx(path, owner, tx)
    }

    // Transaction-aware methods
    fn begin_transaction(&self) -> Transaction<'_, rocksdb::TransactionDB> {
        self.rocks_db.begin_transaction()
    }
    fn put_cado_type_with_tx(
        &self,
        path: CadoPath,
        cado_type: CadoBody,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        self.rocks_db
            .put_cadotype_by_path_with_tx(path, cado_type, tx)
    }
    fn put_cado_data_with_tx(
        &self,
        path: CadoPath,
        cado_data: Vec<u8>,
        metadata: CADOMetadata,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        self.rocks_db
            .put_cado_data_with_tx(path, cado_data, metadata, tx)
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
        self.rocks_db
            .delete_cado_with_tx(path, owner, signature, public_key, chain_id, tx)
    }
    fn put_cado_map_with_tx(
        &self,
        path: CadoPath,
        mapping: CADOMap,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        self.rocks_db.put_cado_map_with_tx(path, mapping, tx)
    }
}

#[cfg(test)]
impl crate::storage::traits::SnapshotStorageTestExt for HybridStorage {
    fn prune_snapshots(&self, keep_last_n: u32) -> Result<(), EldError> {
        crate::storage::traits::SnapshotStorageTestExt::prune_snapshots(
            self.rocks_db.as_ref(),
            keep_last_n,
        )
    }

    fn verify_snapshot(&self, height: i64) -> Result<bool, EldError> {
        let metadata = match self.get_snapshot_metadata(height)? {
            Some(m) => m,
            None => return Ok(false),
        };

        for i in 0..metadata.chunk_count {
            let chunk = match self.get_snapshot_chunk(height, i)? {
                Some(c) => c,
                None => return Ok(false),
            };

            let mut hasher = Sha256::new();
            hasher.update(&chunk.data);
            let hash = hex::encode(hasher.finalize());
            if hash != metadata.chunk_hashes[i as usize] {
                return Ok(false);
            }
        }

        Ok(true)
    }

    fn get_snapshot_chunk_count(&self, height: i64) -> Result<u32, EldError> {
        Ok(self
            .get_snapshot_metadata(height)?
            .map(|m| m.chunk_count)
            .unwrap_or(0))
    }

    fn snapshot_exists(&self, height: i64) -> Result<bool, EldError> {
        Ok(self.get_snapshot_metadata(height)?.is_some())
    }
}
