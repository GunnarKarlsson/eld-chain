use super::RocksDBStorage;
use crate::api::pagination::{PaginationParams, PrefixQueryOptions};
use crate::storage::traits::{
    EpochRecordListOrder, PinboardGlobalFeedOrder, PinboardStorage, SnapshotChunk,
    SnapshotMetadata, SnapshotStorage, SnapshotStorageTestExt,
};
use eld_common::address::Address;
use eld_common::cado::{CADOMap, CADOMetadata, CadoBody, CadoPath, CadoPathKey, CadoType};
use eld_common::constants::cado::{
    PATH_PREFIX_ACCOUNT, PATH_PREFIX_APP_STATE_TIP, PATH_PREFIX_CADO_MAP,
    PATH_PREFIX_CHUNK_REFERENCE, PATH_PREFIX_OTHER_SCOPE, PATH_PREFIX_SNAPSHOT_CHUNK,
    PATH_PREFIX_SNAPSHOT_METADATA, PATH_PREFIX_STAKING_ACCOUNT, PATH_PREFIX_TEST_SCOPE,
    SCOPE_PUBLIC, SCOPE_TEST, SCOPE_USER, TYPE_CADO_MAP, TYPE_CONTENT_MANIFEST,
    TYPE_SNAPSHOT_CHUNK,
};
use eld_common::pinboard::PinboardMessageMetadata;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

fn create_test_storage() -> RocksDBStorage {
    let temp_dir = TempDir::new().unwrap();
    RocksDBStorage::new(temp_dir.path()).unwrap()
}

fn test_snapshot_metadata(
    height: i64,
    chunk_count: u32,
    total_size: u64,
    app_hash: [u8; 32],
    chunk_hashes: &[&str],
) -> SnapshotMetadata {
    SnapshotMetadata {
        height,
        epoch: 1,
        format_version: 1,
        chunk_count,
        total_size,
        compression: "zstd".to_string(),
        created_at: 1234567890,
        app_hash,
        accounts_count: 0,
        staking_accounts_count: 0,
        storage_staking_accounts_count: 0,
        namespaces_count: 0,
        chunk_hashes: chunk_hashes
            .iter()
            .map(|hash| (*hash).to_string())
            .collect(),
    }
}

fn put_test_epoch_record(storage: &RocksDBStorage, epoch: i64) {
    use eld_common::cado::{epoch_record_path_name, CADOMetadata, CadoPathKey, CadoType};
    use eld_common::constants::cado::LATEST;
    use eld_common::validator::EpochRecord;

    let name = epoch_record_path_name(epoch).expect("epoch name");
    let path = CadoPath::new(CadoType::EpochRecord, CadoPathKey::Name(&name)).expect("epoch path");
    let record = EpochRecord {
        epoch,
        start_block: epoch * 10,
        active_validators: vec![],
        active_capacity_validator: None,
        challenged_capacity_validators: vec![],
    };
    let bytes = record.serialize_bin().expect("serialize");
    let cado = CadoBody::immutable(
        bytes,
        CADOMetadata::new(CadoType::EpochRecord, epoch.to_string()),
    );
    storage
        .put_cado_type(path, cado.clone())
        .expect("put epoch");

    let latest_path =
        CadoPath::new(CadoType::EpochRecord, CadoPathKey::Name(LATEST)).expect("latest path");
    storage
        .put_cado_type(latest_path, cado)
        .expect("put latest");
}

#[test]
fn list_epoch_records_chron_desc_asc_and_cursor() {
    let storage = create_test_storage();
    for epoch in [0i64, 2, 5, 10] {
        put_test_epoch_record(&storage, epoch);
    }

    assert_eq!(storage.count_epoch_records().unwrap(), 4);

    let desc = storage
        .list_epoch_records_chron(EpochRecordListOrder::Desc, None, 10)
        .unwrap();
    assert_eq!(
        desc.iter().map(|r| r.epoch).collect::<Vec<_>>(),
        vec![10, 5, 2, 0]
    );

    let page1 = storage
        .list_epoch_records_chron(EpochRecordListOrder::Desc, None, 2)
        .unwrap();
    assert_eq!(page1.len(), 2);
    assert_eq!(page1[0].epoch, 10);
    assert_eq!(page1[1].epoch, 5);

    let page2 = storage
        .list_epoch_records_chron(EpochRecordListOrder::Desc, Some(5), 10)
        .unwrap();
    assert_eq!(
        page2.iter().map(|r| r.epoch).collect::<Vec<_>>(),
        vec![2, 0]
    );

    let asc = storage
        .list_epoch_records_chron(EpochRecordListOrder::Asc, None, 10)
        .unwrap();
    assert_eq!(
        asc.iter().map(|r| r.epoch).collect::<Vec<_>>(),
        vec![0, 2, 5, 10]
    );

    let asc_page2 = storage
        .list_epoch_records_chron(EpochRecordListOrder::Asc, Some(2), 10)
        .unwrap();
    assert_eq!(
        asc_page2.iter().map(|r| r.epoch).collect::<Vec<_>>(),
        vec![5, 10]
    );
}

#[test]
fn verified_proof_submission_claim_is_idempotent_per_sender_and_challenge() {
    let s = create_test_storage();
    assert!(!s
        .has_verified_proof_submission_claim("0xAbCd", "challenge-a")
        .unwrap());
    assert!(s
        .insert_verified_proof_submission_claim("0xAbCd", "challenge-a")
        .unwrap());
    assert!(s
        .has_verified_proof_submission_claim("0xabcd", "challenge-a")
        .unwrap());
    assert!(!s
        .insert_verified_proof_submission_claim("0xabcd", "challenge-a")
        .unwrap());
    assert!(s
        .insert_verified_proof_submission_claim("0xabcd", "challenge-b")
        .unwrap());
}

#[test]
fn verified_proof_challenge_rewarded_dedup_persists() {
    let s = create_test_storage();
    assert!(!s
        .is_verified_proof_challenge_rewarded("challenge-x")
        .unwrap());

    let tx = s.begin_transaction();
    s.put_verified_proof_challenge_rewarded_with_tx("challenge-x", &tx)
        .unwrap();
    tx.commit().unwrap();

    assert!(s
        .is_verified_proof_challenge_rewarded("challenge-x")
        .unwrap());
}

#[test]
fn verified_proof_challenge_failed_dedup_persists() {
    let s = create_test_storage();
    assert!(!s.is_verified_proof_challenge_failed("challenge-y").unwrap());

    let tx = s.begin_transaction();
    s.put_verified_proof_challenge_failed_with_tx("challenge-y", &tx)
        .unwrap();
    tx.commit().unwrap();

    assert!(s.is_verified_proof_challenge_failed("challenge-y").unwrap());
    assert!(!s
        .is_verified_proof_challenge_rewarded("challenge-y")
        .unwrap());
}

const TEST_ADDR_20: &str = "0x1234567890123456789012345678901234567890";
const TEST_HASH_32A: &str = "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
const TEST_HASH_32B: &str = "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
const TEST_HASH_DEAD: &str = "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
/// 32-byte hex id in `0x` + 64 nibbles form (used for multi-scope path fixtures).
const TEST_HASH_32_CONTRACT_STYLE: &str =
    "0x1234567890123456789012345678901234567890123456789012345678901234";
/// Prefix using a scope that is not in [`ALLOWED_SCOPES`], for negative tests only.
const PREFIX_MALICIOUS_ACCOUNT: &str = "/@malicious/account/";
/// Forbidden scope label used in path-validation tests (not an allowed [`SCOPE_ELD_ROOT`]-family scope).
const SCOPE_REJECTED_SYSTEM: &str = "@system";

