mod behaviour;
mod challenges;
mod commands;
mod config;
mod coordinator;
mod inventory;
mod messages;
mod mock;
mod pinboard;
mod storage;
mod swarm;
mod trait_impl;

pub use eld_common::sync_msg::SyncMsg;

// Public surface of the former `sync_coordinator.rs` file (not all names are used in-crate).
#[allow(unused_imports)]
pub use behaviour::EldBehaviour;
pub use config::P2pConfig;
pub use coordinator::P2pSyncCoordinator;
pub use mock::MockP2pSyncCoordinator;
#[allow(unused_imports)]
pub use storage::SyncCoordinatorStorage;
pub use trait_impl::P2pCoordinatorTrait;
