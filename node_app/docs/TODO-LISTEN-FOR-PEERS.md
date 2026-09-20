# OBJECTIVE

Listen for new peer events from the tendermint node inside the eld app,
so action (tbd) can be taken when a new node joins

# DESIGN

Correct Dependencies for Tendermint v0.34toml

[dependencies]
tendermint = "0.34.0"
tendermint-rpc = { version = "0.34.0", features = ["websocket-client"] }
tokio = { version = "1", features = ["full"] }

Important: Do not use tendermint-rpc = "0.40" with Tendermint v0.34 — it will fail to connect due to protocol mismatches.
Updated Working Example for v0.34rust

use tendermint_rpc::{Client, WebSocketClient};
use tendermint_rpc::query::Query;
use tendermint_rpc::event::Event;
use tokio;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Connect to Tendermint v0.34 WebSocket endpoint
    let (client, driver) = WebSocketClient::new("ws://127.0.0.1:26657/websocket").await?;

    // Spawn the driver (handles pings, reconnections, etc.)
    tokio::spawn(async move {
        if let Err(e) = driver.run().await {
            eprintln!("WebSocket driver error: {}", e);
        }
    });

    // Subscribe to NewPeer events
    let query = Query::eq("tm.event", "NewPeer");
    let mut subscription = client.subscribe(query).await?;

    println!("Listening for NewPeer events...");

    // Receive events
    while let Ok(response) = subscription.next().await {
        if let Some(event) = response.into_data().ok() {
            println!("New Peer Event: {:#?}", event);
            // Example: extract peer info
            if let Event::NewPeer { peer_id, .. } = event {
                println!("New peer connected: {}", peer_id);
            }
        }
    }

    Ok(())
}

Key Differences from Newer VersionsFeature
v0.34
v0.37+
Query::from(EventType::NewPeer)
Not available
Available
Use Query::eq("tm.event", "NewPeer")
Yes
Yes (but EventType is preferred)
subscription.next().await
Returns Result<Response>
Same
response.into_data()
Yes
Yes
Event::NewPeer variant
Exists
Exists

Event Structure in v0.34 (NewPeer)rust

pub enum Event {
    NewPeer {
        peer_id: String,
        remote_ip: String,
    },
    // ... other variants
}

You can match on it directly.WebSocket URL (Same as Always)text

ws://127.0.0.1:26657/websocket

Or remote:text

ws://your-node.com:26657/websocket

Make sure:rpc.laddr = "tcp://0.0.0.0:26657" in config.toml
RPC is not disabled
Firewall allows port 26657

# IMPLEMENTATION

Reviewing the TODO file and main.rs to understand the current structure.


[4 tools called]


Checking for existing Tendermint client code or configuration:


[3 tools called]


## Current State Analysis

1. Version mismatch: `Cargo.toml` has `tendermint-rpc = "0.40.1"`, but the TODO suggests `0.34.0`. This needs to be resolved first.
2. Config available: `node_host` and `node_port` in the config can be used to build the WebSocket URL.
3. Existing pattern: `main.rs` uses `handle_server_startup` to spawn and manage servers with error handling and graceful shutdown.

## Integration Strategy

### 1. **Version Compatibility First**
Update `Cargo.toml` to match the TODO requirements:
```toml
tendermint-rpc = { version = "0.34.0", features = ["websocket-client"] }
```

### 2. **Create a Peer Listener Service Function**
Add a function similar to the other servers that:
- Connects to Tendermint's WebSocket endpoint
- Spawns the driver task
- Subscribes to NewPeer events
- Handles events in a loop

### 3. **Integration Point in main()**
Add the peer listener after the config is loaded and before/alongside other servers. Suggested location: after line 436 (after WebSocket server starts) or around line 440 (before orchestrator).

### 4. **Proposed Code Structure**

Here's how you could structure it:

