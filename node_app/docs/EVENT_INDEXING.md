## Event Indexing Design Proposal

### 1. Storage Schema

**Column Family:** `"indexed_events"` (always created, like `indexed_transactions`)

**Data Structure:**
```rust
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IndexedEvent {
    pub tx_id: String,              // Transaction ID (hash)
    pub event_index: u32,           // Index of event within the transaction (0, 1, 2...)
    pub event_type: String,         // Event type (e.g., "Transfer", "ClaimStorageReward")
    pub attributes: Vec<(String, String)>, // Event attributes (key-value pairs)
    pub block_height: u64,          // Block height when event occurred
    pub block_index: u32,           // Transaction index within block
    pub timestamp: u64,             // Unix timestamp
}
```

### 2. Key Structure

**Primary Keys (for direct lookup):**
- `tx_id:{tx_id}:event_index:{index}` → `IndexedEvent` (serialized as JSON)

**Secondary Index Keys (for efficient querying):**

1. **By Event Type (chronological, descending order):**
   - `type:{event_type}:block_height:{height}:block_index:{index}:event_index:{index}` → `tx_id:{tx_id}:event_index:{index}`
   - Use reverse iteration for descending chronological order
   - Format: `type:Transfer:block_height:100:block_index:0:event_index:0`

2. **By Transaction ID (all events for a tx):**
   - `tx_id:{tx_id}:event_index:{index}` → `IndexedEvent` (same as primary key)
   - Iterate with prefix `tx_id:{tx_id}:` to get all events for a transaction

3. **All Events (chronological, descending order):**
   - `block_height:{height}:block_index:{index}:event_index:{index}` → `tx_id:{tx_id}:event_index:{index}`
   - Use reverse iteration for descending order

### 3. Integration Points

**A. Event Capture**
- Location: `chain/node_app/src/consensus/consensus.rs` in `commit()` function
- Extract events from `ResponseDeliverTx` stored in `pending_transactions`
- Each transaction can have multiple events (from `response.events`)

**B. Event Indexing**
- Location: `chain/node_app/src/storage/rocksdb.rs` - new trait method
- Index events after indexing transactions in the `commit()` phase
- Iterate through `pending_transactions`, extract events from each `ResponseDeliverTx`

### 4. Implementation Flow

**In `commit()` function:**
```rust
// After indexing transactions
if let Some(indexer) = &self.indexer {
    let mut pending_txs = self.pending_transactions.lock().unwrap();
    let block_height = current_state.envelope.block_height;
    
    for (block_idx, (tx, response)) in pending_txs.iter().enumerate() {
        let tx_id = indexer.calculate_tx_id(tx);
        
        // Index each event from the response
        for (event_idx, event) in response.events.iter().enumerate() {
            indexer.index_event(
                &tx_id,
                event_idx as u32,
                event,
                block_height,
                block_idx as u32,
            )?;
        }
    }
}
```

### 5. Storage Trait

**New trait method in `TransactionIndexerStorage`:**
```rust
pub trait TransactionIndexerStorage: Send + Sync {
    // ... existing methods ...
    
    /// Index an event with all secondary indexes
    fn index_event(
        &self,
        tx_id: &str,
        event_index: u32,
        event: &Event,  // ABCI Event type
        block_height: u64,
        block_index: u32,
    ) -> Result<(), EldError>;
    
    /// Get all events for a transaction
    fn get_events_by_tx_id(&self, tx_id: &str) -> Result<Vec<IndexedEvent>, EldError>;
    
    /// Get events by type (paginated, descending chronological order)
    fn get_events_by_type(
        &self,
        event_type: &str,
        page: u32,
        limit: u32,
    ) -> Result<(Vec<IndexedEvent>, u64), EldError>;
}
```

### 6. Key Design Decisions

1. **Primary key includes event_index**: One transaction can emit multiple events, so each event needs a unique key.
2. **Chronological ordering**: Use `block_height:block_index:event_index` in secondary keys for natural sorting. RocksDB reverse iteration provides descending order.
3. **Event type filtering**: Prefix `type:{event_type}:` enables efficient filtering by type.
4. **Timestamp storage**: Store timestamp in the value (IndexedEvent struct) for display, not in the key (to avoid key bloat).

### 7. Query Patterns Supported

1. **Get all events for a transaction:**
   - Prefix scan: `tx_id:{tx_id}:`
   - Returns events in order (event_index ascending)

2. **Get events by type (newest first):**
   - Prefix scan: `type:{event_type}:`
   - Reverse iteration for descending chronological order
   - Pagination using offset/limit

3. **Get all events (newest first):**
   - Scan all keys with prefix `block_height:`
   - Reverse iteration for descending order

### 8. Error Handling

- Event indexing failures should log but not block consensus
- Use `handle_recoverable_eld_error()` for non-fatal indexing errors
- If event indexing fails, transaction indexing should still succeed

### 9. Serialization

- Use JSON serialization (same as transactions) to avoid bincode issues with variable-length sequences
- Store `IndexedEvent` as JSON in the database

This design supports:
- Querying by transaction ID
- Querying by event type
- Descending chronological order (newest first)
- Efficient prefix-based lookups
- Multiple events per transaction

Should I proceed with implementation?