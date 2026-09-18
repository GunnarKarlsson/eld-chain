/// Native token denomination limits (fixed-point integer representation).
pub mod token {
    pub const MAX_COIN_DECIMALS: u128 = 1_000_000;
    pub const MAX_COIN_UNITS: u128 = 1_000_000_000;
    pub const MAX_COIN: u128 = MAX_COIN_DECIMALS * MAX_COIN_UNITS;
}

/// Consensus / staking / capacity protocol parameters (epoch geometry and limits).
pub mod protocol {
    pub const MIN_STAKE_AMOUNT: u128 = 5; //TODO: Should be much larger for production
    pub const VALIDATORS_PER_EPOCH: usize = 4; // N = 4 validators per epoch
    pub const BLOCKS_PER_EPOCH: i64 = 20; // M = 20 blocks per epoch
    pub const ACTIVE_STORAGE_VALIDATOR_PER_EPOCH: usize = 1; // Only one storage validator per epoch
    pub const CHALLENGES_PER_EPOCH: usize = 5; // N randomly selected providers to challenge per epoch
    pub const CHUNKS_PER_CHALLENGE: usize = 10; // Number of chunks to challenge per provider

    /// Default capacity registration duration in blocks (eventual consistency lease).
    pub const DEFAULT_REGISTRATION_DURATION_BLOCKS: u64 = 1000;

    pub const DEFAULT_TX_FEE: u128 = 5000;

    /// Native units minted to the validator set each block (local-dev).
    pub const BLOCK_REWARD: u128 = 10;

    /// Native units minted to `capacity_provider` on each successful `VerifiedProof`.
    /// Must stay in sync with consensus minting in `process_verified_proof_tx`.
    pub const VERIFIED_PROOF_REWARD_BASE_AMOUNT: u128 = 1000;
}

/// Pinboard parameters (post TTL, upload chunking).
pub mod pinboard {
    /// Default pinboard post TTL in blocks (relative to the block height where the post is committed).
    pub const DEFAULT_PINBOARD_POST_TTL_BLOCKS: u64 = 1000;

    /// Maximum chunk size for content uploads (1KB)
    pub const MAX_CHUNK_SIZE: usize = 1024;
}

/// Libp2p / GossipSub topic strings (capacity challenges, proofs, content sync).
pub mod p2p {
    /// P2P topic prefix for capacity provider challenge topics
    /// Format: "{ELD_STORAGE_CHALLENGE_TOPIC_PREFIX}{provider_id}"
    pub const ELD_STORAGE_CHALLENGE_TOPIC_PREFIX: &str = "eld-storage-challenge-topic-";

    /// P2P topic prefix for capacity provider proof response topics
    /// Format: "{ELD_STORAGE_PROOF_TOPIC_PREFIX}{provider_id}"
    pub const ELD_STORAGE_PROOF_TOPIC_PREFIX: &str = "eld-storage-proof-topic-";

    /// GossipSub topic for content sync between nodes (announce, inventory, heartbeats).
    pub const P2P_TOPIC_CONTENT_SYNC: &str = "eld-content-sync";

    /// GossipSub topic for content-sync responses from capacity providers.
    pub const P2P_TOPIC_CONTENT_SYNC_RESPONSE: &str = "eld-content-sync-response";

    /// Maximum GossipSub RPC / transmit size in bytes (must fit largest serialized
    /// [`crate::sync_msg::SyncMsg`], e.g. pinboard `ContentResponse` with base64 payload).
    /// All network participants should use a compatible value (same or higher).
    pub const GOSSIPSUB_MAX_TRANSMIT_SIZE_BYTES: usize = 1024 * 1024;
}

pub mod test {
    // Chain ID for transaction signing/verification in tests
    pub const MOCK_CHAIN_ID: &str = "eld-testnet-tempelhof";
}

