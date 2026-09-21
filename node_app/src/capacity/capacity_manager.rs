use crate::capacity::capacity_registration::{
    CapacityRegistrationService, CapacityRegistrationTxParams,
};
use blake3::Hasher as Blake3Hasher;
use eld_common::address::Address;
use eld_common::capacity::{CapacityConfig, CapacityProofMerkleTree, Slot, SlotAllocator, SlotMap};
use eld_common::capacity_proof::{ChallengeProof, ChunkProof, SlotState};
use eld_common::constants::pinboard::MAX_CHUNK_SIZE;
use eld_common::error::EldError;
use eld_common::{CapacityMerkleRoot, CapacitySeed, ChallengeId};
use hex;
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info};

// Constants matching eld_proof_access
const PROOF_RATIO: f64 = 0.8; // 80% proof slots, 20% open

/// Inputs for [`CapacityManager::generate_capacity_proof`].
pub struct CapacityProofGenerationParams {
    pub challenge_id: ChallengeId,
    pub challenger: Address,
    pub provider_id: Address,
    pub chunk_indices: Vec<usize>,
    pub block_height: i64,
    pub expected_merkle_root: CapacityMerkleRoot,
    pub expiration_block: i64,
    pub timestamp: u64,
}

/// Main manager for capacity proof lifecycle
pub struct CapacityManager {
    config: CapacityConfig,
    /// Wallet name from `ELD_CAPACITY_VALIDATOR_WALLET_NAME` — local slots/`provider_id`
    /// and on-chain `RegisterCapacity` / capacity-validator identity.
    capacity_validator_wallet_name: String,
    slot_allocator: Arc<Mutex<SlotAllocator>>,
    registration_service: Arc<CapacityRegistrationService>,
    merkle_tree: Arc<Mutex<Option<CapacityProofMerkleTree>>>,
    initialized: Arc<Mutex<bool>>,
    cli: Arc<eld_client::facade::ChainClient>,
    consensus_config: Arc<std::sync::Mutex<crate::config::ConsensusConfig>>,
}

impl CapacityManager {
    /// Create a new CapacityManager.
    ///
    /// `capacity_validator_wallet_name` must match a wallet in `wallets.json` whose address equals
    /// `config.provider_id` (the caller is responsible for that consistency). That wallet signs
    /// on-chain `RegisterCapacity` and owns local capacity slots.
    pub fn new(
        config: CapacityConfig,
        capacity_validator_wallet_name: String,
        cli: Arc<eld_client::facade::ChainClient>,
        consensus_config: Arc<std::sync::Mutex<crate::config::ConsensusConfig>>,
    ) -> Self {
        let slot_allocator = SlotAllocator::new(&config.capacity_dir, &config.provider_id);

        let registration_service = CapacityRegistrationService::new();

        Self {
            config,
            capacity_validator_wallet_name,
            slot_allocator: Arc::new(Mutex::new(slot_allocator)),
            registration_service: Arc::new(registration_service),
            merkle_tree: Arc::new(Mutex::new(None)),
            initialized: Arc::new(Mutex::new(false)),
            cli,
            consensus_config,
        }
    }

    /// Initialize capacity proof system
    /// This should be called on node startup
    pub async fn initialize(&self) -> Result<(), EldError> {
        let mut initialized = self.initialized.lock().await;
        if *initialized {
            info!("Capacity manager already initialized");
            return Ok(());
        }

        info!("Initializing capacity manager");

        // Check if slot map exists and load it if present
        let slot_allocator = self.slot_allocator.lock().await;

        if slot_allocator.slot_map_exists() {
            drop(slot_allocator);
            let mut slot_allocator = self.slot_allocator.lock().await;
            slot_allocator.load_slot_map()?;
            info!("Loaded existing slot map");

            // Build merkle tree from loaded slot map
            if let Some(slot_map) = slot_allocator.get_slot_map() {
                let merkle_tree = self.build_merkle_tree(slot_map).await?;
                self.set_merkle_tree(merkle_tree).await;
                info!("Built merkle tree from loaded slot map");
            }
        } else {
            // Slot map will be created when capacity is registered
            info!("No existing slot map found. Slot map will be created during capacity registration.");
        }

        *initialized = true;
        Ok(())
    }

    /// Allocate local capacity if it is not already allocated.
    /// This encapsulates allocation logic so callers don't need to manage seeds or slot maps.
    pub async fn allocate_capacity(&self, capacity_size_mb: u64) -> Result<(), EldError> {
        // Check if capacity is already allocated
        let slot_allocator = self.slot_allocator.lock().await;
        let capacity_already_allocated = slot_allocator.slot_map_exists();
        drop(slot_allocator);

        if !capacity_already_allocated {
            info!(
                capacity_size_mb = capacity_size_mb,
                "Capacity not yet allocated, starting allocation"
            );

            // Generate random seed
            let mut seed_bytes = [0u8; 32];
            let mut rng = rand::rng();
            rng.fill_bytes(&mut seed_bytes);
            let seed = CapacitySeed::new(seed_bytes);

            let capacity_bytes = capacity_size_mb * 1024 * 1024;

            // Allocate capacity (create slot map, write data, build merkle tree)
            match self.allocate_capacity_internal(capacity_bytes, seed).await {
                Ok(merkle_root) => {
                    info!(
                        capacity_size_mb = capacity_size_mb,
                        merkle_root = %merkle_root,
                        "Capacity allocated successfully"
                    );
                }
                Err(e) => {
                    error!(
                        error = %e,
                        "Failed to allocate capacity"
                    );
                    return Err(e);
                }
            }
        } else {
            info!("Capacity already allocated, skipping allocation");
        }

        Ok(())
    }

    /// Check if capacity is registered on-chain
    pub async fn is_registered(&self) -> bool {
        self.registration_service.is_registered().await
    }

    /// Register capacity on-chain
    /// This submits a registration transaction after capacity has been allocated
    /// Handles wallet retrieval, chain ID, and chunk count calculation internally
    pub async fn register_capacity_onchain(
        &self,
        capacity_bytes: u64,
        seed: CapacitySeed,
        merkle_root: CapacityMerkleRoot,
    ) -> Result<(), EldError> {
        info!(
            provider_id = %self.config.provider_id,
            capacity_bytes = capacity_bytes,
            "Registering capacity on-chain"
        );

        // Check if already registered
        if self.is_registered().await {
            info!("Capacity already registered, skipping");
            return Ok(());
        }

        // Get CLI and consensus config
        let cli = &self.cli;
        let consensus_config = &self.consensus_config;

        // Get chain ID from consensus config
        let chain_id = consensus_config
            .lock()
            .unwrap_or_else(|e| {
                error!("Failed to acquire consensus config lock: {}", e);
                std::process::exit(1);
            })
            .chain_id
            .clone();

        let wallet_name = self.capacity_validator_wallet_name.as_str();
        let wallet = cli
            .get_wallet_by_name(wallet_name.to_string())
            .await?
            .ok_or_else(|| EldError::StorageError {
                operation: "register_capacity_onchain".to_string(),
                details: format!(
                    "Failed to get capacity-validator wallet '{wallet_name}' for RegisterCapacity"
                ),
            })?;

        // Calculate chunk count
        let chunk_count = capacity_bytes.div_ceil(MAX_CHUNK_SIZE as u64) as u32;

        // On-chain RegisterCapacity sender is the capacity-validator wallet address.
        let provider_address = wallet.address.hex_with_prefix();

        info!(
            capacity_validator_wallet = %wallet_name,
            register_sender = %provider_address,
            local_provider_id = %self.config.provider_id,
            "Submitting RegisterCapacity with capacity-validator wallet"
        );

        // Submit registration transaction
        self.registration_service
            .submit_capacity_registration_tx(CapacityRegistrationTxParams {
                provider_address: &provider_address,
                capacity_bytes,
                seed,
                merkle_root,
                chunk_count,
                wallet: &wallet,
                cli: cli.as_ref(),
                chain_id: &chain_id,
            })
            .await?;

        info!("Capacity registration transaction submitted successfully");
        Ok(())
    }

