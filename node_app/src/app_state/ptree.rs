#[cfg(test)]
mod tests {
    use patricia_tree::PatriciaMap;
    use eld_common::constants::cado::*;
    use eld_common::cado::{CadoBody, CADOMetadata, CADOMut, CADOMap, CadoPath, CadoPathKey, CadoType};
    use eld_common::account::Account;
    use eld_common::address::Address;
    use eld_common::coin::Coin;
    use eld_common::nonce::Nonce;
    use eld_common::staking_account::StakingAccount;
    use crate::storage::traits::{SnapshotMetadata, SnapshotChunk, SnapshotChunkMetadata};
    use crate::app_state::AppStateTip;
    use sha2::{Digest, Sha256};

    fn create_test_cado(data: &[u8], owner: &str) -> CadoBody {
        CadoBody::Mutable(CADOMut {
            hash: [0; 32],
            data: data.to_vec(),
            metadata: CADOMetadata {
                type_: "test".to_string(),
                owner: owner.to_string(),
            },
            latest_hash: [0; 32],
        })
    }

    fn create_account_cado(address: &str, balance: u64, nonce: u32) -> CadoBody {
        let addr = Address::parse_hex_str(address).expect("Failed to parse address");
        let account = Account::new(
            addr,
            Coin::new(balance).expect("Failed to create coin"),
            Nonce::new(nonce),
        );
        let account_serialized = bincode::serialize(&account).expect("Failed to serialize account");
        let metadata = CADOMetadata {
            type_: TYPE_ACCOUNT.to_string(),
            owner: address.to_string(),
        };
        CadoBody::Mutable(CADOMut {
            hash: [0; 32],
            data: account_serialized.clone(),
            metadata,
            latest_hash: Sha256::digest(&account_serialized).into(),
        })
    }

    fn create_staking_account_cado(address: &str, stake_balance: Coin, originator: &str) -> CadoBody {
        let address_addr = Address::parse_hex_str(address)
            .expect("Failed to parse address");
        let originator_addr = Address::parse_hex_str(originator)
            .expect("Failed to parse originator");
        let staking_account = StakingAccount {
            address: address_addr,
            stake_balance,
            originator: originator_addr,
        };
        let staking_account_serialized = staking_account
            .serialize_bin()
            .expect("Failed to serialize staking account");
        let metadata = CADOMetadata::new(CadoType::StakingAccount, address.to_string());
        CadoBody::mutable_new(staking_account_serialized, metadata)
    }

    fn create_cado_map_cado(from: &str, to: &str) -> CadoBody {
        let cado_map = CADOMap {
            from: from.to_string(),
            to: to.to_string(),
        };
        let cado_map_serialized = bincode::serialize(&cado_map).expect("Failed to serialize CADO map");
        let metadata = CADOMetadata {
            type_: TYPE_CADO_MAP.to_string(),
            owner: "system".to_string(),
        };
        CadoBody::Mutable(CADOMut {
            hash: [0; 32],
            data: cado_map_serialized.clone(),
            metadata,
            latest_hash: Sha256::digest(&cado_map_serialized).into(),
        })
    }

    fn create_app_state_tip_cado(block_height: i64, cado_root_hash: [u8; 32], app_hash: Vec<u8>) -> CadoBody {
        let app_state_tip = AppStateTip {
            block_height,
            cado_root_hash,
            app_hash,
        };
        let app_state_tip_serialized =
            bincode::serialize(&app_state_tip).expect("Failed to serialize app state tip");
        let metadata = CADOMetadata {
            type_: TYPE_APP_STATE_TIP.to_string(),
            owner: "system".to_string(),
        };
        CadoBody::Mutable(CADOMut {
            hash: [0; 32],
            data: app_state_tip_serialized.clone(),
            metadata,
            latest_hash: Sha256::digest(&app_state_tip_serialized).into(),
        })
    }