/// Transaction type constants
pub mod tx_type {
    pub const TX_TYPE_TRANSFER: &str = "Transfer";
    pub const TX_TYPE_UNSTAKE: &str = "Unstake";
    pub const TX_TYPE_STAKE: &str = "Stake";
    pub const TX_TYPE_VERIFIED_PROOF: &str = "VerifiedProof";
    pub const TX_TYPE_REGISTER_CAPACITY: &str = "RegisterCapacity";
    pub const TX_TYPE_UNREGISTER_CAPACITY: &str = "UnregisterCapacity";
    pub const TX_TYPE_UPDATE_CAPACITY_MERKLE_ROOT: &str = "UpdateCapacityMerkleRoot";
    pub const TX_TYPE_POST_MESSAGE: &str = "PostMessage";
    pub const TX_TYPE_ADD_NAMESPACE: &str = "AddNamespace";
}

/// CometBFT / ABCI `codespace` string returned on application errors.
pub const ELD_CODESPACE: &str = "eld";

/// Constants for ABCI query paths
pub mod abci_query {
    pub const CADO_LIST: &str = "cado_list";
    pub const CADO: &str = "cado";
    pub const STAKING_ACCOUNT: &str = "staking_account";
    pub const ACTIVE_VALIDATORS: &str = "active_validators";
    pub const ALL_VALIDATORS: &str = "all_validators";
    /// ABCI query path for registered capacity validators.
    pub const ACTIVE_CAPACITY_VALIDATORS: &str = "capacity_validators";
    /// Query path for a single capacity provider by address. Request data = address (UTF-8).
    pub const CAPACITY_PROVIDER: &str = "capacity_provider";
    pub const EPOCH_INFO: &str = "epoch_info";
    pub const ACCOUNT_COUNT: &str = "account_count";
    pub const METADATA_COUNT: &str = "metadata_count";
    pub const ACCOUNT_VIEW: &str = "account_view";
    pub const SINGLE_KEY: &str = "single_key";
    /// Pinboard read queries. Request data = UTF-8 path under ELD root + [`PINBOARD`].
    pub const PINBOARD: &str = "pinboard";
    pub const PINBOARD_SEGMENT_POST: &str = "post";
    pub const PINBOARD_SEGMENT_WALLET: &str = "wallet";
    pub const PINBOARD_SEGMENT_TAG: &str = "tag";
    pub const PINBOARD_SEGMENT_GC_METRICS: &str = "gc_metrics";
    /// Global pinboard feed query (all posts, paginated). Request data = UTF-8 JSON.
    pub const PINBOARD_FEED: &str = "pinboard_feed";
    pub const OTHER: &str = "other";
    pub const ESTIMATE_FEE: &str = "estimate_fee";
    pub const CADO_PATHS: &str = "cado_paths";
}

/// Constants for CADO path validation and security
pub mod cado {
    // Pre-computed sha256("latest")
    pub const LATEST: &str = "0x5e1e2bcac305958b27077ca136f35f0abae7cf38c9af678f7d220ed0cb51d4f8";

    /// CADO scope constants
    pub const SCOPE_ELD_ROOT: &str = "@eld";
    pub const SCOPE_USER: &str = "@user";
    pub const SCOPE_PUBLIC: &str = "@public";
    pub const SCOPE_CONTRACT: &str = "@contract";
    pub const SCOPE_TEST: &str = "@test";
    pub const SCOPE_OTHER: &str = "@other";
    pub const SCOPE_ELD: &str = "@eld";

    /// CADO type constants
    pub const TYPE_ACCOUNT: &str = "account";
    pub const TYPE_STAKING_ACCOUNT: &str = "staking_account";
    pub const TYPE_CONTRACT_INFO: &str = "contract_info";
    pub const TYPE_CONTRACT_STATE: &str = "contract_state";
    pub const TYPE_CADO_MAP: &str = "cado_map";
    pub const TYPE_APP_STATE_TIP: &str = "app_state_tip";
    pub const TYPE_EPOCH_RECORD: &str = "epoch_record";
    pub const TYPE_NAMESPACE: &str = "namespace";
    pub const TYPE_SNAPSHOT_METADATA: &str = "snapshot_metadata";
    pub const TYPE_SNAPSHOT_CHUNK: &str = "snapshot_chunk";
    pub const TYPE_CHUNK_REFERENCE: &str = "chunk_reference";
    pub const TYPE_APP_STATE_SNAPSHOT: &str = "app_state_snapshot";
    pub const TYPE_STORAGE_STAKING_ACCOUNT: &str = "storage_staking_account";
    /// Content manifest segment (not a [`crate::cado::CadoType`] wire path today).
    pub const TYPE_CONTENT_MANIFEST: &str = "content_manifest";
    /// Account content index (client / display).
    pub const TYPE_ACCOUNT_CONTENT: &str = "account_content";

