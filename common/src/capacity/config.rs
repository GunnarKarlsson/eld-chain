use crate::address::Address;
use std::path::PathBuf;

/// Configuration for capacity proof
#[derive(Debug, Clone)]
pub struct CapacityConfig {
    pub capacity_dir: PathBuf,
    pub max_capacity_gb: u64,
    pub provider_id: Address,
    pub auto_register: bool,
    pub registration_retry_interval_secs: u64,
    pub tendermint_rpc_url: String,
}
