use crate::error::EldError;
use crate::logging::{LogSanitizer, SanitizedLoggable};
use crate::tx::{PostMessageUserRequest, Tx};
use crate::Address;
use ed25519_dalek::Signer;
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};
use std::fmt;
use tracing::debug;

// JSON structure for deserialization
#[derive(Clone, Serialize, Deserialize)]
pub struct JsonWallet {
    pub name: String,
    pub address: String,
    pub keypair: JsonKeypair,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct JsonKeypair {
    pub public_key: String,  // hex encoded
    pub private_key: String, // hex encoded
    pub keytype: String,
}

impl fmt::Debug for JsonKeypair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JsonKeypair")
            .field("public_key", &self.public_key)
            .field("private_key", &"[redacted]")
            .field("keytype", &self.keytype)
            .finish()
    }
}

impl fmt::Debug for JsonWallet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JsonWallet")
            .field("name", &self.name)
            .field("address", &self.address)
            .field("keypair", &self.keypair)
            .finish()
    }
}

#[derive(Clone)]
pub struct Wallet {
    pub name: String,
    pub address: Address,
    signing_key: SigningKey,
    pub public_key: [u8; 32], // Added public_key field
}

impl Wallet {
    pub fn from_signing_key(name: String, signing_key: SigningKey) -> Self {
        let verifying_key = signing_key.verifying_key();
        let address = Address::from_public_key(&verifying_key)
            .expect("Failed to derive address from Ed25519 public key");
        Wallet {
            name,
            address,
            signing_key,
            public_key: verifying_key.to_bytes(),
        }
    }

    pub fn from_json_wallet(json_wallet: JsonWallet) -> Result<Self, EldError> {
        let private_key_bytes: [u8; 32] = hex::decode(&json_wallet.keypair.private_key)
            .map_err(|e| {
                EldError::make_validation_error(
                    "private_key",
                    &json_wallet.keypair.private_key,
                    format!("Invalid hex format: {e}"),
                )
            })?
            .try_into()
            .map_err(|_| {
                EldError::make_validation_error(
                    "private_key",
                    &json_wallet.keypair.private_key,
                    "Private key must be 32 bytes",
                )
            })?;
        let signing_key = SigningKey::from_bytes(&private_key_bytes);
        let verifying_key = signing_key.verifying_key();
        let derived_address =
            Address::from_public_key(&verifying_key).map_err(|e| EldError::WalletError {
                operation: "derive_address".to_string(),
                wallet_name: json_wallet.name.clone(),
                details: format!("Failed to derive address from public key: {e}"),
            })?;
        // generate address here
        // and check consistent with json
        let address = Address::parse_hex_str(&json_wallet.address)?;

        debug!(
            "json config address {} ==  derived address {} ?",
            address, derived_address
        );
        if address != derived_address {
            return Err(EldError::WalletError {
                operation: "load_wallet".to_string(),
                wallet_name: json_wallet.name.clone(),
                details: format!(
                    "Wallet address mismatch: configured {} does not match address derived from private key {}",
                    address.hex_with_prefix(),
                    derived_address.hex_with_prefix(),
                ),
            });
        }
        Ok(Wallet {
            name: json_wallet.name,
            address,
            signing_key,
            public_key: verifying_key.to_bytes(),
        })
    }

    pub fn from_json_str(json_str: &str) -> Result<Self, EldError> {
        let json_wallet: JsonWallet = serde_json::from_str(json_str).map_err(|e| {
            EldError::make_validation_error(
                "json_str",
                json_str,
                format!("Failed to parse JSON: {e}"),
            )
        })?;
        Self::from_json_wallet(json_wallet)
    }

    /// Serializes this wallet to the JSON persistence format (includes keypair for storage).
    pub fn to_json(&self) -> JsonWallet {
        JsonWallet {
            name: self.name.clone(),
            address: self.address.hex(),
            keypair: JsonKeypair {
                public_key: hex::encode(self.signing_key.verifying_key().to_bytes()),
                private_key: hex::encode(self.signing_key.to_bytes()),
                keytype: "ed25519_keypair".to_string(),
            },
        }
    }

    pub fn sign(&self, tx: &mut Tx, chain_id: &str) -> String {
        // TODO: Change to Result
        tx.sign(&self.signing_key, chain_id)
    }

    pub fn sign_bytes(&self, bytes: &[u8]) -> String {
        let signature = self.signing_key.sign(bytes);
        hex::encode(signature.to_bytes())
    }

    /// Provider signature over a P2P capacity challenge response.
    pub fn sign_capacity_challenge_response(
        &self,
        challenge_proof: &crate::capacity_proof::ChallengeProof,
    ) -> Result<(String, String), String> {
        crate::capacity_proof::sign_capacity_challenge_response(&self.signing_key, challenge_proof)
    }