    /// `/{SCOPE_ELD_ROOT}/` — every CADO path under the ELD root scope starts with this.
    ///
    /// Must stay aligned with [`SCOPE_ELD_ROOT`].
    pub const PATH_PREFIX_ELD_ROOT_SCOPE: &str = "/@eld/";
    /// `/{SCOPE_TEST}/`
    pub const PATH_PREFIX_TEST_SCOPE: &str = "/@test/";
    /// `/{SCOPE_OTHER}/`
    pub const PATH_PREFIX_OTHER_SCOPE: &str = "/@other/";
    /// `/{SCOPE_ELD_ROOT}/{TYPE_ACCOUNT}/`
    pub const PATH_PREFIX_ACCOUNT: &str = "/@eld/account/";
    /// `/{SCOPE_ELD_ROOT}/{TYPE_STAKING_ACCOUNT}/`
    pub const PATH_PREFIX_STAKING_ACCOUNT: &str = "/@eld/staking_account/";
    /// `/{SCOPE_ELD_ROOT}/{TYPE_CONTRACT_INFO}/`
    pub const PATH_PREFIX_CONTRACT_INFO: &str = "/@eld/contract_info/";
    /// `/{SCOPE_ELD_ROOT}/{TYPE_CONTRACT_STATE}/`
    pub const PATH_PREFIX_CONTRACT_STATE: &str = "/@eld/contract_state/";
    /// `/{SCOPE_ELD_ROOT}/{TYPE_CADO_MAP}/`
    pub const PATH_PREFIX_CADO_MAP: &str = "/@eld/cado_map/";
    /// `/{SCOPE_ELD_ROOT}/{TYPE_ACCOUNT_CONTENT}/`
    pub const PATH_PREFIX_ACCOUNT_CONTENT: &str = "/@eld/account_content/";
    /// `/{SCOPE_ELD_ROOT}/{TYPE_APP_STATE_SNAPSHOT}/`
    pub const PATH_PREFIX_APP_STATE_SNAPSHOT: &str = "/@eld/app_state_snapshot/";
    /// `/{SCOPE_ELD_ROOT}/{TYPE_SNAPSHOT_METADATA}/`
    pub const PATH_PREFIX_SNAPSHOT_METADATA: &str = "/@eld/snapshot_metadata/";
    /// `/{SCOPE_ELD_ROOT}/{TYPE_SNAPSHOT_CHUNK}/`
    pub const PATH_PREFIX_SNAPSHOT_CHUNK: &str = "/@eld/snapshot_chunk/";
    /// `/{SCOPE_ELD_ROOT}/{TYPE_CHUNK_REFERENCE}/`
    pub const PATH_PREFIX_CHUNK_REFERENCE: &str = "/@eld/chunk_reference/";
    /// `/{SCOPE_ELD_ROOT}/{TYPE_APP_STATE_TIP}/`
    pub const PATH_PREFIX_APP_STATE_TIP: &str = "/@eld/app_state_tip/";
    /// `/{SCOPE_ELD_ROOT}/{TYPE_EPOCH_RECORD}/`
    pub const PATH_PREFIX_EPOCH_RECORD: &str = "/@eld/epoch_record/";
    /// `/{SCOPE_ELD_ROOT}/{TYPE_NAMESPACE}/` — on-chain namespace registry CADOs.
    pub const PATH_PREFIX_NAMESPACE_REGISTRY: &str = "/@eld/namespace/";
    /// `/{SCOPE_ELD_ROOT}/{PINBOARD}/` (see [`super::abci_query::PINBOARD`]).
    pub const PATH_PREFIX_PINBOARD: &str = "/@eld/pinboard/";
    /// `/{SCOPE_ELD_ROOT}/{TYPE_CONTENT_MANIFEST}/`
    pub const PATH_PREFIX_CONTENT_MANIFEST: &str = "/@eld/content_manifest/";