#[test]
fn test_pinboard_temp_blob_extend_ttl_keeps_bytes() {
    let storage = create_test_storage();
    let content_key = "deadbeef";
    let original = b"pinboard-bytes";

    assert!(!storage
        .extend_pinboard_temp_blob_ttl(content_key, 2_000)
        .unwrap());

    storage
        .put_pinboard_temp_blob(content_key, original, 1_000)
        .unwrap();

    assert!(storage
        .extend_pinboard_temp_blob_ttl(content_key, 1_500)
        .unwrap());
    assert_eq!(
        storage
            .get_pinboard_temp_blob(content_key)
            .unwrap()
            .as_deref(),
        Some(original.as_slice())
    );
    assert_eq!(
        storage.pinboard_temp_blob_expires_at(content_key).unwrap(),
        Some(1_500)
    );

    assert!(storage
        .extend_pinboard_temp_blob_ttl(content_key, 1_200)
        .unwrap());
    assert_eq!(
        storage.pinboard_temp_blob_expires_at(content_key).unwrap(),
        Some(1_500)
    );
}

#[test]
fn test_pinboard_metadata_roundtrip_and_indexes() {
    let storage = create_test_storage();

    let addr = Address::parse_hex_str("0x0123456789abcdef0123456789abcdef01234567").unwrap();
    let meta = PinboardMessageMetadata {
        message_id: "msg1".to_string(),
        original_signer: addr,
        content_key: "deadbeef".to_string(),
        content_type: "application/json".to_string(),
        expires_height: 50,
        visibility: "Public".to_string(),
        topic: Some("general".to_string()),
        tags: vec!["dapp".to_string()],
        committed_height: 10,
        received_timestamp: 1_710_000_000,
        namespace: None,
    };

    let tx = storage.begin_transaction();
    storage
        .put_pinboard_metadata_with_tx(&meta.message_id, &meta, &tx)
        .unwrap();
    storage
        .put_pinboard_wallet_index_with_tx(
            &addr.hex_with_prefix(),
            meta.committed_height,
            &meta.message_id,
            &tx,
        )
        .unwrap();
    storage
        .put_pinboard_tag_index_with_tx(&meta.tags[0], meta.committed_height, &meta.message_id, &tx)
        .unwrap();
    storage
        .put_pinboard_expiry_index_with_tx(meta.expires_height, &meta.message_id, &tx)
        .unwrap();
    tx.commit().unwrap();

    let got = storage
        .get_pinboard_metadata(&meta.message_id)
        .unwrap()
        .unwrap();
    assert_eq!(got, meta);
}

#[test]
fn test_pinboard_namespace_secondary_path_key() {
    use eld_common::pinboard::pinboard_namespace_content_path;

    let storage = create_test_storage();
    let addr = Address::parse_hex_str("0x0123456789abcdef0123456789abcdef01234567").unwrap();
    let meta = PinboardMessageMetadata {
        message_id: "msgcaptain".to_string(),
        original_signer: addr,
        content_key: "cafebabe".to_string(),
        content_type: "text/plain".to_string(),
        expires_height: 50,
        visibility: "Public".to_string(),
        topic: None,
        tags: vec![],
        committed_height: 10,
        received_timestamp: 1_710_000_000,
        namespace: Some("captainhook".to_string()),
    };

    let path_key = pinboard_namespace_content_path("captainhook", "msgcaptain");
    let tx = storage.begin_transaction();
    storage
        .put_pinboard_metadata_with_tx(&meta.message_id, &meta, &tx)
        .unwrap();
    tx.commit().unwrap();

    assert_eq!(
        storage.get_pinboard_metadata("msgcaptain").unwrap(),
        Some(meta.clone())
    );
    assert_eq!(
        storage
            .get_pinboard_metadata_by_path_key(&path_key)
            .unwrap(),
        Some(meta)
    );
    assert!(storage
        .get_pinboard_metadata_by_path_key("/@eld/pinboard/post/0x0/msgcaptain")
        .unwrap()
        .is_none());
}

#[test]
fn test_pinboard_refcount_update() {
    let storage = create_test_storage();
    let ck = "aa";
    let tx = storage.begin_transaction();
    assert_eq!(
        storage
            .update_pinboard_refcount_with_tx(ck, 1, &tx)
            .unwrap(),
        1
    );
    assert_eq!(
        storage
            .update_pinboard_refcount_with_tx(ck, 2, &tx)
            .unwrap(),
        3
    );
    assert_eq!(
        storage
            .update_pinboard_refcount_with_tx(ck, -1, &tx)
            .unwrap(),
        2
    );
    tx.commit().unwrap();

    let raw = storage
        .db
        .get_cf(storage.pinboard_refcounts_cf().unwrap(), ck.as_bytes())
        .unwrap()
        .unwrap();
    let persisted: u64 = bincode::deserialize(&raw).unwrap();
    assert_eq!(persisted, 2);
}

#[test]
fn test_pinboard_list_by_tag_paginated() {
    let storage = create_test_storage();

    let tx = storage.begin_transaction();
    storage
        .put_pinboard_tag_index_with_tx("dapp", 10, "m1", &tx)
        .unwrap();
    storage
        .put_pinboard_tag_index_with_tx("dapp", 11, "m2", &tx)
        .unwrap();
    storage
        .put_pinboard_tag_index_with_tx("dapp", 12, "m3", &tx)
        .unwrap();
    storage
        .put_pinboard_tag_index_with_tx("other", 10, "x1", &tx)
        .unwrap();
    tx.commit().unwrap();

    let page0 = storage
        .get_pinboard_message_ids_by_tag_secure(
            "dapp",
            PrefixQueryOptions::default().with_pagination(PaginationParams {
                page: 0,
                page_size: 2,
                cursor: None,
            }),
        )
        .unwrap();
    assert_eq!(page0.items, vec!["m1".to_string(), "m2".to_string()]);
    assert!(page0.has_more);

    let page1 = storage
        .get_pinboard_message_ids_by_tag_secure(
            "dapp",
            PrefixQueryOptions::default().with_pagination(PaginationParams {
                page: 1,
                page_size: 2,
                cursor: None,
            }),
        )
        .unwrap();
    assert_eq!(page1.items, vec!["m3".to_string()]);
    assert!(!page1.has_more);
}

#[test]
fn test_pinboard_list_global_paginated_ordering() {
    let storage = create_test_storage();

    let tx = storage.begin_transaction();
    storage
        .put_pinboard_commit_index_with_tx(10, "m1", &tx)
        .unwrap();
    storage
        .put_pinboard_commit_index_with_tx(11, "m2", &tx)
        .unwrap();
    storage
        .put_pinboard_commit_index_with_tx(12, "m3", &tx)
        .unwrap();
    tx.commit().unwrap();

    let asc_page0 = storage
        .get_pinboard_message_ids_global_secure(
            PinboardGlobalFeedOrder::Asc,
            PrefixQueryOptions::default().with_pagination(PaginationParams {
                page: 0,
                page_size: 2,
                cursor: None,
            }),
        )
        .unwrap();
    assert_eq!(asc_page0.items, vec!["m1".to_string(), "m2".to_string()]);
    assert!(asc_page0.has_more);

    let asc_page1 = storage
        .get_pinboard_message_ids_global_secure(
            PinboardGlobalFeedOrder::Asc,
            PrefixQueryOptions::default().with_pagination(PaginationParams {
                page: 1,
                page_size: 2,
                cursor: None,
            }),
        )
        .unwrap();
    assert_eq!(asc_page1.items, vec!["m3".to_string()]);
    assert!(!asc_page1.has_more);

    let desc_page0 = storage
        .get_pinboard_message_ids_global_secure(
            PinboardGlobalFeedOrder::Desc,
            PrefixQueryOptions::default().with_pagination(PaginationParams {
                page: 0,
                page_size: 2,
                cursor: None,
            }),
        )
        .unwrap();
    assert_eq!(desc_page0.items, vec!["m3".to_string(), "m2".to_string()]);
    assert!(desc_page0.has_more);
}

