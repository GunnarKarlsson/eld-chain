mod abci_interface;
mod api;
mod app_state;
mod broadcast_log;
mod capacity;
pub mod config;
mod content;
pub mod errors;
mod indexer;
mod node_identity;
mod p2p_keypair;
mod process_logging;
mod runtime;
mod storage;
mod sys_disk;
mod wallet;

use crate::errors::handle_fatal_eld_error;
use crate::process_logging::init_default_logging;
use clap::Parser;
use tracing::info;

#[derive(Parser, Debug)]
#[command(
    name = "eld-node",
    author,
    version,
    about = "Eld ABCI application, REST API, and libp2p content sync"
)]
pub(crate) struct Args {
    /// Chain Id
    #[arg()]
    chainid: Option<String>,
    /// Initialize data from database
    #[arg(long)]
    init_data: bool,
    /// Path to consensus configuration file
    #[arg(long)]
    config_path: Option<String>,
    /// Path to database directory
    #[arg(long)]
    db_path: Option<String>,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    init_default_logging().unwrap_or_else(|e| handle_fatal_eld_error(e));
    info!("Eld node starting");
    if let Err(e) = runtime::run(args).await {
        handle_fatal_eld_error(e);
    }
}

#[cfg(test)]
mod tests {
    use crate::storage::rocksdb::RocksDBStorage;
    use std::path::PathBuf;

    #[test]
    fn test_invalid_db_path() {
        let invalid_path = PathBuf::from("/invalid/rocksdb");
        let rocksdb_result = RocksDBStorage::new(&invalid_path);
        assert!(rocksdb_result.is_err());
    }
}
