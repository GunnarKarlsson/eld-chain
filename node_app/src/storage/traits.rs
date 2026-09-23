use crate::indexer::{IndexedEvent, IndexedTransaction, TransactionStatus};
use bincode;
use eld_common::address::Address;
use eld_common::cado::{CADOMap, CADOMetadata, CadoBody, CadoPath, DeserializableBin};
use eld_common::error::EldError;
use eld_common::pinboard::PinboardMessageMetadata;
use eld_common::storage::AccountStorage;
use eld_common::tx::Tx;
use eld_common::validator::EpochRecord;
use rocksdb::Transaction;
use serde::{Deserialize, Serialize};
use std::str;

/// Chronological ordering for epoch record listing (`GET /epochs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpochRecordListOrder {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PinboardGlobalFeedOrder {
    /// Oldest committed height first.
    Asc,
    /// Newest committed height first.
    #[default]
    Desc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinboardGcDeletedItem {
    pub wallet: String,
    pub message_id: String,
    pub deleted_at_height: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinboardGcMetrics {
    pub deleted_count: u64,
    pub scanned_count: u64,
    pub last_cursor_key: Option<String>,
    pub lag_blocks: u64,
    pub deleted_items: Vec<PinboardGcDeletedItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMetadata {
    pub height: i64,
    pub epoch: u64,
    pub format_version: u32,
    pub chunk_count: u32,
    pub total_size: u64,
    pub compression: String,
    pub created_at: u64,
    pub app_hash: [u8; 32],
    pub accounts_count: u64,
    pub staking_accounts_count: u64,
    pub storage_staking_accounts_count: u64,
    pub namespaces_count: u64,
    pub chunk_hashes: Vec<String>,
}

/// Account-like totals for one committed snapshot. Not per chunk.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SnapshotStateCounts {
    pub accounts_count: u64,
    pub staking_accounts_count: u64,
    pub storage_staking_accounts_count: u64,
    pub namespaces_count: u64,
}

impl SnapshotMetadata {
    pub fn deserialize_bin(data: &[u8]) -> Result<Self, EldError> {
        bincode::deserialize(data).map_err(|e| EldError::StorageError {
            operation: "deserialize_snapshot_metadata".to_string(),
            details: format!("Failed to deserialize SnapshotMetadata: {e}"),
        })
    }
}

impl DeserializableBin for SnapshotMetadata {
    fn deserialize_bin(data: &[u8]) -> Result<Self, EldError> {
        SnapshotMetadata::deserialize_bin(data)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotChunk {
    pub index: u32,
    pub data: Vec<u8>,
    pub hash: String,
    pub size: u64,
}

impl SnapshotChunk {
    pub fn serialize_bin(&self) -> Result<Vec<u8>, EldError> {
        bincode::serialize(self).map_err(|e| EldError::StorageError {
            operation: "serialize_snapshot_chunk".to_string(),
            details: format!("Failed to serialize SnapshotChunk: {e}"),
        })
    }

    pub fn deserialize_bin(data: &[u8]) -> Result<Self, EldError> {
        bincode::deserialize(data).map_err(|e| EldError::StorageError {
            operation: "deserialize_snapshot_chunk".to_string(),
            details: format!("Failed to deserialize SnapshotChunk: {e}"),
        })
    }
}

impl DeserializableBin for SnapshotChunk {
    fn deserialize_bin(data: &[u8]) -> Result<Self, EldError> {
        SnapshotChunk::deserialize_bin(data)
    }
}

pub trait SnapshotStorage: Send + Sync {
    // Metadata operations
    fn put_snapshot_metadata(&self, metadata: &SnapshotMetadata) -> Result<(), EldError>;
    fn get_snapshot_metadata(&self, height: i64) -> Result<Option<SnapshotMetadata>, EldError>;
    fn get_latest_snapshot_metadata(&self) -> Result<Option<SnapshotMetadata>, EldError>;
    fn list_snapshots(&self, limit: u32) -> Result<Vec<SnapshotMetadata>, EldError>;
    fn delete_snapshot_metadata(&self, height: i64) -> Result<(), EldError>;

    // Chunk operations
    fn put_snapshot_chunk(&self, height: i64, chunk: &SnapshotChunk) -> Result<(), EldError>;
    fn get_snapshot_chunk(
        &self,
        height: i64,
        chunk_index: u32,
    ) -> Result<Option<SnapshotChunk>, EldError>;
    fn delete_snapshot_chunk(&self, height: i64, chunk_index: u32) -> Result<(), EldError>;
}

/// Snapshot pruning and verification helpers used only from unit tests.
#[cfg(test)]
pub trait SnapshotStorageTestExt: SnapshotStorage {
    fn prune_snapshots(&self, keep_last_n: u32) -> Result<(), EldError>;
    fn verify_snapshot(&self, height: i64) -> Result<bool, EldError>;
    fn get_snapshot_chunk_count(&self, height: i64) -> Result<u32, EldError>;
    fn snapshot_exists(&self, height: i64) -> Result<bool, EldError>;
}

/// Implemented by [`crate::storage::rocksdb::RocksDBStorage`] and [`crate::storage::hybrid_storage::HybridStorage`].
/// Many call paths use concrete types rather than `dyn CADOStorage`, so the compiler may report
/// individual required methods as unused; they are still part of the storage contract.
#[allow(dead_code)]
pub trait CADOStorage: Send + Sync + 'static {
    fn put_cado_type(&self, path: CadoPath, cado_type: CadoBody) -> Result<(), EldError>;
    fn put_cado_data(
        &self,
        path: CadoPath,
        cado_data: Vec<u8>,
        metadata: CADOMetadata,
    ) -> Result<(), EldError>;
    fn put_cado_map(&self, path: CadoPath, mapping: CADOMap) -> Result<(), EldError>;
    fn get_cado_by_path(&self, path: CadoPath) -> Result<Option<CadoBody>, EldError>;

    /// Loads a CADO at `path` and deserializes its payload as `T`.
    ///
    /// Returns [`EldError::NotFoundError`] when no CADO exists at `path` (identifier is the path
    /// string). Other errors come from storage or from [`DeserializableBin::deserialize_bin`].
    fn get_deserialized_cado_by_path<T: DeserializableBin>(
        &self,
        path: CadoPath,
    ) -> Result<T, EldError> {
        use eld_common::cado::DeserializeBinSliceExt;

        let cado = self
            .get_cado_by_path(path.clone())?
            .ok_or_else(|| EldError::NotFoundError {
                resource_type: "CADO".to_string(),
                identifier: path.to_string(),
            })?;
        cado.data().deserialize_bin::<T>()
    }

    fn get_cado_paths_by_prefix(&self, prefix: &str) -> Result<Vec<CadoPath>, EldError>;
    fn get_cados_by_prefix(&self, prefix: &str) -> Result<Vec<CadoBody>, EldError>;

    /// Lists persisted [`EpochRecord`] CADOs in epoch-number order (newest first when `Desc`).
    fn list_epoch_records_chron(
        &self,
        order: EpochRecordListOrder,
        after_epoch: Option<i64>,
        fetch_limit: usize,
    ) -> Result<Vec<EpochRecord>, EldError>;

    /// Count of per-epoch records (excludes the `LATEST` alias path).
    fn count_epoch_records(&self) -> Result<u64, EldError>;
    fn search_cado_hash(&self, prefix: &str) -> Result<Vec<(String, CadoBody)>, EldError>;
    fn search_cado_name(&self, prefix: &str) -> Result<Vec<(String, CadoBody)>, EldError>;
    fn search_cado_path(&self, prefix: &str) -> Result<Vec<(String, CadoBody)>, EldError>;
    fn get_cado_map(&self, path: CadoPath) -> Result<Option<CADOMap>, EldError>;
    fn delete_cado(
        &self,
        path: CadoPath,
        owner: &str,
        signature: &str,
        public_key: &str,
        chain_id: &str,
    ) -> Result<(), EldError>;
    fn system_delete_cado(&self, path: CadoPath, owner: &str) -> Result<(), EldError>;
    fn system_delete_cado_with_tx(
        &self,
        path: CadoPath,
        owner: &str,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError>;

    // Transaction-aware methods
    fn begin_transaction(&self) -> Transaction<'_, rocksdb::TransactionDB>;
    fn put_cado_type_with_tx(
        &self,
        path: CadoPath,
        cado_type: CadoBody,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError>;
    fn put_cado_data_with_tx(
        &self,
        path: CadoPath,
        cado_data: Vec<u8>,
        metadata: CADOMetadata,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError>;
    fn delete_cado_with_tx(
        &self,
        path: CadoPath,
        owner: &str,
        signature: &str,
        public_key: &str,
        chain_id: &str,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError>;
    fn put_cado_map_with_tx(
        &self,
        path: CadoPath,
        mapping: CADOMap,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError>;
}

/// Transaction-aware pinboard storage operations (written during consensus commit).
pub trait PinboardStorage: Send + Sync + 'static {
    fn put_pinboard_metadata_with_tx(
        &self,
        message_id: &str,
        meta: &PinboardMessageMetadata,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError>;

    fn put_pinboard_commit_index_with_tx(
        &self,
        committed_height: u64,
        message_id: &str,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError>;

    fn put_pinboard_wallet_index_with_tx(
        &self,
        wallet: &str,
        committed_height: u64,
        message_id: &str,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError>;

    fn put_pinboard_tag_index_with_tx(
        &self,
        tag: &str,
        committed_height: u64,
        message_id: &str,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError>;

    fn put_pinboard_expiry_index_with_tx(
        &self,
        expires_height: u64,
        message_id: &str,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError>;

    fn update_pinboard_refcount_with_tx(
        &self,
        content_key: &str,
        delta: i64,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<u64, EldError>;

    fn delete_pinboard_temp_blob_with_tx(
        &self,
        content_key: &str,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError>;
}

/// Read/query pinboard storage operations (used by ABCI info queries).
pub trait PinboardQueryStorage: Send + Sync + 'static {
    fn get_pinboard_metadata(
        &self,
        message_id: &str,
    ) -> Result<Option<PinboardMessageMetadata>, EldError>;

    /// Lookup by full namespace content path key (`/@{namespace}/{message_id}`). Not for Eld paths.
    #[allow(dead_code)]
    fn get_pinboard_metadata_by_path_key(
        &self,
        path_key: &str,
    ) -> Result<Option<PinboardMessageMetadata>, EldError>;

    fn get_pinboard_temp_blob(&self, content_key: &str) -> Result<Option<Vec<u8>>, EldError>;

    fn get_pinboard_message_ids_global_secure(
        &self,
        order: PinboardGlobalFeedOrder,
        options: crate::api::pagination::PrefixQueryOptions,
    ) -> Result<crate::api::pagination::PaginatedResult<String>, EldError>;

    fn get_pinboard_message_ids_by_tag_secure(
        &self,
        tag: &str,
        options: crate::api::pagination::PrefixQueryOptions,
    ) -> Result<crate::api::pagination::PaginatedResult<String>, EldError>;

    fn get_pinboard_message_ids_by_wallet_secure(
        &self,
        wallet: &str,
        options: crate::api::pagination::PrefixQueryOptions,
    ) -> Result<crate::api::pagination::PaginatedResult<String>, EldError>;

    fn get_pinboard_gc_metrics(&self) -> Result<PinboardGcMetrics, EldError>;
}

/// Chain-wide dedup for rewarded [`VerifiedProof`](eld_common::tx::VerifiedProofTx) challenges.
pub trait VerifiedProofRewardDedupStorage: Send + Sync {
    fn is_verified_proof_challenge_rewarded(&self, challenge_id: &str) -> Result<bool, EldError>;

    fn put_verified_proof_challenge_rewarded_with_tx(
        &self,
        challenge_id: &str,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError>;
}

pub trait ConsensusConnectionStorage:
    AccountStorage
    + SnapshotStorage
    + CADOStorage
    + PinboardStorage
    + PinboardQueryStorage
    + VerifiedProofRewardDedupStorage
    + Clone
    + Send
    + Sync
    + 'static
{
}

/// Trait for transaction indexing storage operations
/// All database operations for transaction indexing should go through this trait
pub trait TransactionIndexerStorage: Send + Sync {
    /// Calculate transaction ID from transaction
    fn calculate_tx_id(&self, tx: &Tx) -> String;

    /// Index a transaction with all secondary indexes
    fn index_transaction(
        &self,
        tx: &Tx,
        block_height: u64,
        block_index: u32,
        status: TransactionStatus,
        gas_used: Option<u64>,
    ) -> Result<(), EldError>;

    /// Get a transaction by ID
    fn get_indexed_transaction(&self, tx_id: &str) -> Result<Option<IndexedTransaction>, EldError>;

    /// Total primary indexed transactions (`0x…` bodies), unaffected by REST filters.
    fn get_indexed_transactions_primary_total(&self) -> Result<u64, EldError>;

    /// Chronological descending list keyed by `(block_height, block_index)` via `block_pos:` secondary keys.
    /// When `fetch_one_extra_row` is true, up to `limit + 1` matching rows may be returned.
    fn list_indexed_transactions_chron_desc(
        &self,
        cursor_exclusive: Option<(u64, u32)>,
        limit: u32,
        fetch_one_extra_row: bool,
        block_height: Option<u64>,
        sender: Option<Address>,
        payload_type: Option<&str>,
    ) -> Result<Vec<IndexedTransaction>, EldError>;

    /// Scan all `block_pos:` rows applying optional filters (for filtered-result totals).
    fn count_indexed_transactions_matching_filters_chron_scan(
        &self,
        block_height: Option<u64>,
        sender: Option<Address>,
        payload_type: Option<&str>,
    ) -> Result<u64, EldError>;

    /// Index an event with all secondary indexes
    fn index_event(
        &self,
        tx_id: &str,
        event_index: u32,
        event: &abci::types::Event,
        block_height: u64,
        block_index: u32,
    ) -> Result<(), EldError>;

    /// Get all events for a transaction
    fn get_events_by_tx_id(&self, tx_id: &str) -> Result<Vec<IndexedEvent>, EldError>;

    /// Get all events (paginated, descending chronological order)
    fn get_all_events(&self, page: u32, limit: u32) -> Result<(Vec<IndexedEvent>, u64), EldError>;

    /// Count successful verified proof reward index entries for `capacity_provider` in
    /// `from_height..=to_height`. Returns `(successful_proofs, total_rewards)` using
    /// [`eld_common::constants::protocol::VERIFIED_PROOF_REWARD_BASE_AMOUNT`].
    fn aggregate_verified_proof_rewards(
        &self,
        provider: &str,
        from_height: u64,
        to_height: u64,
    ) -> Result<(u64, u128), EldError>;

    /// Lifetime network-wide successful verified-proof count from a single meta key (O(1) read).
    /// Returns `(successful_proofs, total_native_rewards)`.
    fn global_verified_proof_rewards(&self) -> Result<(u64, u128), EldError>;

    /// Last block height fully processed by the TM-backed indexer (`None` if never synced).
    fn get_indexer_cursor(&self) -> Result<Option<u64>, EldError>;

    /// Persist the indexer cursor after processing a block.
    fn set_indexer_cursor(&self, height: u64) -> Result<(), EldError>;

    /// Whether the chronological `block_pos:` secondary index exists for `(height, index)`.
    fn block_position_indexed(&self, block_height: u64, block_index: u32)
        -> Result<bool, EldError>;
}

/// Persisted deduplication for outbound [`VerifiedProof`](eld_common::tx::VerifiedProofTx) submissions.
///
/// Must only be consulted under the same serialization as nonce assignment (see
/// [`crate::wallet::VerifiedProofChainSubmitter`]) unless the store implementation is fully atomic.
/// Persisted VerifiedProof submission dedup. Implemented by [`crate::storage::hybrid_storage::HybridStorage`]
/// (which delegates to RocksDB helpers — RocksDB does not implement this trait).
pub trait VerifiedProofSubmissionClaimStorage: Send + Sync {
    /// Records `(sender_address_hex, challenge_id)` pairs so each proof is submitted at most once.
    ///
    /// Callers that use the non-transactional RocksDB implementation **must** serialize claim
    /// checks / writes for a given wallet (e.g. one [`crate::wallet::VerifiedProofChainSubmitter`] mutex).
    ///
    /// Prefer **claim-after-success**: call [`has_verified_proof_submission_claim`] before broadcast
    /// to skip duplicates, then [`insert_verified_proof_submission_claim`] only after a successful
    /// `send_tx_rpc` so a failed submit can be retried.
    ///
    /// If this `(sender, challenge_id)` was never claimed, persist it and return `true`.
    /// Otherwise return `false` (do not broadcast again).
    fn insert_verified_proof_submission_claim(
        &self,
        sender_address_hex: &str,
        challenge_id: &str,
    ) -> Result<bool, EldError>;

    /// Returns `true` if `(sender, challenge_id)` was already claimed (no write).
    fn has_verified_proof_submission_claim(
        &self,
        sender_address_hex: &str,
        challenge_id: &str,
    ) -> Result<bool, EldError>;
}