#[test]
fn test_snapshot_storage() {
    let storage = create_test_storage();
    let metadata = test_snapshot_metadata(100, 5, 1024, [1u8; 32], &["hash1", "hash2"]);

    // Test put and get
    storage.put_snapshot_metadata(&metadata).unwrap();
    let retrieved = storage.get_snapshot_metadata(100).unwrap();
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().height, 100);

    // Test chunk storage
    let chunk = SnapshotChunk {
        index: 0,
        data: vec![1, 2, 3, 4, 5],
        hash: "chunk_hash".to_string(),
        size: 5,
    };

    storage.put_snapshot_chunk(100, &chunk).unwrap();
    let retrieved_chunk = storage.get_snapshot_chunk(100, 0).unwrap();
    assert!(retrieved_chunk.is_some());
    assert_eq!(retrieved_chunk.unwrap().data, vec![1, 2, 3, 4, 5]);
}

#[test]
fn test_secure_prefix_query_paths() {
    let storage = create_test_storage();

    // Create some test CADOs
    let test_paths = vec![
        format!(
            "{}{}/{}",
            PATH_PREFIX_TEST_SCOPE, TYPE_CADO_MAP, TEST_HASH_32A
        ),
        format!(
            "{}{}/{}",
            PATH_PREFIX_TEST_SCOPE, TYPE_CADO_MAP, TEST_HASH_32B
        ),
        format!(
            "{}{}/0x1111111111111111111111111111111111111111111111111111111111111111",
            PATH_PREFIX_TEST_SCOPE, TYPE_CADO_MAP
        ),
        format!(
            "{}{}/0x2222222222222222222222222222222222222222222222222222222222222222",
            PATH_PREFIX_OTHER_SCOPE, TYPE_CADO_MAP
        ),
        format!(
            "{}{}/0x3333333333333333333333333333333333333333333333333333333333333333",
            PATH_PREFIX_TEST_SCOPE, TYPE_CADO_MAP
        ),
    ];

    for path_str in &test_paths {
        let path = CadoPath::parse(path_str.as_str()).unwrap();
        let cado_type = CadoBody::immutable(
            vec![1, 2, 3],
            CADOMetadata::new(CadoType::CadoMap, "test_owner"),
        );
        storage.put_cado_type(path, cado_type).unwrap();
    }

    // Test secure prefix query with default options
    let options = PrefixQueryOptions::default();
    let result = storage
        .get_cado_paths_by_prefix_secure(PATH_PREFIX_TEST_SCOPE.trim_end_matches('/'), options)
        .unwrap();

    assert_eq!(result.items.len(), 4); // Should find 4 paths under test scope prefix
    assert!(!result.has_more); // Should not have more pages
    assert_eq!(result.page, 0);
    assert_eq!(result.page_size, 100);
}

#[test]
fn test_secure_prefix_query_cados() {
    let storage = create_test_storage();

    // Create a single test CADO with unique data
    let path = CadoPath::parse(&format!(
        "{PATH_PREFIX_TEST_SCOPE}{TYPE_CADO_MAP}/{TEST_HASH_32A}"
    ))
    .unwrap();
    let cado_type = CadoBody::immutable(
        vec![1, 2, 3, 4, 5], // Unique data
        CADOMetadata::new(CadoType::CadoMap, "test_owner"),
    );
    storage.put_cado_type(path, cado_type).unwrap();

    // Test secure prefix query
    let options = PrefixQueryOptions::default();
    let result = storage
        .get_cados_by_prefix_secure(PATH_PREFIX_TEST_SCOPE.trim_end_matches('/'), options)
        .unwrap();

    assert_eq!(result.items.len(), 1); // Should find 1 CADO

    // Check the data
    match &result.items[0] {
        CadoBody::Immutable(cado) => assert_eq!(cado.data(), vec![1, 2, 3, 4, 5]),
        _ => panic!("Expected immutable CADO"),
    }
}

#[test]
fn test_pagination() {
    let storage = create_test_storage();

    // Create 25 test CADOs
    for i in 0..25 {
        let path_str = format!("{PATH_PREFIX_TEST_SCOPE}{TYPE_CADO_MAP}/0x{i:064x}");
        let path = CadoPath::parse(&path_str).unwrap();
        let cado_type = CadoBody::immutable(
            vec![i as u8],
            CADOMetadata::new(CadoType::CadoMap, "test_owner"),
        );
        storage.put_cado_type(path, cado_type).unwrap();
    }

    // Test first page (10 items per page)
    let pagination = PaginationParams {
        page: 0,
        page_size: 10,
        cursor: None,
    };
    let options = PrefixQueryOptions::default().with_pagination(pagination);
    let result = storage
        .get_cado_paths_by_prefix_secure(PATH_PREFIX_TEST_SCOPE.trim_end_matches('/'), options)
        .unwrap();

    assert_eq!(result.items.len(), 10);
    assert!(result.has_more); // Should have more pages
    assert_eq!(result.page, 0);

    // Test second page
    let pagination = PaginationParams {
        page: 1,
        page_size: 10,
        cursor: None,
    };
    let options = PrefixQueryOptions::default().with_pagination(pagination);
    let result = storage
        .get_cado_paths_by_prefix_secure(PATH_PREFIX_TEST_SCOPE.trim_end_matches('/'), options)
        .unwrap();

    assert_eq!(result.items.len(), 10);
    assert!(result.has_more); // Should have more pages
    assert_eq!(result.page, 1);

    // Test third page
    let pagination = PaginationParams {
        page: 2,
        page_size: 10,
        cursor: None,
    };
    let options = PrefixQueryOptions::default().with_pagination(pagination);
    let result = storage
        .get_cado_paths_by_prefix_secure(PATH_PREFIX_TEST_SCOPE.trim_end_matches('/'), options)
        .unwrap();

    assert_eq!(result.items.len(), 5); // Should have 5 remaining items
    assert!(!result.has_more); // Should not have more pages
    assert_eq!(result.page, 2);
}

#[test]
fn test_memory_limits() {
    let storage = create_test_storage();

    // Create a large CADO
    let path = CadoPath::parse(&format!(
        "{PATH_PREFIX_TEST_SCOPE}{TYPE_CADO_MAP}/{TEST_HASH_32A}"
    ))
    .unwrap();
    let large_data = vec![0u8; 1024 * 1024]; // 1MB of data
    let cado_type = CadoBody::immutable(
        large_data,
        CADOMetadata::new(CadoType::CadoMap, "test_owner"),
    );
    storage.put_cado_type(path, cado_type).unwrap();

    // Test with very low memory limit
    let options = PrefixQueryOptions::with_limits(
        1000,
        1024, // Only 1KB limit
        "test_client".to_string(),
    );

    let result =
        storage.get_cados_by_prefix_secure(PATH_PREFIX_TEST_SCOPE.trim_end_matches('/'), options);
    // Should succeed but with memory limit warning in logs
    assert!(result.is_ok());
}