    // Legacy constants for backward compatibility
    /// Allowed scopes for CADO paths
    /// These scopes define the namespace boundaries for different types of data
    pub const ALLOWED_SCOPES: &[&str] = &[
        SCOPE_ELD_ROOT, // System and core functionality
        SCOPE_USER,     // User-specific data
        SCOPE_PUBLIC,   // Publicly accessible data
        SCOPE_CONTRACT, // Smart contract data
        SCOPE_TEST,     // Testing purposes
        SCOPE_OTHER,    // Other/miscellaneous data
        SCOPE_ELD,      // Eld network data
    ];

    /// Valid CADO types that can be stored in the system
    /// Each type has specific validation rules and storage requirements
    pub const VALID_TYPES: &[&str] = &[
        // Account-related types
        TYPE_ACCOUNT,
        TYPE_STAKING_ACCOUNT,
        TYPE_STORAGE_STAKING_ACCOUNT,
        // Contract-related types
        TYPE_CONTRACT_INFO,
        TYPE_CONTRACT_STATE,
        TYPE_CADO_MAP,
        // System and state types
        TYPE_APP_STATE_TIP,
        TYPE_EPOCH_RECORD,
        TYPE_SNAPSHOT_METADATA,
        TYPE_SNAPSHOT_CHUNK,
        TYPE_CHUNK_REFERENCE,
        TYPE_APP_STATE_SNAPSHOT,
        TYPE_NAMESPACE,
    ];

    /// CADO types that require 20-byte hex addresses (Ed25519-derived addresses)
    pub const ADDRESS_TYPES: &[&str] = &[
        TYPE_ACCOUNT,
        TYPE_STAKING_ACCOUNT,
        TYPE_STORAGE_STAKING_ACCOUNT,
    ];

    /// CADO types that require 32-byte hex identifiers
    pub const HASH_TYPES: &[&str] = &[
        TYPE_CONTRACT_INFO,
        TYPE_CONTRACT_STATE,
        TYPE_CADO_MAP,
        TYPE_APP_STATE_TIP,
        TYPE_EPOCH_RECORD,
        TYPE_SNAPSHOT_METADATA,
        TYPE_SNAPSHOT_CHUNK,
        TYPE_CHUNK_REFERENCE,
        TYPE_APP_STATE_SNAPSHOT,
    ];

    /// CADO types that require a 4-part path format: /@scope/type/sender_address/identifier
    pub const FOUR_PART_PATH_TYPES: &[&str] = &[];

    /// CADO types that are allowed to be deleted by users (require signature verification)
    /// These types represent user-controlled data that can be removed through transactions
    pub const USER_DELETABLE_TYPES: &[&str] = &[];

    /// CADO types that are allowed to be deleted by the system (require system authentication)
    /// These types represent system-managed data that can be cleaned up automatically
    pub const SYSTEM_DELETABLE_TYPES: &[&str] = &[
        TYPE_CHUNK_REFERENCE,   // System cleanup of old chunk references
        TYPE_SNAPSHOT_CHUNK,    // System cleanup of old snapshot chunks
        TYPE_SNAPSHOT_METADATA, // System cleanup of old snapshot metadata
    ];

    /// All CADO types that are allowed to be deleted (combines user and system deletable types)
    pub const ALL_DELETABLE_TYPES: &[&str] = &[
        TYPE_CHUNK_REFERENCE,
        TYPE_SNAPSHOT_CHUNK,
        TYPE_SNAPSHOT_METADATA,
    ];

    /// CADO types stored for node persistence / sync only — never part of chain-state
    /// `committed_cado_cache` or the consensus MPT (avoids snapshot-in-snapshot recursion).
    pub const INFRASTRUCTURE_CADO_TYPES: &[&str] = &[
        TYPE_APP_STATE_TIP,
        TYPE_APP_STATE_SNAPSHOT,
        TYPE_SNAPSHOT_METADATA,
        TYPE_SNAPSHOT_CHUNK,
    ];

