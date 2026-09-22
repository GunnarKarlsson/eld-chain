use crate::storage::traits::{CADOStorage, EpochRecordListOrder};
use eld_common::cado::{CADOMap, CADOMetadata, CadoBody, CadoPath};
use eld_common::error::EldError;
use eld_common::validator::EpochRecord;
use rocksdb::Transaction;

use super::RocksDBStorage;

mod delete;
mod epochs;
mod get;
mod map;
mod path;
mod prefix;
mod put;

impl CADOStorage for RocksDBStorage {
    fn put_cado_type(&self, path: CadoPath, cado_type: CadoBody) -> Result<(), EldError> {
        self.put_cado_type(path, cado_type)
    }
    fn put_cado_data(
        &self,
        path: CadoPath,
        cado_data: Vec<u8>,
        metadata: CADOMetadata,
    ) -> Result<(), EldError> {
        self.put_cado_data(path, cado_data, metadata)
    }
    fn put_cado_map(&self, path: CadoPath, mapping: CADOMap) -> Result<(), EldError> {
        self.put_cado_map(path, mapping)
    }
    fn get_cado_by_path(&self, path: CadoPath) -> Result<Option<CadoBody>, EldError> {
        self.get_cado_by_path(path)
    }
    fn get_cado_paths_by_prefix(&self, prefix: &str) -> Result<Vec<CadoPath>, EldError> {
        self.get_cado_paths_by_prefix(prefix)
    }
    fn get_cados_by_prefix(&self, prefix: &str) -> Result<Vec<CadoBody>, EldError> {
        self.get_cados_by_prefix(prefix)
    }

    fn list_epoch_records_chron(
        &self,
        order: EpochRecordListOrder,
        after_epoch: Option<i64>,
        fetch_limit: usize,
    ) -> Result<Vec<EpochRecord>, EldError> {
        self.list_epoch_records_chron(order, after_epoch, fetch_limit)
    }

    fn count_epoch_records(&self) -> Result<u64, EldError> {
        self.count_epoch_records()
    }

    fn search_cado_hash(&self, prefix: &str) -> Result<Vec<(String, CadoBody)>, EldError> {
        self.search_cado_hash(prefix)
    }
    fn search_cado_name(&self, prefix: &str) -> Result<Vec<(String, CadoBody)>, EldError> {
        self.search_cado_name(prefix)
    }
    fn search_cado_path(&self, prefix: &str) -> Result<Vec<(String, CadoBody)>, EldError> {
        self.search_cado_path(prefix)
    }
    fn get_cado_map(&self, path: CadoPath) -> Result<Option<CADOMap>, EldError> {
        self.get_cado_map(path)
    }
    fn delete_cado(
        &self,
        path: CadoPath,
        owner: &str,
        signature: &str,
        public_key: &str,
        chain_id: &str,
    ) -> Result<(), EldError> {
        self.delete_cado(path, owner, signature, public_key, chain_id)
    }

    fn system_delete_cado(&self, path: CadoPath, owner: &str) -> Result<(), EldError> {
        self.system_delete_cado(path, owner)
    }

    fn system_delete_cado_with_tx(
        &self,
        path: CadoPath,
        owner: &str,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        self.system_delete_cado_with_tx(path, owner, tx)
    }
    // Transaction-aware methods
    fn begin_transaction(&self) -> Transaction<'_, rocksdb::TransactionDB> {
        self.begin_transaction()
    }
    fn put_cado_type_with_tx(
        &self,
        path: CadoPath,
        cado_type: CadoBody,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        self.put_cadotype_by_path_with_tx(path, cado_type, tx)
    }
    fn put_cado_data_with_tx(
        &self,
        path: CadoPath,
        cado_data: Vec<u8>,
        metadata: CADOMetadata,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        self.put_cado_data_with_tx(path, cado_data, metadata, tx)
    }
    fn delete_cado_with_tx(
        &self,
        path: CadoPath,
        owner: &str,
        signature: &str,
        public_key: &str,
        chain_id: &str,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        self.delete_cado_with_tx(path, owner, signature, public_key, chain_id, tx)
    }
    fn put_cado_map_with_tx(
        &self,
        path: CadoPath,
        mapping: CADOMap,
        tx: &Transaction<'_, rocksdb::TransactionDB>,
    ) -> Result<(), EldError> {
        self.put_cado_map_with_tx(path, mapping, tx)
    }
}
