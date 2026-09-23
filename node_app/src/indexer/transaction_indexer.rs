use crate::indexer::{IndexedEvent, IndexedTransaction, TransactionStatus};
use crate::storage::rocksdb::RocksDBStorage;
use crate::storage::traits::TransactionIndexerStorage;
use abci::types::Event;
use eld_common::address::Address;
use eld_common::error::EldError;
use eld_common::tx::Tx;
use std::sync::Arc;

/// Transaction indexer for storing and querying indexed transactions
/// This is a thin wrapper that delegates all database operations to RocksDBStorage
/// through the TransactionIndexerStorage trait
pub struct TransactionIndexer {
    storage: Arc<RocksDBStorage>,
}

impl TransactionIndexer {
    /// Create a new transaction indexer
    pub fn new(storage: Arc<RocksDBStorage>) -> Self {
        Self { storage }
    }

    /// Calculate transaction ID from transaction
    /// Delegates to RocksDBStorage's implementation through the trait
    pub fn calculate_tx_id(&self, tx: &Tx) -> String {
        self.storage.calculate_tx_id(tx)
    }

    /// Index a transaction
    /// Delegates to RocksDBStorage's implementation which handles all database operations
    pub fn index_transaction(
        &self,
        tx: &Tx,
        block_height: u64,
        block_index: u32,
        status: TransactionStatus,
        gas_used: Option<u64>,
    ) -> Result<(), EldError> {
        self.storage
            .index_transaction(tx, block_height, block_index, status, gas_used)
    }

    /// Get a transaction by ID
    /// Delegates to RocksDBStorage's implementation which handles all database operations
    pub fn get_transaction(&self, tx_id: &str) -> Result<Option<IndexedTransaction>, EldError> {
        self.storage.get_indexed_transaction(tx_id)
    }

    /// Primary indexed transactions count (matches meta; not filtered by query params).
    pub fn indexed_transactions_primary_total(&self) -> Result<u64, EldError> {
        self.storage.get_indexed_transactions_primary_total()
    }

    /// Newest-first by `(block_height, block_index)` using the sortable `block_pos:` index.
    pub fn list_transactions_chron_desc(
        &self,
        cursor_exclusive: Option<(u64, u32)>,
        limit: u32,
        fetch_one_extra_row: bool,
        block_height: Option<u64>,
        sender: Option<Address>,
        payload_type: Option<&str>,
    ) -> Result<Vec<IndexedTransaction>, EldError> {
        self.storage.list_indexed_transactions_chron_desc(
            cursor_exclusive,
            limit,
            fetch_one_extra_row,
            block_height,
            sender,
            payload_type,
        )
    }

    /// Scan `block_pos:` rows with optional filters (full scan; used for filtered totals).
    pub fn count_transactions_matching_chron_filters(
        &self,
        block_height: Option<u64>,
        sender: Option<Address>,
        payload_type: Option<&str>,
    ) -> Result<u64, EldError> {
        self.storage
            .count_indexed_transactions_matching_filters_chron_scan(
                block_height,
                sender,
                payload_type,
            )
    }

    /// Index an event
    /// Delegates to RocksDBStorage's implementation which handles all database operations
    pub fn index_event(
        &self,
        tx_id: &str,
        event_index: u32,
        event: &Event,
        block_height: u64,
        block_index: u32,
    ) -> Result<(), EldError> {
        self.storage
            .index_event(tx_id, event_index, event, block_height, block_index)
    }

    /// Get all events for a transaction
    /// Delegates to RocksDBStorage's implementation which handles all database operations
    pub fn get_events_by_tx_id(&self, tx_id: &str) -> Result<Vec<IndexedEvent>, EldError> {
        self.storage.get_events_by_tx_id(tx_id)
    }

    /// Get all events (paginated, descending chronological order)
    /// Delegates to RocksDBStorage's implementation which handles all database operations
    pub fn get_all_events(
        &self,
        page: u32,
        limit: u32,
    ) -> Result<(Vec<IndexedEvent>, u64), EldError> {
        self.storage.get_all_events(page, limit)
    }

    /// Successful proof count and total rewards for `capacity_provider` between block heights
    /// (inclusive), using the `vp:` secondary index.
    pub fn aggregate_verified_proof_rewards(
        &self,
        provider: &str,
        from_height: u64,
        to_height: u64,
    ) -> Result<(u64, u128), EldError> {
        self.storage
            .aggregate_verified_proof_rewards(provider, from_height, to_height)
    }

    /// Lifetime network-wide verified proof rewards (rollup meta key).
    pub fn global_verified_proof_rewards(&self) -> Result<(u64, u128), EldError> {
        self.storage.global_verified_proof_rewards()
    }

    /// Last block height processed by the TM sync loop (`None` if never synced).
    pub fn get_indexer_cursor(&self) -> Result<Option<u64>, EldError> {
        self.storage.get_indexer_cursor()
    }

    /// Persist indexer cursor after a block is processed.
    pub fn set_indexer_cursor(&self, height: u64) -> Result<(), EldError> {
        self.storage.set_indexer_cursor(height)
    }

    /// Whether `block_pos:{height}:{index}` exists in the index.
    pub fn block_position_indexed(
        &self,
        block_height: u64,
        block_index: u32,
    ) -> Result<bool, EldError> {
        self.storage
            .block_position_indexed(block_height, block_index)
    }

    /// Start the Tendermint RPC sync thread.
    pub fn start(self: Arc<Self>, tendermint_rpc_url: String) {
        crate::indexer::sync::spawn_indexer_sync(self, tendermint_rpc_url);
    }
}
