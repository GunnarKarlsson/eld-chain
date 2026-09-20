use eld_common::tx::Tx;
use serde::{Deserialize, Serialize};

pub mod sync;
pub mod transaction_indexer;

pub use transaction_indexer::TransactionIndexer;

/// Transaction status for indexed transactions
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum TransactionStatus {
    Success,
    Failed,
}

/// Indexed event structure for storage in database
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IndexedEvent {
    pub tx_id: String,                     // Transaction ID (hash)
    pub event_index: u32,                  // Index of event within the transaction (0, 1, 2...)
    pub event_type: String,                // Event type (e.g., "Transfer", "VerifiedProof")
    pub attributes: Vec<(String, String)>, // Event attributes (key-value pairs)
    pub block_height: u64,                 // Block height when event occurred
    pub block_index: u32,                  // Transaction index within block
    pub timestamp: u64,                    // Unix timestamp
}

/// Indexed transaction structure
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IndexedTransaction {
    pub id: String,                // Transaction ID (hash)
    pub block_height: u64,         // Block height when confirmed
    pub block_index: u32,          // Index within the block
    pub timestamp: u64,            // Unix timestamp
    pub tx: Tx,                    // Full transaction
    pub status: TransactionStatus, // Success/Failed
    pub gas_used: Option<u64>,     // If available
    pub events: Vec<IndexedEvent>, // Associated events
}
