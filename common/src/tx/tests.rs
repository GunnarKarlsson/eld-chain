use crate::constants::{protocol::DEFAULT_TX_FEE, test::MOCK_CHAIN_ID, tx_type};
use crate::Address;
use tracing::info;

use super::*;
use ed25519_dalek::SigningKey;

/// Documented throwaway seed (all 0x01). Not a live-network key.
fn throwaway_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[1u8; 32])
}

#[test]
fn test_tx_with_transfer_sign_verify() {
    let signing_key = throwaway_signing_key();
    let verifying_key = signing_key.verifying_key();
    let sender = Address::from_public_key(&verifying_key)
        .expect("Failed to derive address from public key in test");
    let recipient = Address::parse_hex_str("0x0987654321098765432109876543210987654321")
        .expect("valid recipient in test");

    let transfer = TransferTx::new(sender, recipient, 1.into()).expect("valid transfer");
    let mut tx = Tx {
        sig: TxSig::empty(),
        nonce: 1u32.into(),
        payload: Payload::new(transfer),
        public_key: TxPublicKey::from(&verifying_key),
        fee: DEFAULT_TX_FEE.into(),
    };

    tx.sign(&signing_key, MOCK_CHAIN_ID).expect("sign");
    let json = serde_json::to_string(&tx).expect("Failed to serialize transaction to JSON in test");
    info!("Rust JSON: {}", json);
    let hex = hex::encode(&json);
    info!("Rust Hex: {}", hex);

    assert!(
        tx.verify(MOCK_CHAIN_ID).expect("verify"),
        "Verification failed for original tx"
    );

    let decoded_json = String::from_utf8(hex::decode(&hex).expect("Failed to decode hex in test"))
        .expect("Failed to convert decoded hex to UTF-8 in test");
    let decoded_tx: Tx = serde_json::from_str(&decoded_json)
        .expect("Failed to deserialize JSON to transaction in test");
    assert!(
        decoded_tx.verify(MOCK_CHAIN_ID).expect("verify"),
        "Verification failed for decoded tx"
    );
}

#[test]
fn test_transfer_tx_hex_json_serialization() {
    // Create a signing key for testing using known test bytes
    let signing_key = throwaway_signing_key();
    let verifying_key = signing_key.verifying_key();
    let sender = Address::from_public_key(&verifying_key)
        .expect("Failed to derive address from public key in test");
    let recipient = Address::parse_hex_str("0x0987654321098765432109876543210987654321")
        .expect("valid recipient in test");

    // Create a transfer transaction
    let transfer = TransferTx::new(sender, recipient, 1000.into()).expect("valid transfer");

    // Create the payload
    let payload = Payload::new(transfer);

    // Create and sign the transaction
    let mut tx = Tx {
        sig: TxSig::empty(),
        nonce: 1u32.into(),
        payload,
        public_key: TxPublicKey::from(&verifying_key),
        fee: DEFAULT_TX_FEE.into(),
    };

    // Sign the transaction
    tx.sign(&signing_key, MOCK_CHAIN_ID).expect("sign");

    // Verify the original transaction
    assert!(
        tx.verify(MOCK_CHAIN_ID).expect("verify"),
        "Original transaction verification failed"
    );

    // Convert to hex-encoded JSON
    let tx_json = serde_json::to_string(&tx).expect("Failed to serialize tx to JSON");
    let tx_hex = hex::encode(tx_json.as_bytes());

    // Convert back from hex-encoded JSON
    let tx_json_bytes = hex::decode(tx_hex).expect("Failed to decode hex");
    let tx_json_str = String::from_utf8(tx_json_bytes).expect("Failed to convert bytes to string");
    let decoded_tx: Tx =
        serde_json::from_str(&tx_json_str).expect("Failed to deserialize JSON to tx");

    // Verify the decoded transaction
    assert!(
        decoded_tx.verify(MOCK_CHAIN_ID).expect("verify"),
        "Decoded transaction verification failed"
    );

    // Compare original and decoded transactions
    assert_eq!(tx.sig, decoded_tx.sig, "Transaction signature mismatch");
    assert_eq!(tx.nonce, decoded_tx.nonce, "Transaction nonce mismatch");
    assert_eq!(
        tx.public_key, decoded_tx.public_key,
        "Transaction public key mismatch"
    );
    assert_eq!(
        tx.payload, decoded_tx.payload,
        "Transaction payload mismatch"
    );

    // Verify specific transfer details
    match decoded_tx.payload.inner {
        PayloadInner::Transfer(t) => {
            assert_eq!(t.sender, sender);
            assert_eq!(
                t.recipient,
                Address::parse_hex_str("0x0987654321098765432109876543210987654321").unwrap()
            );
            assert_eq!(t.amount, 1000);
        }
        _ => panic!("Wrong payload type after deserialization"),
    }
}