    pub fn verify(&self, tx: &Tx, chain_id: &str) -> bool {
        // TODO: Change to Result
        tx.verify(chain_id)
    }

    /// User-signed pinboard commitment over `message_bytes` (see `crate::tx::PostMessageUserRequest`).
    pub fn sign_post_message_user_commitment(
        &self,
        message_bytes: &[u8],
        input: crate::tx::PostMessageUserRequestInput,
    ) -> Result<PostMessageUserRequest, String> {
        crate::tx::build_signed_post_message_user_request(&self.signing_key, message_bytes, input)
    }

    /// Validator-signed outer [`Tx`] wrapping [`crate::tx::PostMessageTx`] (submission to chain).
    pub fn sign_post_message_chain_tx(
        &self,
        message_bytes: &[u8],
        user_request: &PostMessageUserRequest,
        validator_tx_nonce: u32,
        received_timestamp: u64,
        chain_id: &str,
        tx_fee: u128,
    ) -> Result<Tx, String> {
        crate::tx::validate_message_and_build_post_message_tx(
            message_bytes,
            user_request,
            &self.signing_key,
            validator_tx_nonce,
            received_timestamp,
            chain_id,
            tx_fee,
        )
    }
}

impl fmt::Display for Wallet {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "\tname:\t\t{}\n\taddress:\t{}\n\tpublic_key:\t{}\n\n",
            self.name,
            self.address,
            hex::encode(self.signing_key.verifying_key())
        )
    }
}

impl SanitizedLoggable for Wallet {
    fn sanitized_log(&self) -> String {
        format!(
            "Wallet {{ name: {}, address: {}, public_key: {} }}",
            self.name,
            LogSanitizer::sanitize_address(&self.address.hex_with_prefix()),
            LogSanitizer::sanitize_public_key(&hex::encode(self.public_key))
        )
    }
}