    fn create_snapshot_metadata_cado(height: i64, epoch: u64, chunk_count: u32) -> CadoBody {
        let snapshot_metadata = SnapshotMetadata {
            height,
            epoch,
            format_version: 1,
            chunk_count,
            total_size: 1000000,
            compression: "lz4".to_string(),
            created_at: 1632739200,
            app_hash: vec![0; 32],
            chunk_hashes: vec!["hash1".to_string(), "hash2".to_string()],
        };
        let snapshot_metadata_serialized = bincode::serialize(&snapshot_metadata).expect("Failed to serialize snapshot metadata");
        let metadata = CADOMetadata {
            type_: TYPE_SNAPSHOT_METADATA.to_string(),
            owner: "system".to_string(),
        };
        CadoBody::Mutable(CADOMut {
            hash: [0; 32],
            data: snapshot_metadata_serialized.clone(),
            metadata,
            latest_hash: Sha256::digest(&snapshot_metadata_serialized).into(),
        })
    }

    fn create_snapshot_chunk_cado(index: u32, data: Vec<u8>, hash: &str) -> CadoBody {
        let snapshot_chunk = SnapshotChunk {
            index,
            data,
            hash: hash.to_string(),
            size: 1000,
            metadata: SnapshotChunkMetadata {
                accounts_count: 100,
                staking_accounts_count: 50,
                devices_count: 20,
                manifests_count: 30,
                chunk_proofs_count: 0,
            },
        };
        let snapshot_chunk_serialized = bincode::serialize(&snapshot_chunk).expect("Failed to serialize snapshot chunk");
        let metadata = CADOMetadata {
            type_: TYPE_SNAPSHOT_CHUNK.to_string(),
            owner: "system".to_string(),
        };
        CadoBody::Mutable(CADOMut {
            hash: [0; 32],
            data: snapshot_chunk_serialized.clone(),
            metadata,
            latest_hash: Sha256::digest(&snapshot_chunk_serialized).into(),
        })
    }

    fn path_account(address: &str) -> String {
        let addr = Address::parse_hex_str(address).expect("address");
        CadoPath::new(CadoType::Account, CadoPathKey::Address(addr))
            .expect("account path")
            .as_str()
            .to_string()
    }

    fn path_staking_account(address: &str) -> String {
        let addr = Address::parse_hex_str(address).expect("address");
        CadoPath::new(CadoType::StakingAccount, CadoPathKey::Address(addr))
            .expect("staking account path")
            .as_str()
            .to_string()
    }

    fn path_with_name(ty: CadoType, name: &str) -> String {
        CadoPath::new(ty, CadoPathKey::Name(name))
            .expect("cado path")
            .as_str()
            .to_string()
    }

    #[test]
    fn test_basic_operations() {
        let mut map = PatriciaMap::new();
        
        // Test insertions
        map.insert(b"foo", 1);
        map.insert(b"bar", 2);
        map.insert(b"baz", 3);
        
        assert_eq!(map.len(), 3, "Map should contain 3 elements");

        // Test retrievals
        assert_eq!(map.get(b"foo"), Some(&1), "Should retrieve value for 'foo'");
        assert_eq!(map.get(b"bar"), Some(&2), "Should retrieve value for 'bar'");
        assert_eq!(map.get(b"baz"), Some(&3), "Should retrieve value for 'baz'");
        
        // Test non-existent key
        assert_eq!(map.get(b"nonexistent"), None, "Should return None for non-existent key");
    }

    #[test]
    fn test_account_path() {
        let mut map = PatriciaMap::new();
        
        // Test account path (ELD root account prefix + 0x{20-byte-hex})
        let address = "0xe17404c417fa10cc04fdf73604fcacca8d0a687c";
        let path = path_account(address);
        let value = create_account_cado(address, 1000, 0);
        
        map.insert(path.as_bytes(), value.clone());
        
        let result = map.get(path.as_bytes());
        assert!(result.is_some(), "Should retrieve stored value");
        assert_eq!(result.unwrap().to_string(), value.to_string());

        // Add another account with a different hex value
        let address2 = "0xe17404c417fa10cc04fdf73604fcacca8d0a688";
        let path2 = path_account(address2);
        let value2 = create_account_cado(address2, 2000, 1);
        
        map.insert(path2.as_bytes(), value2.clone());

        // Both values should be retrievable
        let result1 = map.get(path.as_bytes());
        let result2 = map.get(path2.as_bytes());
        assert!(result1.is_some() && result2.is_some(), "Should retrieve both values");
        assert_eq!(result1.unwrap().to_string(), value.to_string());
        assert_eq!(result2.unwrap().to_string(), value2.to_string());
    }

