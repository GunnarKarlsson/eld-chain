# P2P Capacity and Storage Protocol

This document describes the P2P protocol for storage allocation, storage sync, and capacity validation between Eld nodes and storage clients. This protocol enables Android clients and other storage providers to participate in the Eld storage network.

## Overview

The P2P protocol uses libp2p with GossipSub for message broadcasting and mDNS for peer discovery. The protocol handles:

1. **Content Synchronization**: Nodes announce available content and respond to content requests
2. **Capacity Challenges**: Validators challenge storage providers to prove they maintain allocated capacity
3. **Proof Validation**: Storage providers respond to challenges with Merkle proofs

## Network Configuration

### P2P Ports
- **TCP Port**: Default `4001` (configurable via `p2p_tcp_port`)
- **UDP Port**: Default `4002` (configurable via `p2p_udp_port`)

### P2P Stack
- **Transport**: TCP and QUIC (UDP)
- **Security**: Noise protocol with Ed25519 keypairs
- **Multiplexing**: Yamux
- **Discovery**: mDNS (local network)
- **PubSub**: GossipSub

## Message Types

All messages are serialized using **bincode** (Rust's binary serialization format) before transmission over GossipSub.

### SyncMsg Enum

The protocol uses a single enum `SyncMsg` that contains all message types:

```rust
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum SyncMsg {
    Announce {
        content_id: String,
    },
    ContentRequest {
        content_id: String,
    },
    ContentResponse {
        content_id: String,
        content: String,  // Base64-encoded content
    },
    CapacityChallenge {
        challenge_id: String,
        challenger: String,
        provider_id: String,
        chunk_indices: Vec<usize>,
        block_height: i64,
        merkle_root: [u8; 32],
        seed: [u8; 32],
        expiration_block: i64,
        timestamp: u64,
    },
    CapacityChallengeResponse {
        challenge_id: String,
        provider_id: String,
        challenger: String,
        block_height: i64,
        proofs: Vec<ChunkProof>,
        generated_at: u64,
    },
}
```

## Message Structures

### 1. Announce

**Purpose**: Broadcast that a node has specific content available.

**Fields**:
- `content_id` (String): SHA256 hash of the content, hex-encoded

**Topic**: `eld-content-sync` (default content sync topic)

**Serialization**: Bincode

**Usage**: Nodes broadcast this when they have content available. Other nodes that don't have the content will respond with `ContentRequest`.

### 2. ContentRequest

**Purpose**: Request specific content from peers.

**Fields**:
- `content_id` (String): SHA256 hash of the requested content, hex-encoded

**Topic**: `eld-content-sync`

**Serialization**: Bincode

**Usage**: Sent when a node receives an `Announce` for content it doesn't have, or during periodic sync checks.

### 3. ContentResponse

**Purpose**: Provide requested content to peers.

**Fields**:
- `content_id` (String): SHA256 hash of the content, hex-encoded
- `content` (String): Base64-encoded content bytes

**Topic**: `eld-content-sync`

**Serialization**: Bincode

**Content Encoding**: The raw content bytes are Base64-encoded before being placed in the `content` field.

**Usage**: Sent in response to a `ContentRequest` when the node has the requested content locally.

### 4. CapacityChallenge

**Purpose**: Challenge a storage provider to prove they maintain allocated capacity.

**Fields**:
- `challenge_id` (String): Unique challenge identifier (SHA256 hash of challenge details), hex-encoded
- `challenger` (String): Validator address (0x... format) issuing the challenge
- `provider_id` (String): Storage provider address (0x... format) being challenged
- `chunk_indices` (Vec<usize>): List of chunk indices to prove (typically 10 chunks)
- `block_height` (i64): Block height when challenge was issued
- `merkle_root` ([u8; 32]): Expected Merkle root from on-chain state
- `seed` ([u8; 32]): Provider's seed for deterministic verification
- `expiration_block` (i64): Block height when challenge expires
- `timestamp` (u64): Unix timestamp when challenge was created

**Topic**: `eld-storage-challenge-topic-{provider_id}` (provider-specific topic)

**Serialization**: Bincode

**Challenge ID Generation**:
```rust
let mut hasher = Sha256::new();
hasher.update(challenger.as_bytes());
hasher.update(provider_id.as_bytes());
hasher.update(&block_height.to_be_bytes());
hasher.update(&timestamp.to_be_bytes());
for &idx in &chunk_indices {
    hasher.update(&idx.to_be_bytes());
}
let challenge_id = hex::encode(hasher.finalize());
```

**Chunk Selection**: Deterministic selection based on:
- Epoch number
- Provider ID
- Block height
- Validator address

**Usage**: Sent by storage validators to challenge storage providers. Providers must subscribe to their challenge topic: `eld-storage-challenge-topic-{provider_id}`.

### 5. CapacityChallengeResponse

**Purpose**: Respond to a capacity challenge with Merkle proofs.

**Fields**:
- `challenge_id` (String): Challenge ID from the original challenge
- `provider_id` (String): Storage provider address (0x... format)
- `challenger` (String): Validator address (0x... format) that issued the challenge
- `block_height` (i64): Block height from the original challenge
- `proofs` (Vec<ChunkProof>): List of proofs for requested chunks
- `generated_at` (u64): Unix timestamp when proofs were generated

**Topic**: `eld-storage-proof-topic-{provider_id}` (provider-specific topic)

**Serialization**: Bincode

**Usage**: Sent by storage providers in response to `CapacityChallenge`. Validators must subscribe to the proof topic before sending challenges.

## ChunkProof Structure

Each proof in `CapacityChallengeResponse.proofs` is a `ChunkProof`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkProof {
    pub chunk_index: usize,           // Index of the chunk in the capacity file
    pub chunk_data: Vec<u8>,          // Actual chunk data from disk (max 1024 bytes)
    pub chunk_hash: [u8; 32],        // SHA256 hash of chunk_data
    pub merkle_proof: Vec<[u8; 32]>, // Merkle path from leaf to root
    pub slot_state: SlotState,        // Proof/Open/Content indicator
}
```

### SlotState Enum

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SlotState {
    Proof,                            // Deterministic proof slot (80% of slots)
    Open,                             // Open slot (20% of slots, contains zeros)
    Content {                         // Content slot (contains user data)
        deal_id: String,
        committed_hash: [u8; 32],
    },
}
```

### Chunk Hash Calculation

Chunk hash is computed using SHA256:
```rust
let mut hasher = Sha256::new();
hasher.update(&chunk_data);
let chunk_hash = hasher.finalize();
```

## Protocol Flows

### Content Synchronization Flow

1. **Node A** has content with `content_id = "abc123..."` and broadcasts `Announce { content_id: "abc123..." }` on topic `eld-content-sync`.

2. **Node B** receives the `Announce` and checks if it has the content locally:
   - If content exists: Ignore the announce
   - If content missing: Send `ContentRequest { content_id: "abc123..." }` on topic `eld-content-sync`

3. **Node A** (or any node with the content) receives `ContentRequest`:
   - If content exists locally: Read content, Base64-encode it, send `ContentResponse { content_id: "abc123...", content: "base64data..." }` on topic `eld-content-sync`
   - If content missing: Ignore the request

4. **Node B** receives `ContentResponse`:
   - Decode Base64 content
   - Verify `content_id` matches hash of decoded content (TODO: currently not verified)
   - Chunk content using `MAX_CHUNK_SIZE` (1024 bytes)
   - Store chunks in capacity slots
   - Remove from missing content tracker

### Periodic Sync

Nodes periodically check for missing content (default: every 300 seconds):

1. Query missing content tracker for all missing content IDs
2. For each missing content ID:
   - Check if content now exists locally (cleanup)
   - If still missing: Broadcast `ContentRequest` for that content
   - Update attempt count and timestamp

### Capacity Challenge Flow

1. **Storage Validator** (selected for current epoch):
   - Selects providers to challenge (typically 5 providers per epoch)
   - For each provider:
     - Generates deterministic chunk indices (typically 10 chunks)
     - Creates `CapacityChallenge` message
     - Subscribes to proof topic: `eld-storage-proof-topic-{provider_id}`
     - Publishes challenge to topic: `eld-storage-challenge-topic-{provider_id}`

2. **Storage Provider** receives `CapacityChallenge`:
   - Verifies `provider_id` matches its own address
   - Verifies Merkle root matches on-chain state
   - Reads chunk data from capacity file for each requested `chunk_index`
   - Generates Merkle proof for each chunk
   - Creates `CapacityChallengeResponse` with proofs
   - Publishes response to topic: `eld-storage-proof-topic-{provider_id}`

3. **Storage Validator** receives `CapacityChallengeResponse`:
   - Validates each proof:
     - Verify chunk hash matches chunk data
     - Verify Merkle proof validates against expected Merkle root
     - Verify chunk indices match request
   - If valid: Submit `VerifiedProof` transaction to blockchain
   - If invalid: Record failure (may lead to slashing after threshold)

## Topic Naming Conventions

### Content Sync Topic
- **Name**: `eld-content-sync`
- **Usage**: All content synchronization messages (Announce, ContentRequest, ContentResponse)
- **Subscribers**: All nodes in the network

### Challenge Topics
- **Prefix**: `eld-storage-challenge-topic-`
- **Format**: `eld-storage-challenge-topic-{provider_id}`
- **Example**: `eld-storage-challenge-topic-0x1234...abcd`
- **Usage**: Capacity challenges for specific providers
- **Subscribers**: Storage providers subscribe to their own challenge topic

### Proof Topics
- **Prefix**: `eld-storage-proof-topic-`
- **Format**: `eld-storage-proof-topic-{provider_id}`
- **Example**: `eld-storage-proof-topic-0x1234...abcd`
- **Usage**: Proof responses from storage providers
- **Subscribers**: Storage validators subscribe to proof topics for providers they challenge

## Serialization Details

### Bincode Serialization

All `SyncMsg` variants are serialized using **bincode** (Rust's binary serialization):

```rust
// Serialization
let msg = SyncMsg::Announce { content_id: "abc123".to_string() };
let serialized = bincode::serialize(&msg)?;

// Deserialization
let msg: SyncMsg = bincode::deserialize(&data)?;
```

### Base64 Encoding

Content in `ContentResponse` is Base64-encoded using the standard Base64 alphabet (RFC 4648):

```rust
// Encoding
let content_base64 = base64::engine::general_purpose::STANDARD.encode(&content_bytes);

// Decoding
let content_bytes = base64::engine::general_purpose::STANDARD.decode(&content_base64)?;
```

### Hex Encoding

- `content_id`: Hex-encoded SHA256 hash (64 hex characters)
- `challenge_id`: Hex-encoded SHA256 hash (64 hex characters)
- `challenger`: Hex-encoded address with `0x` prefix
- `provider_id`: Hex-encoded address with `0x` prefix
- `merkle_root`: 32-byte array (not hex-encoded in message, but hex-encoded for logging)
- `seed`: 32-byte array (not hex-encoded in message)
- `chunk_hash`: 32-byte array (not hex-encoded in message)

## Storage Allocation

### Capacity File Structure

Storage providers allocate a fixed-size capacity file:

- **File Path**: `{capacity_dir}/capacity_{provider_id}.dat`
- **Slot Map Path**: `{capacity_dir}/capacity_{provider_id}.slots.json`
- **Content Slot Map Path**: `{capacity_dir}/capacity_{provider_id}.content-slot-map.json`

### Slot Allocation

The capacity file is divided into fixed-size chunks:

- **Chunk Size**: `MAX_CHUNK_SIZE = 1024` bytes (1 KB)
- **Slot Types**:
  - **Proof Slots**: 80% of slots contain deterministic proof data
  - **Open Slots**: 20% of slots contain zeros (available for content)
  - **Content Slots**: Open slots that have been filled with user content

### Slot Map Structure

```rust
pub struct SlotMap {
    pub slots: Vec<Slot>,
    pub capacity_bytes: u64,
    pub seed: [u8; 32],
    pub provider_id: String,
}

pub struct Slot {
    pub offset: u64,      // Byte offset in capacity file
    pub size: usize,      // Size in bytes (typically 1024)
    pub state: SlotState,  // Proof/Open/Content
}
```

### Merkle Tree

A Merkle tree is built from all slot hashes:

- **Leaf Nodes**: SHA256 hash of each chunk's data
- **Internal Nodes**: SHA256 hash of left child + right child
- **Root**: The Merkle root is stored on-chain and used for proof validation

### Proof Generation

When generating proofs for a challenge:

1. Read chunk data from capacity file at `slot.offset`
2. Calculate `chunk_hash = SHA256(chunk_data)`
3. Generate Merkle proof path from leaf to root
4. Include `slot_state` to indicate slot type

### Proof Validation

When validating proofs:

1. Verify `chunk_hash == SHA256(chunk_data)` for each proof
2. Verify Merkle proof path validates against expected Merkle root
3. Verify chunk indices match the challenge request
4. Verify slot states are consistent (Proof slots have deterministic data, Open slots have zeros)

## Storage Provider Implementation Requirements

This section describes what a storage provider must implement to participate in the Eld storage network protocol.

### Prerequisites

1. **Wallet and Address**:
   - Generate or load an Ed25519 keypair
   - Derive provider address (0x... hex format) from public key
   - Ensure wallet has sufficient balance for transaction fees

2. **On-Chain Registration**:
   - Submit `RegisterCapacity` transaction with:
     - `provider_id`: Provider's wallet address
     - `capacity_bytes`: Total allocated capacity in bytes
     - `seed`: 32-byte random seed for deterministic slot generation
     - `merkle_root`: Initial Merkle root (all Proof/Open slots)
     - `chunk_count`: Number of chunks (capacity_bytes / MAX_CHUNK_SIZE)
   - Wait for transaction confirmation on-chain
   - Provider will appear in `app_state.envelope.storage_providers`

### Initial Setup

#### 1. Capacity Allocation

**Step 1: Allocate Capacity File**
- Create capacity file: `{capacity_dir}/capacity_{provider_id}.dat`
- Pre-allocate file to desired size (e.g., 1 GB, 10 GB)
- File size determines total capacity: `capacity_bytes = file_size`

**Step 2: Generate Slot Map**
- Calculate chunk count: `chunk_count = ceil(capacity_bytes / MAX_CHUNK_SIZE)`
- Generate deterministic slot offsets using Fisher-Yates shuffle:
  - Seed: `hash("SLOT_OFFSET_MAP", seed, provider_id)`
  - Permute slot indices deterministically
- Assign slot states (80% Proof, 20% Open):
  - Seed: `hash("SLOT_STATE_MAP", seed, provider_id)`
  - For each chunk: `if (random % 100) < 80 => Proof, else => Open`
- Create `SlotMap` structure:
  ```rust
  SlotMap {
      slots: Vec<Slot>,  // One slot per chunk
      capacity_bytes: u64,
      seed: [u8; 32],
      provider_id: String,
  }
  ```
- Save slot map to: `{capacity_dir}/capacity_{provider_id}.slots.json`

**Step 3: Write Initial Data**
- For each slot:
  - **Proof slots**: Write deterministic data:
    ```rust
    chunk_data = generate_chunk_data(provider_id, seed, chunk_index)
    ```
  - **Open slots**: Write zeros (`vec![0u8; MAX_CHUNK_SIZE]`)
- Write data to capacity file at `slot.offset`
- Sync file to disk (fsync)

**Step 4: Build Merkle Tree**
- Calculate leaf hash for each chunk: `SHA256(chunk_data)`
- Build binary Merkle tree bottom-up:
  - Leaf nodes: chunk hashes
  - Internal nodes: `SHA256(left_child || right_child)`
  - Root: Final Merkle root
- Store Merkle tree in memory for proof generation

**Step 5: Register On-Chain**
- Submit `RegisterCapacity` transaction with:
  - Initial Merkle root from Step 4
  - All other registration parameters
- Wait for transaction confirmation

### P2P Network Setup

#### 1. Initialize P2P Stack

```rust
// Create Ed25519 keypair
let local_key = identity::Keypair::generate_ed25519();
let local_peer_id = PeerId::from(local_key.public());

// Configure GossipSub
let gossipsub_config = gossipsub::ConfigBuilder::default()
    .heartbeat_interval(Duration::from_secs(1))
    .flood_publish(true)
    .allow_self_origin(true)
    .build()?;

// Create swarm with TCP and QUIC
let mut swarm = SwarmBuilder::with_existing_identity(local_key)
    .with_tcp(noise_config, yamux_config)?
    .with_quic()
    .with_behaviour(|key| EldBehaviour {
        gossipsub: gossipsub::Behaviour::new(
            MessageAuthenticity::Signed(key),
            gossipsub_config,
        )?,
        mdns: mdns::Behaviour::new(mdns_config, key.public().to_peer_id())?,
    })?
    .build();

// Listen on ports
swarm.listen_on("/ip4/0.0.0.0/tcp/4001".parse()?)?;
swarm.listen_on("/ip4/0.0.0.0/udp/4002/quic-v1".parse()?)?;
```

#### 2. Subscribe to Topics

**Required Subscriptions**:
- `eld-content-sync`: For content synchronization
- `eld-storage-challenge-topic-{provider_id}`: To receive challenges

```rust
// Subscribe to content sync topic
let content_topic = IdentTopic::new("eld-content-sync");
swarm.behaviour_mut().gossipsub.subscribe(&content_topic)?;

// Subscribe to challenge topic
let challenge_topic = IdentTopic::new(
    &format!("eld-storage-challenge-topic-{}", provider_id)
);
swarm.behaviour_mut().gossipsub.subscribe(&challenge_topic)?;
```

### Content Storage Implementation

#### 1. Store Content

When receiving `ContentResponse` or storing new content:

```rust
async fn store_content(content_id: String, content_bytes: Vec<u8>) -> Result<(), Error> {
    // 1. Chunk content
    let chunks: Vec<Vec<u8>> = content_bytes
        .chunks(MAX_CHUNK_SIZE)
        .map(|c| c.to_vec())
        .collect();
    
    // 2. Find available Open slots
    let mut slot_allocator = slot_allocator.lock().await;
    let available_slots = slot_allocator.find_open_slots(chunks.len())?;
    
    // 3. Write chunks to slots
    let mut content_hashes = Vec::new();
    for (i, chunk) in chunks.iter().enumerate() {
        let slot_index = available_slots[i];
        let slot = &slot_allocator.slots[slot_index];
        
        // Write chunk to file
        file.seek(SeekFrom::Start(slot.offset))?;
        file.write_all(chunk)?;
        
        // Update slot state to Content
        slot_allocator.slots[slot_index].state = SlotState::Content {
            deal_id: content_id.clone(),
            committed_hash: sha256(chunk),
        };
        
        content_hashes.push(sha256(chunk));
    }
    
    // 4. Update Merkle tree
    let new_merkle_root = update_merkle_tree(available_slots, content_hashes)?;
    
    // 5. Save slot map
    slot_allocator.save_slot_map()?;
    
    // 6. Submit merkle root update transaction
    submit_merkle_root_update(new_merkle_root).await?;
    
    // 7. Track content-to-slot mapping
    content_slot_map.insert(content_id, available_slots);
    
    Ok(())
}
```

#### 2. Retrieve Content

```rust
async fn get_content(content_id: String) -> Result<Vec<u8>, Error> {
    // 1. Look up slot indices
    let slot_indices = content_slot_map.get(&content_id)?;
    
    // 2. Read chunks from slots
    let mut content = Vec::new();
    for &slot_index in slot_indices {
        let slot = &slot_allocator.slots[slot_index];
        file.seek(SeekFrom::Start(slot.offset))?;
        let mut chunk = vec![0u8; slot.size];
        file.read_exact(&mut chunk)?;
        content.extend_from_slice(&chunk);
    }
    
    // 3. Verify content_id matches hash
    let calculated_id = hex::encode(sha256(&content));
    if calculated_id != content_id {
        return Err(Error::ContentMismatch);
    }
    
    Ok(content)
}
```

### Challenge Handling Implementation

#### 1. Receive Challenge

When `CapacityChallenge` message is received:

```rust
async fn handle_capacity_challenge(challenge: CapacityChallenge) -> Result<(), Error> {
    // 1. Verify challenge is for this provider
    if challenge.provider_id != my_provider_id {
        return Ok(()); // Ignore challenges for other providers
    }
    
    // 2. Verify Merkle root matches on-chain state
    let on_chain_root = query_merkle_root_from_chain(&challenge.provider_id).await?;
    if on_chain_root != challenge.merkle_root {
        return Err(Error::MerkleRootMismatch);
    }
    
    // 3. Check challenge expiration
    let current_block = get_current_block_height().await?;
    if current_block > challenge.expiration_block {
        return Err(Error::ChallengeExpired);
    }
    
    // 4. Generate proofs for requested chunks
    let proofs = generate_proofs(&challenge).await?;
    
    // 5. Send proof response
    send_proof_response(challenge, proofs).await?;
    
    Ok(())
}
```

#### 2. Generate Proofs

```rust
async fn generate_proofs(challenge: &CapacityChallenge) -> Result<Vec<ChunkProof>, Error> {
    let mut proofs = Vec::new();
    let slot_map = slot_allocator.get_slot_map()?;
    
    for &chunk_index in &challenge.chunk_indices {
        // 1. Get slot info
        let slot = &slot_map.slots[chunk_index];
        
        // 2. Read chunk data from file
        file.seek(SeekFrom::Start(slot.offset))?;
        let mut chunk_data = vec![0u8; slot.size];
        file.read_exact(&mut chunk_data)?;
        
        // 3. Calculate chunk hash
        let chunk_hash = sha256(&chunk_data);
        
        // 4. Generate Merkle proof path
        let merkle_proof = merkle_tree.generate_proof(chunk_index);
        
        // 5. Create proof
        proofs.push(ChunkProof {
            chunk_index,
            chunk_data,
            chunk_hash,
            merkle_proof,
            slot_state: slot.state.clone(),
        });
    }
    
    Ok(proofs)
}
```

#### 3. Send Proof Response

```rust
async fn send_proof_response(
    challenge: CapacityChallenge,
    proofs: Vec<ChunkProof>,
) -> Result<(), Error> {
    let response = SyncMsg::CapacityChallengeResponse {
        challenge_id: challenge.challenge_id,
        provider_id: challenge.provider_id,
        challenger: challenge.challenger,
        block_height: challenge.block_height,
        proofs,
        generated_at: current_timestamp(),
    };
    
    // Serialize message
    let serialized = bincode::serialize(&response)?;
    
    // Publish to proof topic
    let proof_topic = format!("eld-storage-proof-topic-{}", challenge.provider_id);
    let topic = IdentTopic::new(&proof_topic);
    swarm.behaviour_mut().gossipsub.publish(topic, serialized)?;
    
    Ok(())
}
```

### Content Synchronization Implementation

#### 1. Handle Announce Messages

```rust
async fn handle_announce(announce: Announce) {
    if has_content(&announce.content_id) {
        return; // Already have content
    }
    
    // Request content from peers
    let request = SyncMsg::ContentRequest {
        content_id: announce.content_id,
    };
    broadcast_message("eld-content-sync", request).await;
}
```

#### 2. Handle Content Requests

```rust
async fn handle_content_request(request: ContentRequest) {
    if !has_content(&request.content_id) {
        return; // Don't have content
    }
    
    // Read content
    let content_bytes = get_content(&request.content_id).await?;
    
    // Base64 encode
    let content_base64 = base64_encode(&content_bytes);
    
    // Send response
    let response = SyncMsg::ContentResponse {
        content_id: request.content_id,
        content: content_base64,
    };
    broadcast_message("eld-content-sync", response).await;
}
```

#### 3. Handle Content Responses

```rust
async fn handle_content_response(response: ContentResponse) {
    // Decode content
    let content_bytes = base64_decode(&response.content)?;
    
    // Verify content_id matches hash (TODO: currently not enforced)
    // let calculated_id = hex::encode(sha256(&content_bytes));
    // assert_eq!(calculated_id, response.content_id);
    
    // Store content in slots
    store_content(response.content_id, content_bytes).await?;
    
    // Remove from missing content tracker
    missing_content_tracker.mark_synced(&response.content_id)?;
}
```

### Merkle Tree Maintenance

#### 1. Initial Tree Construction

```rust
fn build_merkle_tree(slot_map: &SlotMap) -> CapacityProofMerkleTree {
    // Calculate leaf hashes
    let mut leaf_hashes = Vec::new();
    for slot in &slot_map.slots {
        let chunk_data = read_chunk_at_offset(slot.offset)?;
        leaf_hashes.push(sha256(&chunk_data));
    }
    
    // Build tree bottom-up
    CapacityProofMerkleTree::build(leaf_hashes)
}
```

#### 2. Incremental Updates

When content is stored in slots:

```rust
fn update_merkle_tree(
    slot_indices: &[usize],
    new_content_hashes: &[[u8; 32]],
) -> Result<[u8; 32], Error> {
    // Update leaf hashes for affected slots
    for (i, &slot_index) in slot_indices.iter().enumerate() {
        merkle_tree.update_leaf(slot_index, new_content_hashes[i]);
    }
    
    // Recalculate tree from updated leaves
    let new_root = merkle_tree.recalculate_root();
    
    Ok(new_root)
}
```

#### 3. Proof Generation

```rust
fn generate_proof(chunk_index: usize) -> Vec<[u8; 32]> {
    // Generate Merkle proof path from leaf to root
    let mut proof_path = Vec::new();
    let mut current_index = chunk_index;
    
    while current_index > 0 {
        let sibling_index = if current_index % 2 == 0 {
            current_index - 1
        } else {
            current_index + 1
        };
        
        proof_path.push(merkle_tree.get_node_hash(sibling_index));
        current_index = (current_index - 1) / 2;
    }
    
    proof_path
}
```

### On-Chain Updates

#### 1. Register Capacity

Submit `RegisterCapacity` transaction:
- Transaction type: `"RegisterCapacity"`
- Fields: `provider_id`, `capacity_bytes`, `seed`, `merkle_root`, `chunk_count`
- Sign with provider's wallet
- Submit via Tendermint RPC

#### 2. Update Merkle Root

After storing content, submit `UpdateCapacityMerkleRoot` transaction:
- Transaction type: `"UpdateCapacityMerkleRoot"`
- Fields: `sender` (provider_id), `merkle_root` (new root)
- Sign with provider's wallet
- Submit via Tendermint RPC

### Periodic Tasks

#### 1. Content Sync Check

Run periodically (default: every 300 seconds):

```rust
async fn periodic_sync_check() {
    let missing_content = missing_content_tracker.get_all_missing()?;
    
    for content_id in missing_content {
        // Check if content now exists
        if has_content(&content_id) {
            missing_content_tracker.mark_synced(&content_id)?;
            continue;
        }
        
        // Request content
        let request = SyncMsg::ContentRequest { content_id };
        broadcast_message("eld-content-sync", request).await;
        
        // Update attempt count
        missing_content_tracker.increment_attempts(&content_id)?;
    }
}
```

#### 2. Registration Status Check

Check if capacity is registered on-chain:

```rust
async fn check_registration_status() -> Result<CapacityRegistrationStatus, Error> {
    // Query app_state.envelope.storage_providers
    let providers = query_storage_providers().await?;
    
    if providers.iter().any(|p| p.address == my_provider_id) {
        Ok(CapacityRegistrationStatus::Registered)
    } else {
        Ok(CapacityRegistrationStatus::NotRegistered)
    }
}
```

### Error Handling

1. **Challenge Errors**:
   - Invalid Merkle root: Log error, do not respond
   - Expired challenge: Log warning, do not respond
   - Missing chunks: Return error in proof response

2. **Storage Errors**:
   - Disk full: Reject content storage, log error
   - File I/O errors: Retry with exponential backoff
   - Slot map corruption: Rebuild from capacity file

3. **Network Errors**:
   - Topic subscription failures: Retry subscription
   - Message send failures: Log and continue
   - Peer disconnections: Automatic reconnection via mDNS

### Summary Checklist

A storage provider must implement:

- [ ] Ed25519 keypair generation and wallet management
- [ ] Capacity file allocation and slot map generation
- [ ] Merkle tree construction and maintenance
- [ ] On-chain capacity registration
- [ ] P2P stack initialization (libp2p, GossipSub, mDNS)
- [ ] Topic subscriptions (content sync, challenge topic)
- [ ] Content storage and retrieval
- [ ] Content synchronization (Announce, Request, Response)
- [ ] Challenge handling and proof generation
- [ ] Merkle root updates after content storage
- [ ] Periodic sync checks for missing content
- [ ] Error handling and retry logic

## Constants

### Challenge Constants
- `CHALLENGES_PER_EPOCH`: 5 providers challenged per epoch
- `CHUNKS_PER_CHALLENGE`: 10 chunks per challenge
- `PROOF_RATIO`: 0.8 (80% proof slots, 20% open slots)

### Content Constants
- `MAX_CHUNK_SIZE`: 1024 bytes (1 KB)

### Topic Prefixes
- `ELD_STORAGE_CHALLENGE_TOPIC_PREFIX`: `"eld-storage-challenge-topic-"`
- `ELD_STORAGE_PROOF_TOPIC_PREFIX`: `"eld-storage-proof-topic-"`

## Implementation Notes for Android Clients

### Required Libraries

1. **libp2p**: For P2P networking (GossipSub, mDNS)
2. **Bincode**: For message serialization/deserialization
3. **Base64**: For content encoding/decoding
4. **SHA256**: For content hashing and Merkle tree operations

### Key Implementation Steps

1. **Initialize P2P Stack**:
   - Create Ed25519 keypair
   - Initialize GossipSub with signed messages
   - Initialize mDNS for peer discovery
   - Listen on TCP and QUIC ports

2. **Subscribe to Topics**:
   - Subscribe to `eld-content-sync` for content sync
   - Subscribe to `eld-storage-challenge-topic-{provider_id}` for challenges
   - Subscribe to `eld-storage-proof-topic-{provider_id}` when challenging providers

3. **Handle Messages**:
   - Deserialize incoming messages using bincode
   - Route messages to appropriate handlers based on `SyncMsg` variant
   - Implement content storage and retrieval logic
   - Implement capacity proof generation and validation

4. **Content Storage**:
   - Implement chunking logic (1024 bytes per chunk)
   - Store chunks in capacity slots
   - Track content-to-slot mappings
   - Build and maintain Merkle tree

5. **Capacity Proofs**:
   - Implement Merkle tree construction
   - Implement proof generation for challenges
   - Implement proof validation for received responses

### Message Handling Pseudocode

```kotlin
// Example message handling structure
fun handleSyncMessage(data: ByteArray) {
    val msg = bincode.deserialize<SyncMsg>(data)
    
    when (msg) {
        is SyncMsg.Announce -> {
            if (!hasContent(msg.contentId)) {
                sendContentRequest(msg.contentId)
            }
        }
        is SyncMsg.ContentRequest -> {
            if (hasContent(msg.contentId)) {
                val content = readContent(msg.contentId)
                val base64Content = base64Encode(content)
                sendContentResponse(msg.contentId, base64Content)
            }
        }
        is SyncMsg.ContentResponse -> {
            val content = base64Decode(msg.content)
            storeContent(msg.contentId, content)
        }
        is SyncMsg.CapacityChallenge -> {
            if (msg.providerId == myProviderId) {
                generateAndSendProofs(msg)
            }
        }
        is SyncMsg.CapacityChallengeResponse -> {
            validateProofs(msg)
        }
    }
}
```

## Error Handling

### Common Errors

1. **Deserialization Errors**: Invalid bincode data
   - Log error and discard message
   - Do not propagate to other peers

2. **Content Not Found**: Requested content doesn't exist
   - Ignore `ContentRequest` if content missing
   - Continue periodic sync attempts

3. **Invalid Proof**: Proof validation fails
   - Record failure in challenge tracker
   - May lead to slashing after threshold

4. **Topic Subscription Failures**: Failed to subscribe to topic
   - Log warning but continue operation
   - Retry subscription if needed

## Security Considerations

1. **Message Signing**: All GossipSub messages are signed with Ed25519 keypairs
2. **Merkle Root Verification**: Providers verify Merkle root matches on-chain state
3. **Challenge Expiration**: Challenges expire at `expiration_block` to prevent replay attacks
4. **Deterministic Challenges**: Challenge generation is deterministic to prevent manipulation
5. **Content Verification**: Content IDs should be verified against content hash (TODO: currently not enforced)

## References

- Implementation: `chain/node_app/src/content/sync/sync_coordinator.rs`
- Capacity Management: `chain/node_app/src/capacity/capacity_manager.rs`
- Challenge Validation: `chain/node_app/src/capacity/challenge_validator.rs`
- Constants: `chain/common/src/constants.rs`