#[test]
fn test_stake_tx_hex_json_serialization() {
    // Create a signing key for testing using known test bytes
    let signing_key = throwaway_signing_key();
    let verifying_key = signing_key.verifying_key();
    let sender = Address::from_public_key(&verifying_key)
        .expect("Failed to derive address from public key in test");

    // Create a stake transaction
    let stake = StakeTx::new(
        sender,
        1000.into(),
        Some(hex::encode(verifying_key.to_bytes())),
    )
    .expect("valid stake");

    // Create the payload
    let payload = Payload::new(stake);

    // Create and sign the transaction
    let mut tx = Tx {
        sig: TxSig::empty(),
        nonce: 1u32.into(),
        payload,
        public_key: TxPublicKey::from(&verifying_key),
        fee: DEFAULT_TX_FEE.into(),
    };

    // Sign the transaction
    tx.sign(&signing_key, MOCK_CHAIN_ID).expect("sign");

    // Verify the original transaction
    assert!(
        tx.verify(MOCK_CHAIN_ID).expect("verify"),
        "Original transaction verification failed"
    );

    // Convert to hex-encoded JSON
    let tx_json = serde_json::to_string(&tx).expect("Failed to serialize tx to JSON");
    let tx_hex = hex::encode(tx_json.as_bytes());

    // Convert back from hex-encoded JSON
    let tx_json_bytes = hex::decode(tx_hex).expect("Failed to decode hex");
    let tx_json_str = String::from_utf8(tx_json_bytes).expect("Failed to convert bytes to string");
    let decoded_tx: Tx =
        serde_json::from_str(&tx_json_str).expect("Failed to deserialize JSON to tx");

    // Verify the decoded transaction
    assert!(
        decoded_tx.verify(MOCK_CHAIN_ID).expect("verify"),
        "Decoded transaction verification failed"
    );

    // Compare original and decoded transactions
    assert_eq!(tx.sig, decoded_tx.sig, "Transaction signature mismatch");
    assert_eq!(tx.nonce, decoded_tx.nonce, "Transaction nonce mismatch");
    assert_eq!(
        tx.public_key, decoded_tx.public_key,
        "Transaction public key mismatch"
    );
    assert_eq!(
        tx.payload, decoded_tx.payload,
        "Transaction payload mismatch"
    );

    // Verify specific stake details
    match decoded_tx.payload.inner {
        PayloadInner::Stake(s) => {
            assert_eq!(s.sender, sender);
            assert_eq!(s.amount, 1000);
            assert_eq!(s.public_key, Some(hex::encode(verifying_key.to_bytes())));
        }
        _ => panic!("Wrong payload type after deserialization"),
    }
}

