use super::*;
use crate::abci_interface::snapshot::codec::deserialize_abci_snapshot;
use crate::abci_interface::snapshot::SnapshotManager;
use crate::app_state::app_state_snapshot::AppStateSnapshot;
use crate::storage::hybrid_storage::HybridStorage;
use crate::storage::rocksdb::RocksDBStorage;
use crate::storage::traits::CADOStorage;
use eld_common::account::Account;
use eld_common::address::Address;
use eld_common::cado::{CADOMap, CADOMetadata, CadoBody, CadoPath, CadoPathKey, CadoType};
use eld_common::coin::Coin;
use eld_common::constants::cado::{LATEST, PATH_PREFIX_ACCOUNT};
use eld_common::error::EldError;
use eld_common::namespace::{slug_to_namespace_cadopath, NamespaceRecord};
use eld_common::nonce::Nonce;
use rocksdb::Transaction;

// Simple mock storage for testing
struct MockStorage;

impl CADOStorage for MockStorage {
    fn put_cado_type(&self, _path: CadoPath, _cado_type: CadoBody) -> Result<(), EldError> {
        Ok(())
    }

    fn put_cado_data(
        &self,
        _path: CadoPath,
        _cado_data: Vec<u8>,
        _metadata: CADOMetadata,
    ) -> Result<(), EldError> {
        Ok(())
    }

    fn put_cado_map(&self, _path: CadoPath, _mapping: CADOMap) -> Result<(), EldError> {
        Ok(())
    }

    fn get_cado_by_path(&self, _path: CadoPath) -> Result<Option<CadoBody>, EldError> {
        Ok(None)
    }

    fn get_cado_paths_by_prefix(&self, _prefix: &str) -> Result<Vec<CadoPath>, EldError> {
        Ok(vec![])
    }

    fn get_cados_by_prefix(&self, _prefix: &str) -> Result<Vec<CadoBody>, EldError> {
        Ok(vec![])
    }

    fn list_epoch_records_chron(
        &self,
        _order: crate::storage::traits::EpochRecordListOrder,
        _after_epoch: Option<i64>,
        _fetch_limit: usize,
    ) -> Result<Vec<eld_common::validator::EpochRecord>, EldError> {
        Ok(vec![])
    }

    fn count_epoch_records(&self) -> Result<u64, EldError> {
        Ok(0)
    }

    fn search_cado_hash(&self, _prefix: &str) -> Result<Vec<(String, CadoBody)>, EldError> {
        Ok(vec![])
    }

    fn search_cado_name(&self, _prefix: &str) -> Result<Vec<(String, CadoBody)>, EldError> {
        Ok(vec![])
    }

    fn search_cado_path(&self, _prefix: &str) -> Result<Vec<(String, CadoBody)>, EldError> {
        Ok(vec![])
    }

    fn get_cado_map(&self, _path: CadoPath) -> Result<Option<CADOMap>, EldError> {
        Ok(None)
    }

    fn delete_cado(
        &self,
        _path: CadoPath,
        _owner: &str,
        _signature: &str,
        _public_key: &str,
        _chain_id: &str,
    ) -> Result<(), EldError> {
        Ok(())
    }

    fn system_delete_cado(&self, _path: CadoPath, _owner: &str) -> Result<(), EldError> {
        Ok(())
    }

    fn system_delete_cado_with_tx(
        &self,
        _path: CadoPath,
        _owner: &str,
        _tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        Ok(())
    }

    fn begin_transaction(&self) -> Transaction<'_, rocksdb::TransactionDB> {
        unimplemented!("Mock storage doesn't support transactions")
    }

    fn put_cado_type_with_tx(
        &self,
        _path: CadoPath,
        _cado_type: CadoBody,
        _tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        Ok(())
    }

    fn put_cado_data_with_tx(
        &self,
        _path: CadoPath,
        _cado_data: Vec<u8>,
        _metadata: CADOMetadata,
        _tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        Ok(())
    }

    fn delete_cado_with_tx(
        &self,
        _path: CadoPath,
        _owner: &str,
        _signature: &str,
        _public_key: &str,
        _chain_id: &str,
        _tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        Ok(())
    }

    fn put_cado_map_with_tx(
        &self,
        _path: CadoPath,
        _mapping: CADOMap,
        _tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        Ok(())
    }
}