#[test]
fn test_result_limits() {
    let storage = create_test_storage();

    // Create many test CADOs
    for i in 0..50 {
        let path_str = format!("{PATH_PREFIX_TEST_SCOPE}{TYPE_CADO_MAP}/0x{i:064x}");
        let path = CadoPath::parse(&path_str).unwrap();
        let cado_type = CadoBody::immutable(
            vec![i as u8],
            CADOMetadata::new(CadoType::CadoMap, "test_owner"),
        );
        storage.put_cado_type(path, cado_type).unwrap();
    }

    // Test with low result limit
    let pagination = PaginationParams {
        page: 0,
        page_size: 10, // Small page size
        cursor: None,
    };
    let options = PrefixQueryOptions::with_limits(
        10, // Only 10 results max
        50 * 1024 * 1024,
        "test_client".to_string(),
    )
    .with_pagination(pagination);

    let result = storage
        .get_cado_paths_by_prefix_secure(PATH_PREFIX_TEST_SCOPE.trim_end_matches('/'), options)
        .unwrap();
    assert_eq!(result.items.len(), 10); // Should be limited to 10 results
}

#[test]
fn test_snapshot_manager_multiple_snapshots() {
    let storage = create_test_storage();

    // Create multiple snapshots
    for height in 100..105 {
        let metadata =
            test_snapshot_metadata(height, 2, 1024, [height as u8; 32], &["hash1", "hash2"]);
        storage.put_snapshot_metadata(&metadata).unwrap();

        // Add chunks for each snapshot
        for chunk_index in 0..2 {
            let chunk = SnapshotChunk {
                index: chunk_index,
                data: vec![height as u8, chunk_index as u8],
                hash: format!("hash_{height}_{chunk_index}"),
                size: 2,
            };
            storage.put_snapshot_chunk(height, &chunk).unwrap();
        }
    }

    // Test list snapshots
    let snapshots = storage.list_snapshots(10).unwrap();
    assert_eq!(snapshots.len(), 5);

    // Test get latest snapshot
    let latest = storage.get_latest_snapshot_metadata().unwrap();
    assert!(latest.is_some());
    assert_eq!(latest.unwrap().height, 104);

    // Test prune snapshots
    storage.prune_snapshots(2).unwrap();
    let snapshots = storage.list_snapshots(10).unwrap();
    assert_eq!(snapshots.len(), 2); // Should keep only the latest 2
}

#[test]
fn test_snapshot_manager_retrieval_edge_cases() {
    let storage = create_test_storage();

    // Test getting non-existent snapshot
    let result = storage.get_snapshot_metadata(999).unwrap();
    assert!(result.is_none());

    // Test getting non-existent chunk
    let result = storage.get_snapshot_chunk(999, 0).unwrap();
    assert!(result.is_none());

    // Test snapshot exists
    let exists = storage.snapshot_exists(999).unwrap();
    assert!(!exists);

    // Create a snapshot and test
    let metadata = test_snapshot_metadata(100, 1, 512, [1u8; 32], &["hash1"]);
    storage.put_snapshot_metadata(&metadata).unwrap();

    let exists = storage.snapshot_exists(100).unwrap();
    assert!(exists);

    // Test chunk count
    let count = storage.get_snapshot_chunk_count(100).unwrap();
    assert_eq!(count, 0); // No chunks added yet

    // Add a chunk and test count
    let chunk = SnapshotChunk {
        index: 0,
        data: vec![1, 2, 3],
        hash: "chunk_hash".to_string(),
        size: 3,
    };
    storage.put_snapshot_chunk(100, &chunk).unwrap();

    let count = storage.get_snapshot_chunk_count(100).unwrap();
    assert_eq!(count, 1); // Now has one chunk
}

#[test]
fn test_cado_path_validation_security() {
    // Test valid paths
    let valid_path = CadoPath::parse(&format!("{PATH_PREFIX_ACCOUNT}{TEST_ADDR_20}")).unwrap();
    assert!(RocksDBStorage::validate_cado_path_security(&valid_path).is_ok());

    // Test reserved scope names - create a path with reserved scope
    let reserved_scope_path =
        CadoPath::parse(&format!("/{SCOPE_REJECTED_SYSTEM}/account/{TEST_ADDR_20}")).unwrap();
    assert!(RocksDBStorage::validate_cado_path_security(&reserved_scope_path).is_err());
}

#[test]
fn test_cado_path_validation_in_storage_functions() {
    let storage = create_test_storage();

    // Test put_cado_data with reserved scope path
    let malicious_path =
        CadoPath::parse(&format!("/{SCOPE_REJECTED_SYSTEM}/account/{TEST_ADDR_20}")).unwrap();
    let metadata = CADOMetadata::new(
        CadoType::Account,
        "0x1234567890123456789012345678901234567890",
    );
    let result = storage.put_cado_data(malicious_path, vec![1, 2, 3], metadata);
    assert!(result.is_err());

    // Test get_cado_by_path with reserved scope path
    let malicious_path2 =
        CadoPath::parse(&format!("/{SCOPE_REJECTED_SYSTEM}/account/{TEST_ADDR_20}")).unwrap();
    let result = storage.get_cado_by_path(malicious_path2);
    assert!(result.is_err());

    // Test put_cado_data_with_tx with reserved scope path
    let malicious_path3 =
        CadoPath::parse(&format!("/{SCOPE_REJECTED_SYSTEM}/account/{TEST_ADDR_20}")).unwrap();
    let tx = storage.begin_transaction();
    let metadata = CADOMetadata::new(
        CadoType::Account,
        "0x1234567890123456789012345678901234567890",
    );
    let result = storage.put_cado_data_with_tx(malicious_path3, vec![1, 2, 3], metadata, &tx);
    assert!(result.is_err());
}

#[test]
fn test_snapshot_operations_with_corrupted_data() {
    use tracing_subscriber::fmt::Subscriber;
    let _ = Subscriber::builder().with_test_writer().try_init();

    let storage = create_test_storage();
    // Insert a valid snapshot
    let metadata = test_snapshot_metadata(1, 1, 100, [1u8; 32], &["hash1"]);
    storage.put_snapshot_metadata(&metadata).unwrap();

    // Insert a corrupted snapshot (invalid bincode data)
    let corrupted_path =
        CadoPath::parse(&format!("{PATH_PREFIX_SNAPSHOT_METADATA}{TEST_HASH_DEAD}")).unwrap();
    let corrupted_data = vec![0xde, 0xad, 0xbe, 0xef, 0x00, 0x01]; // Not valid bincode
    let corrupted_meta = CADOMetadata::new(CadoType::SnapshotMetadata, "system");
    storage
        .put_cado_data(corrupted_path, corrupted_data, corrupted_meta)
        .unwrap();

    // list_snapshots should not panic and should return the valid snapshot
    let snapshots = storage.list_snapshots(10).unwrap();
    assert!(snapshots.iter().any(|m| m.height == 1));
    // prune_snapshots should not panic
    storage.prune_snapshots(0).unwrap();
}