    #[test]
    fn test_account_cado() {
        let mut map = PatriciaMap::new();
        
        // Create an account with initial balance and nonce
        let address = "0xe17404c417fa10cc04fdf73604fcacca8d0a687c";
        let initial_balance = 1000;
        let initial_nonce = 0;
        let account_cado = create_account_cado(address, initial_balance, initial_nonce);
        
        // Store the account
        let path = path_account(address);
        map.insert(path.as_bytes(), account_cado.clone());
        
        // Retrieve and verify the account
        let result = map.get(path.as_bytes());
        assert!(result.is_some(), "Should retrieve stored account");
        
        if let Some(CadoBody::Mutable(cado)) = result {
            let account: Account =
                Account::deserialize_bin(cado.data()).expect("Failed to deserialize account");
            assert_eq!(account.address().hex_with_prefix(), address);
            assert_eq!(account.balance(), Coin::new(initial_balance).unwrap());
            assert_eq!(account.nonce().value(), initial_nonce);
        } else {
            panic!("Retrieved CADO is not a mutable type");
        }

        // Update the account with new balance and nonce
        let updated_balance = 2000;
        let updated_nonce = 1;
        let updated_account_cado = create_account_cado(address, updated_balance, updated_nonce);
        
        map.insert(path.as_bytes(), updated_account_cado.clone());
        
        // Verify the update
        let result = map.get(path.as_bytes());
        assert!(result.is_some(), "Should retrieve updated account");
        
        if let Some(CadoBody::Mutable(cado)) = result {
            let account: Account =
                Account::deserialize_bin(cado.data()).expect("Failed to deserialize account");
            assert_eq!(account.address().hex_with_prefix(), address);
            assert_eq!(account.balance(), Coin::new(updated_balance).unwrap());
            assert_eq!(account.nonce().value(), updated_nonce);
        } else {
            panic!("Retrieved CADO is not a mutable type");
        }
    }

    #[test]
    fn test_prefix_operations() {
        let mut map = PatriciaMap::new();
        
        // Insert multiple accounts with common prefixes
        let base_path = format!("{}0x", PATH_PREFIX_ACCOUNT);
        
        // Insert accounts with different addresses but same prefix
        map.insert(format!("{}abc123", base_path).as_bytes(), create_test_cado(b"account1", "owner1"));
        map.insert(format!("{}abc456", base_path).as_bytes(), create_test_cado(b"account2", "owner2"));
        map.insert(format!("{}def789", base_path).as_bytes(), create_test_cado(b"account3", "owner3"));
        
        // Count entries with prefix "abc"
        let abc_prefix = format!("{}abc", base_path);
        let abc_count = map.iter_prefix(abc_prefix.as_bytes()).count();
        assert_eq!(abc_count, 2, "Should find 2 entries with abc prefix");
        
        // Count entries with prefix "def"
        let def_prefix = format!("{}def", base_path);
        let def_count = map.iter_prefix(def_prefix.as_bytes()).count();
        assert_eq!(def_count, 1, "Should find 1 entry with def prefix");
    }

    #[test]
    fn test_mixed_types() {
        let mut map = PatriciaMap::new();
        
        let address = "0xe17404c417fa10cc04fdf73604fcacca8d0a687c";
        
        // Add different types of CADOs for the same address
        let account_path = path_account(address);
        let staking_path = path_staking_account(address);
        
        let account_value = create_test_cado(b"account", "owner1");
        let staking_value = create_test_cado(b"staking", "owner1");
        
        map.insert(account_path.as_bytes(), account_value.clone());
        map.insert(staking_path.as_bytes(), staking_value.clone());
        
        // All values should be retrievable
        assert_eq!(map.get(account_path.as_bytes()).unwrap().to_string(), account_value.to_string());
        assert_eq!(map.get(staking_path.as_bytes()).unwrap().to_string(), staking_value.to_string());
        
        // Prefix search should work
        let prefix = PATH_PREFIX_ELD_ROOT_SCOPE.to_string();
        let prefix_count = map.iter_prefix(prefix.as_bytes()).count();
        assert_eq!(
            prefix_count,
            2,
            "Should find all 2 entries under ELD root scope prefix"
        );
    }