#[test]
fn test_unstake_tx_hex_json_serialization() {
    // Create a signing key for testing using known test bytes
    let signing_key = throwaway_signing_key();
    let verifying_key = signing_key.verifying_key();
    let sender = Address::from_public_key(&verifying_key)
        .expect("Failed to derive address from public key in test");

    // Create an unstake transaction
    let unstake = UnstakeTx::new(sender, 1000.into()).expect("valid unstake");

    // Create and sign the transaction
    let mut tx = Tx {
        sig: TxSig::empty(),
        nonce: 1u32.into(),
        payload: Payload::new(unstake),
        public_key: TxPublicKey::from(&verifying_key),
        fee: DEFAULT_TX_FEE.into(),
    };

    // Sign the transaction
    tx.sign(&signing_key, MOCK_CHAIN_ID).expect("sign");

    // Convert to hex-encoded JSON
    let tx_json = serde_json::to_string(&tx).expect("Failed to serialize tx to JSON");
    info!("Original JSON: {}", tx_json);
    let tx_hex = hex::encode(tx_json.as_bytes());

    // Convert back from hex-encoded JSON
    let tx_json_bytes = hex::decode(tx_hex).expect("Failed to decode hex");
    let tx_json_str = String::from_utf8(tx_json_bytes).expect("Failed to convert bytes to string");
    info!("Decoded JSON: {}", tx_json_str);
    let decoded_tx: Tx =
        serde_json::from_str(&tx_json_str).expect("Failed to deserialize JSON to tx");
    info!(
        "Re-encoded JSON: {}",
        serde_json::to_string(&decoded_tx)
            .expect("Failed to serialize decoded transaction to JSON in test")
    );

    // Verify the decoded transaction
    assert!(
        decoded_tx.verify(MOCK_CHAIN_ID).expect("verify"),
        "Decoded transaction verification failed"
    );

    // Compare original and decoded transactions
    assert_eq!(tx.sig, decoded_tx.sig, "Transaction signature mismatch");
    assert_eq!(tx.nonce, decoded_tx.nonce, "Transaction nonce mismatch");
    assert_eq!(
        tx.public_key, decoded_tx.public_key,
        "Transaction public key mismatch"
    );
    assert_eq!(
        tx.payload, decoded_tx.payload,
        "Transaction payload mismatch"
    );

    // Verify specific unstake details
    match decoded_tx.payload.inner {
        PayloadInner::Unstake(u) => {
            assert_eq!(u.sender, sender);
            assert_eq!(u.amount, 1000);
        }
        _ => panic!("Wrong payload type after deserialization"),
    }
}

#[test]
fn test_chain_id_verification() {
    let signing_key = throwaway_signing_key();
    let verifying_key = signing_key.verifying_key();
    let sender = Address::from_public_key(&verifying_key)
        .expect("Failed to derive address from public key in test");
    let recipient = Address::parse_hex_str("0x4567890123456789012345678901234567890123")
        .expect("valid recipient in test");

    let transfer = TransferTx::new(sender, recipient, 100.into()).expect("valid transfer");

    let mut tx = Tx {
        sig: TxSig::empty(),
        nonce: 1u32.into(),
        payload: Payload::new(transfer),
        public_key: TxPublicKey::from(&verifying_key),
        fee: DEFAULT_TX_FEE.into(),
    };

    // Sign with MOCK_CHAIN_ID
    tx.sign(&signing_key, MOCK_CHAIN_ID).expect("sign");

    // Verify with same chain ID should succeed
    assert!(
        tx.verify(MOCK_CHAIN_ID).expect("verify"),
        "Transaction should verify with correct chain ID"
    );

    // Verify with different chain ID should fail
    assert!(
        !tx.verify("different-chain").expect("verify"),
        "Transaction should not verify with wrong chain ID"
    );

    // Verify with empty chain ID should fail
    assert!(
        !tx.verify("").expect("verify"),
        "Transaction should not verify with empty chain ID"
    );
}

#[test]
fn test_post_message_validate_signature_and_build_tx() {
    let user_secret_bytes = [7u8; 32];
    let user_signing_key = SigningKey::from_bytes(&user_secret_bytes);
    let user_verifying_key = user_signing_key.verifying_key();
    let original_signer_addr = Address::from_public_key(&user_verifying_key)
        .expect("Failed to derive user address in test");

    let message_bytes = b"hello pinboard";

    let user_request = build_signed_post_message_user_request(
        &user_signing_key,
        message_bytes,
        PostMessageUserRequestInput {
            expires_height: 123_456,
            visibility: "Public".to_string(),
            topic: Some("general".to_string()),
            tags: vec![],
            content_type: "text/plain".to_string(),
            fee_amount: 1000,
            namespace: None,
        },
    )
    .expect("build user request");

    validate_post_message_user_signature(message_bytes, &user_request)
        .expect("User signature should validate");

    assert_eq!(
        user_request.content_key,
        calculate_message_content_key(message_bytes)
    );

    let validator_secret_bytes = [9u8; 32];
    let validator_signing_key = SigningKey::from_bytes(&validator_secret_bytes);
    let tx = validate_message_and_build_post_message_tx(
        message_bytes,
        &user_request,
        &validator_signing_key,
        7,
        1_710_000_000,
        MOCK_CHAIN_ID,
        DEFAULT_TX_FEE,
    )
    .expect("Failed to build PostMessage tx");

    assert_eq!(
        tx.payload.r#type,
        tx_type::TX_TYPE_POST_MESSAGE.to_string(),
        "Unexpected tx payload type"
    );
    assert!(
        tx.verify(MOCK_CHAIN_ID).expect("verify"),
        "Built tx should verify"
    );

    match tx.payload.inner {
        PayloadInner::PostMessage(post_message_tx) => {
            assert_eq!(post_message_tx.original_signer, original_signer_addr);
            assert_eq!(post_message_tx.content_key, user_request.content_key);
            assert_eq!(post_message_tx.message_id, user_request.message_id);
            assert_eq!(post_message_tx.visibility, "Public");
            assert_eq!(post_message_tx.topic, Some("general".to_string()));
            assert_eq!(post_message_tx.content_type, "text/plain");
            assert_eq!(post_message_tx.fee_amount, 1000);
        }
        _ => panic!("Wrong payload type after building PostMessage tx"),
    }
}