#[test]
fn test_enhanced_cado_path_validation() {
    // Test valid paths
    let valid_paths = [
        format!("{PATH_PREFIX_ACCOUNT}{TEST_ADDR_20}"),
        format!("/{SCOPE_USER}/{TYPE_CADO_MAP}/{TEST_HASH_32_CONTRACT_STYLE}"),
        format!("/{SCOPE_PUBLIC}/{TYPE_CADO_MAP}/{TEST_HASH_32_CONTRACT_STYLE}"),
        format!("/{SCOPE_TEST}/{TYPE_CADO_MAP}/{TEST_HASH_32_CONTRACT_STYLE}"),
    ];

    for path_str in &valid_paths {
        let path = CadoPath::parse(path_str).unwrap();
        assert!(
            RocksDBStorage::validate_cado_path_security_enhanced(&path).is_ok(),
            "Valid path should pass: {path_str}"
        );
    }

    // Test forbidden scopes (these should be rejected by enhanced validation)
    let forbidden_scopes = ["@system", "@admin", "@root", "@internal"];
    for scope in forbidden_scopes.iter() {
        let path_str = format!("/{scope}/account/{TEST_ADDR_20}");
        let path = CadoPath::parse(&path_str).unwrap();
        assert!(
            RocksDBStorage::validate_cado_path_security_enhanced(&path).is_err(),
            "Forbidden scope should be rejected: {scope}"
        );
    }

    // Test invalid types (this will be caught by CadoPath::new, so we test the enhanced validation directly)
    // Create a path with a valid type first, then test the enhanced validation logic
    let valid_path = CadoPath::parse(&format!("{PATH_PREFIX_ACCOUNT}{TEST_ADDR_20}")).unwrap();
    assert!(
        RocksDBStorage::validate_cado_path_security_enhanced(&valid_path).is_ok(),
        "Valid path should pass enhanced validation"
    );

    // Test path length limits (create a path that's too long)
    // We need to create a path that passes CadoPath::new but fails our length validation
    // Let's test this by creating a path with a very long scope name
    let long_scope = "@".to_string() + &"a".repeat(500);
    let long_path = format!("/{long_scope}/account/0x1234567890123456789012345678901234567890");
    let path = CadoPath::parse(&long_path).unwrap();
    assert!(
        RocksDBStorage::validate_cado_path_security_enhanced(&path).is_err(),
        "Path exceeding length limit should be rejected"
    );

    // Test that the original validation still works for basic cases
    let reserved_scope_path =
        CadoPath::parse(&format!("/{SCOPE_REJECTED_SYSTEM}/account/{TEST_ADDR_20}")).unwrap();
    assert!(RocksDBStorage::validate_cado_path_security(&reserved_scope_path).is_err());
}

#[test]
fn test_cado_prefix_validation() {
    // Test valid prefixes
    let valid_prefixes = vec![
        PATH_PREFIX_ACCOUNT.to_string(),
        format!("/{}/{}/", SCOPE_USER, TYPE_CONTENT_MANIFEST),
        format!("/{}/{}/", SCOPE_PUBLIC, TYPE_CADO_MAP),
    ];

    for prefix in &valid_prefixes {
        assert!(
            RocksDBStorage::validate_cado_prefix_security(prefix).is_ok(),
            "Valid prefix should pass: {prefix}"
        );
    }

    // Test malicious prefixes
    let malicious_prefixes = vec![
        format!("{}../", PATH_PREFIX_ACCOUNT),
        format!("{}..%2f", PATH_PREFIX_ACCOUNT),
        format!("{}//", PATH_PREFIX_ACCOUNT.trim_end_matches('/')),
        format!("{}~", PATH_PREFIX_ACCOUNT),
        "invalid_prefix".to_string(), // doesn't start with /
    ];

    for prefix in &malicious_prefixes {
        assert!(
            RocksDBStorage::validate_cado_prefix_security(prefix).is_err(),
            "Malicious prefix should be rejected: {prefix}"
        );
    }

    // Test prefix length limits
    let long_prefix = format!("{}{}", PATH_PREFIX_ACCOUNT, "a".repeat(300));
    assert!(
        RocksDBStorage::validate_cado_prefix_security(&long_prefix).is_err(),
        "Prefix exceeding length limit should be rejected"
    );

    // Test control characters in prefix
    let control_char_prefix = format!("{PATH_PREFIX_ACCOUNT}\x00");
    assert!(
        RocksDBStorage::validate_cado_prefix_security(&control_char_prefix).is_err(),
        "Prefix with null bytes should be rejected"
    );
}

#[test]
fn test_rate_limiting() {
    let storage = create_test_storage();

    // Test that we can make requests up to the limit
    for i in 0..60 {
        let result = storage.get_cado_paths_by_prefix(PATH_PREFIX_TEST_SCOPE);
        assert!(result.is_ok(), "Request {i} should succeed");
    }

    // Test that the 61st request is rate limited
    let result = storage.get_cado_paths_by_prefix(PATH_PREFIX_TEST_SCOPE);
    assert!(result.is_err(), "61st request should be rate limited");
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Rate limit exceeded"));

    // Test that requests for different prefixes are tracked separately
    let result = storage.get_cado_paths_by_prefix(PATH_PREFIX_OTHER_SCOPE);
    assert!(
        result.is_ok(),
        "Different prefix should not be rate limited"
    );
}

#[test]
fn test_rate_limiting_with_different_clients() {
    let storage = create_test_storage();

    // Test that different client IDs are tracked separately in secure methods
    let options1 = PrefixQueryOptions {
        client_id: "client1".to_string(),
        ..Default::default()
    };
    let options2 = PrefixQueryOptions {
        client_id: "client2".to_string(),
        ..Default::default()
    };

    // Client 1 should be able to make 60 requests
    for i in 0..60 {
        let result =
            storage.get_cado_paths_by_prefix_secure(PATH_PREFIX_TEST_SCOPE, options1.clone());
        assert!(result.is_ok(), "Client 1 request {i} should succeed");
    }

    // Client 1 should be rate limited on the 61st request
    let result = storage.get_cado_paths_by_prefix_secure(PATH_PREFIX_TEST_SCOPE, options1.clone());
    assert!(
        result.is_err(),
        "Client 1 61st request should be rate limited"
    );

    // Client 2 should still be able to make requests
    let result = storage.get_cado_paths_by_prefix_secure(PATH_PREFIX_TEST_SCOPE, options2.clone());
    assert!(result.is_ok(), "Client 2 should not be rate limited");
}

#[test]
fn test_rate_limit_configuration() {
    let temp_dir = tempfile::tempdir().unwrap();
    let mut storage = RocksDBStorage::new(temp_dir.path()).unwrap();

    // Test configuration update (default is 60; tighten to 30)
    storage.update_rate_limiter_config(30);

    // Test that the new limit is enforced
    for i in 0..30 {
        let result = storage.get_cado_paths_by_prefix(PATH_PREFIX_TEST_SCOPE);
        assert!(result.is_ok(), "Request {i} should succeed");
    }

    // Test that the 31st request is rate limited
    let result = storage.get_cado_paths_by_prefix(PATH_PREFIX_TEST_SCOPE);
    assert!(result.is_err(), "31st request should be rate limited");
}