    /// Submit a transaction to update the capacity provider's merkle root on-chain
    pub async fn submit_merkle_root_update(
        &self,
        new_merkle_root: CapacityMerkleRoot,
    ) -> Result<(), EldError> {
        use eld_common::tx::{Payload, Tx, UpdateCapacityMerkleRootTx};

        info!(
            provider_id = %self.config.provider_id,
            merkle_root = %new_merkle_root,
            "Submitting merkle root update transaction"
        );

        // Get wallet by provider_id (address)
        let wallet = match self
            .cli
            .get_wallet_by_address(&self.config.provider_id.to_string())
            .await
        {
            Ok(Some(wallet)) => {
                info!(
                    provider_id = %self.config.provider_id,
                    wallet_address = %wallet.address.hex_with_prefix(),
                    "Found wallet for provider"
                );
                wallet
            }
            Ok(None) => {
                error!(
                    provider_id = %self.config.provider_id,
                    "Wallet not found for provider_id - cannot submit merkle root update"
                );
                return Err(EldError::StorageError {
                    operation: "submit_merkle_root_update".to_string(),
                    details: format!(
                        "Failed to get wallet for provider_id '{}'. Make sure the wallet exists and matches the provider_id.",
                        self.config.provider_id
                    ),
                });
            }
            Err(e) => return Err(e),
        };

        // Get chain ID
        let chain_id = self
            .consensus_config
            .lock()
            .map_err(|e| EldError::StorageError {
                operation: "submit_merkle_root_update".to_string(),
                details: format!("Failed to acquire consensus config lock: {e}"),
            })?
            .chain_id
            .clone();

        // Get next nonce
        let provider_address = self.config.provider_id;
        let next_nonce = match self
            .cli
            .get_next_nonce_for_account(provider_address.to_string())
            .await
        {
            Ok(Some(nonce)) => {
                info!(
                    provider_id = %self.config.provider_id,
                    next_nonce = %nonce,
                    "Retrieved next nonce for account"
                );
                nonce
            }
            Ok(None) => {
                error!(
                    provider_id = %self.config.provider_id,
                    "Failed to get next nonce for account"
                );
                return Err(EldError::StorageError {
                    operation: "submit_merkle_root_update".to_string(),
                    details: format!(
                        "Failed to get next nonce for account '{provider_address}'. Account may not exist on-chain yet."
                    ),
                });
            }
            Err(e) => {
                error!(
                    provider_id = %self.config.provider_id,
                    error = %e,
                    "Failed to get next nonce for account"
                );
                return Err(e);
            }
        };

        let sender_addr = provider_address;

        // Create transaction (edge: raw [u8; 32] field)
        let update_tx = UpdateCapacityMerkleRootTx::new(sender_addr, *new_merkle_root.as_bytes())?;

        let mut tx = Tx::new(next_nonce, Payload::new(update_tx), wallet.verifying_key());

        // Calculate dynamic fee
        let fee_config = self.cli.get_fee_config();
        let dynamic_fee = eld_common::fee::calculate_dynamic_fee(&tx, fee_config).map_err(|e| {
            EldError::StorageError {
                operation: "submit_merkle_root_update".to_string(),
                details: format!("Failed to calculate dynamic fee: {e}"),
            }
        })?;
        tx.fee = dynamic_fee.into();

        // Sign transaction
        wallet.sign(&mut tx, &chain_id)?;

        // Verify transaction before sending
        if !wallet.verify(&tx, &chain_id)? {
            error!("Transaction verification failed before submission");
            return Err(EldError::StorageError {
                operation: "submit_merkle_root_update".to_string(),
                details: "Transaction verification failed".to_string(),
            });
        }

        // Serialize to JSON and hex
        let json = serde_json::to_string(&tx).map_err(|e| EldError::StorageError {
            operation: "submit_merkle_root_update".to_string(),
            details: format!("Failed to serialize transaction: {e}"),
        })?;
        let hex_encoded = hex::encode(&json);

        // Submit transaction via RPC
        info!(
            provider_id = %self.config.provider_id,
            tx_nonce = %tx.nonce,
            tx_fee = %tx.fee,
            "Submitting merkle root update transaction to Tendermint RPC"
        );

        match self.cli.send_tx_rpc(&hex_encoded).await {
            Ok(response) => {
                crate::broadcast_log::log_deliver_tx_events(&response);
                info!(
                    provider_id = %self.config.provider_id,
                    merkle_root = new_merkle_root.to_hex(),
                    tx_nonce = %tx.nonce,
                    "Successfully submitted merkle root update transaction to Tendermint RPC"
                );
            }
            Err(e) => {
                error!(
                    provider_id = %self.config.provider_id,
                    merkle_root = new_merkle_root.to_hex(),
                    tx_nonce = %tx.nonce,
                    error = %e,
                    error_details = format!("{:?}", e),
                    "Failed to submit merkle root update transaction to Tendermint RPC"
                );
                return Err(EldError::StorageError {
                    operation: "submit_merkle_root_update".to_string(),
                    details: format!("Failed to submit transaction: {e}"),
                });
            }
        }

        Ok(())
    }

    /// Allocate capacity locally (create slot map, write data, build merkle tree)
    /// This is called on node startup if capacity is not already allocated
    async fn allocate_capacity_internal(
        &self,
        capacity_bytes: u64,
        seed: CapacitySeed,
    ) -> Result<CapacityMerkleRoot, EldError> {
        use std::fs::OpenOptions;
        use std::io::{Seek, SeekFrom, Write};

        info!(
            provider_id = %self.config.provider_id,
            capacity_bytes = capacity_bytes,
            capacity_gb = capacity_bytes / (1024 * 1024 * 1024),
            "Starting local capacity allocation"
        );

        // Calculate chunk count (ceiling division)
        let chunk_count = (capacity_bytes as usize).div_ceil(MAX_CHUNK_SIZE);

        info!(
            provider_id = %self.config.provider_id,
            chunk_count = chunk_count,
            chunk_size_bytes = MAX_CHUNK_SIZE,
            "Calculated chunk count"
        );

        // Get slot allocator
        let slot_allocator = self.slot_allocator.lock().await;
        let capacity_file = slot_allocator.capacity_file_path().to_path_buf();
        drop(slot_allocator);

        // Ensure capacity directory exists
        if let Some(parent_dir) = capacity_file.parent() {
            std::fs::create_dir_all(parent_dir).map_err(|e| EldError::StorageError {
                operation: "allocate_capacity".to_string(),
                details: format!(
                    "Failed to create capacity directory {}: {}",
                    parent_dir.display(),
                    e
                ),
            })?;
            info!(
                provider_id = %self.config.provider_id,
                directory = %parent_dir.display(),
                "Ensured capacity directory exists"
            );
        }

        // Remove immutable flag if file exists (from previous run)
        #[cfg(target_os = "macos")]
        {
            if capacity_file.exists() {
                use std::process::Command;
                if let Some(capacity_file_str) = capacity_file.to_str() {
                    let _ = Command::new("chflags")
                        .args(["nouchg", capacity_file_str])
                        .output();
                }
            }
        }

        #[cfg(target_os = "linux")]
        {
            if capacity_file.exists() {
                use std::process::Command;
                if let Some(capacity_file_str) = capacity_file.to_str() {
                    let _ = Command::new("chattr")
                        .args(&["-i", capacity_file_str])
                        .output();
                }
            }
        }

        // Create or truncate the capacity file
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .read(true)
            .write(true)
            .open(&capacity_file)
            .map_err(|e| EldError::StorageError {
                operation: "allocate_capacity".to_string(),
                details: format!("Failed to create capacity file: {e}"),
            })?;

        // Pre-allocate the file
        file.set_len(capacity_bytes)
            .map_err(|e| EldError::StorageError {
                operation: "allocate_capacity".to_string(),
                details: format!("Failed to pre-allocate capacity file: {e}"),
            })?;

        info!(
            provider_id = %self.config.provider_id,
            file_size = capacity_bytes,
            "Pre-allocated capacity file"
        );

        // === Generate deterministic random offsets ===
        let mut chunk_offsets: Vec<(u64, usize)> = Vec::with_capacity(chunk_count);
        let num_permutable_slots = chunk_count.saturating_sub(1);
        let slot_size = MAX_CHUNK_SIZE as u64;

        // Generate deterministic permutation using Fisher-Yates shuffle
        let mut slot_indices: Vec<usize> = (0..num_permutable_slots).collect();
        let base_hasher =
            Self::create_deterministic_hasher(b"OFFSET_MAP", &seed, &self.config.provider_id);

        // Shuffle using deterministic randomness
        for i in (1..num_permutable_slots).rev() {
            let mut slot_hasher = base_hasher.clone();
            slot_hasher.update(&i.to_le_bytes());
            let hash = slot_hasher.finalize();
            let random_u64 = u64::from_le_bytes(hash.as_bytes()[..8].try_into().unwrap());
            let j = (random_u64 as usize) % (i + 1);
            slot_indices.swap(i, j);
        }

        // Assign first (chunk_count - 1) chunks to permuted slots
        for &slot_idx in slot_indices.iter() {
            let offset = (slot_idx as u64) * slot_size;
            chunk_offsets.push((offset, MAX_CHUNK_SIZE));
        }

        // Last chunk always goes at the end
        let last_chunk_size = {
            let remainder = capacity_bytes as usize % MAX_CHUNK_SIZE;
            if remainder > 0 {
                remainder
            } else {
                MAX_CHUNK_SIZE
            }
        };
        let last_offset = capacity_bytes.saturating_sub(last_chunk_size as u64);
        chunk_offsets.push((last_offset, last_chunk_size));

        // Verify no overlapping offsets
        for (i, &(offset1, size1)) in chunk_offsets.iter().enumerate() {
            let end1 = offset1 + size1 as u64;
            for (j, &(offset2, size2)) in chunk_offsets.iter().enumerate().skip(i + 1) {
                let end2 = offset2 + size2 as u64;
                if offset1 < end2 && end1 > offset2 {
                    return Err(EldError::StorageError {
                        operation: "allocate_capacity".to_string(),
                        details: format!(
                            "Overlapping offsets detected: chunk {i} [{offset1}, {end1}) overlaps with chunk {j} [{offset2}, {end2})"
                        ),
                    });
                }
            }
        }

        info!(
            provider_id = %self.config.provider_id,
            total_chunks = chunk_offsets.len(),
            "Generated random offsets for all chunks (verified no overlaps)"
        );

        // === Initialize slot map with Proof/Open slots ===
        let mut slots = Vec::with_capacity(chunk_count);
        let mut proof_count = 0;
        let mut open_count = 0;

        // Use deterministic hashing to decide slot states
        let state_seed =
            Self::create_deterministic_hasher(b"SLOT_STATE_MAP", &seed, &self.config.provider_id)
                .finalize();

        for (chunk_index, (offset, size)) in chunk_offsets.iter().enumerate().take(chunk_count) {
            // Deterministically decide if this slot is Proof or Open
            let mut state_hasher = Blake3Hasher::new();
            state_hasher.update(state_seed.as_bytes());
            state_hasher.update(&chunk_index.to_le_bytes());
            let hash = state_hasher.finalize();
            let random_u64 = u64::from_le_bytes(hash.as_bytes()[..8].try_into().unwrap());
            let ratio_threshold = (PROOF_RATIO * 100.0) as u64;
            let slot_state = if (random_u64 % 100) < ratio_threshold {
                proof_count += 1;
                SlotState::Proof
            } else {
                open_count += 1;
                SlotState::Open
            };

            slots.push(Slot {
                offset: *offset,
                size: *size,
                state: slot_state,
            });
        }

        // Create slot map (edge/DB: seed stored as [u8; 32])
        let slot_map = SlotMap {
            slots: slots.clone(),
            capacity_bytes,
            seed: *seed.as_bytes(),
            provider_id: self.config.provider_id,
        };

        info!(
            provider_id = %self.config.provider_id,
            total_slots = chunk_count,
            proof_slots = proof_count,
            open_slots = open_count,
            proof_ratio = format!("{:.1}%", (proof_count as f64 / chunk_count as f64) * 100.0),
            "Initialized slot map with Proof and Open slots"
        );

        // === Write data to capacity file and build Merkle tree ===
        let mut leaf_hashes = Vec::with_capacity(chunk_count);
        let mut chunk_data_buffer = vec![0u8; MAX_CHUNK_SIZE];

        for (chunk_index, slot) in slots.iter().enumerate().take(chunk_count) {
            let actual_chunk_size = slot.size;

            match slot.state {
                SlotState::Proof => {
                    // Generate deterministic chunk data for Proof slots
                    let chunk_data =
                        Self::generate_chunk_data(&self.config.provider_id, &seed, chunk_index);
                    chunk_data_buffer[..actual_chunk_size]
                        .copy_from_slice(&chunk_data[..actual_chunk_size]);

                    // Write proof data at slot offset
                    file.seek(SeekFrom::Start(slot.offset)).map_err(|e| {
                        EldError::StorageError {
                            operation: "allocate_capacity".to_string(),
                            details: format!("Failed to seek to slot offset: {e}"),
                        }
                    })?;
                    file.write_all(&chunk_data_buffer[..actual_chunk_size])
                        .map_err(|e| EldError::StorageError {
                            operation: "allocate_capacity".to_string(),
                            details: format!("Failed to write proof chunk: {e}"),
                        })?;

                    // Hash the proof chunk for Merkle tree
                    let chunk_hash = Self::hash_chunk(&chunk_data[..actual_chunk_size]);
                    leaf_hashes.push(chunk_hash);
                }
                SlotState::Open => {
                    // Write zeros to Open slots
                    chunk_data_buffer[..actual_chunk_size].fill(0);

                    file.seek(SeekFrom::Start(slot.offset)).map_err(|e| {
                        EldError::StorageError {
                            operation: "allocate_capacity".to_string(),
                            details: format!("Failed to seek to open slot offset: {e}"),
                        }
                    })?;
                    file.write_all(&chunk_data_buffer[..actual_chunk_size])
                        .map_err(|e| EldError::StorageError {
                            operation: "allocate_capacity".to_string(),
                            details: format!("Failed to write zeros to open slot: {e}"),
                        })?;

                    // Hash the zero chunk for Merkle tree
                    let zero_chunk = vec![0u8; actual_chunk_size];
                    let zero_hash = Self::hash_chunk(&zero_chunk);
                    leaf_hashes.push(zero_hash);
                }
                SlotState::Content { .. } => {
                    return Err(EldError::StorageError {
                        operation: "allocate_capacity".to_string(),
                        details: "Content slots should not exist during initial allocation"
                            .to_string(),
                    });
                }
            }

            if chunk_index % 100 == 0 || chunk_index == chunk_count - 1 {
                info!(
                    provider_id = %self.config.provider_id,
                    chunk_index = chunk_index,
                    offset = slot.offset,
                    state = ?slot.state,
                    total_chunks = chunk_count,
                    progress = format!("{:.1}%", (chunk_index + 1) as f64 / chunk_count as f64 * 100.0),
                    "Processed slot"
                );
            }
        }

        // Sync file to disk
        file.sync_all().map_err(|e| EldError::StorageError {
            operation: "allocate_capacity".to_string(),
            details: format!("Failed to fsync capacity file: {e}"),
        })?;

        info!(
            provider_id = %self.config.provider_id,
            "Capacity file synced to disk"
        );

        // Save slot map to disk
        let mut slot_allocator = self.slot_allocator.lock().await;
        slot_allocator.set_slot_map(slot_map.clone());
        let slots_file_path = slot_allocator.slots_file_path().to_path_buf();
        match slot_allocator.save_slot_map() {
            Ok(()) => {
                info!(
                    provider_id = %self.config.provider_id,
                    slots_file = %slots_file_path.display(),
                    "Slot map saved to disk"
                );
            }
            Err(e) => {
                error!(
                    provider_id = %self.config.provider_id,
                    slots_file = %slots_file_path.display(),
                    error = %e,
                    "Failed to save slot map to disk"
                );
                return Err(e);
            }
        }
        drop(slot_allocator);

        // Build Merkle tree
        let merkle_tree = CapacityProofMerkleTree::build(leaf_hashes);
        let merkle_root = merkle_tree.root();
        self.set_merkle_tree(merkle_tree).await;

        info!(
            provider_id = %self.config.provider_id,
            merkle_root = merkle_root.to_hex(),
            "Merkle tree built successfully"
        );

        // Set file immutable flag
        #[cfg(target_os = "linux")]
        {
            use std::process::Command;
            if let Some(capacity_file_str) = capacity_file.to_str() {
                let _ = Command::new("chattr")
                    .args(&["+i", capacity_file_str])
                    .output();
            }
        }

        #[cfg(target_os = "macos")]
        {
            use std::process::Command;
            if let Some(capacity_file_str) = capacity_file.to_str() {
                let _ = Command::new("chflags")
                    .args(["uchg", capacity_file_str])
                    .output();
            }
        }

        info!(
            provider_id = %self.config.provider_id,
            capacity_bytes = capacity_bytes,
            merkle_root = merkle_root.to_hex(),
            "Capacity allocation completed successfully"
        );

        Ok(merkle_root)
    }