    /// Maximum allowed path length to prevent resource exhaustion attacks
    pub const MAX_PATH_LENGTH: usize = 512;

    /// Maximum allowed prefix length for search operations
    pub const MAX_PREFIX_LENGTH: usize = 256;

    /// Max serialized app state snapshot payload (epoch boundary; includes committed cache).
    pub const MAX_APP_STATE_SNAPSHOT_CADO_SIZE_BYTES: usize = 256 * 1024 * 1024;

    #[cfg(test)]
    mod path_prefix_tests {
        use super::*;
        use crate::constants::abci_query;

        #[test]
        fn path_prefixes_match_scope_and_type_segments() {
            assert_eq!(PATH_PREFIX_ELD_ROOT_SCOPE, format!("/{SCOPE_ELD_ROOT}/"));
            assert_eq!(PATH_PREFIX_TEST_SCOPE, format!("/{SCOPE_TEST}/"));
            assert_eq!(PATH_PREFIX_OTHER_SCOPE, format!("/{SCOPE_OTHER}/"));
            assert_eq!(
                PATH_PREFIX_ACCOUNT,
                format!("/{SCOPE_ELD_ROOT}/{TYPE_ACCOUNT}/")
            );
            assert_eq!(
                PATH_PREFIX_STAKING_ACCOUNT,
                format!("/{SCOPE_ELD_ROOT}/{TYPE_STAKING_ACCOUNT}/")
            );
            assert_eq!(
                PATH_PREFIX_CONTRACT_INFO,
                format!("/{SCOPE_ELD_ROOT}/{TYPE_CONTRACT_INFO}/")
            );
            assert_eq!(
                PATH_PREFIX_CONTRACT_STATE,
                format!("/{SCOPE_ELD_ROOT}/{TYPE_CONTRACT_STATE}/")
            );
            assert_eq!(
                PATH_PREFIX_CADO_MAP,
                format!("/{SCOPE_ELD_ROOT}/{TYPE_CADO_MAP}/")
            );
            assert_eq!(
                PATH_PREFIX_ACCOUNT_CONTENT,
                format!("/{SCOPE_ELD_ROOT}/{TYPE_ACCOUNT_CONTENT}/")
            );
            assert_eq!(
                PATH_PREFIX_APP_STATE_SNAPSHOT,
                format!("/{SCOPE_ELD_ROOT}/{TYPE_APP_STATE_SNAPSHOT}/")
            );
            assert_eq!(
                PATH_PREFIX_SNAPSHOT_METADATA,
                format!("/{SCOPE_ELD_ROOT}/{TYPE_SNAPSHOT_METADATA}/")
            );
            assert_eq!(
                PATH_PREFIX_SNAPSHOT_CHUNK,
                format!("/{SCOPE_ELD_ROOT}/{TYPE_SNAPSHOT_CHUNK}/")
            );
            assert_eq!(
                PATH_PREFIX_CHUNK_REFERENCE,
                format!("/{SCOPE_ELD_ROOT}/{TYPE_CHUNK_REFERENCE}/")
            );
            assert_eq!(
                PATH_PREFIX_APP_STATE_TIP,
                format!("/{SCOPE_ELD_ROOT}/{TYPE_APP_STATE_TIP}/")
            );
            assert_eq!(
                PATH_PREFIX_EPOCH_RECORD,
                format!("/{SCOPE_ELD_ROOT}/{TYPE_EPOCH_RECORD}/")
            );
            assert_eq!(
                PATH_PREFIX_NAMESPACE_REGISTRY,
                format!("/{SCOPE_ELD_ROOT}/{TYPE_NAMESPACE}/")
            );
            assert_eq!(
                PATH_PREFIX_CONTENT_MANIFEST,
                format!("/{SCOPE_ELD_ROOT}/{TYPE_CONTENT_MANIFEST}/")
            );
            assert_eq!(
                PATH_PREFIX_PINBOARD,
                format!("/{}/{}/", SCOPE_ELD_ROOT, abci_query::PINBOARD)
            );
        }
    }
}
