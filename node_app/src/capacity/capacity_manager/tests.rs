use super::*;
use eld_common::address::Address;
use eld_common::capacity::{CapacityConfig, CapacityProofMerkleTree, Slot, SlotMap};
use eld_common::capacity_proof::SlotState;
use eld_common::constants::pinboard::MAX_CHUNK_SIZE;
use eld_common::error::EldError;
use eld_common::{CapacityMerkleRoot, CapacitySeed, ChallengeId};
use std::io::Seek;
use std::sync::Arc;
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

    let cli_config = eld_client::config::ClientConfig {
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
async fn test_build_merkle_tree_hashes_content_slots_from_file() {
    let temp_dir = TempDir::new().unwrap();
    let provider_id = test_addr(TEST_PROVIDER);
    let config = CapacityConfig {
        capacity_dir: temp_dir.path().to_path_buf(),
        max_capacity_gb: 10,
        provider_id,
        auto_register: true,
        registration_retry_interval_secs: 60,
        tendermint_rpc_url: "http://127.0.0.1:26657".to_string(),
    };

    let deps = create_test_dependencies();
    let manager = CapacityManager::new(
        config,
        "wallet1".to_string(),
        deps.cli,
        deps.consensus_config,
    );

    let content = {
        let mut bytes = vec![0u8; MAX_CHUNK_SIZE];
        bytes[..4].copy_from_slice(b"eld!");
        bytes
    };
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
                state: SlotState::Content {
                    deal_id: "content-1".to_string(),
                    committed_hash: [0x11; 32],
                },
            },
        ],
        capacity_bytes: 2 * MAX_CHUNK_SIZE as u64,
        seed: [0x42; 32],
        provider_id,
    };

    let capacity_file_path = {
        let allocator = manager.slot_allocator.lock().await;
        allocator.capacity_file_path().to_path_buf()
    };
    let mut file = std::fs::File::create(&capacity_file_path).unwrap();
    use std::io::Write;
    file.write_all(&vec![0u8; MAX_CHUNK_SIZE]).unwrap();
    file.write_all(&content).unwrap();
    file.flush().unwrap();
    drop(file);

    let merkle_tree = manager.build_merkle_tree(&slot_map).await.unwrap();
    let content_hash = CapacityManager::hash_chunk(&content);
    let zero_hash = CapacityManager::hash_chunk(&vec![0u8; MAX_CHUNK_SIZE]);
    assert_ne!(content_hash, zero_hash);

    let proof = merkle_tree.generate_proof(1);
    assert!(CapacityProofMerkleTree::verify_proof(
        &content_hash,
        &proof,
        &merkle_tree.root(),
        1
    ));
    assert!(!CapacityProofMerkleTree::verify_proof(
        &zero_hash,
        &proof,
        &merkle_tree.root(),
        1
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
                let expected_data =
                    CapacityManager::generate_chunk_data(&provider_id, &seed, proof.chunk_index);
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