    #[test]
    fn test_staking_account_path() {
        let mut map = PatriciaMap::new();
        
        // Test staking account path (staking account prefix + 0x{20-byte-hex})
        let address = "0xe17404c417fa10cc04fdf73604fcacca8d0a687c";
        let path = path_staking_account(address);
        let value = create_staking_account_cado(address, 1000, "originator1");
        
        map.insert(path.as_bytes(), value.clone());
        
        let result = map.get(path.as_bytes());
        assert!(result.is_some(), "Should retrieve stored value");
        assert_eq!(result.unwrap().to_string(), value.to_string());

        // Add another staking account
        let address2 = "0xe17404c417fa10cc04fdf73604fcacca8d0a688";
        let path2 = path_staking_account(address2);
        let value2 = create_staking_account_cado(address2, 2000, "originator2");
        
        map.insert(path2.as_bytes(), value2.clone());

        // Both values should be retrievable
        let result1 = map.get(path.as_bytes());
        let result2 = map.get(path2.as_bytes());
        assert!(result1.is_some() && result2.is_some(), "Should retrieve both values");
        assert_eq!(result1.unwrap().to_string(), value.to_string());
        assert_eq!(result2.unwrap().to_string(), value2.to_string());
    }

    #[test]
    fn test_hash_paths() {
        let mut map = PatriciaMap::new();

        let map_path = path_with_name(
            CadoType::CadoMap,
            "0x869d68cd0d14420d74148c777655c98d75b73ecf3a958367de78c6fcd5a564f1",
        );
        let map_value = create_test_cado(b"cado_map", "owner1");
        map.insert(map_path.as_bytes(), map_value.clone());

        let chunk_path = path_with_name(
            CadoType::ChunkReference,
            "0x869d68cd0d14420d74148c777655c98d75b73ecf3a958367de78c6fcd5a564f1",
        );
        let chunk_value = create_test_cado(b"chunk_reference", "owner1");
        map.insert(chunk_path.as_bytes(), chunk_value.clone());

        let map_result = map.get(map_path.as_bytes());
        let chunk_result = map.get(chunk_path.as_bytes());
        assert!(
            map_result.is_some() && chunk_result.is_some(),
            "Should retrieve both values"
        );
        assert_eq!(map_result.unwrap().to_string(), map_value.to_string());
        assert_eq!(chunk_result.unwrap().to_string(), chunk_value.to_string());
    }

    #[test]
    fn test_cado_map_path() {
        let mut map = PatriciaMap::new();
        
        // Test CADO map path (cado_map type prefix + 0x{32-byte-hex})
        let from = "0x869d68cd0d14420d74148c777655c98d75b73ecf3a958367de78c6fcd5a564f1";
        let to = "0x969d68cd0d14420d74148c777655c98d75b73ecf3a958367de78c6fcd5a564f2";
        let path = path_with_name(CadoType::CadoMap, from);
        let value = create_cado_map_cado(from, to);
        
        map.insert(path.as_bytes(), value.clone());
        
        let result = map.get(path.as_bytes());
        assert!(result.is_some(), "Should retrieve stored value");
        assert_eq!(result.unwrap().to_string(), value.to_string());

        // Add another CADO map
        let from2 = "0xa69d68cd0d14420d74148c777655c98d75b73ecf3a958367de78c6fcd5a564f3";
        let to2 = "0xb69d68cd0d14420d74148c777655c98d75b73ecf3a958367de78c6fcd5a564f4";
        let path2 = path_with_name(CadoType::CadoMap, from2);
        let value2 = create_cado_map_cado(from2, to2);
        
        map.insert(path2.as_bytes(), value2.clone());

        // Both values should be retrievable
        let result1 = map.get(path.as_bytes());
        let result2 = map.get(path2.as_bytes());
        assert!(result1.is_some() && result2.is_some(), "Should retrieve both values");
        assert_eq!(result1.unwrap().to_string(), value.to_string());
        assert_eq!(result2.unwrap().to_string(), value2.to_string());
    }

