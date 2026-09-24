use crate::capacity::capacity_registration::CapacityRegistrationService;
use eld_common::address::Address;
use eld_common::capacity::{CapacityConfig, CapacityProofMerkleTree, SlotAllocator};
use eld_common::error::EldError;
use eld_common::protocol_constants::ProtocolHandle;
use eld_common::{CapacityMerkleRoot, ChallengeId};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

mod allocation;
mod chain_txs;
mod hashing;
mod merkle;
mod proofs;
mod slot_io;

#[cfg(test)]
mod tests;

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
    pub(super) config: CapacityConfig,
    /// Wallet name from `ELD_CAPACITY_VALIDATOR_WALLET_NAME` — local slots/`provider_id`
    /// and on-chain `RegisterCapacity` / capacity-validator identity.
    pub(super) capacity_validator_wallet_name: String,
    pub(super) slot_allocator: Arc<Mutex<SlotAllocator>>,
    pub(super) registration_service: Arc<CapacityRegistrationService>,
    pub(super) merkle_tree: Arc<Mutex<Option<CapacityProofMerkleTree>>>,
    pub(super) initialized: Arc<Mutex<bool>>,
    pub(super) cli: Arc<eld_client::facade::ChainClient>,
    pub(super) consensus_config: Arc<std::sync::Mutex<crate::config::ConsensusConfig>>,
    pub(super) protocol: ProtocolHandle,
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
        protocol: ProtocolHandle,
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
            protocol,
        }
    }

    pub(crate) fn protocol_handle(&self) -> ProtocolHandle {
        self.protocol.clone()
    }

    pub(super) fn protocol_fee_config(&self) -> Result<eld_common::fee::FeeConfig, EldError> {
        self.protocol
            .get()
            .map(|constants| constants.fee_config())
            .ok_or_else(|| EldError::InitializationError {
                component: "protocol_constants".into(),
                details: "protocol constants are not loaded".into(),
            })
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

            // Drop the allocator lock before the build. Content leaves read the
            // capacity file through the same allocator.
            let loaded_slot_map = slot_allocator.get_slot_map().cloned();
            drop(slot_allocator);

            if let Some(slot_map) = loaded_slot_map {
                let merkle_tree = self.build_merkle_tree(&slot_map).await?;
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
}