#[test]
fn test_rate_limiting_all_prefix_methods() {
    let storage = create_test_storage();

    // Test each prefix-based method individually
    // get_cado_paths_by_prefix
    for i in 0..60 {
        let result = storage.get_cado_paths_by_prefix(PATH_PREFIX_TEST_SCOPE);
        assert!(
            result.is_ok(),
            "get_cado_paths_by_prefix request {i} should succeed"
        );
    }
    let result = storage.get_cado_paths_by_prefix(PATH_PREFIX_TEST_SCOPE);
    assert!(
        result.is_err(),
        "get_cado_paths_by_prefix 61st request should be rate limited"
    );
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Rate limit exceeded"));

    // Reset rate limiter for next test
    let mut storage = create_test_storage();
    storage.update_rate_limiter_config(60);

    // get_cados_by_prefix
    for i in 0..60 {
        let result = storage.get_cados_by_prefix(PATH_PREFIX_TEST_SCOPE);
        assert!(
            result.is_ok(),
            "get_cados_by_prefix request {i} should succeed"
        );
    }
    let result = storage.get_cados_by_prefix(PATH_PREFIX_TEST_SCOPE);
    assert!(
        result.is_err(),
        "get_cados_by_prefix 61st request should be rate limited"
    );
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Rate limit exceeded"));

    // Reset rate limiter for next test
    let mut storage = create_test_storage();
    storage.update_rate_limiter_config(60);

    // search_cado_path
    for i in 0..60 {
        let result = storage.search_cado_path(PATH_PREFIX_TEST_SCOPE);
        assert!(
            result.is_ok(),
            "search_cado_path request {i} should succeed"
        );
    }
    let result = storage.search_cado_path(PATH_PREFIX_TEST_SCOPE);
    assert!(
        result.is_err(),
        "search_cado_path 61st request should be rate limited"
    );
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Rate limit exceeded"));

    // Reset rate limiter for next test
    let mut storage = create_test_storage();
    storage.update_rate_limiter_config(60);

    // search_cado_hash
    for i in 0..60 {
        let result = storage.search_cado_hash(PATH_PREFIX_TEST_SCOPE);
        assert!(
            result.is_ok(),
            "search_cado_hash request {i} should succeed"
        );
    }
    let result = storage.search_cado_hash(PATH_PREFIX_TEST_SCOPE);
    assert!(
        result.is_err(),
        "search_cado_hash 61st request should be rate limited"
    );
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Rate limit exceeded"));

    // Reset rate limiter for next test
    let mut storage = create_test_storage();
    storage.update_rate_limiter_config(60);

    // search_cado_name
    for i in 0..60 {
        let result = storage.search_cado_name(PATH_PREFIX_TEST_SCOPE);
        assert!(
            result.is_ok(),
            "search_cado_name request {i} should succeed"
        );
    }
    let result = storage.search_cado_name(PATH_PREFIX_TEST_SCOPE);
    assert!(
        result.is_err(),
        "search_cado_name 61st request should be rate limited"
    );
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Rate limit exceeded"));
}

#[test]
fn test_rate_limiter_cleanup() {
    let storage = create_test_storage();

    // Make some requests
    for i in 0..10 {
        let result = storage.get_cado_paths_by_prefix(PATH_PREFIX_TEST_SCOPE);
        assert!(result.is_ok(), "Request {i} should succeed");
    }

    // Verify that requests are tracked
    assert_eq!(
        storage
            .rate_limiter
            .get_request_count("default", PATH_PREFIX_TEST_SCOPE),
        10
    );

    // Clean up old entries
    storage.rate_limiter.cleanup();

    // The cleanup should not affect current requests
    assert_eq!(
        storage
            .rate_limiter
            .get_request_count("default", PATH_PREFIX_TEST_SCOPE),
        10
    );
}

#[test]
fn test_all_cado_operations_validation() {
    let storage = create_test_storage();

    // Create a malicious path that should be rejected by all operations
    let malicious_path =
        CadoPath::parse(&format!("/{SCOPE_REJECTED_SYSTEM}/account/{TEST_ADDR_20}")).unwrap();
    let valid_path = CadoPath::parse(&format!("{PATH_PREFIX_ACCOUNT}{TEST_ADDR_20}")).unwrap();

    // Test all CADO operations that should be protected
    let metadata = CADOMetadata::new(
        CadoType::Account,
        "0x1234567890123456789012345678901234567890",
    );
    let mapping = CADOMap {
        from: "from".to_string(),
        to: "to".to_string(),
    };
    let cado_type = CadoBody::mutable_new(vec![1, 2, 3], metadata.clone());

    // Test put_cado_map
    assert!(storage
        .put_cado_map(malicious_path.clone(), mapping.clone())
        .is_err());
    assert!(storage
        .put_cado_map(valid_path.clone(), mapping.clone())
        .is_ok());

    // Test get_cado_map
    assert!(storage.get_cado_map(malicious_path.clone()).is_err());
    assert!(storage.get_cado_map(valid_path.clone()).is_ok());

    // Test put_cado_type
    assert!(storage
        .put_cado_type(malicious_path.clone(), cado_type.clone())
        .is_err());
    assert!(storage
        .put_cado_type(valid_path.clone(), cado_type.clone())
        .is_ok());

    // Test delete_cado (requires valid signature, but path validation should happen first)
    assert!(storage
        .delete_cado(
            malicious_path.clone(),
            "owner",
            "signature",
            "public_key",
            "chain_id"
        )
        .is_err());

    // Test system_delete_cado
    assert!(storage
        .system_delete_cado(malicious_path.clone(), "owner")
        .is_err());

    // Test transaction-based operations
    let tx = storage.begin_transaction();
    assert!(storage
        .put_cado_map_with_tx(malicious_path.clone(), mapping.clone(), &tx)
        .is_err());
    assert!(storage
        .put_cadotype_by_path_with_tx(malicious_path.clone(), cado_type.clone(), &tx)
        .is_err());
    assert!(storage
        .delete_cado_with_tx(
            malicious_path.clone(),
            "owner",
            "signature",
            "public_key",
            "chain_id",
            &tx
        )
        .is_err());

    // Test prefix-based operations with malicious scope
    assert!(storage.search_cado_path(PREFIX_MALICIOUS_ACCOUNT).is_err());
    assert!(storage
        .get_cado_paths_by_prefix(PREFIX_MALICIOUS_ACCOUNT)
        .is_err());
    assert!(storage
        .get_cados_by_prefix(PREFIX_MALICIOUS_ACCOUNT)
        .is_err());

    // Test valid prefix operations
    assert!(storage.search_cado_path(PATH_PREFIX_ACCOUNT).is_ok());
    assert!(storage
        .get_cado_paths_by_prefix(PATH_PREFIX_ACCOUNT)
        .is_ok());
    assert!(storage.get_cados_by_prefix(PATH_PREFIX_ACCOUNT).is_ok());
}

#[test]
fn test_cado_deletion_validation() {
    let storage = create_test_storage();

    // Test system-deletable types (should be allowed for system deletion)
    let system_deletable_paths = vec![
        format!("{}{}", PATH_PREFIX_CHUNK_REFERENCE, TEST_HASH_32A),
        format!("{}{}", PATH_PREFIX_SNAPSHOT_CHUNK, TEST_HASH_32B),
        format!("{}{}", PATH_PREFIX_SNAPSHOT_METADATA, TEST_HASH_32A),
    ];

    for path_str in system_deletable_paths {
        let path = CadoPath::parse(&path_str).unwrap();

        // Should be rejected for user deletion
        assert!(!path.is_user_deletable());
        assert!(path.validate_user_deletion().is_err());

        // Should be allowed for system deletion
        assert!(path.is_system_deletable());
        assert!(path.validate_system_deletion().is_ok());

        // Should be deletable overall
        assert!(path.is_deletable());
        assert!(!path.is_protected());
    }

    // Test protected types (should not be deletable at all)
    let protected_paths = vec![
        format!("{}{}", PATH_PREFIX_ACCOUNT, TEST_ADDR_20),
        format!("{}{}", PATH_PREFIX_STAKING_ACCOUNT, TEST_ADDR_20),
        format!("{}{}", PATH_PREFIX_CADO_MAP, TEST_HASH_32A),
        format!("{}{}", PATH_PREFIX_APP_STATE_TIP, TEST_HASH_32A),
    ];

    for path_str in protected_paths {
        let path = CadoPath::parse(&path_str).unwrap();

        // Should be rejected for both user and system deletion
        assert!(!path.is_user_deletable());
        assert!(!path.is_system_deletable());
        assert!(path.validate_user_deletion().is_err());
        assert!(path.validate_system_deletion().is_err());

        // Should not be deletable overall
        assert!(!path.is_deletable());
        assert!(path.is_protected());
    }

    // Test system deletion scope validation
    let non_chain_system_path = CadoPath::parse(&format!(
        "/{SCOPE_USER}/{TYPE_SNAPSHOT_CHUNK}/{TEST_HASH_32A}"
    ))
    .unwrap();
    assert!(non_chain_system_path.is_system_deletable());
    assert!(non_chain_system_path.validate_system_deletion().is_err()); // Wrong scope

    // Test storage layer validation
    let protected_path = CadoPath::parse(&format!("{PATH_PREFIX_ACCOUNT}{TEST_ADDR_20}")).unwrap();

    // Test user deletion validation in storage
    assert!(storage
        .delete_cado(
            protected_path.clone(),
            "owner",
            "signature",
            "public_key",
            "chain_id"
        )
        .is_err());

    // Test system deletion validation in storage
    assert!(storage
        .system_delete_cado(protected_path.clone(), "owner")
        .is_err());
    // Note: system_deletable_path would fail due to CADO not existing, but validation should pass
}

