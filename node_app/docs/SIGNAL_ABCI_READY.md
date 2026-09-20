Checking where `commit()` is called and how block height is tracked to signal readiness when block height > 0:


[4 tools called]


Checking the `ConsensusConnection` structure to see where to add the ready channel:


[3 tools called]


Checking `ConsensusConnection::new()`:


[1 tool called]


Here’s how to detect when block height > 0 and signal readiness using a oneshot channel:

## Solution: Block Height-Based Readiness Detection

### Architecture Overview

1. Add a ready channel to `ConsensusConnection`
2. Check block height in `commit()` and signal when `block_height > 0`
3. Pass the channel through `init_abci_server()` to `ConsensusConnection`
4. Wait for the signal in `main.rs` before submitting the transaction

### Implementation Details

#### 1. Modify `ConsensusConnection` struct

Add fields to track readiness:

```rust
pub struct ConsensusConnection<S>
where
    S: ConsensusConnectionStorage,
{
    // ... existing fields ...
    pub ready_tx: Option<oneshot::Sender<()>>,  // Add this
    pub ready_signaled: Arc<Mutex<bool>>,        // Add this to prevent double-signaling
    // ... rest of fields ...
}
```

#### 2. Modify `ConsensusConnection::new()`

Accept the ready channel:

```rust
pub fn new(
    consensus_config: Arc<Mutex<ConsensusConfig>>,
    committed_state: Arc<Mutex<AppState>>,
    current_state: Arc<Mutex<Option<AppState>>>,
    storage: Arc<S>,
    snapshot_manager: Arc<SnapshotManager<S>>,
    p2p_sync_coordinator: Arc<dyn P2pCoordinatorTrait>,
    ready_tx: Option<oneshot::Sender<()>>,  // Add this parameter
) -> Self {
    Self {
        // ... existing fields ...
        ready_tx,
        ready_signaled: Arc::new(Mutex::new(false)),
        // ... rest of fields ...
    }
}
```

#### 3. Modify `commit()` to check block height and signal

In the `commit()` method, after getting `current_state`:

```rust
async fn commit(&self, _commit_request: RequestCommit) -> ResponseCommit {
    info!("commit()");
    
    // ... existing code to get current_state ...
    
    // Check if we should signal readiness (block_height > 0 and not already signaled)
    if current_state.envelope.block_height > 0 {
        let mut signaled = match self.ready_signaled.lock() {
            Ok(s) => s,
            Err(_) => continue, // Skip if lock fails, not critical
        };
        
        if !*signaled {
            // Signal that the node is ready (first commit after genesis)
            if let Some(tx) = self.ready_tx.take() {
                let _ = tx.send(());
                *signaled = true;
                info!(
                    block_height = current_state.envelope.block_height,
                    "Node is READY: First consensus commit completed"
                );
            }
        }
    }
    
    // ... rest of existing commit() logic ...
}
```

Note: Using `take()` on `ready_tx` ensures we only signal once, even if `ready_signaled` fails to update.

#### 4. Modify `init_abci_server()` to accept and pass ready channel

```rust
pub async fn init_abci_server<S>(
    consensus_config: Arc<Mutex<ConsensusConfig>>,
    storage: Arc<S>,
    snapshot_manager: Arc<SnapshotManager<S>>,
    p2p_sync_coordinator: Arc<dyn crate::content::sync::P2pCoordinatorTrait>,
    init_data: bool,
    ready_tx: Option<oneshot::Sender<()>>,  // Add this parameter
) -> Result<
    Server<ConsensusConnection<S>, MempoolConnection, InfoConnection<S>, SnapshotConnection<S>>,
    EldError,
>
where
    S: ConsensusConnectionStorage,
{
    // ... existing initialization code ...
    
    let consensus = ConsensusConnection::new(
        consensus_config.clone(),
        committed_state_mutex.clone(),
        current_state,
        storage.clone(),
        snapshot_manager.clone(),
        p2p_sync_coordinator,
        ready_tx,  // Pass ready channel here
    );
    
    // ... rest of code ...
}
```

#### 5. Modify `main.rs` to create channel and wait

```rust
// Before initializing ABCI server
let (ready_tx, ready_rx) = oneshot::channel();

let consensus_server = init_abci_server(
    consensus_config.clone(),
    storage.clone(),
    snapshot_manager,
    p2p_sync_coordinator.clone(),
    args.init_data,
    Some(ready_tx),  // Pass ready channel
)
.await
.unwrap_or_else(|e| {
    handle_fatal_eld_error(e);
});

// ... spawn server as before ...

// After spawning the server, wait for readiness
info!("Waiting for node to be ready (block height > 0)...");
match ready_rx.await {
    Ok(_) => {
        info!("Node is READY: First consensus commit completed, block height > 0");
    }
    Err(_) => {
        error!("Ready channel was closed before signaling");
        // Decide whether to continue or exit
    }
}

// Now safe to submit capacity registration transaction
if let Some((capacity_manager, capacity_bytes, seed, merkle_root)) = pending_registration {
    capacity_manager.register_capacity_onchain(capacity_bytes, seed, merkle_root).await?;
}
```

### Important Considerations

1. Block height timing:
   - Genesis: `block_height = 0`
   - After first `commit()`: `block_height > 0`
   - The check happens in `commit()`, which Tendermint calls after the first block is finalized

2. Single signaling:
   - Using `Option::take()` ensures we only signal once
   - `ready_signaled` is a backup guard

3. Error handling:
   - If the channel is dropped before signaling, `ready_rx.await` will return an error
   - Decide whether to continue or exit in that case

4. Timeout (optional):
   - Add a timeout to avoid waiting indefinitely:
   ```rust
   match tokio::time::timeout(Duration::from_secs(30), ready_rx).await {
       Ok(Ok(_)) => info!("Node is READY"),
       Ok(Err(_)) => error!("Ready channel error"),
       Err(_) => error!("Timeout waiting for node readiness"),
   }
   ```

### Alternative: Simpler Approach (No Mutex)

If you prefer to avoid the mutex, you can rely solely on `Option::take()`:

```rust
// In commit()
if current_state.envelope.block_height > 0 {
    if let Some(tx) = self.ready_tx.take() {
        let _ = tx.send(());
        info!("Node is READY: First consensus commit completed");
    }
}
```

This works because `take()` is atomic and ensures only one call succeeds.

### Summary

- Signal readiness when `block_height > 0` in `commit()`
- Use a oneshot channel to notify the main thread
- Wait for the signal before submitting the capacity registration transaction
- More reliable than a fixed delay and aligns with Tendermint’s consensus state

This ensures the node has completed at least one consensus round before submitting transactions.