#[test]
fn has_cado_in_working_set_checks_staged_and_committed_cache() {
    use eld_common::address::Address;
    use eld_common::cado::{CADOMetadata, CadoBody, CadoPath, CadoPathKey, CadoType};

    let address =
        Address::parse_hex_str("0x1234567890123456789012345678901234567890").expect("address");
    let path =
        CadoPath::new(CadoType::Account, CadoPathKey::Address(address)).expect("account path");
    let cado = CadoBody::mutable_new(
        vec![1],
        CADOMetadata::new(CadoType::Account, address.hex_with_prefix()),
    );

    let mut envelope = AppStateEnvelope::default();
    assert!(!envelope.has_cado_in_working_set(&path));

    envelope.update_cado_cache(path.clone(), cado.clone());
    assert!(envelope.has_cado_in_working_set(&path));

    envelope.cado_cache.clear();
    envelope
        .committed_cado_cache
        .insert(path.as_str().as_bytes(), cado);
    assert!(envelope.has_cado_in_working_set(&path));
}

#[test]
fn update_cado_cache_refuses_infrastructure_paths() {
    use crate::app_state::app_state_snapshot::AppStateSnapshot;
    use eld_common::cado::{CADOMetadata, CadoBody, CadoType};

    let mut envelope = AppStateEnvelope::default();
    let path = AppStateSnapshot::latest_path().expect("path");
    let cado = CadoBody::immutable(
        vec![1],
        CADOMetadata::new(CadoType::AppStateSnapshot, "system"),
    );

    envelope.update_cado_cache(path.clone(), cado);

    assert!(
        !envelope.cado_cache.contains_key(path.as_str()),
        "infrastructure CADO must not be staged in cado_cache"
    );
}

#[test]
fn has_persisted_app_state_tip_detects_latest_app_state_tip_in_path_index() {
    let temp_dir = tempfile::TempDir::new().expect("tempdir");
    let storage = RocksDBStorage::new(temp_dir.path()).expect("rocksdb");

    assert!(
        !AppState::has_persisted_app_state_tip(&storage).expect("check empty db"),
        "empty db should have no persisted AppStateTip"
    );

    let app_state_tip = AppStateTip {
        block_height: 42,
        cado_root_hash: [7u8; 32],
        app_hash: [1u8; 32],
    };
    let serialized = bincode::serialize(&app_state_tip).expect("serialize");
    let metadata = CADOMetadata::new(CadoType::AppStateTip, "system");
    let cado = CadoBody::immutable(serialized, metadata);
    let latest_path =
        CadoPath::new(CadoType::AppStateTip, CadoPathKey::Name(LATEST)).expect("path");

    storage
        .put_cado_type(latest_path.clone(), cado)
        .expect("persist AppStateTip");

    assert!(
        storage
            .get_cado_map(latest_path)
            .expect("map lookup")
            .is_none(),
        "AppStateTip commit path does not use cado_map"
    );
    assert!(
        AppState::has_persisted_app_state_tip(&storage).expect("check persisted db"),
        "AppStateTip at LATEST path_index should be detected"
    );
}