#[test]
fn system_delete_cado_with_tx_commits_atomically() {
    let storage = create_test_storage();
    let height = 100i64;
    let chunk = SnapshotChunk {
        index: 0,
        data: vec![1, 2, 3],
        hash: "0xabc".to_string(),
        size: 3,
    };
    storage.put_snapshot_chunk(height, &chunk).unwrap();
    assert!(storage.get_snapshot_chunk(height, 0).unwrap().is_some());

    let chunk_id = format!("{height}_{}", chunk.index);
    let chunk_hash = Sha256::digest(chunk_id);
    let snapshot_chunk_id = format!("0x{}", hex::encode(chunk_hash));
    let path = CadoPath::new(
        CadoType::SnapshotChunk,
        CadoPathKey::Name(&snapshot_chunk_id),
    )
    .unwrap();

    let tx = storage.begin_transaction();
    storage
        .system_delete_cado_with_tx(path, "system", &tx)
        .unwrap();
    tx.commit().unwrap();

    assert!(storage.get_snapshot_chunk(height, 0).unwrap().is_none());
}

#[test]
fn system_delete_cado_with_tx_rollback_leaves_cado_intact() {
    let storage = create_test_storage();
    let height = 200i64;
    let chunk = SnapshotChunk {
        index: 1,
        data: vec![4, 5, 6],
        hash: "0xdef".to_string(),
        size: 3,
    };
    storage.put_snapshot_chunk(height, &chunk).unwrap();
    assert!(storage.get_snapshot_chunk(height, 1).unwrap().is_some());

    let chunk_id = format!("{height}_{}", chunk.index);
    let chunk_hash = Sha256::digest(chunk_id);
    let snapshot_chunk_id = format!("0x{}", hex::encode(chunk_hash));
    let path = CadoPath::new(
        CadoType::SnapshotChunk,
        CadoPathKey::Name(&snapshot_chunk_id),
    )
    .unwrap();

    let tx = storage.begin_transaction();
    storage
        .system_delete_cado_with_tx(path, "system", &tx)
        .unwrap();
    drop(tx);

    assert!(storage.get_snapshot_chunk(height, 1).unwrap().is_some());
}

#[test]
fn test_block_pos_chron_desc_order_newest_first_and_continuation_tuple() {
    use crate::indexer::TransactionStatus;
    use crate::storage::traits::TransactionIndexerStorage;
    use ed25519_dalek::SigningKey;
    use eld_common::constants::{protocol::DEFAULT_TX_FEE, test::MOCK_CHAIN_ID};
    use eld_common::tx::{Payload, TransferTx, Tx, TxPublicKey, TxSig};

    fn transfer_tx_fixture(signing_key: &SigningKey, nonce: u32) -> Tx {
        let verifying_key = signing_key.verifying_key();
        let sender = Address::from_public_key(&verifying_key).unwrap();
        let recipient =
            Address::parse_hex_str("0x0987654321098765432109876543210987654321").unwrap();
        let inner = TransferTx::new(sender, recipient, 1.into()).unwrap();
        let mut tx = Tx {
            sig: TxSig::empty(),
            nonce: nonce.into(),
            payload: Payload::new(inner),
            public_key: TxPublicKey::from(&verifying_key),
            fee: DEFAULT_TX_FEE.into(),
        };
        tx.sign(signing_key, MOCK_CHAIN_ID).expect("sign");
        tx
    }

    let storage = create_test_storage();
    let secret = SigningKey::from_bytes(&[11u8; 32]);

    let first = transfer_tx_fixture(&secret, 1);
    storage
        .index_transaction(&first, 9, 0, TransactionStatus::Success, Some(21_000), &[])
        .unwrap();
    storage
        .index_transaction(
            &transfer_tx_fixture(&secret, 2),
            10,
            0,
            TransactionStatus::Success,
            None,
            &[],
        )
        .unwrap();

    let stored = storage
        .get_indexed_transaction(&storage.calculate_tx_id(&first))
        .unwrap()
        .unwrap();
    assert_eq!(stored.gas_used, Some(21_000));

    let all = storage
        .list_indexed_transactions_chron_desc(None, 50, false, None, None, None)
        .unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].block_height, 10);
    assert_eq!(all[1].block_height, 9);

    let page_probe = storage
        .list_indexed_transactions_chron_desc(None, 1, true, None, None, None)
        .unwrap();
    assert_eq!(page_probe.len(), 2);

    let page2 = storage
        .list_indexed_transactions_chron_desc(Some((10, 0)), 10, false, None, None, None)
        .unwrap();
    assert_eq!(page2.len(), 1);
    assert_eq!(page2[0].block_height, 9);
}

#[test]
fn test_index_transaction_stores_response_events() {
    use crate::indexer::TransactionStatus;
    use crate::storage::traits::TransactionIndexerStorage;
    use abci::types::{Event, EventAttribute};
    use ed25519_dalek::SigningKey;
    use eld_common::constants::{protocol::DEFAULT_TX_FEE, test::MOCK_CHAIN_ID};
    use eld_common::tx::{Payload, TransferTx, Tx, TxPublicKey, TxSig};

    let storage = create_test_storage();
    let secret = SigningKey::from_bytes(&[13u8; 32]);
    let verifying_key = secret.verifying_key();
    let sender = Address::from_public_key(&verifying_key).unwrap();
    let recipient = Address::parse_hex_str("0x0987654321098765432109876543210987654321").unwrap();
    let inner = TransferTx::new(sender, recipient, 1.into()).unwrap();
    let mut tx = Tx {
        sig: TxSig::empty(),
        nonce: 1.into(),
        payload: Payload::new(inner),
        public_key: TxPublicKey::from(&verifying_key),
        fee: DEFAULT_TX_FEE.into(),
    };
    tx.sign(&secret, MOCK_CHAIN_ID).expect("sign");

    let events = vec![Event {
        r#type: "transfer".to_string(),
        attributes: vec![EventAttribute {
            key: b"amount".to_vec(),
            value: b"1".to_vec(),
            index: true,
        }],
    }];

    storage
        .index_transaction(&tx, 4, 1, TransactionStatus::Success, Some(21_000), &events)
        .unwrap();

    let stored = storage
        .get_indexed_transaction(&storage.calculate_tx_id(&tx))
        .unwrap()
        .unwrap();
    assert_eq!(stored.events.len(), 1);
    assert_eq!(stored.events[0].event_index, 0);
    assert_eq!(stored.events[0].event_type, "transfer");
    assert_eq!(stored.events[0].block_height, 4);
    assert_eq!(stored.events[0].block_index, 1);
    assert_eq!(
        stored.events[0].attributes,
        vec![("amount".to_string(), "1".to_string())]
    );
    assert_eq!(stored.events[0].tx_id, stored.id);
}

