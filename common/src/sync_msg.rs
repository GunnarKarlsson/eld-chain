//! P2P message types for content sync and capacity challenges.
//! Used by both eld_node_app and cli for GossipSub messaging.

use crate::address::Address;
use crate::capacity_proof::ChunkProof;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum SyncMsg {
    Heartbeat {
        timestamp: u64,
    },
    ContentSyncHeartbeat {
        timestamp: u64,
    },
    ContentSyncHeartbeatResponse {
        provider_id: Address,
        timestamp: u64,
    },
    Announce {
        content_id: String,
    },
    ContentRequest {
        content_id: String,
    },
    ContentResponse {
        content_id: String,
        content: String,
    },
    CapacityChallenge {
        challenge_id: String,
        challenger: Address,
        provider_id: Address,
        chunk_indices: Vec<usize>,
        block_height: i64,
        merkle_root: [u8; 32],
        seed: [u8; 32],
        expiration_block: i64,
        timestamp: u64,
    },
    CapacityChallengeResponse {
        challenge_id: String,
        provider_id: Address,
        challenger: Address,
        block_height: i64,
        proofs: Vec<ChunkProof>,
        generated_at: u64,
        provider_pubkey: String,
        provider_signature: String,
    },
    ContentInventoryRequest {
        timestamp: u64,
    },
    ContentInventoryResponse {
        items_json: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    fn addr(hex: &str) -> Address {
        Address::parse_hex_str(hex).expect("test address")
    }

    const PROVIDER: &str = "0x2222222222222222222222222222222222222222";
    const CHALLENGER: &str = "0x1111111111111111111111111111111111111111";

    #[test]
    fn capacity_challenge_address_fields_round_trip_json() {
        let msg = SyncMsg::CapacityChallenge {
            challenge_id: "challenge-1".to_string(),
            challenger: addr(CHALLENGER),
            provider_id: addr(PROVIDER),
            chunk_indices: vec![0, 3, 7],
            block_height: 42,
            merkle_root: [1u8; 32],
            seed: [2u8; 32],
            expiration_block: 142,
            timestamp: 1_700_000_000,
        };

        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(json.contains(PROVIDER));
        assert!(json.contains(CHALLENGER));

        let roundtrip: SyncMsg = serde_json::from_str(&json).expect("deserialize");
        match roundtrip {
            SyncMsg::CapacityChallenge {
                challenger,
                provider_id,
                chunk_indices,
                ..
            } => {
                assert_eq!(challenger, addr(CHALLENGER));
                assert_eq!(provider_id, addr(PROVIDER));
                assert_eq!(chunk_indices, vec![0, 3, 7]);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn capacity_challenge_response_address_fields_round_trip_json() {
        use crate::capacity_proof::{ChunkProof, SlotState};

        let msg = SyncMsg::CapacityChallengeResponse {
            challenge_id: "challenge-2".to_string(),
            provider_id: addr(PROVIDER),
            challenger: addr(CHALLENGER),
            block_height: 99,
            proofs: vec![ChunkProof {
                chunk_index: 0,
                chunk_data: vec![1, 2, 3],
                chunk_hash: [9u8; 32],
                merkle_proof: vec![[8u8; 32]],
                slot_state: SlotState::Proof,
            }],
            generated_at: 1_700_000_001,
            provider_pubkey: "aa".to_string(),
            provider_signature: "bb".to_string(),
        };

        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(json.contains(PROVIDER));
        assert!(json.contains(CHALLENGER));

        let roundtrip: SyncMsg = serde_json::from_str(&json).expect("deserialize");
        match roundtrip {
            SyncMsg::CapacityChallengeResponse {
                provider_id,
                challenger,
                block_height,
                ..
            } => {
                assert_eq!(provider_id, addr(PROVIDER));
                assert_eq!(challenger, addr(CHALLENGER));
                assert_eq!(block_height, 99);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn content_sync_heartbeat_response_provider_id_round_trip_json() {
        let msg = SyncMsg::ContentSyncHeartbeatResponse {
            provider_id: addr(PROVIDER),
            timestamp: 1_700_000_002,
        };

        let json = serde_json::to_string(&msg).expect("serialize");
        assert!(json.contains(PROVIDER));

        let roundtrip: SyncMsg = serde_json::from_str(&json).expect("deserialize");
        match roundtrip {
            SyncMsg::ContentSyncHeartbeatResponse {
                provider_id,
                timestamp,
            } => {
                assert_eq!(provider_id, addr(PROVIDER));
                assert_eq!(timestamp, 1_700_000_002);
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }
}