#[test]
fn initialize_with_data_restores_committed_cado_objects_from_app_state_snapshot() {
    let temp_dir = tempfile::TempDir::new().expect("tempdir");
    let rocksdb = std::sync::Arc::new(RocksDBStorage::new(temp_dir.path()).expect("rocksdb"));
    let storage = HybridStorage::new(rocksdb.clone());

    let mut original = AppState::default();
    original.envelope.init_empty_trie();
    original.envelope.block_height = 55;
    original.set_app_hash([0xAB; 32]);

    let account_path = CadoPath::parse(&format!(
        "{}{}",
        PATH_PREFIX_ACCOUNT, "0x1234567890123456789012345678901234567890"
    ))
    .expect("account path");
    let account_cado = CadoBody::mutable_new(
        vec![1, 2, 3, 4],
        CADOMetadata::new(
            CadoType::Account,
            "0x1234567890123456789012345678901234567890",
        ),
    );
    let account_hash = account_cado.content_hash();
    original
        .envelope
        .committed_cado_cache
        .insert(account_path.as_str().as_bytes(), account_cado.clone());
    original
        .envelope
        .state_trie
        .insert(account_path.as_str().as_bytes(), &account_hash);
    original.envelope.committed_cado_cache.calculate_hash();

    let app_state_tip = AppStateTip {
        block_height: original.envelope.block_height,
        cado_root_hash: original.envelope.state_trie.root_hash(),
        app_hash: original.app_hash().bytes().expect("app hash"),
    };
    let latest_app_state_tip_path =
        CadoPath::new(CadoType::AppStateTip, CadoPathKey::Name(LATEST)).expect("sv path");
    let app_state_tip_cado = CadoBody::immutable(
        bincode::serialize(&app_state_tip).expect("serialize app state tip"),
        CADOMetadata::new(CadoType::AppStateTip, "system"),
    );
    storage
        .put_cado_type(latest_app_state_tip_path, app_state_tip_cado)
        .expect("persist app state tip");

    let app_state_snapshot = AppStateSnapshot::new(&original).expect("snapshot");
    let app_state_snapshot_path = AppStateSnapshot::latest_path().expect("app state snapshot path");
    storage
        .put_cado_type(
            app_state_snapshot_path,
            app_state_snapshot
                .to_cado()
                .expect("app state snapshot to cado"),
        )
        .expect("persist app state snapshot");

    let mut restored = AppState::default();
    restored
        .initialize_with_data(&storage)
        .expect("initialize with persisted data");

    assert_eq!(restored, original);
    let restored_account = restored
        .envelope
        .committed_cado_cache
        .get(account_path.as_str().as_bytes())
        .expect("restored committed CADO missing");
    assert_eq!(restored_account.content_hash(), account_hash);
}

#[test]
fn initialize_with_data_strips_infrastructure_from_app_state_snapshot() {
    use crate::app_state::app_state_snapshot::AppStateSnapshot;

    let temp_dir = tempfile::TempDir::new().expect("tempdir");
    let rocksdb = std::sync::Arc::new(RocksDBStorage::new(temp_dir.path()).expect("rocksdb"));
    let storage = HybridStorage::new(rocksdb.clone());

    let mut original = AppState::default();
    original.envelope.init_empty_trie();
    original.envelope.block_height = 60;
    original.set_app_hash([0xAB; 32]);

    let account_path = CadoPath::parse(&format!(
        "{}{}",
        PATH_PREFIX_ACCOUNT, "0x1234567890123456789012345678901234567890"
    ))
    .expect("account path");
    let account_cado = CadoBody::mutable_new(
        vec![1, 2, 3],
        CADOMetadata::new(
            CadoType::Account,
            "0x1234567890123456789012345678901234567890",
        ),
    );
    let account_hash = account_cado.content_hash();
    original
        .envelope
        .committed_cado_cache
        .insert(account_path.as_str().as_bytes(), account_cado);
    original
        .envelope
        .state_trie
        .insert(account_path.as_str().as_bytes(), &account_hash);

    let app_state_tip = AppStateTip {
        block_height: original.envelope.block_height,
        cado_root_hash: original.envelope.state_trie.root_hash(),
        app_hash: original.app_hash().bytes().expect("app hash"),
    };
    let latest_app_state_tip_path =
        CadoPath::new(CadoType::AppStateTip, CadoPathKey::Name(LATEST)).expect("sv path");
    storage
        .put_cado_type(
            latest_app_state_tip_path,
            CadoBody::immutable(
                bincode::serialize(&app_state_tip).expect("serialize app state tip"),
                CADOMetadata::new(CadoType::AppStateTip, "system"),
            ),
        )
        .expect("persist app state tip");

    let mut app_state_snapshot = AppStateSnapshot::new(&original).expect("snapshot");
    let infra_path = AppStateSnapshot::latest_path().expect("infra path");
    app_state_snapshot.committed_cado_cache.insert(
        infra_path.as_str().as_bytes(),
        CadoBody::immutable(
            vec![9, 9, 9],
            CADOMetadata::new(CadoType::AppStateSnapshot, "system"),
        ),
    );
    storage
        .put_cado_type(
            AppStateSnapshot::latest_path().expect("app state path"),
            app_state_snapshot
                .to_cado()
                .expect("app state snapshot cado"),
        )
        .expect("persist polluted app state snapshot");

    let mut restored = AppState::default();
    restored
        .initialize_with_data(&storage)
        .expect("initialize with polluted trie snapshot");

    assert_eq!(restored.envelope.committed_cado_cache.len(), 1);
    assert!(restored
        .envelope
        .committed_cado_cache
        .get(account_path.as_str().as_bytes())
        .is_some());
    assert!(restored
        .envelope
        .committed_cado_cache
        .get(infra_path.as_str().as_bytes())
        .is_none());
}