```rust
// Add this function before main()
async fn start_peer_listener(
    node_host: String,
    node_port: String,
    shutdown_tx: broadcast::Sender<()>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use tendermint_rpc::{Client, WebSocketClient};
    use tendermint_rpc::query::Query;
    use tendermint_rpc::event::Event;
    use futures_util::StreamExt;
    
    let ws_url = format!("ws://{}:{}/websocket", node_host, node_port);
    info!("Connecting to Tendermint WebSocket at {}", ws_url);
    
    // Connect to Tendermint v0.34 WebSocket endpoint
    let (client, driver) = WebSocketClient::new(&ws_url).await?;

    // Spawn the driver (handles pings, reconnections, etc.)
    tokio::spawn(async move {
        if let Err(e) = driver.run().await {
            error!("WebSocket driver error: {}", e);
        }
    });

    // Subscribe to NewPeer events
    let query = Query::eq("tm.event", "NewPeer");
    let mut subscription = client.subscribe(query).await?;

    info!("Listening for NewPeer events from Tendermint...");

    // Receive events
    while let Ok(response) = subscription.next().await {
        if let Some(event) = response.into_data().ok() {
            info!("New Peer Event: {:#?}", event);
            // Example: extract peer info
            if let Event::NewPeer { peer_id, .. } = event {
                info!("New peer connected: {}", peer_id);
                // TODO: Add your peer handling logic here
                // e.g., update peer registry, trigger sync, etc.
            }
        }
    }

    Ok(())
}
```

### 5. **Integration in main() Function**

Add this after line 436 (after WebSocket server initialization):

```rust
// ... existing code ...

// Get Tendermint node configuration for peer listening
let node_host = config.node_host.clone();
let node_port = config.node_port.clone();

// ... existing code for other servers ...

// Create shutdown channel for peer listener
let shutdown_tx_peer_listener = shutdown_tx.clone();

// Spawn peer listener with error handling
let mut peer_listener_handle = handle_server_startup(
    "Peer Listener".to_string(),
    async move {
        start_peer_listener(node_host, node_port, shutdown_tx_peer_listener).await
    },
    shutdown_tx.clone(),
)
.await;
```

### 6. **Update the Shutdown Loop**

Add the peer listener to the `tokio::select!` loop (around line 629):

```rust
let mut peer_listener_completed = false;

loop {
    tokio::select! {
        // ... existing server handles ...
        result = &mut peer_listener_handle, if !peer_listener_completed => {
            peer_listener_completed = true;
            match result {
                Ok(_) => {
                    info!("Peer listener task completed");
                }
                Err(e) => {
                    error!("Peer listener task panicked: {}", e);
                }
            }
        }
        // ... rest of select! ...
    }
    
    // Update shutdown condition
    if content_completed || cado_completed || consensus_completed || peer_listener_completed {
        let _ = shutdown_tx.send(());
        break;
    }
}
```

### 7. **Error Handling Considerations**

- Connection failures: The TODO code uses `?` which will propagate errors. Consider retry logic or graceful degradation.
- Driver errors: Already handled in the spawned task.
- Subscription errors: The `while let Ok(...)` pattern handles this, but you might want to add reconnection logic.

### 8. **Configuration Validation**

Add validation similar to the WebSocket port check (around line 423):

```rust
let node_host = config.node_host.as_deref().ok_or_else(|| {
    EldError::InitializationError {
        component: "Peer Listener".to_string(),
        details: "node_host configuration is required but not provided".to_string(),
    }
})?;

let node_port = config.node_port.as_deref().ok_or_else(|| {
    EldError::InitializationError {
        component: "Peer Listener".to_string(),
        details: "node_port configuration is required but not provided".to_string(),
    }
})?;
```

## Questions to Consider

1. What should happen when a NewPeer event is received? (e.g., update peer registry, trigger sync, log only)
2. Should the peer listener be optional or required? (affects error handling)
3. Reconnection strategy: should it retry on connection failure or fail fast?
4. Event processing: should peer events trigger async work, or just logging?

This follows the existing server pattern in `main.rs` and integrates with the shutdown mechanism. Should I provide the complete integrated code, or do you want to adjust the approach first?