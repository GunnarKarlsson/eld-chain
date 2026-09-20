use eld_common::error::EldError;
use libp2p::identity;
use serde::Deserialize;
use std::fs;

/// P2P keypair configuration loaded from `config/p2p_keypair.json`.
/// Format is similar to entries in `wallets.json`, but without a `name` field.
#[derive(Debug, Deserialize)]
pub struct P2PKeypair {
    /// Present in `p2p_keypair.json` for human readability; not used after load.
    #[allow(dead_code)]
    #[serde(default)]
    address: String,
    pub keypair: P2PKeypairInner,
}

#[derive(Debug, Deserialize)]
pub struct P2PKeypairInner {
    /// Optional in file; not used by `load_libp2p_keypair` (secret key is authoritative).
    #[allow(dead_code)]
    #[serde(default)]
    public_key: String,
    pub private_key: String, // hex-encoded ed25519 secret key (32 bytes)
    pub keytype: String,
}

impl P2PKeypair {
    /// Loads the P2P keypair config from the given path and converts it to a libp2p identity keypair.
    pub fn load_libp2p_keypair(path: &str) -> Result<identity::Keypair, EldError> {
        let contents = fs::read_to_string(path).map_err(|e| EldError::FileSystemError {
            operation: "read".to_string(),
            path: path.to_string(),
            details: e.to_string(),
        })?;

        let cfg: P2PKeypair =
            serde_json::from_str(&contents).map_err(|e| EldError::ConfigError {
                file: path.to_string(),
                details: format!("Failed to parse P2P keypair config: {e}"),
            })?;

        // For now we expect ed25519 or ed25519_keypair; other values are rejected.
        if cfg.keypair.keytype != "ed25519" && cfg.keypair.keytype != "ed25519_keypair" {
            return Err(EldError::ConfigError {
                file: path.to_string(),
                details: format!(
                    "Unsupported P2P key type '{}', expected 'ed25519' or 'ed25519_keypair'",
                    cfg.keypair.keytype
                ),
            });
        }

        let secret_bytes_vec =
            hex::decode(&cfg.keypair.private_key).map_err(|e| EldError::ConfigError {
                file: path.to_string(),
                details: format!("Invalid hex in P2P private key: {e}"),
            })?;

        if secret_bytes_vec.len() != 32 {
            return Err(EldError::ConfigError {
                file: path.to_string(),
                details: format!(
                    "Invalid P2P private key length: {}, expected 32 bytes",
                    secret_bytes_vec.len()
                ),
            });
        }

        let mut secret_bytes: [u8; 32] = [0u8; 32];
        secret_bytes.copy_from_slice(&secret_bytes_vec);

        let keypair = identity::Keypair::ed25519_from_bytes(&mut secret_bytes).map_err(|e| {
            EldError::InitializationError {
                component: "P2P Sync Coordinator".to_string(),
                details: format!("Failed to convert P2P secret key to libp2p keypair: {e}"),
            }
        })?;

        Ok(keypair)
    }
}