    #[test]
    fn test_app_state_tip_path() {
        let mut map = PatriciaMap::new();
        
        // Test app state tip path (app_state_tip type prefix + 0x{32-byte-hex})
        let block_height = 100;
        let cado_root_hash = [0u8; 32];
        let app_hash = vec![1u8; 32];
        let path = path_with_name(
            CadoType::AppStateTip,
            "0x869d68cd0d14420d74148c777655c98d75b73ecf3a958367de78c6fcd5a564f1",
        );
        let value = create_app_state_tip_cado(block_height, cado_root_hash, app_hash.clone());
        
        map.insert(path.as_bytes(), value.clone());
        
        let result = map.get(path.as_bytes());
        assert!(result.is_some(), "Should retrieve stored value");
        assert_eq!(result.unwrap().to_string(), value.to_string());

        // Add another state version
        let block_height2 = 101;
        let cado_root_hash2 = [1u8; 32];
        let app_hash2 = vec![2u8; 32];
        let path2 = path_with_name(
            CadoType::AppStateTip,
            "0x969d68cd0d14420d74148c777655c98d75b73ecf3a958367de78c6fcd5a564f2",
        );
        let value2 = create_app_state_tip_cado(block_height2, cado_root_hash2, app_hash2);
        
        map.insert(path2.as_bytes(), value2.clone());

        // Both values should be retrievable
        let result1 = map.get(path.as_bytes());
        let result2 = map.get(path2.as_bytes());
        assert!(result1.is_some() && result2.is_some(), "Should retrieve both values");
        assert_eq!(result1.unwrap().to_string(), value.to_string());
        assert_eq!(result2.unwrap().to_string(), value2.to_string());
    }

    #[test]
    fn test_snapshot_paths() {
        let mut map = PatriciaMap::new();
        
        // Test snapshot metadata path (snapshot_metadata type prefix + 0x{32-byte-hex})
        let metadata_path = path_with_name(
            CadoType::SnapshotMetadata,
            "0x869d68cd0d14420d74148c777655c98d75b73ecf3a958367de78c6fcd5a564f1",
        );
        let metadata_value = create_snapshot_metadata_cado(100, 1, 10);
        
        map.insert(metadata_path.as_bytes(), metadata_value.clone());

        // Test snapshot chunk path (snapshot_chunk type prefix + 0x{32-byte-hex})
        let chunk_path = path_with_name(
            CadoType::SnapshotChunk,
            "0x869d68cd0d14420d74148c777655c98d75b73ecf3a958367de78c6fcd5a564f1",
        );
        let chunk_value = create_snapshot_chunk_cado(1, vec![1, 2, 3], "hash1");
        
        map.insert(chunk_path.as_bytes(), chunk_value.clone());

        // Both values should be retrievable
        let metadata_result = map.get(metadata_path.as_bytes());
        let chunk_result = map.get(chunk_path.as_bytes());
        assert!(metadata_result.is_some() && chunk_result.is_some(), "Should retrieve both values");
        assert_eq!(metadata_result.unwrap().to_string(), metadata_value.to_string());
        assert_eq!(chunk_result.unwrap().to_string(), chunk_value.to_string());
    }