#[test]
fn test_post_message_validation_rejects_signer_pubkey_mismatch() {
    let user_secret_bytes = [21u8; 32];
    let user_signing_key = SigningKey::from_bytes(&user_secret_bytes);
    let message_bytes = b"hello";

    let mut user_request = build_signed_post_message_user_request(
        &user_signing_key,
        message_bytes,
        PostMessageUserRequestInput {
            expires_height: 42,
            visibility: "Public".to_string(),
            topic: None,
            tags: vec![],
            content_type: "text/plain".to_string(),
            fee_amount: 1000,
            namespace: None,
        },
    )
    .expect("build user request");

    user_request.original_signer = "0x1234567890123456789012345678901234567890".to_string();
    let err = validate_post_message_user_signature(message_bytes, &user_request)
        .expect_err("mismatched signer should fail");
    assert_eq!(err, "original_signer does not match original_signer_pubkey");
}

#[test]
fn test_post_message_validation_rejects_tampered_message() {
    let user_secret_bytes = [17u8; 32];
    let user_signing_key = SigningKey::from_bytes(&user_secret_bytes);

    let original_message = b"original message";
    let user_request = build_signed_post_message_user_request(
        &user_signing_key,
        original_message,
        PostMessageUserRequestInput {
            expires_height: 321_000,
            visibility: "Public".to_string(),
            topic: None,
            tags: vec![],
            content_type: "application/json".to_string(),
            fee_amount: 1000,
            namespace: None,
        },
    )
    .expect("build user request");

    let tampered_message = b"tampered message";
    let validation_result = validate_post_message_user_signature(tampered_message, &user_request);
    assert!(
        validation_result.is_err(),
        "Validation should fail when message bytes are changed"
    );
}

#[test]
fn test_post_message_namespace_affects_message_id() {
    let user_secret_bytes = [11u8; 32];
    let user_signing_key = SigningKey::from_bytes(&user_secret_bytes);
    let message_bytes = b"namespace test";

    let without = build_signed_post_message_user_request(
        &user_signing_key,
        message_bytes,
        PostMessageUserRequestInput {
            expires_height: 100,
            visibility: "Public".to_string(),
            topic: None,
            tags: vec![],
            content_type: "text/plain".to_string(),
            fee_amount: 1000,
            namespace: None,
        },
    )
    .expect("without namespace");

    let with_ns = build_signed_post_message_user_request(
        &user_signing_key,
        message_bytes,
        PostMessageUserRequestInput {
            expires_height: 100,
            visibility: "Public".to_string(),
            topic: None,
            tags: vec![],
            content_type: "text/plain".to_string(),
            fee_amount: 1000,
            namespace: Some("peterpan".to_string()),
        },
    )
    .expect("with namespace");

    assert_ne!(without.message_id, with_ns.message_id);
    assert_eq!(with_ns.namespace, Some("peterpan".to_string()));
}

#[test]
fn test_validate_post_message_content_type_allowlist() {
    validate_post_message_content_type("text/plain").expect("text/plain");
    validate_post_message_content_type("application/json").expect("application/json");
    validate_post_message_content_type("image/png").expect("image/png");
    assert!(validate_post_message_content_type("").is_err());
    assert!(validate_post_message_content_type(" ").is_err());
    validate_post_message_content_type(" text/plain").expect("trim spaces");
    validate_post_message_content_type("\timage/png\n").expect("trim ws");
    assert!(validate_post_message_content_type("text/html").is_err());
    assert!(validate_post_message_content_type("image/jpeg").is_err());
}

