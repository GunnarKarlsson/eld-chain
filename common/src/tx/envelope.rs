use crate::error::EldError;
use crate::nonce::Nonce;
use crate::tx::parts::{HasSender, TxAmount, TxPublicKey, TxSig};
use crate::tx::payloads::{
    validate_post_message_user_signature, PostMessageTx, PostMessageUserRequest,
};
use crate::tx::Payload;
use crate::Address;
use ed25519_dalek::{Signer, SigningKey, Verifier};
use serde::{Deserialize, Serialize};
use tracing::warn;

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct Tx {
    pub sig: TxSig,
    pub nonce: Nonce,
    pub payload: Payload,
    pub public_key: TxPublicKey,
    pub fee: TxAmount,
}

impl Tx {
    pub fn new(
        nonce: impl Into<Nonce>,
        payload: Payload,
        public_key: impl Into<TxPublicKey>,
    ) -> Self {
        Self {
            sig: TxSig::empty(),
            nonce: nonce.into(),
            payload,
            public_key: public_key.into(),
            fee: 0.into(),
        }
    }

    pub fn sign(&mut self, signing_key: &SigningKey, chain_id: &str) -> Result<String, EldError> {
        self.sig = TxSig::empty();
        let json = serde_json::to_string(self).map_err(|e| EldError::TransactionError {
            tx_type: "sign".to_string(),
            details: format!("Failed to serialize transaction to JSON for signing: {e}"),
        })?;

        // Create the signing bytes: JSON bytes + chain_id bytes
        let mut all_bytes = json.as_bytes().to_vec();
        all_bytes.extend_from_slice(chain_id.as_bytes());

        let signature = signing_key.sign(&all_bytes);
        self.sig = signature.into();
        Ok(self.sig.as_str().to_string())
    }

    pub fn verify(&self, chain_id: &str) -> Result<bool, EldError> {
        // Step 1: Verify the signature using public key
        let tx_to_verify = Tx {
            sig: TxSig::empty(),
            nonce: self.nonce,
            payload: self.payload.clone(),
            public_key: self.public_key.clone(),
            fee: self.fee,
        };

        let json =
            serde_json::to_string(&tx_to_verify).map_err(|e| EldError::TransactionError {
                tx_type: "verify".to_string(),
                details: format!("Failed to serialize transaction to JSON for verification: {e}"),
            })?;

        // Create the verification bytes: JSON bytes + chain_id bytes
        let mut all_bytes = json.as_bytes().to_vec();
        all_bytes.extend_from_slice(chain_id.as_bytes());

        let public_key = self.public_key.to_verifying_key()?;
        let signature = self.sig.to_signature()?;

        let sig_valid = public_key.verify(&all_bytes, &signature).is_ok();
        if !sig_valid {
            warn!("signature is not valid");
            return Ok(false);
        }

        // Step 2: Verify the sender address is derived from the public key
        let derived_address = Address::from_public_key(&public_key)?;
        let payload_sender = self.payload.inner.sender();

        let addr_valid = payload_sender == derived_address;
        if !addr_valid {
            warn!("address is not valid");
        }
        Ok(addr_valid)
    }
}

impl std::fmt::Display for Tx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Tx {{\n sig: {}\n nonce: {}\n payload: {}\n public_key: {}\n fee: {}\n }}",
            self.sig, self.nonce, self.payload, self.public_key, self.fee
        )
    }
}

pub fn validate_message_and_build_post_message_tx(
    message_bytes: &[u8],
    user_request: &PostMessageUserRequest,
    validator_signing_key: &SigningKey,
    validator_tx_nonce: u32,
    received_timestamp: u64,
    chain_id: &str,
    tx_fee: u128,
) -> Result<Tx, String> {
    validate_post_message_user_signature(message_bytes, user_request)?;

    let validator_pubkey = validator_signing_key.verifying_key();
    let sender = Address::from_public_key(&validator_pubkey).map_err(|e| e.to_string())?;
    let original_signer =
        Address::parse_hex_str(&user_request.original_signer).map_err(|e| e.to_string())?;

    let post_message = PostMessageTx::new(
        sender,
        original_signer,
        user_request.original_signer_pubkey.clone(),
        user_request.content_key.clone(),
        user_request.message_id.clone(),
        user_request.expires_height,
        user_request.visibility.clone(),
        user_request.topic.clone(),
        user_request.tags.clone(),
        user_request.content_type.clone(),
        user_request.fee_amount,
        received_timestamp,
        user_request.user_signature.clone(),
        user_request.namespace.clone(),
    )
    .map_err(|e| e.to_string())?;

    let mut tx = Tx::new(
        Nonce::new(validator_tx_nonce),
        Payload::new(post_message),
        validator_pubkey,
    );
    tx.fee = tx_fee.into();
    tx.sign(validator_signing_key, chain_id)
        .map_err(|e| e.to_string())?;

    Ok(tx)
}