    #[test]
    fn test_chunk_reference_path() {
        let mut map = PatriciaMap::new();
        
        // Test chunk reference path (chunk_reference type prefix + 0x{32-byte-hex})
        let path = path_with_name(
            CadoType::ChunkReference,
            "0x869d68cd0d14420d74148c777655c98d75b73ecf3a958367de78c6fcd5a564f1",
        );
        let value = create_test_cado(b"chunk_reference", "owner1");
        
        map.insert(path.as_bytes(), value.clone());
        
        let result = map.get(path.as_bytes());
        assert!(result.is_some(), "Should retrieve stored value");
        assert_eq!(result.unwrap().to_string(), value.to_string());

        // Add another chunk reference
        let path2 = path_with_name(
            CadoType::ChunkReference,
            "0x969d68cd0d14420d74148c777655c98d75b73ecf3a958367de78c6fcd5a564f2",
        );
        let value2 = create_test_cado(b"chunk_reference2", "owner2");
        
        map.insert(path2.as_bytes(), value2.clone());

        // Both values should be retrievable
        let result1 = map.get(path.as_bytes());
        let result2 = map.get(path2.as_bytes());
        assert!(result1.is_some() && result2.is_some(), "Should retrieve both values");
        assert_eq!(result1.unwrap().to_string(), value.to_string());
        assert_eq!(result2.unwrap().to_string(), value2.to_string());
    }

    #[test]
    fn test_account_paths_from_prod() {
        let mut map = PatriciaMap::new();
        
        // These are the exact account paths from production
        let account_paths = vec![
            path_account("0x23b1f0b6199479b5d04fb54e21df14d51530b7b1"),
            path_account("0x5de5bb98d243d7a7d74ae1d592cef1becb43957c"),
            path_account("0x5ffd39cc9d6ca5003d2414a784242f2fa9baaab3"),
            path_account("0xdd6756748bfe442e61681e243896200b4347bd69"),
            path_account("0xe17404c417fa10cc04fdf73604fcacca8d0a687c"),
        ];

        // Insert all paths with account CADOs
        for path in &account_paths {
            // Extract address from path
            let addr = path.split("/").last().unwrap();
            let account_cado = create_account_cado(addr, 100000000000, 0);
            map.insert(path.as_bytes(), account_cado);
        }

        // Verify we can read all paths
        for path in &account_paths {
            let result = map.get(path.as_bytes());
            assert!(result.is_some(), "Failed to get account path: {}", path);
            
            // Verify the account data
            if let Some(CadoBody::Mutable(cado)) = result {
                let account: Account =
                    Account::deserialize_bin(cado.data()).expect("Failed to deserialize account");
                let addr = path.split("/").last().unwrap();
                assert_eq!(account.address().hex_with_prefix(), addr);
                assert_eq!(account.balance(), Coin::new(100000000000).unwrap());
                assert_eq!(account.nonce(), Nonce::new(Nonce::ZERO));
            } else {
                panic!("Retrieved CADO is not a mutable type");
            }
        }

        // Test prefix search
        let prefix = PATH_PREFIX_ACCOUNT;
        let prefix_count = map.iter_prefix(prefix.as_bytes()).count();
        assert_eq!(prefix_count, 5, "Should find all 5 accounts under the prefix");
    }

    #[test]
    fn test_staking_account_cado() {
        let mut map = PatriciaMap::new();
        
        // Create a staking account with initial stake balance
        let address = "0xe17404c417fa10cc04fdf73604fcacca8d0a687c";
        let stake_balance = Coin::new(1000).expect("Failed to create coin");
        let originator = "0x5de5bb98d243d7a7d74ae1d592cef1becb43957c";
        let staking_account_cado = create_staking_account_cado(address, stake_balance, originator);
        
        // Store the staking account
        let path = path_staking_account(address);
        map.insert(path.as_bytes(), staking_account_cado.clone());
        
        // Retrieve and verify the staking account
        let result = map.get(path.as_bytes());
        assert!(result.is_some(), "Should retrieve stored staking account");
        
        if let Some(CadoBody::Mutable(cado)) = result {
            let staking_account: StakingAccount = bincode::deserialize(&cado.data).expect("Failed to deserialize staking account");
            let expected_address = Address::parse_hex_str(address).expect("Failed to parse address");
            let expected_originator = Address::parse_hex_str(originator).expect("Failed to parse originator");
            assert_eq!(staking_account.address, expected_address);
            assert_eq!(staking_account.stake_balance, stake_balance);
            assert_eq!(staking_account.originator, expected_originator);
        } else {
            panic!("Retrieved CADO is not a mutable type");
        }

        // Update the staking account with new stake balance
        let updated_stake_balance = Coin::new(2000).expect("Failed to create coin");
        let updated_staking_account_cado = create_staking_account_cado(address, updated_stake_balance, originator);
        
        map.insert(path.as_bytes(), updated_staking_account_cado.clone());
        
        // Verify the update
        let result = map.get(path.as_bytes());
        assert!(result.is_some(), "Should retrieve updated staking account");
        
        if let Some(CadoBody::Mutable(cado)) = result {
            let staking_account: StakingAccount = bincode::deserialize(&cado.data).expect("Failed to deserialize staking account");
            let expected_address = Address::parse_hex_str(address).expect("Failed to parse address");
            let expected_originator = Address::parse_hex_str(originator).expect("Failed to parse originator");
            assert_eq!(staking_account.address, expected_address);
            assert_eq!(staking_account.stake_balance, updated_stake_balance);
            assert_eq!(staking_account.originator, expected_originator);
        } else {
            panic!("Retrieved CADO is not a mutable type");
        }
    }