#[test]
fn test_canonicalize_post_message_tags_normalizes_address_hex_case() {
    let out =
        canonicalize_post_message_tags(&["0x0123456789ABCDEF0123456789ABCDEF01234567".to_string()])
            .expect("canonical tags");
    assert_eq!(out[0], "0x0123456789abcdef0123456789abcdef01234567");
}

#[test]
fn test_canonicalize_post_message_tags_non_address_unchanged() {
    let out = canonicalize_post_message_tags(&["dapp-inbox".to_string()]).expect("canonical tags");
    assert_eq!(out[0], "dapp-inbox");
}

#[test]
fn test_canonicalize_post_message_tags_rejects_too_many() {
    let tags: Vec<String> = (0..POST_MESSAGE_MAX_TAGS + 1)
        .map(|i| format!("t{i}"))
        .collect();
    assert!(canonicalize_post_message_tags(&tags).is_err());
}

#[test]
fn test_canonicalize_post_message_tags_rejects_oversized_tag() {
    let tag = "a".repeat(POST_MESSAGE_MAX_TAG_UTF8_BYTES + 1);
    assert!(canonicalize_post_message_tags(&[tag]).is_err());
}

#[test]
fn test_add_namespace_tx_sign_verify_hex_json() {
    let signing_key = throwaway_signing_key();
    let verifying_key = signing_key.verifying_key();
    let sender = Address::from_public_key(&verifying_key)
        .expect("Failed to derive address from public key in test");

    let add_namespace =
        AddNamespaceTx::new(sender, "peter".to_string(), 1.into()).expect("valid add_namespace");
    let mut tx = Tx {
        sig: TxSig::empty(),
        nonce: 1u32.into(),
        payload: Payload::new(add_namespace),
        public_key: TxPublicKey::from(&verifying_key),
        fee: DEFAULT_TX_FEE.into(),
    };

    tx.sign(&signing_key, MOCK_CHAIN_ID).expect("sign");
    assert!(
        tx.verify(MOCK_CHAIN_ID).expect("verify"),
        "Original AddNamespace transaction verification failed"
    );
    assert_eq!(
        tx.payload.r#type,
        tx_type::TX_TYPE_ADD_NAMESPACE.to_string()
    );

    let tx_json = serde_json::to_string(&tx).expect("serialize tx");
    let tx_hex = hex::encode(tx_json.as_bytes());
    let decoded_tx: Tx = serde_json::from_str(
        &String::from_utf8(hex::decode(tx_hex).expect("decode hex")).expect("utf8"),
    )
    .expect("deserialize tx");

    assert!(
        decoded_tx.verify(MOCK_CHAIN_ID).expect("verify"),
        "Decoded AddNamespace transaction verification failed"
    );

    match decoded_tx.payload.inner {
        PayloadInner::AddNamespace(t) => {
            assert_eq!(t.sender, sender);
            assert_eq!(t.namespace_slug, "peter");
            assert_eq!(t.registration_fee, TxAmount(1));
        }
        _ => panic!("Wrong payload type after AddNamespace deserialization"),
    }
}

#[test]
fn tx_public_key_try_from_rejects_invalid_hex() {
    let err = TxPublicKey::try_from("not-hex").expect_err("invalid hex");
    assert!(err.to_string().contains("public_key"));
}

#[test]
fn tx_sig_try_from_rejects_wrong_length() {
    let err = TxSig::try_from("aa").expect_err("too short");
    assert!(err.to_string().contains("signature"));
}

#[test]
fn tx_sig_try_from_empty_is_unsigned() {
    let sig = TxSig::try_from("").expect("empty signature is unsigned");
    assert_eq!(sig, TxSig::empty());
}

#[test]
fn verify_returns_err_for_unsigned_tx() {
    let signing_key = throwaway_signing_key();
    let verifying_key = signing_key.verifying_key();
    let sender = Address::from_public_key(&verifying_key)
        .expect("Failed to derive address from public key in test");
    let recipient = Address::parse_hex_str("0x0987654321098765432109876543210987654321")
        .expect("valid recipient in test");
    let transfer = TransferTx::new(sender, recipient, 1.into()).expect("valid transfer");
    let tx = Tx {
        sig: TxSig::empty(),
        nonce: 1u32.into(),
        payload: Payload::new(transfer),
        public_key: TxPublicKey::from(&verifying_key),
        fee: DEFAULT_TX_FEE.into(),
    };

    assert!(tx.verify(MOCK_CHAIN_ID).is_err());
}