#[tokio::test]
async fn app_state_snapshot_roundtrip_via_abci_payload_restores_original_state() {
    let src_dir = tempfile::TempDir::new().expect("src tempdir");
    let src_rocksdb = std::sync::Arc::new(RocksDBStorage::new(src_dir.path()).expect("rocksdb"));
    let src_storage = HybridStorage::new(src_rocksdb.clone());

    let mut original = AppState::default();
    original.envelope.init_empty_trie();
    original.envelope.block_height = 77;
    original.set_app_hash([0xCD; 32]);

    let account_path = CadoPath::parse(&format!(
        "{}{}",
        PATH_PREFIX_ACCOUNT, "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    ))
    .expect("account path");
    let account_cado = CadoBody::mutable_new(
        vec![9, 8, 7, 6, 5],
        CADOMetadata::new(
            CadoType::Account,
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ),
    );
    let account_hash = account_cado.content_hash();
    original
        .envelope
        .committed_cado_cache
        .insert(account_path.as_str().as_bytes(), account_cado);
    original
        .envelope
        .state_trie
        .insert(account_path.as_str().as_bytes(), &account_hash);
    original.envelope.committed_cado_cache.calculate_hash();

    let app_state_tip = AppStateTip {
        block_height: original.envelope.block_height,
        cado_root_hash: original.envelope.state_trie.root_hash(),
        app_hash: original.app_hash().bytes().expect("app hash"),
    };
    let latest_app_state_tip_path =
        CadoPath::new(CadoType::AppStateTip, CadoPathKey::Name(LATEST)).expect("sv path");
    let app_state_tip_cado = CadoBody::immutable(
        bincode::serialize(&app_state_tip).expect("serialize app state tip"),
        CADOMetadata::new(CadoType::AppStateTip, "system"),
    );
    src_storage
        .put_cado_type(latest_app_state_tip_path, app_state_tip_cado)
        .expect("persist app state tip");

    let app_state_snapshot = AppStateSnapshot::new(&original).expect("snapshot");
    let app_state_snapshot_path = AppStateSnapshot::latest_path().expect("app state snapshot path");
    src_storage
        .put_cado_type(
            app_state_snapshot_path,
            app_state_snapshot
                .to_cado()
                .expect("app state snapshot to cado"),
        )
        .expect("persist app state snapshot");

    let manager = SnapshotManager::new(src_rocksdb.clone());
    manager
        .create_snapshot_from_latest_state(original.envelope.block_height)
        .await
        .expect("create snapshot from canonical state");
    let payload = manager
        .get_snapshot(original.envelope.block_height)
        .await
        .expect("read created snapshot")
        .expect("snapshot payload missing");

    let decoded = deserialize_abci_snapshot(&payload).expect("decode abci snapshot payload");

    let restore_dir = tempfile::TempDir::new().expect("restore tempdir");
    let restore_rocksdb =
        std::sync::Arc::new(RocksDBStorage::new(restore_dir.path()).expect("restore rocksdb"));
    let restore_storage = HybridStorage::new(restore_rocksdb.clone());

    let restore_app_state_tip_path =
        CadoPath::new(CadoType::AppStateTip, CadoPathKey::Name(LATEST)).expect("tip path");
    let restored_app_state_tip = AppStateTip {
        block_height: decoded.app_state_snapshot.block_height,
        cado_root_hash: decoded.app_state_snapshot.state_trie_root,
        app_hash: decoded.app_state_snapshot.app_hash,
    };
    let restore_app_state_tip_cado = CadoBody::immutable(
        bincode::serialize(&restored_app_state_tip).expect("serialize restored app state tip"),
        CADOMetadata::new(CadoType::AppStateTip, "system"),
    );
    restore_storage
        .put_cado_type(restore_app_state_tip_path, restore_app_state_tip_cado)
        .expect("persist restored app state tip");

    restore_storage
        .put_cado_type(
            AppStateSnapshot::latest_path().expect("app state path"),
            decoded
                .app_state_snapshot
                .to_cado()
                .expect("restored app state snapshot to cado"),
        )
        .expect("persist restored app state snapshot");

    let mut restored = AppState::default();
    restored
        .initialize_with_data(&restore_storage)
        .expect("initialize restored app state");

    assert_eq!(restored, original);
}

#[test]
fn test_get_cado_instance_invalid_data() {
    let mut state = AppState::default();
    let storage = MockStorage;
    let path = CadoPath::parse(&format!(
        "{}{}",
        PATH_PREFIX_ACCOUNT, "0x1234567890123456789012345678901234567890"
    ))
    .unwrap();
    state.envelope.cado_cache.insert(
        path.as_str().to_string(),
        CadoBody::mutable_new(
            vec![0xFF, 0xFF], // Invalid data
            CADOMetadata::new(
                CadoType::Account,
                "0x1234567890123456789012345678901234567890",
            ),
        ),
    );
    let result = state.envelope.get_cado_instance::<i32>(&storage, &path);
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

#[test]
fn get_account_from_cado_prefers_committed_cache_before_db() {
    let temp_dir = tempfile::TempDir::new().expect("tempdir");
    let storage = RocksDBStorage::new(temp_dir.path()).expect("rocksdb");
    let mut envelope = AppStateEnvelope::default();
    let address =
        Address::parse_hex_str("0x1234567890123456789012345678901234567890").expect("address");
    let path =
        CadoPath::new(CadoType::Account, CadoPathKey::Address(address)).expect("account path");

    let db_account = Account::new(
        address,
        Coin::new(1).expect("coin"),
        Nonce::new(Nonce::ZERO),
    );
    let db_cado = CadoBody::mutable_new(
        bincode::serialize(&db_account).expect("serialize db account"),
        CADOMetadata::new(CadoType::Account, address.hex_with_prefix()),
    );
    storage
        .put_cado_type(path.clone(), db_cado)
        .expect("put db account");

    let committed_account = Account::new(
        address,
        Coin::new(2).expect("coin"),
        Nonce::new(Nonce::ZERO),
    );
    let committed_cado = CadoBody::mutable_new(
        bincode::serialize(&committed_account).expect("serialize committed account"),
        CADOMetadata::new(CadoType::Account, address.hex_with_prefix()),
    );
    envelope
        .committed_cado_cache
        .insert(path.as_str().as_bytes(), committed_cado);

    let got = envelope
        .get_account_from_cado(&storage, &path)
        .expect("account from lookup");
    assert_eq!(got.account.balance(), committed_account.balance());
}

#[test]
fn get_cado_instance_falls_back_to_db_when_missing_in_caches() {
    let temp_dir = tempfile::TempDir::new().expect("tempdir");
    let storage = RocksDBStorage::new(temp_dir.path()).expect("rocksdb");
    let mut envelope = AppStateEnvelope::default();
    let address =
        Address::parse_hex_str("0x1234567890123456789012345678901234567890").expect("address");
    let path =
        CadoPath::new(CadoType::Account, CadoPathKey::Address(address)).expect("account path");

    let db_account = Account::new(
        address,
        Coin::new(3).expect("coin"),
        Nonce::new(Nonce::ZERO),
    );
    let db_cado = CadoBody::mutable_new(
        bincode::serialize(&db_account).expect("serialize db account"),
        CADOMetadata::new(CadoType::Account, address.hex_with_prefix()),
    );
    storage
        .put_cado_type(path.clone(), db_cado)
        .expect("put db account");

    let got = envelope
        .get_cado_instance::<Account>(&storage, &path)
        .expect("lookup result")
        .expect("account exists");
    assert_eq!(got.instance.balance(), db_account.balance());
}

#[test]
fn validate_add_namespace_layer_a_same_block() {
    let mut envelope = AppStateEnvelope::default();
    let owner =
        Address::parse_hex_str("0xe17404c417fa10cc04fdf73604fcacca8d0a687c").expect("address");
    envelope.namespace_registry_cache.insert(
        "peter".to_string(),
        NamespaceRecord {
            namespace_slug: "peter".to_string(),
            owner,
            registered_height: 1,
        },
    );
    assert!(envelope.validate_add_namespace_not_taken("peter").is_err());
}

#[test]
fn validate_add_namespace_layer_b_committed_cache() {
    let mut envelope = AppStateEnvelope::default();
    let path = slug_to_namespace_cadopath("peter").expect("path");
    envelope.committed_cado_cache.insert(
        path.as_str().as_bytes(),
        CadoBody::mutable_new(
            vec![1, 2, 3],
            CADOMetadata::new(CadoType::Namespace, "peter"),
        ),
    );
    assert!(envelope.validate_add_namespace_not_taken("peter").is_err());
}

#[test]
fn resolve_namespace_prefers_block_cache() {
    let mut envelope = AppStateEnvelope::default();
    let owner =
        Address::parse_hex_str("0xe17404c417fa10cc04fdf73604fcacca8d0a687c").expect("address");
    let staged = NamespaceRecord {
        namespace_slug: "peter".to_string(),
        owner,
        registered_height: 99,
    };
    envelope
        .namespace_registry_cache
        .insert("peter".to_string(), staged.clone());
    let resolved = envelope.resolve_namespace("peter").expect("resolve");
    assert_eq!(resolved, Some(staged));
}

#[test]
fn insert_epoch_records_index_from_path_inserts_valid_epoch_paths_only() {
    use eld_common::cado::epoch_record_path_name;
    use eld_common::constants::cado::LATEST;

    let mut envelope = AppStateEnvelope::default();

    let epoch_key = epoch_record_path_name(4).expect("name");
    let epoch_path =
        CadoPath::new(CadoType::EpochRecord, CadoPathKey::Name(&epoch_key)).expect("path");
    envelope.insert_epoch_records_index_from_path(epoch_path.as_str());
    assert_eq!(
        envelope
            .epoch_records_index
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![4]
    );

    let latest_path =
        CadoPath::new(CadoType::EpochRecord, CadoPathKey::Name(LATEST)).expect("latest");
    envelope.insert_epoch_records_index_from_path(latest_path.as_str());
    assert_eq!(
        envelope
            .epoch_records_index
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![4]
    );

    let account_path = CadoPath::parse(&format!(
        "{}{}",
        PATH_PREFIX_ACCOUNT, "0x1234567890123456789012345678901234567890"
    ))
    .expect("account path");
    envelope.insert_epoch_records_index_from_path(account_path.as_str());
    assert_eq!(
        envelope
            .epoch_records_index
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![4]
    );
}

#[test]
fn rebuild_epoch_records_index_populates_from_committed_cache() {
    let mut envelope = AppStateEnvelope::default();
    insert_test_epoch_cado(&mut envelope, 1);
    insert_test_epoch_cado(&mut envelope, 3);
    insert_test_epoch_cado(&mut envelope, 7);
    envelope.epoch_records_index.insert(99);

    envelope.rebuild_epoch_records_index();

    assert_eq!(
        envelope
            .epoch_records_index
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![1, 3, 7]
    );
}

#[test]
fn collect_epoch_numbers_scans_cache_when_index_empty() {
    let mut committed = AppStateEnvelope::default();
    insert_test_epoch_cado(&mut committed, 0);
    insert_test_epoch_cado(&mut committed, 2);

    let epochs = committed.collect_epoch_numbers(None);
    assert_eq!(epochs.into_iter().collect::<Vec<_>>(), vec![0, 2]);
}

#[test]
fn collect_epoch_numbers_uses_index_when_populated() {
    let mut committed = AppStateEnvelope::default();
    insert_test_epoch_cado(&mut committed, 5);
    committed.epoch_records_index.insert(0);
    committed.epoch_records_index.insert(2);

    let epochs = committed.collect_epoch_numbers(None);
    assert_eq!(epochs.into_iter().collect::<Vec<_>>(), vec![0, 2]);
}

#[test]
fn epoch_records_index_collects_committed_and_staged_epochs() {
    use eld_common::cado::epoch_record_path_name;
    use eld_common::constants::cado::LATEST;

    let mut committed = AppStateEnvelope::default();
    for epoch in [0i64, 2] {
        insert_test_epoch_cado(&mut committed, epoch);
        let key = epoch_record_path_name(epoch).expect("name");
        let path = CadoPath::new(CadoType::EpochRecord, CadoPathKey::Name(&key)).expect("path");
        committed.insert_epoch_records_index_from_path(path.as_str());
    }

    let mut current = AppStateEnvelope::default();
    let key = epoch_record_path_name(5).expect("name");
    let path = CadoPath::new(CadoType::EpochRecord, CadoPathKey::Name(&key)).expect("path");
    current.cado_cache.insert(
        path.as_str().to_string(),
        CadoBody::immutable(vec![], CADOMetadata::new(CadoType::EpochRecord, &key)),
    );
    let latest_path =
        CadoPath::new(CadoType::EpochRecord, CadoPathKey::Name(LATEST)).expect("latest");
    current.cado_cache.insert(
        latest_path.as_str().to_string(),
        CadoBody::immutable(vec![], CADOMetadata::new(CadoType::EpochRecord, LATEST)),
    );

    let epochs = committed.collect_epoch_numbers(Some(&current));
    assert_eq!(epochs.into_iter().collect::<Vec<_>>(), vec![0, 2, 5]);
}

fn insert_test_epoch_cado(envelope: &mut AppStateEnvelope, epoch: i64) {
    use eld_common::cado::epoch_record_path_name;

    let key = epoch_record_path_name(epoch).expect("name");
    let path = CadoPath::new(CadoType::EpochRecord, CadoPathKey::Name(&key)).expect("path");
    envelope.committed_cado_cache.insert(
        path.as_str().as_bytes(),
        CadoBody::immutable(vec![], CADOMetadata::new(CadoType::EpochRecord, &key)),
    );
}

fn insert_test_namespace_cado(envelope: &mut AppStateEnvelope, record: &NamespaceRecord) {
    let path = slug_to_namespace_cadopath(&record.namespace_slug).expect("namespace path");
    let bytes = record.serialize_bin().expect("serialize");
    envelope.committed_cado_cache.insert(
        path.as_str().as_bytes(),
        CadoBody::immutable(
            bytes,
            CADOMetadata::new(CadoType::Namespace, &record.namespace_slug),
        ),
    );
}

#[test]
fn rebuild_namespace_registry_index_populates_from_committed_cache() {
    let owner =
        Address::parse_hex_str("0xe17404c417fa10cc04fdf73604fcacca8d0a687c").expect("address");
    let alpha = NamespaceRecord {
        namespace_slug: "alpha".to_string(),
        owner,
        registered_height: 10,
    };
    let beta = NamespaceRecord {
        namespace_slug: "beta".to_string(),
        owner,
        registered_height: 20,
    };

    let mut envelope = AppStateEnvelope::default();
    insert_test_namespace_cado(&mut envelope, &alpha);
    insert_test_namespace_cado(&mut envelope, &beta);
    envelope
        .namespace_registry_index
        .insert("stale".to_string(), alpha.clone());

    envelope
        .rebuild_namespace_registry_index()
        .expect("rebuild namespace index");

    assert_eq!(envelope.namespace_registry_index.len(), 2);
    assert_eq!(
        envelope
            .namespace_registry_index
            .get("alpha")
            .map(|record| record.registered_height),
        Some(10)
    );
    assert_eq!(
        envelope
            .namespace_registry_index
            .get("beta")
            .map(|record| record.registered_height),
        Some(20)
    );
}

#[test]
fn collect_namespace_records_scans_cache_when_index_empty() {
    let owner =
        Address::parse_hex_str("0xe17404c417fa10cc04fdf73604fcacca8d0a687c").expect("address");
    let record = NamespaceRecord {
        namespace_slug: "peter".to_string(),
        owner,
        registered_height: 42,
    };

    let mut envelope = AppStateEnvelope::default();
    insert_test_namespace_cado(&mut envelope, &record);

    let records = envelope.collect_namespace_records(None).expect("collect");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].namespace_slug, "peter");
}