    #[test]
    fn test_cado_map_cado() {
        let mut map = PatriciaMap::new();
        
        // Create a CADO map
        let from = "0x869d68cd0d14420d74148c777655c98d75b73ecf3a958367de78c6fcd5a564f1";
        let to = "0x969d68cd0d14420d74148c777655c98d75b73ecf3a958367de78c6fcd5a564f2";
        let cado_map_cado = create_cado_map_cado(from, to);
        
        // Store the CADO map
        let path = path_with_name(CadoType::CadoMap, from);
        map.insert(path.as_bytes(), cado_map_cado.clone());
        
        // Retrieve and verify the CADO map
        let result = map.get(path.as_bytes());
        assert!(result.is_some(), "Should retrieve stored CADO map");
        
        if let Some(CadoBody::Mutable(cado)) = result {
            let cado_map: CADOMap = bincode::deserialize(&cado.data).expect("Failed to deserialize CADO map");
            assert_eq!(cado_map.from, from);
            assert_eq!(cado_map.to, to);
        } else {
            panic!("Retrieved CADO is not a mutable type");
        }
    }

    #[test]
    fn test_app_state_tip_cado() {
        let mut map = PatriciaMap::new();
        
        // Create a state version
        let block_height = 100;
        let cado_root_hash = [0u8; 32];
        let app_hash = vec![1u8; 32];
        let app_state_tip_cado = create_app_state_tip_cado(block_height, cado_root_hash, app_hash.clone());
        
        // Store the state version
        let path = path_with_name(
            CadoType::AppStateTip,
            "0x869d68cd0d14420d74148c777655c98d75b73ecf3a958367de78c6fcd5a564f1",
        );
        map.insert(path.as_bytes(), app_state_tip_cado.clone());
        
        // Retrieve and verify the state version
        let result = map.get(path.as_bytes());
        assert!(result.is_some(), "Should retrieve stored state version");
        
        if let Some(CadoBody::Mutable(cado)) = result {
            let app_state_tip: AppStateTip =
                bincode::deserialize(&cado.data).expect("Failed to deserialize app state tip");
            assert_eq!(app_state_tip.block_height, block_height);
            assert_eq!(app_state_tip.cado_root_hash, cado_root_hash);
            assert_eq!(app_state_tip.app_hash, app_hash);
        } else {
            panic!("Retrieved CADO is not a mutable type");
        }
    }

