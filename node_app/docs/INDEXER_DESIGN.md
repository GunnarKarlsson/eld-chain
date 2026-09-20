## Indexer Service Design Proposal

### Overview
Index transactions and events in RocksDB when enabled via config, and expose query endpoints.

### 1. Configuration

**Add to `ConsensusConfig` in `chain/node_app/src/config/mod.rs`:**
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsensusConfig {
    // ... existing fields ...
    /// Enable transaction indexing (default: false)
    #[serde(default)]
    pub indexer: bool,
}
```

**Config JSON example:**
```json
{
    "chain_id": "...",
    "app_host": "...",
    "app_port": "...",
    "indexer": true,
    // ... other fields ...
}
```

### 2. Storage Schema

**New RocksDB Column Family:**
- Column family: `"indexed_transaction"`

**Key-Value Structure:**
- **Primary Key**: Transaction ID (SHA256 hash of serialized transaction, hex-encoded)
  - Format: `tx_id` → `IndexedTransaction` (serialized)
- **Secondary Index Keys** (for efficient querying):
  - `block_height:{height}:tx_index:{index}` → `tx_id` (legacy chronological secondary index; decimal height/index)
  - `block_pos:{height_hex16}:{tx_index_hex8}` → `tx_id` — **dual-written** fixed-width hexadecimal key so byte order matches canonical `(block_height, block_index)` (newest-first listing for `GET /transactions`)
  - `sender:{address}:nonce:{nonce}` → `tx_id` (for sender queries)
  - `type:{payload_type}:tx_id` → `tx_id` (for type filtering)

**IndexedTransaction Structure:**
```rust
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IndexedTransaction {
    pub id: String,                    // Transaction ID (hash)
    pub block_height: u64,            // Block height when confirmed
    pub block_index: u32,              // Index within the block
    pub timestamp: u64,                 // Unix timestamp
    pub tx: Tx,                        // Full transaction
    pub status: TransactionStatus,     // Success/Failed
    pub gas_used: Option<u64>,         // If available
    pub events: Vec<IndexedEvent>,      // Associated events
}
```

### 3. Indexer Module Structure

**New Module: `chain/node_app/src/indexer/mod.rs`**

**Core Components:**
1. **`TransactionIndexer`** - Main indexer struct
   - Wraps `Arc<RocksDBStorage>`
   - Methods: `index_transaction()`, `get_transaction()`, `list_transactions_chron_desc()`

2. **Initialization**
   - Check `config.indexer` on startup
   - Create column family (as part of rocksdb struct default column creation) even if indexes is not enable. So this column family is always created
   - Initialize indexer instance

### 4. Integration Points

**A. Transaction Indexing Hook**

**Location:** `chain/node_app/src/consensus/consensus.rs` in the `commit()` function

**When to Index:**
- After successful commit (line ~1814, after `tx.commit()`)
- Index all transactions from the current block that were successfully processed
- Store block height, block index, and transaction data

**Implementation Approach:**
```rust
// In commit() function, after tx.commit():
if let Some(indexer) = &self.indexer {
    // Get all transactions from current_state that were processed
    // Index each transaction with block_height and block_index
    for (index, processed_tx) in processed_transactions.iter().enumerate() {
        indexer.index_transaction(
            processed_tx,
            current_state.envelope.block_height,
            index as u32,
        ).await?;
    }
}
```

**B. Transaction Tracking**

**Option 1 (Recommended):** Track transactions during `deliver_tx`
- Store successful transactions in a temporary buffer in `ConsensusConnection`
- Index them during `commit()` when block is finalized

**Option 2:** Re-query from Tendermint
- Query committed block data from Tendermint RPC
- Less efficient but simpler

### 5. API Endpoints

**Location:** `chain/node_app/src/storage/api.rs` or new `chain/node_app/src/indexer/api.rs`

**Endpoints:**

1. **`GET /transactions`** — Chronological transaction list (newest block / highest index first)
   - Query params:
     - `after_height`, `after_index` — optional continuation: both required together (`400` if only one is set). Means “strictly older than this `(block_height, tx_index)` position” (typically the last row from the prior page).
     - `limit` (default 50, max 100)
     - `block_height`, `sender`, `type` — optional filters (malformed `sender` → HTTP 200 with empty list)
   - No `page`; use `after_*` + `limit`.
   - Response:
     ```json
     {
         "transactions": [IndexedTransaction],
         "pagination": {
             "limit": 50,
             "has_next": false,
             "total": 1234
         }
     }
     ```
   - Empty tail of the index returns HTTP 200 with `transactions: []`.
   - `total`: when listing without filters — full primary-index count; on filtered lists — supplied on the first request (no `after_*`); omitted on filtered continuation (`after_*` set) to avoid repeated full scans.

2. **`GET /transaction?id={tx_id}`** - Get specific transaction
   - Response: `IndexedTransaction` or 404

**Router Integration:**
- Add routes to existing router in `init_router_with_storage()` or create separate indexer router
- Use same CORS and rate limiting patterns

### 6. Transaction ID Calculation

**Method:** SHA256 hash of serialized transaction (without signature for consistency, or with signature if that's the canonical form)

```rust
impl Tx {
    pub fn id(&self) -> String {
        // Serialize transaction (with or without sig based on requirements)
        let serialized = bincode::serialize(self).unwrap();
        let hash = Sha256::digest(&serialized);
        format!("0x{}", hex::encode(hash))
    }
}
```

### 7. Implementation Flow

**Startup:**
1. Load config
2. If `indexer: true`:
   - Check if `indexed_transactions` column family exists
   - Create if missing
   - Initialize `TransactionIndexer` and store in `ConsensusConnection`

**During Consensus:**
1. In `deliver_tx`: Track successful transactions in a buffer
2. In `commit`: After block commit, index all transactions from the buffer
3. Clear buffer after indexing

**Query Time:**
1. Parse query parameters
2. Query RocksDB using appropriate index keys
3. Deserialize and return results

### 8. Error Handling

- Indexing failures should log but not block consensus
- Use `handle_recoverable_eld_error()` for non-fatal indexing errors
- API errors return appropriate HTTP status codes

### 9. Future Extensibility

- Event indexing (store ABCI events alongside transactions)
- Advanced filtering (date ranges, amount ranges)
- Aggregation endpoints (transaction counts, volume stats)
- WebSocket subscriptions for real-time transaction updates

### 10. Testing Considerations

- Unit tests for transaction ID calculation
- Integration tests for indexing and querying
- Test pagination edge cases
- Test with indexer disabled (should not affect consensus)

This design:
- Keeps indexing optional via config
- Uses existing RocksDB infrastructure
- Follows existing patterns (column families, API structure)
- Is non-blocking (indexing failures don't affect consensus)
- Provides a clean query API for the explorer

Should I proceed with implementation, or adjust any part of the design?