impl Wallet {
    /// Full wallet details for interactive CLI output (address and public key are not redacted).
    pub fn terminal_display(&self) -> String {
        format!(
            "Wallet {{ name: {}, address: {}, public_key: {} }}",
            self.name,
            self.address.hex_with_prefix(),
            hex::encode(self.public_key)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::RngCore;
    use tracing::debug;

    #[test]
    fn test_decode_str_keys() {
        // Throwaway fixture: repeating 0x80 is not a live-network key.
        let pk = hex::encode([0x80u8; 32]);
        let _ = hex::decode(&pk).expect("Can decode public key hex string into bytes");
        debug!("pk len: {}", pk.len()); //32 bytes

        let _: [u8; 32] = hex::decode(&pk)
            .expect("Failed to decode public key hex string")
            .try_into()
            .map_err(|_| "Public key must be 32 bytes")
            .expect("Failed to convert public key bytes to 32-byte array");
    }

    #[test]
    fn test_wallet_sign_verify() {
        let mut rng = rand::rng(); // Thread-local RNG
        let mut secret_bytes = [0u8; 32]; // Ed25519 secret key is 32 bytes
        rng.fill_bytes(&mut secret_bytes); // Fill with random bytes
        let signing_key = SigningKey::from_bytes(&secret_bytes); // Construct SigningKey
        let _ = signing_key.verifying_key();

        let _ = Wallet::from_signing_key("TestWallet".to_string(), signing_key);

        // TODO: Add test to sign and verify with wallet
    }

    #[test]
    fn test_display_wallet() {
        let mut rng = rand::rng(); // Thread-local RNG
        let mut secret_bytes = [0u8; 32]; // Ed25519 secret key is 32 bytes
        rng.fill_bytes(&mut secret_bytes); // Fill with random bytes
        let signing_key = SigningKey::from_bytes(&secret_bytes); // Construct SigningKey
        let _ = signing_key.verifying_key();

        let wallet = Wallet::from_signing_key("TestWallet".to_string(), signing_key);
        debug!("wallet:\n{}", wallet);
    }

    #[test]
    fn test_key_properties() {
        let mut rng = rand::rng(); // Thread-local RNG
        let mut secret_bytes = [0u8; 32]; // Ed25519 secret key is 32 bytes
        rng.fill_bytes(&mut secret_bytes); // Fill with random bytes
        let signing_key = SigningKey::from_bytes(&secret_bytes); // Construct SigningKey
        assert_eq!(signing_key.to_bytes().len(), 32);
        let hex_string = hex::encode(signing_key.to_bytes());
        assert_eq!(hex_string.len(), 64);
    }

    fn signing_key_from_hex(hex_str: &str) -> Result<SigningKey, EldError> {
        let bytes = hex::decode(hex_str).map_err(|e| {
            EldError::make_validation_error("hex_str", hex_str, format!("Invalid hex format: {e}"))
        })?;
        let signing_key_bytes: [u8; 32] = bytes.try_into().map_err(|_| {
            EldError::make_validation_error(
                "hex_str",
                hex_str,
                "Hex string must decode to 32 bytes",
            )
        })?;
        Ok(SigningKey::from_bytes(&signing_key_bytes))
    }

    #[test]
    fn json_keypair_debug_redacts_private_key() {
        let keypair = JsonKeypair {
            public_key: "abcd".to_string(),
            private_key: "deadbeef".to_string(),
            keytype: "ed25519_keypair".to_string(),
        };
        let debug = format!("{keypair:?}");
        assert!(debug.contains("abcd"));
        assert!(debug.contains("[redacted]"));
        assert!(!debug.contains("deadbeef"));
    }

    #[test]
    fn from_json_wallet_rejects_address_key_mismatch() {
        let wallet =
            Wallet::from_signing_key("mismatch".to_string(), SigningKey::from_bytes(&[7u8; 32]));
        let mut json = wallet.to_json();
        json.address = "0x0000000000000000000000000000000000000001".to_string();

        assert!(matches!(
            Wallet::from_json_wallet(json),
            Err(EldError::WalletError {
                operation,
                wallet_name,
                ..
            }) if operation == "load_wallet" && wallet_name == "mismatch"
        ));
    }

    #[test]
    fn test_create_signingkey_from_hex_string() {
        // Documented throwaway seed (all 0x01). Not a live-network key.
        let seed = [1u8; 32];
        let expected = SigningKey::from_bytes(&seed);
        let json_private_key = hex::encode(expected.to_bytes());
        assert_eq!(json_private_key.len(), 64);
        let signing_key = signing_key_from_hex(&json_private_key)
            .expect("Failed to create signing key from hex string");

        assert_eq!(signing_key.to_bytes(), seed);
        assert_eq!(
            signing_key.verifying_key().to_bytes(),
            expected.verifying_key().to_bytes()
        );
    }

    fn fixture_wallet(name: &str, seed: u8) -> Wallet {
        Wallet::from_signing_key(name.to_string(), SigningKey::from_bytes(&[seed; 32]))
    }

    #[test]
    fn generated_wallets_json_roundtrip_is_cryptographically_valid() {
        use ed25519_dalek::{Signature, Verifier, VerifyingKey};

        for i in 1u8..=4 {
            let name = format!("wallet-capacity-validator-{i}");
            let wallet = fixture_wallet(&name, i);
            let json = wallet.to_json();

            let signing_key = signing_key_from_hex(&json.keypair.private_key)
                .unwrap_or_else(|e| panic!("{name}: bad private key: {e}"));
            let derived_pk = hex::encode(signing_key.verifying_key().to_bytes());
            assert_eq!(json.keypair.public_key, derived_pk);
            assert_eq!(
                json.address,
                Address::from_public_key(&signing_key.verifying_key())
                    .unwrap()
                    .hex()
            );

            let loaded = Wallet::from_json_wallet(json)
                .unwrap_or_else(|e| panic!("from_json_wallet failed for {name}: {e}"));
            assert_eq!(hex::encode(loaded.public_key), derived_pk);

            let msg = b"capacity-validator-wallet-v1";
            let sig_hex = loaded.sign_bytes(msg);
            let sig_bytes: [u8; 64] = hex::decode(&sig_hex)
                .unwrap()
                .try_into()
                .expect("signature must be 64 bytes");
            let signature = Signature::from_bytes(&sig_bytes);
            let verifying_key = VerifyingKey::from_bytes(&loaded.public_key).unwrap();
            verifying_key
                .verify(msg, &signature)
                .unwrap_or_else(|e| panic!("{name}: verify failed: {e}"));
            assert!(verifying_key.verify(b"tampered", &signature).is_err());
        }
    }

    #[test]
    fn generated_wallets_can_sign_and_verify_tx() {
        use crate::tx::{Payload, PayloadInner, TransferTx, Tx, TxAmount};

        let recipient = fixture_wallet("wallet1", 7).address;
        for i in 1u8..=4 {
            let wallet = fixture_wallet(&format!("wallet-capacity-validator-{i}"), i);
            let transfer = TransferTx::new(wallet.address, recipient, TxAmount(1)).unwrap();
            let mut tx = Tx::new(1u32, Payload::new(PayloadInner::Transfer(transfer)), {
                ed25519_dalek::VerifyingKey::from_bytes(&wallet.public_key).unwrap()
            });
            let chain_id = "eld-testnet-tempelhof";
            wallet.sign(&mut tx, chain_id);
            assert!(
                wallet.verify(&tx, chain_id),
                "wallet-capacity-validator-{i} tx verify failed"
            );
        }
    }
}