#[test]
fn test_verified_proof_reward_index_key_parse() {
    let k = RocksDBStorage::verified_proof_reward_index_key(
        "0x1234567890123456789012345678901234567890",
        5,
        3,
    );
    let key_str = String::from_utf8(k).unwrap();
    let parsed = RocksDBStorage::parse_verified_proof_reward_index_key(&key_str).unwrap();
    assert_eq!(
        parsed.0,
        "0x1234567890123456789012345678901234567890".to_string()
    );
    assert_eq!(parsed.1, 5);
    assert_eq!(parsed.2, 3);
}

#[test]
fn test_aggregate_verified_proof_rewards_by_height_range() {
    use crate::storage::traits::TransactionIndexerStorage;
    const VERIFIED_PROOF_REWARD_BASE_AMOUNT: u128 = 1000;

    let temp_dir = TempDir::new().unwrap();
    let storage = RocksDBStorage::new(temp_dir.path()).unwrap();
    let cf = storage.indexed_transactions_cf().unwrap();
    let addr = "0x1234567890123456789012345678901234567890";
    let norm = addr.to_lowercase();
    for (h, i) in [(5u64, 0u32), (5, 1), (7, 0), (10, 0)] {
        let key = RocksDBStorage::verified_proof_reward_index_key(&norm, h, i);
        storage.db.put_cf(cf, key, []).unwrap();
    }
    let other = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_lowercase();
    storage
        .db
        .put_cf(
            cf,
            RocksDBStorage::verified_proof_reward_index_key(&other, 6, 0),
            [],
        )
        .unwrap();

    let (c, t) = storage
        .aggregate_verified_proof_rewards(addr, 5, 7, VERIFIED_PROOF_REWARD_BASE_AMOUNT)
        .unwrap();
    assert_eq!(c, 3);
    assert_eq!(t, 3 * VERIFIED_PROOF_REWARD_BASE_AMOUNT);
}

#[test]
fn test_global_verified_proof_rewards_zero_when_unset() {
    use crate::storage::traits::TransactionIndexerStorage;

    let storage = create_test_storage();
    let (c, t) = storage.global_verified_proof_rewards(1000).unwrap();
    assert_eq!(c, 0);
    assert_eq!(t, 0);
}

#[test]
fn test_index_verified_proof_success_bumps_global_rollup() {
    use crate::indexer::TransactionStatus;
    use crate::storage::traits::TransactionIndexerStorage;
    use ed25519_dalek::SigningKey;
    use eld_common::constants::test::MOCK_CHAIN_ID;
    const VERIFIED_PROOF_REWARD_BASE_AMOUNT: u128 = 1000;
    use eld_common::tx::{Payload, Tx, TxPublicKey, TxSig, VerifiedProofTx};

    fn verified_proof_tx_fixture(signing_key: &SigningKey, nonce: u32) -> Tx {
        use eld_common::capacity_proof::{ChunkProof, SlotState};

        let verifying_key = signing_key.verifying_key();
        let sender = Address::from_public_key(&verifying_key).unwrap();
        let provider = Address::parse_hex_str(TEST_ADDR_20).unwrap();
        let inner = VerifiedProofTx::new(
            sender,
            provider,
            "challenge-1".to_string(),
            1,
            2,
            1_600_000_000,
            vec![ChunkProof {
                chunk_index: 0,
                chunk_data: vec![1],
                chunk_hash: [2u8; 32],
                merkle_proof: vec![],
                slot_state: SlotState::Proof,
            }],
            1_600_000_000,
            hex::encode([3u8; 32]),
            hex::encode([4u8; 64]),
        )
        .unwrap();
        let mut tx = Tx {
            sig: TxSig::empty(),
            nonce: nonce.into(),
            payload: Payload::new(inner),
            public_key: TxPublicKey::from(&verifying_key),
            fee: 0.into(),
        };
        tx.sign(signing_key, MOCK_CHAIN_ID).expect("sign");
        tx
    }

    let storage = create_test_storage();
    let secret = SigningKey::from_bytes(&[29u8; 32]);

    storage
        .index_transaction(
            &verified_proof_tx_fixture(&secret, 1),
            100,
            0,
            TransactionStatus::Success,
            None,
            &[],
        )
        .unwrap();
    storage
        .index_transaction(
            &verified_proof_tx_fixture(&secret, 2),
            100,
            1,
            TransactionStatus::Success,
            None,
            &[],
        )
        .unwrap();

    let (c, t) = storage.global_verified_proof_rewards(1000).unwrap();
    assert_eq!(c, 2);
    assert_eq!(t, 2 * VERIFIED_PROOF_REWARD_BASE_AMOUNT);
}

#[test]
fn test_index_verified_proof_failed_does_not_bump_global_rollup() {
    use crate::indexer::TransactionStatus;
    use crate::storage::traits::TransactionIndexerStorage;
    use ed25519_dalek::SigningKey;
    use eld_common::constants::test::MOCK_CHAIN_ID;
    use eld_common::tx::{Payload, PayloadInner, Tx, TxPublicKey, TxSig, VerifiedProofTx};

    fn verified_proof_tx_fixture(signing_key: &SigningKey, nonce: u32) -> Tx {
        use eld_common::capacity_proof::{ChunkProof, SlotState};

        let verifying_key = signing_key.verifying_key();
        let sender = Address::from_public_key(&verifying_key).unwrap();
        let provider = Address::parse_hex_str(TEST_ADDR_20).unwrap();
        let inner = VerifiedProofTx::new(
            sender,
            provider,
            "challenge-1".to_string(),
            1,
            2,
            1_600_000_000,
            vec![ChunkProof {
                chunk_index: 0,
                chunk_data: vec![1],
                chunk_hash: [2u8; 32],
                merkle_proof: vec![],
                slot_state: SlotState::Proof,
            }],
            1_600_000_000,
            hex::encode([3u8; 32]),
            hex::encode([4u8; 64]),
        )
        .unwrap();
        let mut tx = Tx {
            sig: TxSig::empty(),
            nonce: nonce.into(),
            payload: Payload::new(inner),
            public_key: TxPublicKey::from(&verifying_key),
            fee: 0.into(),
        };
        tx.sign(signing_key, MOCK_CHAIN_ID).expect("sign");
        tx
    }

    let storage = create_test_storage();
    let secret = SigningKey::from_bytes(&[31u8; 32]);

    storage
        .index_transaction(
            &verified_proof_tx_fixture(&secret, 1),
            200,
            0,
            TransactionStatus::Failed,
            None,
            &[],
        )
        .unwrap();

    let mut counted_failure = verified_proof_tx_fixture(&secret, 2);
    if let PayloadInner::VerifiedProof(vp) = &mut counted_failure.payload.inner {
        vp.failed = true;
    }
    counted_failure
        .sign(&secret, MOCK_CHAIN_ID)
        .expect("re-sign after failed flag");
    storage
        .index_transaction(
            &counted_failure,
            200,
            1,
            TransactionStatus::Success,
            None,
            &[],
        )
        .unwrap();

    let (c, t) = storage.global_verified_proof_rewards(1000).unwrap();
    assert_eq!(c, 0);
    assert_eq!(t, 0);
}
