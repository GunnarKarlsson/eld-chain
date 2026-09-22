use super::CapacityManager;
use blake3::Hasher as Blake3Hasher;
use eld_common::capacity::{CapacityProofMerkleTree, Slot, SlotMap};
use eld_common::capacity_proof::SlotState;
use eld_common::constants::pinboard::MAX_CHUNK_SIZE;
use eld_common::error::EldError;
use eld_common::{CapacityMerkleRoot, CapacitySeed};
use rand::RngCore;
use tracing::{error, info};

// Constants matching eld_proof_access
const PROOF_RATIO: f64 = 0.8; // 80% proof slots, 20% open

impl CapacityManager {
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
}