    /// Get slot allocator
    pub fn slot_allocator(&self) -> Arc<Mutex<SlotAllocator>> {
        self.slot_allocator.clone()
    }

    /// Get configuration
    pub fn config(&self) -> &CapacityConfig {
        &self.config
    }

    /// Wallet name from `ELD_CAPACITY_VALIDATOR_WALLET_NAME` (local slots + RegisterCapacity).
    pub fn capacity_validator_wallet_name(&self) -> &str {
        self.capacity_validator_wallet_name.as_str()
    }

    /// Alias for admin/status APIs that historically exposed `capacity_wallet_name`.
    pub fn capacity_wallet_name(&self) -> &str {
        self.capacity_validator_wallet_name()
    }

    /// Hash a chunk using SHA256 (matching eld_proof_access)
    pub fn hash_chunk(chunk_data: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"CHUNK_HASH");
        hasher.update(chunk_data);
        hasher.finalize().into()
    }

    /// Generate deterministic chunk data for Proof slots
    pub fn generate_chunk_data(
        provider_id: &Address,
        seed: &CapacitySeed,
        chunk_index: usize,
    ) -> Vec<u8> {
        let mut hasher = Self::create_deterministic_hasher(b"CAPACITY_PROOF", seed, provider_id);
        hasher.update(&chunk_index.to_le_bytes());

        // Use BLAKE3 XOF to generate exactly MAX_CHUNK_SIZE bytes
        let mut output = vec![0u8; MAX_CHUNK_SIZE];
        hasher.finalize_xof().fill(&mut output);
        output
    }

    /// Create a deterministic BLAKE3 hasher with a prefix, seed, and provider_id
    fn create_deterministic_hasher(
        prefix: &[u8],
        seed: &CapacitySeed,
        provider_id: &Address,
    ) -> Blake3Hasher {
        let mut hasher = Blake3Hasher::new();
        hasher.update(prefix);
        hasher.update(seed.as_bytes());
        hasher.update(provider_id.canonical_hex_with_prefix().as_bytes());
        hasher
    }

    /// Build merkle tree from slot map
    /// This computes leaf hashes for all slots and builds the merkle tree
    pub async fn build_merkle_tree(
        &self,
        slot_map: &SlotMap,
    ) -> Result<CapacityProofMerkleTree, EldError> {
        info!(
            provider_id = %slot_map.provider_id,
            slot_count = slot_map.slots.len(),
            "Building merkle tree from slot map"
        );

        let mut leaf_hashes = Vec::with_capacity(slot_map.slots.len());

        // For each slot, compute its hash based on state
        for (chunk_index, slot) in slot_map.slots.iter().enumerate() {
            let chunk_hash = match &slot.state {
                SlotState::Proof => {
                    // Generate deterministic proof data and hash it
                    let chunk_data = Self::generate_chunk_data(
                        &slot_map.provider_id,
                        &CapacitySeed::new(slot_map.seed),
                        chunk_index,
                    );
                    Self::hash_chunk(&chunk_data[..slot.size])
                }
                SlotState::Open => {
                    // Hash zero chunk
                    let zero_chunk = vec![0u8; slot.size];
                    Self::hash_chunk(&zero_chunk)
                }
                SlotState::Content { .. } => {
                    // For content slots, we need to read from file
                    // This will be handled when content is actually stored
                    // For now, we'll need to read from file if it exists
                    // TODO: Handle content slots during initial build
                    // For now, treat as zero (will be updated when content is stored)
                    let zero_chunk = vec![0u8; slot.size];
                    Self::hash_chunk(&zero_chunk)
                }
            };
            leaf_hashes.push(chunk_hash);
        }

        let merkle_tree = CapacityProofMerkleTree::build(leaf_hashes);
        let merkle_root = merkle_tree.root();

        info!(
            provider_id = %slot_map.provider_id,
            merkle_root = merkle_root.to_hex(),
            "Merkle tree built successfully"
        );

        Ok(merkle_tree)
    }

    /// Update merkle tree when content is stored in slots
    /// This incrementally updates only the affected leaves
    pub async fn update_merkle_tree_for_content(
        &self,
        slot_indices: &[usize],
        content_hashes: Vec<[u8; 32]>,
    ) -> Result<CapacityMerkleRoot, EldError> {
        if slot_indices.len() != content_hashes.len() {
            return Err(EldError::StorageError {
                operation: "update_merkle_tree_for_content".to_string(),
                details: format!(
                    "Mismatch: {} slot indices but {} content hashes",
                    slot_indices.len(),
                    content_hashes.len()
                ),
            });
        }

        let mut merkle_tree_guard = self.merkle_tree.lock().await;
        let merkle_tree = merkle_tree_guard
            .as_mut()
            .ok_or_else(|| EldError::StorageError {
                operation: "update_merkle_tree_for_content".to_string(),
                details: "Merkle tree not initialized".to_string(),
            })?;

        // Validate slot indices are within bounds
        let leaf_count = merkle_tree.leaf_count();
        for &slot_index in slot_indices {
            if slot_index >= leaf_count {
                return Err(EldError::StorageError {
                    operation: "update_merkle_tree_for_content".to_string(),
                    details: format!(
                        "Slot index {} out of bounds (max: {})",
                        slot_index,
                        leaf_count - 1
                    ),
                });
            }
        }

        // Prepare updates: (leaf_index, new_hash)
        let updates: Vec<(usize, [u8; 32])> = slot_indices
            .iter()
            .zip(content_hashes.iter())
            .map(|(&idx, &hash)| (idx, hash))
            .collect();

        merkle_tree.update_leaves(updates);
        let new_root = merkle_tree.root();

        info!(
            slots_updated = slot_indices.len(),
            new_merkle_root = new_root.to_hex(),
            "Updated merkle tree for content storage"
        );

        Ok(new_root)
    }

    /// Store content chunks in open slots
    /// Finds open slots, writes chunks, marks slots as Content, and updates Merkle tree
    /// Returns the slot indices used for the chunks
    pub async fn store_content_chunks(
        &self,
        content_id: String,
        chunks: Vec<Vec<u8>>,
    ) -> Result<Vec<usize>, EldError> {
        use eld_common::constants::pinboard::MAX_CHUNK_SIZE;

        info!(
            provider_id = %self.config.provider_id,
            content_id = %content_id,
            chunk_count = chunks.len(),
            "Storing content chunks in slots"
        );

        // Find open slots for all chunks
        let mut slot_allocator = self.slot_allocator.lock().await;
        let num_slots_needed = chunks.len();
        let slot_indices = slot_allocator.find_open_slots(num_slots_needed)?;

        // Write each chunk to its slot and collect hashes and actual sizes
        let mut content_hashes = Vec::with_capacity(chunks.len());
        let mut slot_sizes = Vec::with_capacity(chunks.len());
        for (chunk_idx, chunk_data) in chunks.iter().enumerate() {
            let slot_index = slot_indices[chunk_idx];
            let actual_size = chunk_data.len();

            // Ensure chunk data fits in slot (pad if needed)
            let mut padded_chunk = chunk_data.clone();
            if padded_chunk.len() < MAX_CHUNK_SIZE {
                padded_chunk.resize(MAX_CHUNK_SIZE, 0);
            } else if padded_chunk.len() > MAX_CHUNK_SIZE {
                return Err(EldError::StorageError {
                    operation: "store_content_chunks".to_string(),
                    details: format!(
                        "Chunk {} size {} exceeds slot size {}",
                        chunk_idx,
                        padded_chunk.len(),
                        MAX_CHUNK_SIZE
                    ),
                });
            }

            // Write chunk to slot and get hash
            let chunk_hash = slot_allocator.write_chunk_to_slot(slot_index, &padded_chunk)?;
            content_hashes.push(chunk_hash);
            slot_sizes.push((slot_index, actual_size));
        }

        // Calculate committed hash (hash of all chunk hashes)
        let mut committed_hasher = Sha256::new();
        committed_hasher.update(b"CONTENT_COMMIT");
        for hash in &content_hashes {
            committed_hasher.update(hash);
        }
        let committed_hash: [u8; 32] = committed_hasher.finalize().into();

        // Mark slots as Content (with actual sizes)
        slot_allocator.mark_slots_as_content(&slot_sizes, content_id.clone(), committed_hash)?;

        // Save slot map to disk
        slot_allocator.save_slot_map()?;

        // Extract slot indices for Merkle tree update
        let slot_indices: Vec<usize> = slot_sizes.iter().map(|(idx, _)| *idx).collect();
        drop(slot_allocator);

        // Update Merkle tree incrementally
        let new_merkle_root = self
            .update_merkle_tree_for_content(&slot_indices, content_hashes)
            .await?;

        info!(
            provider_id = %self.config.provider_id,
            content_id = %content_id,
            slots_used = slot_indices.len(),
            merkle_root = new_merkle_root.to_hex(),
            "Successfully stored content chunks in slots, submitting merkle root update"
        );

        // Submit transaction to update merkle root on-chain
        match self.submit_merkle_root_update(new_merkle_root).await {
            Ok(()) => {
                info!(
                    provider_id = %self.config.provider_id,
                    content_id = %content_id,
                    merkle_root = new_merkle_root.to_hex(),
                    "Successfully submitted merkle root update transaction"
                );
            }
            Err(e) => {
                // Log error but don't fail the content storage
                // The merkle root update can be retried later if needed
                error!(
                    provider_id = %self.config.provider_id,
                    content_id = %content_id,
                    merkle_root = new_merkle_root.to_hex(),
                    error = %e,
                    error_details = format!("{:?}", e),
                    "Failed to submit merkle root update transaction (content storage succeeded)"
                );
            }
        }

        Ok(slot_indices)
    }

    /// Retrieve content from slots by content_id
    /// Reads chunks from slots and reconstructs the original content
    pub async fn get_content_from_slots(&self, content_id: &str) -> Result<Vec<u8>, EldError> {
        info!(
            provider_id = %self.config.provider_id,
            content_id = %content_id,
            "Retrieving content from slots"
        );

        // Find slot indices and sizes for this content
        let slot_allocator = self.slot_allocator.lock().await;
        let slot_sizes = slot_allocator.find_slots_by_content_id(content_id)?;

        info!(
            provider_id = %self.config.provider_id,
            content_id = %content_id,
            slot_count = slot_sizes.len(),
            "Found slots for content"
        );

        // Read chunks from slots in order and trim padding using actual sizes
        let mut content = Vec::new();
        for (slot_index, actual_size) in &slot_sizes {
            let chunk_data = slot_allocator.read_chunk_from_slot(*slot_index)?;

            // Trim padding to actual size
            if chunk_data.len() >= *actual_size {
                content.extend_from_slice(&chunk_data[..*actual_size]);
            } else {
                // If actual_size is larger than what we read, something is wrong
                return Err(EldError::StorageError {
                    operation: "get_content_from_slots".to_string(),
                    details: format!(
                        "Chunk size mismatch: expected {}, got {}",
                        actual_size,
                        chunk_data.len()
                    ),
                });
            }
        }

        drop(slot_allocator);

        info!(
            provider_id = %self.config.provider_id,
            content_id = %content_id,
            content_size = content.len(),
            "Successfully retrieved content from slots"
        );

        Ok(content)
    }

    /// Delete content from slots, release the occupied slots, and update merkle root on-chain.
    pub async fn delete_content_from_slots(&self, content_id: &str) -> Result<(), EldError> {
        info!(
            provider_id = %self.config.provider_id,
            content_id = %content_id,
            "Deleting content from slots"
        );

        let mut slot_allocator = self.slot_allocator.lock().await;
        let slot_sizes = slot_allocator.find_slots_by_content_id(content_id)?;

        let slot_zero_hashes: Vec<[u8; 32]> = {
            let slot_map = slot_allocator.slot_map_ref()?;
            let mut hashes = Vec::with_capacity(slot_sizes.len());
            for (slot_index, _) in &slot_sizes {
                let slot_size = slot_map
                    .slots
                    .get(*slot_index)
                    .ok_or_else(|| EldError::StorageError {
                        operation: "delete_content_from_slots".to_string(),
                        details: format!("Slot index {slot_index} out of bounds"),
                    })?
                    .size;
                hashes.push(Self::hash_chunk(&vec![0u8; slot_size]));
            }
            hashes
        };

        for (slot_index, _) in &slot_sizes {
            slot_allocator.write_chunk_to_slot(*slot_index, &[])?;
        }

        let released = slot_allocator.remove_content_by_id(content_id)?;
        slot_allocator.save_slot_map()?;
        drop(slot_allocator);

        let released_indices: Vec<usize> = released.iter().map(|(idx, _)| *idx).collect();
        let previous_merkle_root = self.get_merkle_root().await?;
        let new_merkle_root = self
            .update_merkle_tree_for_content(&released_indices, slot_zero_hashes)
            .await?;

        if previous_merkle_root == new_merkle_root {
            info!(
                provider_id = %self.config.provider_id,
                content_id = %content_id,
                released_slots = released_indices.len(),
                merkle_root = new_merkle_root.to_hex(),
                "Deleted content slots; merkle root unchanged, skipping merkle root update tx"
            );
            return Ok(());
        }

        match self.submit_merkle_root_update(new_merkle_root).await {
            Ok(()) => {
                info!(
                    provider_id = %self.config.provider_id,
                    content_id = %content_id,
                    released_slots = released_indices.len(),
                    previous_merkle_root = previous_merkle_root.to_hex(),
                    merkle_root = new_merkle_root.to_hex(),
                    "Deleted content slots and submitted merkle root update"
                );
            }
            Err(e) => {
                error!(
                    provider_id = %self.config.provider_id,
                    content_id = %content_id,
                    released_slots = released_indices.len(),
                    previous_merkle_root = previous_merkle_root.to_hex(),
                    merkle_root = new_merkle_root.to_hex(),
                    error = %e,
                    "Deleted content slots but failed to submit merkle root update"
                );
            }
        }

        Ok(())
    }

    /// Get current merkle root
    pub async fn get_merkle_root(&self) -> Result<CapacityMerkleRoot, EldError> {
        let merkle_tree_guard = self.merkle_tree.lock().await;
        let merkle_tree = merkle_tree_guard
            .as_ref()
            .ok_or_else(|| EldError::StorageError {
                operation: "get_merkle_root".to_string(),
                details: "Merkle tree not initialized".to_string(),
            })?;
        Ok(merkle_tree.root())
    }

    /// Get registration info needed for on-chain registration
    /// Returns (capacity_bytes, seed, merkle_root) if capacity is allocated
    pub async fn get_registration_info(
        &self,
    ) -> Result<(u64, CapacitySeed, CapacityMerkleRoot), EldError> {
        let slot_allocator = self.slot_allocator.lock().await;
        let slot_map = slot_allocator
            .get_slot_map()
            .ok_or_else(|| EldError::StorageError {
                operation: "get_registration_info".to_string(),
                details: "Slot map not loaded - capacity not allocated".to_string(),
            })?;

        let capacity_bytes = slot_map.capacity_bytes;
        let seed = CapacitySeed::new(slot_map.seed);
        drop(slot_allocator);

        let merkle_root = self.get_merkle_root().await?;

        Ok((capacity_bytes, seed, merkle_root))
    }

    /// Set merkle tree (typically after building from slot map)
    pub async fn set_merkle_tree(&self, merkle_tree: CapacityProofMerkleTree) {
        *self.merkle_tree.lock().await = Some(merkle_tree);
    }

    /// Generate capacity proof for a challenge
    pub async fn generate_capacity_proof(
        &self,
        params: CapacityProofGenerationParams,
    ) -> Result<ChallengeProof, EldError> {
        let CapacityProofGenerationParams {
            challenge_id,
            challenger,
            provider_id,
            chunk_indices,
            block_height,
            expected_merkle_root,
            expiration_block: _expiration_block,
            timestamp: _timestamp,
        } = params;

        // Verify this challenge is for us
        if provider_id != self.config.provider_id {
            return Err(EldError::ValidationError {
                field: "provider_id".to_string(),
                value: provider_id.to_string(),
                details: "Challenge is not for this provider".to_string(),
            });
        }

        // Verify we're initialized
        let initialized = self.initialized.lock().await;
        if !*initialized {
            return Err(EldError::ValidationError {
                field: "initialized".to_string(),
                value: "false".to_string(),
                details: "Capacity manager not initialized".to_string(),
            });
        }
        drop(initialized);

        // Verify merkle root matches (sanity check)
        let merkle_tree_guard = self.merkle_tree.lock().await;
        let merkle_tree = merkle_tree_guard
            .as_ref()
            .ok_or_else(|| EldError::ValidationError {
                field: "merkle_tree".to_string(),
                value: "none".to_string(),
                details: "Merkle tree not built".to_string(),
            })?;

        let current_root = merkle_tree.root();
        if current_root != expected_merkle_root {
            return Err(EldError::ValidationError {
                field: "merkle_root".to_string(),
                value: expected_merkle_root.to_hex(),
                details: format!(
                    "Expected root {} does not match current root {}",
                    expected_merkle_root.to_hex(),
                    current_root.to_hex()
                ),
            });
        }

        // Get slot allocator to access capacity file
        let slot_allocator = self.slot_allocator.lock().await;
        let capacity_file_path = slot_allocator.capacity_file_path().to_path_buf();
        let slot_map = slot_allocator
            .get_slot_map()
            .ok_or_else(|| EldError::ValidationError {
                field: "slot_map".to_string(),
                value: "none".to_string(),
                details: "Slot map not loaded".to_string(),
            })?
            .clone();
        drop(slot_allocator);

        // Open capacity file for reading
        let mut file = File::open(&capacity_file_path).map_err(|e| EldError::StorageError {
            operation: "open_capacity_file".to_string(),
            details: format!("Failed to open capacity file: {e}"),
        })?;

        // Generate proofs for each requested chunk
        let mut proofs = Vec::new();

        for &chunk_index in &chunk_indices {
            // Get slot info for this chunk
            let slot =
                slot_map
                    .slots
                    .get(chunk_index)
                    .ok_or_else(|| EldError::ValidationError {
                        field: "chunk_index".to_string(),
                        value: chunk_index.to_string(),
                        details: format!("Chunk index {chunk_index} out of bounds"),
                    })?;

            let offset = slot.offset;
            let chunk_size = slot.size;
            let slot_state = slot.state.clone();

            // Read chunk data from file
            file.seek(SeekFrom::Start(offset))
                .map_err(|e| EldError::StorageError {
                    operation: "seek_chunk".to_string(),
                    details: format!("Failed to seek to chunk offset {offset}: {e}"),
                })?;

            let mut chunk_data = vec![0u8; chunk_size];
            file.read_exact(&mut chunk_data)
                .map_err(|e| EldError::StorageError {
                    operation: "read_chunk".to_string(),
                    details: format!("Failed to read chunk data: {e}"),
                })?;

            // Generate Merkle proof for this chunk
            let merkle_proof = merkle_tree.generate_proof(chunk_index);
            let proof_path_length = merkle_proof.len();

            // Calculate chunk hash (SHA256)
            let chunk_hash = Self::hash_chunk(&chunk_data);

            // Clone slot_state for logging
            let slot_state_for_log = slot_state.clone();

            proofs.push(ChunkProof {
                chunk_index,
                chunk_data,
                chunk_hash,
                merkle_proof,
                slot_state,
            });

            info!(
                challenge_id = %challenge_id,
                chunk_index = chunk_index,
                chunk_size = chunk_size,
                proof_path_length = proof_path_length,
                slot_state = ?slot_state_for_log,
                "Generated proof for chunk"
            );
        }

        drop(merkle_tree_guard);

        let generated_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let challenge_proof = ChallengeProof {
            challenge_id: challenge_id.to_hex(),
            provider_id,
            challenger,
            block_height,
            proofs,
            generated_at,
        };

        info!(
            challenge_id = %challenge_proof.challenge_id,
            proof_count = challenge_proof.proofs.len(),
            "Successfully generated all proofs for challenge"
        );

        Ok(challenge_proof)
    }

    /// Sign a P2P capacity challenge response with the provider wallet.
    pub async fn sign_capacity_challenge_response(
        &self,
        challenge_proof: &ChallengeProof,
    ) -> Result<(String, String), EldError> {
        let wallet = match self
            .cli
            .get_wallet_by_address(&challenge_proof.provider_id.to_string())
            .await
        {
            Ok(Some(wallet)) => wallet,
            Ok(None) => {
                return Err(EldError::StorageError {
                    operation: "sign_capacity_challenge_response".to_string(),
                    details: format!(
                        "Failed to get wallet for provider_id '{}'",
                        challenge_proof.provider_id
                    ),
                });
            }
            Err(e) => return Err(e),
        };

        wallet
            .sign_capacity_challenge_response(challenge_proof)
            .map_err(|e| EldError::StorageError {
                operation: "sign_capacity_challenge_response".to_string(),
                details: e,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eld_common::address::Address;
    use eld_common::capacity::{Slot, SlotMap};
    use eld_common::capacity_proof::SlotState;
    use tempfile::TempDir;

    fn test_addr(hex: &str) -> Address {
        Address::parse_hex_str(hex).expect("test address")
    }

    const TEST_PROVIDER: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const TEST_CHALLENGER: &str = "0x1111111111111111111111111111111111111111";

    struct TestDependencies {
        _wallet_dir: TempDir,
        cli: Arc<eld_client::facade::ChainClient>,
        consensus_config: Arc<std::sync::Mutex<crate::config::ConsensusConfig>>,
    }

    // Helper function to create test dependencies
    fn create_test_dependencies() -> TestDependencies {
        let wallet_dir = TempDir::new().expect("wallet temp dir");
        let wallet_path = wallet_dir.path().join("wallets.json");
        std::fs::write(&wallet_path, "[]").expect("empty wallets file");

        let cli_config = eld_client::config::CliConfig {
            node_host: "127.0.0.1".to_string(),
            node_port: "26657".to_string(),
            chain_id: "test-chain".to_string(),
            faucet_host: "127.0.0.1".to_string(),
            faucet_port: "8080".to_string(),
            faucet_end_point: "/faucet/request".to_string(),
            faucet_url: None,
            app_port: "9001".to_string(),
            node_url: None,
            app_url: None,
            p2p_tcp_port: None,
            p2p_udp_port: None,
            single_node: None,
            capacity_size_mb: None,
            capacity_storage_path: None,
            indexer: false,
        };
        let cli = Arc::new(
            eld_client::facade::ChainClient::with_wallets(
                cli_config,
                eld_common::fee::FeeConfig::default(),
                wallet_path,
            )
            .expect("test ChainClient"),
        );

        let consensus_config = Arc::new(std::sync::Mutex::new(crate::config::ConsensusConfig {
            chain_id: "test-chain".to_string(),
            app_host: "127.0.0.1".to_string(),
            app_port: "26658".to_string(),
            accounts: std::collections::HashMap::new(),
            max_tx_bytes: 10 * 1024 * 1024,
            fee_config: eld_common::fee::FeeConfig::default(),
            storage_limits: crate::config::StorageLimits::default(),
        }));

        TestDependencies {
            _wallet_dir: wallet_dir,
            cli,
            consensus_config,
        }
    }

    #[tokio::test]
    async fn register_capacity_onchain_requires_capacity_validator_wallet() {
        let temp_dir = TempDir::new().unwrap();
        let config = CapacityConfig {
            capacity_dir: temp_dir.path().to_path_buf(),
            max_capacity_gb: 10,
            provider_id: test_addr("0x0000000000000000000000000000000000000001"),
            auto_register: true,
            registration_retry_interval_secs: 60,
            tendermint_rpc_url: "http://127.0.0.1:26657".to_string(),
        };
        let deps = create_test_dependencies();
        let cli = deps.cli;
        let consensus_config = deps.consensus_config;
        let manager = CapacityManager::new(
            config,
            "wallet-capacity-validator-missing".to_string(),
            cli,
            consensus_config,
        );
        assert_eq!(
            manager.capacity_validator_wallet_name(),
            "wallet-capacity-validator-missing"
        );
        assert_eq!(
            manager.capacity_wallet_name(),
            manager.capacity_validator_wallet_name(),
            "single-wallet model: capacity_wallet_name aliases CV wallet"
        );

        let err = manager
            .register_capacity_onchain(
                1024,
                CapacitySeed::new([0x42; 32]),
                CapacityMerkleRoot::new([0xAA; 32]),
            )
            .await
            .expect_err("missing CV wallet must fail");
        let details = format!("{err}");
        assert!(
            details.contains("wallet-capacity-validator-missing")
                || details.contains("capacity-validator"),
            "error should mention capacity-validator wallet: {details}"
        );
    }

    #[test]
    fn capacity_manager_uses_single_wallet_name_for_local_and_register() {
        let temp_dir = TempDir::new().unwrap();
        let provider = test_addr("0x014bb5f2250c33ddbb488d9d16258f70bf252883");
        let config = CapacityConfig {
            capacity_dir: temp_dir.path().to_path_buf(),
            max_capacity_gb: 10,
            provider_id: provider,
            auto_register: true,
            registration_retry_interval_secs: 60,
            tendermint_rpc_url: "http://127.0.0.1:26657".to_string(),
        };
        let deps = create_test_dependencies();
        let cli = deps.cli;
        let consensus_config = deps.consensus_config;
        let manager = CapacityManager::new(
            config,
            "wallet-capacity-validator-1".to_string(),
            cli,
            consensus_config,
        );
        assert_eq!(manager.config().provider_id, provider);
        assert_eq!(
            manager.capacity_wallet_name(),
            "wallet-capacity-validator-1"
        );
        assert_eq!(
            manager.capacity_validator_wallet_name(),
            "wallet-capacity-validator-1"
        );
    }

    #[test]
    fn test_hash_chunk() {
        let chunk1 = vec![0x01; 100];
        let chunk2 = vec![0x02; 100];
        let chunk3 = vec![0x01; 100];

        let hash1 = CapacityManager::hash_chunk(&chunk1);
        let hash2 = CapacityManager::hash_chunk(&chunk2);
        let hash3 = CapacityManager::hash_chunk(&chunk3);

        // Different data should produce different hashes
        assert_ne!(hash1, hash2);

        // Same data should produce same hash
        assert_eq!(hash1, hash3);

        // Hash should be 32 bytes
        assert_eq!(hash1.len(), 32);
    }

    #[test]
    fn test_generate_chunk_data_deterministic() {
        let provider_id = test_addr(TEST_PROVIDER);
        let seed = CapacitySeed::new([0x42; 32]);

        // Generate same chunk twice - should be identical
        let chunk1 = CapacityManager::generate_chunk_data(&provider_id, &seed, 5);
        let chunk2 = CapacityManager::generate_chunk_data(&provider_id, &seed, 5);

        assert_eq!(chunk1, chunk2, "Chunk generation should be deterministic");
        assert_eq!(
            chunk1.len(),
            MAX_CHUNK_SIZE,
            "Chunk should be MAX_CHUNK_SIZE bytes"
        );

        // Different chunk index should produce different data
        let chunk3 = CapacityManager::generate_chunk_data(&provider_id, &seed, 6);
        assert_ne!(
            chunk1, chunk3,
            "Different chunk indices should produce different data"
        );

        // Different seed should produce different data
        let seed2 = CapacitySeed::new([0x77; 32]);
        let chunk4 = CapacityManager::generate_chunk_data(&provider_id, &seed2, 5);
        assert_ne!(
            chunk1, chunk4,
            "Different seeds should produce different data"
        );

        // Different provider ID should produce different data
        let provider_id2 = test_addr("0xcccccccccccccccccccccccccccccccccccccccc");
        let chunk5 = CapacityManager::generate_chunk_data(&provider_id2, &seed, 5);
        assert_ne!(
            chunk1, chunk5,
            "Different provider IDs should produce different data"
        );
    }

    #[tokio::test]
    async fn test_build_merkle_tree() {
        let temp_dir = TempDir::new().unwrap();
        let config = CapacityConfig {
            capacity_dir: temp_dir.path().to_path_buf(),
            max_capacity_gb: 10,
            provider_id: test_addr(TEST_PROVIDER),
            auto_register: true,
            registration_retry_interval_secs: 60,
            tendermint_rpc_url: "http://127.0.0.1:26657".to_string(),
        };

        let deps = create_test_dependencies();
        let cli = deps.cli;
        let consensus_config = deps.consensus_config;
        let manager = CapacityManager::new(config, "wallet1".to_string(), cli, consensus_config);

        // Create a slot map with Proof and Open slots
        let slot_map = SlotMap {
            slots: vec![
                Slot {
                    offset: 0,
                    size: MAX_CHUNK_SIZE,
                    state: SlotState::Proof,
                },
                Slot {
                    offset: MAX_CHUNK_SIZE as u64,
                    size: MAX_CHUNK_SIZE,
                    state: SlotState::Open,
                },
                Slot {
                    offset: (2 * MAX_CHUNK_SIZE) as u64,
                    size: MAX_CHUNK_SIZE,
                    state: SlotState::Proof,
                },
            ],
            capacity_bytes: 3 * MAX_CHUNK_SIZE as u64,
            seed: [0x42; 32],
            provider_id: test_addr(TEST_PROVIDER),
        };

        // Build merkle tree
        let merkle_tree = manager.build_merkle_tree(&slot_map).await.unwrap();
        let root = merkle_tree.root();

        // Root should not be all zeros
        assert_ne!(*root.as_bytes(), [0u8; 32]);

        // Should be able to generate proofs
        let proof = merkle_tree.generate_proof(0);
        assert!(!proof.is_empty());

        // Verify proof
        let leaf_hash = CapacityManager::hash_chunk(
            &CapacityManager::generate_chunk_data(
                &test_addr(TEST_PROVIDER),
                &CapacitySeed::new([0x42; 32]),
                0,
            )[..MAX_CHUNK_SIZE],
        );
        assert!(CapacityProofMerkleTree::verify_proof(
            &leaf_hash, &proof, &root, 0
        ));
    }

    #[tokio::test]
    async fn test_update_merkle_tree_for_content() {
        let temp_dir = TempDir::new().unwrap();
        let config = CapacityConfig {
            capacity_dir: temp_dir.path().to_path_buf(),
            max_capacity_gb: 10,
            provider_id: test_addr(TEST_PROVIDER),
            auto_register: true,
            registration_retry_interval_secs: 60,
            tendermint_rpc_url: "http://127.0.0.1:26657".to_string(),
        };

        let deps = create_test_dependencies();
        let cli = deps.cli;
        let consensus_config = deps.consensus_config;
        let manager = CapacityManager::new(config, "wallet1".to_string(), cli, consensus_config);

        // Create slot map
        let slot_map = SlotMap {
            slots: vec![
                Slot {
                    offset: 0,
                    size: MAX_CHUNK_SIZE,
                    state: SlotState::Proof,
                },
                Slot {
                    offset: MAX_CHUNK_SIZE as u64,
                    size: MAX_CHUNK_SIZE,
                    state: SlotState::Open,
                },
            ],
            capacity_bytes: 2 * MAX_CHUNK_SIZE as u64,
            seed: [0x42; 32],
            provider_id: test_addr(TEST_PROVIDER),
        };

        // Build initial merkle tree
        let merkle_tree = manager.build_merkle_tree(&slot_map).await.unwrap();
        let original_root = merkle_tree.root();
        manager.set_merkle_tree(merkle_tree).await;

        // Update merkle tree for content (slot 1 becomes Content)
        let content_hash = [0xAA; 32];
        let new_root = manager
            .update_merkle_tree_for_content(&[1], vec![content_hash])
            .await
            .unwrap();

        // Root should have changed
        assert_ne!(original_root, new_root);

        // Verify we can get the new root
        let current_root = manager.get_merkle_root().await.unwrap();
        assert_eq!(current_root, new_root);
    }

    #[tokio::test]
    async fn test_update_merkle_tree_multiple_slots() {
        let temp_dir = TempDir::new().unwrap();
        let config = CapacityConfig {
            capacity_dir: temp_dir.path().to_path_buf(),
            max_capacity_gb: 10,
            provider_id: test_addr(TEST_PROVIDER),
            auto_register: true,
            registration_retry_interval_secs: 60,
            tendermint_rpc_url: "http://127.0.0.1:26657".to_string(),
        };

        let deps = create_test_dependencies();
        let cli = deps.cli;
        let consensus_config = deps.consensus_config;
        let manager = CapacityManager::new(config, "wallet1".to_string(), cli, consensus_config);

        // Create slot map with multiple open slots
        let slot_map = SlotMap {
            slots: vec![
                Slot {
                    offset: 0,
                    size: MAX_CHUNK_SIZE,
                    state: SlotState::Open,
                },
                Slot {
                    offset: MAX_CHUNK_SIZE as u64,
                    size: MAX_CHUNK_SIZE,
                    state: SlotState::Open,
                },
                Slot {
                    offset: (2 * MAX_CHUNK_SIZE) as u64,
                    size: MAX_CHUNK_SIZE,
                    state: SlotState::Open,
                },
            ],
            capacity_bytes: 3 * MAX_CHUNK_SIZE as u64,
            seed: [0x42; 32],
            provider_id: test_addr(TEST_PROVIDER),
        };

        // Build initial merkle tree
        let merkle_tree = manager.build_merkle_tree(&slot_map).await.unwrap();
        let original_root = merkle_tree.root();
        manager.set_merkle_tree(merkle_tree).await;

        // Update multiple slots
        let content_hashes = vec![[0xAA; 32], [0xBB; 32]];
        let new_root = manager
            .update_merkle_tree_for_content(&[0, 1], content_hashes)
            .await
            .unwrap();

        // Root should have changed
        assert_ne!(original_root, new_root);
    }

    #[tokio::test]
    async fn test_get_merkle_root_no_tree() {
        let temp_dir = TempDir::new().unwrap();
        let config = CapacityConfig {
            capacity_dir: temp_dir.path().to_path_buf(),
            max_capacity_gb: 10,
            provider_id: test_addr(TEST_PROVIDER),
            auto_register: true,
            registration_retry_interval_secs: 60,
            tendermint_rpc_url: "http://127.0.0.1:26657".to_string(),
        };

        let deps = create_test_dependencies();
        let cli = deps.cli;
        let consensus_config = deps.consensus_config;
        let manager = CapacityManager::new(config, "wallet1".to_string(), cli, consensus_config);

        // Try to get root without building tree
        let result = manager.get_merkle_root().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_update_merkle_tree_mismatch() {
        let temp_dir = TempDir::new().unwrap();
        let config = CapacityConfig {
            capacity_dir: temp_dir.path().to_path_buf(),
            max_capacity_gb: 10,
            provider_id: test_addr(TEST_PROVIDER),
            auto_register: true,
            registration_retry_interval_secs: 60,
            tendermint_rpc_url: "http://127.0.0.1:26657".to_string(),
        };

        let deps = create_test_dependencies();
        let cli = deps.cli;
        let consensus_config = deps.consensus_config;
        let manager = CapacityManager::new(config, "wallet1".to_string(), cli, consensus_config);

        // Create slot map
        let slot_map = SlotMap {
            slots: vec![
                Slot {
                    offset: 0,
                    size: MAX_CHUNK_SIZE,
                    state: SlotState::Proof,
                },
                Slot {
                    offset: MAX_CHUNK_SIZE as u64,
                    size: MAX_CHUNK_SIZE,
                    state: SlotState::Open,
                },
            ],
            capacity_bytes: 2 * MAX_CHUNK_SIZE as u64,
            seed: [0x42; 32],
            provider_id: test_addr(TEST_PROVIDER),
        };

        // Build initial merkle tree
        let merkle_tree = manager.build_merkle_tree(&slot_map).await.unwrap();
        manager.set_merkle_tree(merkle_tree).await;

        // Try to update with mismatched slot indices
        let result = manager
            .update_merkle_tree_for_content(&[999], vec![[0xAA; 32]])
            .await;
        assert!(result.is_err());
    }

    /// Helper function to create a test capacity file with N slots
    /// Returns (manager, merkle_root, slot_map)
    async fn create_test_capacity_with_slots(
        num_slots: usize,
        provider_id: Address,
        seed: CapacitySeed,
    ) -> (CapacityManager, CapacityMerkleRoot, SlotMap, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let config = CapacityConfig {
            capacity_dir: temp_dir.path().to_path_buf(),
            max_capacity_gb: 10,
            provider_id,
            auto_register: true,
            registration_retry_interval_secs: 60,
            tendermint_rpc_url: "http://127.0.0.1:26657".to_string(),
        };

        let deps = create_test_dependencies();
        let cli = deps.cli;
        let consensus_config = deps.consensus_config;
        let manager =
            CapacityManager::new(config.clone(), "wallet1".to_string(), cli, consensus_config);

        // Create slot map with mix of Proof and Open slots
        let mut slots = Vec::new();
        let mut offset = 0u64;
        for i in 0..num_slots {
            // Alternate between Proof and Open slots
            let state = if i % 2 == 0 {
                SlotState::Proof
            } else {
                SlotState::Open
            };
            slots.push(Slot {
                offset,
                size: MAX_CHUNK_SIZE,
                state,
            });
            offset += MAX_CHUNK_SIZE as u64;
        }

        let slot_map = SlotMap {
            slots,
            capacity_bytes: (num_slots * MAX_CHUNK_SIZE) as u64,
            seed: *seed.as_bytes(),
            provider_id,
        };

        // The manager's SlotAllocator is created in CapacityManager::new()
        // which uses config.capacity_dir. We need to create the file there.
        // Get the path that the manager's SlotAllocator will use
        let manager_allocator = manager.slot_allocator.lock().await;
        let capacity_file_path = manager_allocator.capacity_file_path().to_path_buf();
        drop(manager_allocator);

        // Ensure the directory exists
        if let Some(parent) = capacity_file_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }

        // Save slot map using the manager's allocator path structure
        // The slot map file should be in the same directory as the capacity file
        let slots_file = capacity_file_path.with_extension("slots.json");
        let slot_map_json = serde_json::to_string_pretty(&slot_map).unwrap();
        std::fs::write(&slots_file, slot_map_json).unwrap();

        // Write chunk data to capacity file (use the manager's expected path)
        let mut file = std::fs::File::create(&capacity_file_path).unwrap();

        for (chunk_index, slot) in slot_map.slots.iter().enumerate() {
            let chunk_data = match &slot.state {
                SlotState::Proof => {
                    // Generate deterministic proof data
                    CapacityManager::generate_chunk_data(&provider_id, &seed, chunk_index)
                }
                SlotState::Open => {
                    // Open slots are zeros
                    vec![0u8; MAX_CHUNK_SIZE]
                }
                SlotState::Content { .. } => {
                    // For tests, treat as zeros
                    vec![0u8; MAX_CHUNK_SIZE]
                }
            };

            use std::io::Write;
            file.seek(std::io::SeekFrom::Start(slot.offset)).unwrap();
            file.write_all(&chunk_data).unwrap();
        }

        // Flush and close the file before initializing
        use std::io::Write;
        file.flush().unwrap();
        drop(file);

        // Verify file exists
        assert!(
            capacity_file_path.exists(),
            "Capacity file should exist at: {}",
            capacity_file_path.display()
        );

        // Initialize manager and build merkle tree
        manager.initialize().await.unwrap();

        // Verify file still exists after initialization
        assert!(
            capacity_file_path.exists(),
            "Capacity file should still exist after initialization at: {}",
            capacity_file_path.display()
        );

        // Verify the manager's allocator points to the same file
        let manager_allocator = manager.slot_allocator.lock().await;
        let manager_capacity_path = manager_allocator.capacity_file_path();
        assert_eq!(
            capacity_file_path, manager_capacity_path,
            "Manager's capacity file path should match created file path"
        );
        drop(manager_allocator);

        let merkle_root = manager.get_merkle_root().await.unwrap();

        (manager, merkle_root, slot_map, temp_dir)
    }

    #[tokio::test]
    async fn test_generate_capacity_proof() {
        let provider_id = test_addr("0x1234123412341234123412341234123412341234");
        let seed = CapacitySeed::new([0x42; 32]);
        let num_slots = 50;

        // Create test capacity with 50 slots
        // Keep temp_dir alive for the test duration
        let (manager, merkle_root, _slot_map, _temp_dir) =
            create_test_capacity_with_slots(num_slots, provider_id, seed).await;

        // Create a mock challenge requesting chunks 0, 5, 10, 15, 20
        let challenge_id = ChallengeId::new([0x01; 32]);
        let challenger = test_addr(TEST_CHALLENGER);
        let chunk_indices = vec![0, 5, 10, 15, 20];
        let block_height = 100;
        let expiration_block = 200;
        let timestamp = 1234567890;

        // Generate proof
        let challenge_proof = manager
            .generate_capacity_proof(CapacityProofGenerationParams {
                challenge_id,
                challenger,
                provider_id,
                chunk_indices: chunk_indices.clone(),
                block_height,
                expected_merkle_root: merkle_root,
                expiration_block,
                timestamp,
            })
            .await
            .unwrap();

        // Verify proof structure
        assert_eq!(challenge_proof.challenge_id, challenge_id.to_hex());
        assert_eq!(challenge_proof.provider_id, provider_id);
        assert_eq!(challenge_proof.challenger, challenger);
        assert_eq!(challenge_proof.block_height, block_height);
        assert_eq!(challenge_proof.proofs.len(), chunk_indices.len());

        // Verify each proof
        for (i, proof) in challenge_proof.proofs.iter().enumerate() {
            let expected_index = chunk_indices[i];
            assert_eq!(proof.chunk_index, expected_index);

            // Verify chunk hash matches chunk data
            let calculated_hash = CapacityManager::hash_chunk(&proof.chunk_data);
            assert_eq!(calculated_hash, proof.chunk_hash);

            // Verify Merkle proof
            let merkle_valid = CapacityProofMerkleTree::verify_proof(
                &proof.chunk_hash,
                &proof.merkle_proof,
                &merkle_root,
                proof.chunk_index,
            );
            assert!(
                merkle_valid,
                "Merkle proof should be valid for chunk {}",
                proof.chunk_index
            );

            // Verify chunk data matches expected based on slot state
            match &proof.slot_state {
                SlotState::Proof => {
                    // Proof slots should match deterministic generation
                    let expected_data = CapacityManager::generate_chunk_data(
                        &provider_id,
                        &seed,
                        proof.chunk_index,
                    );
                    assert_eq!(
                        proof.chunk_data, expected_data,
                        "Proof slot chunk data should match generated data"
                    );
                }
                SlotState::Open => {
                    // Open slots should be zeros
                    assert!(
                        proof.chunk_data.iter().all(|&b| b == 0),
                        "Open slot should contain zeros"
                    );
                }
                SlotState::Content { .. } => {
                    // Content slots can have any data
                }
            }
        }
    }

    #[tokio::test]
    async fn test_validate_challenge_proof() {
        use crate::capacity::challenge_validator::validate_challenge_proof;

        let provider_id = test_addr("0x5678567856785678567856785678567856785678");
        let seed = CapacitySeed::new([0x99; 32]);
        let num_slots = 50;

        // Create test capacity with 50 slots
        // Keep temp_dir alive for the test duration
        let (manager, merkle_root, _slot_map, _temp_dir) =
            create_test_capacity_with_slots(num_slots, provider_id, seed).await;

        // Create a mock challenge requesting chunks 1, 3, 7, 11, 13
        let challenge_id = ChallengeId::new([0x02; 32]);
        let challenger = test_addr(TEST_CHALLENGER);
        let chunk_indices = vec![1, 3, 7, 11, 13];
        let block_height = 200;
        let expiration_block = 300;
        let timestamp = 1234567891;

        // Generate proof
        let challenge_proof = manager
            .generate_capacity_proof(CapacityProofGenerationParams {
                challenge_id,
                challenger,
                provider_id,
                chunk_indices: chunk_indices.clone(),
                block_height,
                expected_merkle_root: merkle_root,
                expiration_block,
                timestamp,
            })
            .await
            .unwrap();

        // Validate proof using validation function
        let validation_result =
            validate_challenge_proof(&challenge_proof.proofs, &merkle_root, &chunk_indices);

        assert!(
            validation_result.is_valid,
            "Proof validation should pass. Errors: {:?}",
            validation_result.errors
        );
        assert!(validation_result.errors.is_empty());
    }

    #[tokio::test]
    async fn test_validate_challenge_proof_invalid_hash() {
        use crate::capacity::challenge_validator::validate_challenge_proof;

        let provider_id = test_addr("0x9999999999999999999999999999999999999999");
        let seed = CapacitySeed::new([0xAA; 32]);
        let num_slots = 50;

        // Create test capacity with 50 slots
        // Keep temp_dir alive for the test duration
        let (manager, merkle_root, _slot_map, _temp_dir) =
            create_test_capacity_with_slots(num_slots, provider_id, seed).await;

        // Create a mock challenge
        let chunk_indices = vec![2, 4, 6];
        let challenge_id = ChallengeId::new([0x03; 32]);
        let challenger = test_addr(TEST_CHALLENGER);
        let block_height = 300;
        let expiration_block = 400;
        let timestamp = 1234567892;

        // Generate proof
        let mut challenge_proof = manager
            .generate_capacity_proof(CapacityProofGenerationParams {
                challenge_id,
                challenger,
                provider_id,
                chunk_indices: chunk_indices.clone(),
                block_height,
                expected_merkle_root: merkle_root,
                expiration_block,
                timestamp,
            })
            .await
            .unwrap();

        // Corrupt one chunk hash
        challenge_proof.proofs[0].chunk_hash[0] ^= 0xFF;

        // Validate proof - should fail
        let validation_result =
            validate_challenge_proof(&challenge_proof.proofs, &merkle_root, &chunk_indices);

        assert!(!validation_result.is_valid, "Proof validation should fail");
        assert!(!validation_result.errors.is_empty());
        assert!(validation_result
            .errors
            .iter()
            .any(|e| e.contains("chunk hash mismatch")));
    }

    #[tokio::test]
    async fn test_validate_challenge_proof_invalid_merkle() {
        use crate::capacity::challenge_validator::validate_challenge_proof;

        let provider_id = test_addr("0xabababababababababababababababababababab");
        let seed = CapacitySeed::new([0xCC; 32]);
        let num_slots = 50;

        // Create test capacity with 50 slots
        // Keep temp_dir alive for the test duration
        let (manager, merkle_root, _slot_map, _temp_dir) =
            create_test_capacity_with_slots(num_slots, provider_id, seed).await;

        // Create a mock challenge
        let chunk_indices = vec![8, 12, 16];
        let challenge_id = ChallengeId::new([0x04; 32]);
        let challenger = test_addr(TEST_CHALLENGER);
        let block_height = 400;
        let expiration_block = 500;
        let timestamp = 1234567893;

        // Generate proof
        let mut challenge_proof = manager
            .generate_capacity_proof(CapacityProofGenerationParams {
                challenge_id,
                challenger,
                provider_id,
                chunk_indices: chunk_indices.clone(),
                block_height,
                expected_merkle_root: merkle_root,
                expiration_block,
                timestamp,
            })
            .await
            .unwrap();

        // Corrupt Merkle proof
        if !challenge_proof.proofs[0].merkle_proof.is_empty() {
            challenge_proof.proofs[0].merkle_proof[0][0] ^= 0xFF;
        }

        // Validate proof - should fail
        let validation_result =
            validate_challenge_proof(&challenge_proof.proofs, &merkle_root, &chunk_indices);

        assert!(!validation_result.is_valid, "Proof validation should fail");
        assert!(!validation_result.errors.is_empty());
        assert!(validation_result
            .errors
            .iter()
            .any(|e| e.contains("Merkle proof verification failed")));
    }

    #[tokio::test]
    async fn test_validate_challenge_proof_wrong_index() {
        use crate::capacity::challenge_validator::validate_challenge_proof;

        let provider_id = test_addr("0xdddddddddddddddddddddddddddddddddddddddd");
        let seed = CapacitySeed::new([0xEE; 32]);
        let num_slots = 50;

        // Create test capacity with 50 slots
        // Keep temp_dir alive for the test duration
        let (manager, merkle_root, _slot_map, _temp_dir) =
            create_test_capacity_with_slots(num_slots, provider_id, seed).await;

        // Create a mock challenge
        let chunk_indices = vec![9, 14, 19];
        let challenge_id = ChallengeId::new([0x05; 32]);
        let challenger = test_addr(TEST_CHALLENGER);
        let block_height = 500;
        let expiration_block = 600;
        let timestamp = 1234567894;

        // Generate proof
        let mut challenge_proof = manager
            .generate_capacity_proof(CapacityProofGenerationParams {
                challenge_id,
                challenger,
                provider_id,
                chunk_indices: chunk_indices.clone(),
                block_height,
                expected_merkle_root: merkle_root,
                expiration_block,
                timestamp,
            })
            .await
            .unwrap();

        // Change chunk index in proof
        challenge_proof.proofs[0].chunk_index = 999;

        // Validate proof with original indices - should fail
        let validation_result =
            validate_challenge_proof(&challenge_proof.proofs, &merkle_root, &chunk_indices);

        assert!(!validation_result.is_valid, "Proof validation should fail");
        assert!(!validation_result.errors.is_empty());
        assert!(validation_result
            .errors
            .iter()
            .any(|e| e.contains("chunk index mismatch")));
    }

    #[tokio::test]
    async fn test_validate_challenge_proof_wrong_count() {
        use crate::capacity::challenge_validator::validate_challenge_proof;

        let provider_id = test_addr("0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee");
        let seed = CapacitySeed::new([0xFF; 32]);
        let num_slots = 50;

        // Create test capacity with 50 slots
        // Keep temp_dir alive for the test duration
        let (manager, merkle_root, _slot_map, _temp_dir) =
            create_test_capacity_with_slots(num_slots, provider_id, seed).await;

        // Create a mock challenge
        let chunk_indices = vec![6, 12, 18, 24];
        let challenge_id = ChallengeId::new([0x06; 32]);
        let challenger = test_addr(TEST_CHALLENGER);
        let block_height = 600;
        let expiration_block = 700;
        let timestamp = 1234567895;

        // Generate proof
        let mut challenge_proof = manager
            .generate_capacity_proof(CapacityProofGenerationParams {
                challenge_id,
                challenger,
                provider_id,
                chunk_indices: chunk_indices.clone(),
                block_height,
                expected_merkle_root: merkle_root,
                expiration_block,
                timestamp,
            })
            .await
            .unwrap();

        // Remove one proof to create count mismatch
        challenge_proof.proofs.pop();

        // Validate proof - should fail due to count mismatch
        let validation_result =
            validate_challenge_proof(&challenge_proof.proofs, &merkle_root, &chunk_indices);

        assert!(!validation_result.is_valid, "Proof validation should fail");
        assert!(!validation_result.errors.is_empty());
        assert!(validation_result
            .errors
            .iter()
            .any(|e| e.contains("Proof count mismatch")));
    }

    #[tokio::test]
    async fn test_generate_capacity_proof_wrong_provider() {
        let provider_id = test_addr("0xffffffffffffffffffffffffffffffffffffffff");
        let seed = CapacitySeed::new([0x11; 32]);
        let num_slots = 50;

        // Create test capacity with 50 slots
        // Keep temp_dir alive for the test duration
        let (manager, merkle_root, _slot_map, _temp_dir) =
            create_test_capacity_with_slots(num_slots, provider_id, seed).await;

        // Try to generate proof with wrong provider ID
        let result = manager
            .generate_capacity_proof(CapacityProofGenerationParams {
                challenge_id: ChallengeId::new([0x07; 32]),
                challenger: test_addr(TEST_CHALLENGER),
                provider_id: test_addr("0xdeaddeaddeaddeaddeaddeaddeaddeaddeaddead"), // Wrong provider ID
                chunk_indices: vec![0, 1, 2],
                block_height: 100,
                expected_merkle_root: merkle_root,
                expiration_block: 200,
                timestamp: 1234567890,
            })
            .await;

        assert!(result.is_err());
        if let Err(EldError::ValidationError { details, .. }) = result {
            assert!(details.contains("Challenge is not for this provider"));
        } else {
            panic!("Expected ValidationError");
        }
    }

    #[tokio::test]
    async fn test_generate_capacity_proof_wrong_merkle_root() {
        let provider_id = test_addr("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let seed = CapacitySeed::new([0x22; 32]);
        let num_slots = 50;

        // Create test capacity with 50 slots
        let (manager, _merkle_root, _slot_map, _temp_dir) =
            create_test_capacity_with_slots(num_slots, provider_id, seed).await;

        // Try to generate proof with wrong merkle root
        let wrong_root = CapacityMerkleRoot::new([0xFF; 32]);
        let result = manager
            .generate_capacity_proof(CapacityProofGenerationParams {
                challenge_id: ChallengeId::new([0x07; 32]),
                challenger: test_addr(TEST_CHALLENGER),
                provider_id,
                chunk_indices: vec![0, 1, 2],
                block_height: 100,
                expected_merkle_root: wrong_root, // Wrong merkle root
                expiration_block: 200,
                timestamp: 1234567890,
            })
            .await;

        assert!(result.is_err());
        if let Err(EldError::ValidationError { details, .. }) = result {
            assert!(details.contains("does not match current root"));
        } else {
            panic!("Expected ValidationError");
        }
    }
}
