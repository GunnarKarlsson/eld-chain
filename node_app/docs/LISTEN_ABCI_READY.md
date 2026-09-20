Reviewing how the ABCI server starts and how we can emit a READY event:


[6 tools called]


Reviewing how the server starts and how we can signal readiness:


[3 tools called]


Here are options to implement a READY event instead of a sleep delay:

## Architecture Overview

The ABCI server is spawned via `handle_server_startup()`, which runs `server.run(address)`. We need to signal when the server is bound and ready to accept requests.

## Solution Options

### Option 1: Ready Channel in `handle_server_startup()` (Recommended)

Add an optional `oneshot::Sender` to `handle_server_startup()` and signal readiness after the server binds.

**Flow:**
```
1. Create oneshot channel: (ready_tx, ready_rx) = oneshot::channel()
2. Pass ready_tx to handle_server_startup()
3. In server task:
   - Start server.run() in background
   - Signal ready_tx.send() after binding (or immediately if bind is sync)
4. Main thread: await ready_rx before submitting transaction
```

**Pros:**
- Clean separation of concerns
- Reusable for other servers
- No polling

**Cons:**
- Requires modifying `handle_server_startup()` signature
- Need to determine exact "ready" point

### Option 2: Wrap `server.run()` with Ready Signal

Wrap the `server.run()` call to signal after binding:

```rust
let consensus_server = async move {
    // Signal ready immediately after run() starts (assuming bind is sync)
    ready_tx.send(()).ok();
    
    consensus_server.run(address)
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
};
```

**Pros:**
- Minimal changes to existing code
- No need to modify `handle_server_startup()`

**Cons:**
- Assumes `server.run()` binds synchronously
- Less explicit about readiness

### Option 3: Health Check Polling (Fallback)

Poll a health endpoint until it responds:

```rust
// After spawning server
loop {
    if health_check_succeeds().await {
        break;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
}
```

**Pros:**
- No code changes to server startup
- Works even if bind is async

**Cons:**
- Polling overhead
- Still a delay (though bounded)
- Requires a health endpoint

### Option 4: Modify `init_abci_server()` to Return Ready Channel

Have `init_abci_server()` return both the server and a ready channel, and signal when the server is constructed and ready.

**Pros:**
- Signals readiness at the right abstraction level
- Clear contract

**Cons:**
- Requires understanding when the server is actually ready
- More invasive changes

## Recommended Implementation: Option 1

**Changes needed:**

1. Modify `handle_server_startup()` signature:
```rust
async fn handle_server_startup<T>(
    server_name: String,
    server_future: T,
    shutdown_tx: broadcast::Sender<()>,
    ready_tx: Option<oneshot::Sender<()>>, // Add this
) -> tokio::task::JoinHandle<()>
```

2. In the server task, signal after starting:
```rust
tokio::spawn(async move {
    // Signal ready (server.run() likely binds synchronously)
    if let Some(tx) = ready_tx {
        let _ = tx.send(());
    }
    
    let result = server_future.await;
    // ... rest of error handling
})
```

3. In `main.rs`:
```rust
let (ready_tx, ready_rx) = oneshot::channel();

let consensus_server_handle = handle_server_startup(
    "Consensus Server".to_string(),
    consensus_server,
    shutdown_tx_consensus,
    Some(ready_tx), // Pass ready channel
)
.await;

// Wait for server to be ready
ready_rx.await.expect("Server ready signal");

// Now safe to submit capacity registration transaction
capacity_manager.register_capacity_onchain(...).await?;
```

## Determining "Ready" Point

The ABCI server's `run()` method likely:
1. Binds to the socket synchronously (or very quickly)
2. Then awaits connections

If binding is synchronous, signal immediately after spawning. If it's async, you may need to:
- Check if the ABCI library exposes a "bound" event
- Use a small delay + health check
- Modify the server startup to signal after the first successful bind

## Alternative: Check Server Internals

If the ABCI server exposes a way to check if it's listening, you could:
- Query the server state directly
- Use a readiness probe pattern
- Check if the port is bound using `TcpListener::bind()` (would fail if already bound)

Recommendation: Start with Option 1, signaling immediately after spawning (assuming bind is synchronous). If that's unreliable, add a small delay or a health check as a fallback.