    #[test]
    fn test_snapshot_metadata_cado() {
        let mut map = PatriciaMap::new();
        
        // Create snapshot metadata
        let height = 100;
        let epoch = 1;
        let chunk_count = 10;
        let snapshot_metadata_cado = create_snapshot_metadata_cado(height, epoch, chunk_count);
        
        // Store the snapshot metadata
        let path = path_with_name(
            CadoType::SnapshotMetadata,
            "0x869d68cd0d14420d74148c777655c98d75b73ecf3a958367de78c6fcd5a564f1",
        );
        map.insert(path.as_bytes(), snapshot_metadata_cado.clone());
        
        // Retrieve and verify the snapshot metadata
        let result = map.get(path.as_bytes());
        assert!(result.is_some(), "Should retrieve stored snapshot metadata");
        
        if let Some(CadoBody::Mutable(cado)) = result {
            let snapshot_metadata: SnapshotMetadata = bincode::deserialize(&cado.data).expect("Failed to deserialize snapshot metadata");
            assert_eq!(snapshot_metadata.height, height);
            assert_eq!(snapshot_metadata.epoch, epoch);
            assert_eq!(snapshot_metadata.format_version, 1);
            assert_eq!(snapshot_metadata.chunk_count, chunk_count);
            assert_eq!(snapshot_metadata.total_size, 1000000);
            assert_eq!(snapshot_metadata.compression, "lz4");
            assert_eq!(snapshot_metadata.created_at, 1632739200);
            assert_eq!(snapshot_metadata.app_hash, vec![0; 32]);
            assert_eq!(snapshot_metadata.chunk_hashes, vec!["hash1".to_string(), "hash2".to_string()]);
        } else {
            panic!("Retrieved CADO is not a mutable type");
        }
    }

    #[test]
    fn test_snapshot_chunk_cado() {
        let mut map = PatriciaMap::new();
        
        // Create a snapshot chunk
        let index = 1;
        let data = vec![1, 2, 3];
        let hash = "hash1";
        let snapshot_chunk_cado = create_snapshot_chunk_cado(index, data.clone(), hash);
        
        // Store the snapshot chunk
        let path = path_with_name(
            CadoType::SnapshotChunk,
            "0x869d68cd0d14420d74148c777655c98d75b73ecf3a958367de78c6fcd5a564f1",
        );
        map.insert(path.as_bytes(), snapshot_chunk_cado.clone());
        
        // Retrieve and verify the snapshot chunk
        let result = map.get(path.as_bytes());
        assert!(result.is_some(), "Should retrieve stored snapshot chunk");
        
        if let Some(CadoBody::Mutable(cado)) = result {
            let snapshot_chunk: SnapshotChunk = bincode::deserialize(&cado.data).expect("Failed to deserialize snapshot chunk");
            assert_eq!(snapshot_chunk.index, index);
            assert_eq!(snapshot_chunk.data, data);
            assert_eq!(snapshot_chunk.hash, hash);
            assert_eq!(snapshot_chunk.size, 1000);
            
            let metadata = snapshot_chunk.metadata;
            assert_eq!(metadata.accounts_count, 100);
            assert_eq!(metadata.staking_accounts_count, 50);
            assert_eq!(metadata.devices_count, 20);
            assert_eq!(metadata.manifests_count, 30);
            assert_eq!(metadata.chunk_proofs_count, 0);
        } else {
            panic!("Retrieved CADO is not a mutable type");
        }
    }

    #[test]
    fn test_chunk_reference_cado() {
        let mut map = PatriciaMap::new();
        
        // Create a chunk reference
        let chunk_id = "0x869d68cd0d14420d74148c777655c98d75b73ecf3a958367de78c6fcd5a564f1";
        let owner = "owner1";
        let value = create_test_cado(b"test1", owner);
        
        // Store the chunk reference
        let path = path_with_name(CadoType::ChunkReference, chunk_id);
        map.insert(path.as_bytes(), value.clone());
        
        // Retrieve and verify the chunk reference
        let result = map.get(path.as_bytes());
        assert!(result.is_some(), "Should retrieve stored chunk reference");
        assert_eq!(result.unwrap().to_string(), value.to_string());
        
        // Add another chunk reference
        let chunk_id2 = "0x969d68cd0d14420d74148c777655c98d75b73ecf3a958367de78c6fcd5a564f2";
        let owner2 = "owner2";
        let value2 = create_test_cado(b"test2", owner2);
        let path2 = path_with_name(CadoType::ChunkReference, chunk_id2);

        map.insert(path2.as_bytes(), value2.clone());
        
        // Both chunk references should be retrievable
        let result1 = map.get(path.as_bytes());
        let result2 = map.get(path2.as_bytes());
        assert!(result1.is_some() && result2.is_some(), "Should retrieve both chunk references");
        assert_eq!(result1.unwrap().to_string(), value.to_string());
        assert_eq!(result2.unwrap().to_string(), value2.to_string());
    }
} 