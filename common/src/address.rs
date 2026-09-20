use crate::error::EldError;
use crate::hex_encoding::decode_fixed_hex;
use crate::public_key::PublicKey;
use ed25519_dalek::VerifyingKey;
use hex;
use serde::de::{Error, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Copy, Default)]
pub struct Address {
    value: [u8; 20],
}

impl Address {
    const LENGTH: usize = 20;

    pub fn from_public_key(public_key: &VerifyingKey) -> Result<Self, EldError> {
        let pk_bytes = public_key.to_bytes();
        let hash = Sha256::digest(pk_bytes);
        Self::from_slice(&hash[..Self::LENGTH])
    }

    pub fn is_from_public_key(&self, other_public_key: &[u8; PublicKey::LEN]) -> bool {
        let hash = Sha256::digest(other_public_key);
        let other = &hash[..Self::LENGTH];
        self.value == other
    }

    // New hex() method
    pub fn hex(&self) -> String {
        hex::encode(self.value)
    }

    pub fn hex_with_prefix(&self) -> String {
        format!("0x{}", hex::encode(self.value))
    }

    /// Canonical wire form (`0x` + lowercase hex).
    pub fn canonical_hex_with_prefix(&self) -> String {
        self.hex_with_prefix()
    }

    /// Parses an Eld account address from a hex string.
    ///
    /// Accepts an optional `0x` or `0X` prefix followed by exactly 40 hexadecimal
    /// characters (20 bytes). Hex digits may be upper or lower case.
    ///
    /// Examples: `"0x1234…"`, `"1234…"` (without prefix). Canonical [`fmt::Display`] /
    /// serde output stays `0x` + lowercase.
    pub fn parse_hex_str(s: &str) -> Result<Self, EldError> {
        let value = decode_fixed_hex::<{ Self::LENGTH }>(s, "address")?;
        Ok(Address { value })
    }

    /// Builds an address from exactly 20 bytes.
    pub fn from_bytes(bytes: [u8; Self::LENGTH]) -> Self {
        Address { value: bytes }
    }

    /// Returns a borrowed view of the 20-byte address value.
    pub fn as_bytes(&self) -> &[u8; Self::LENGTH] {
        &self.value
    }

    pub fn matches_str(&self, other: &str) -> bool {
        Self::parse_hex_str(other).is_ok_and(|candidate| candidate == *self)
    }

    pub fn verify_derives_from_pubkey_hex(&self, pubkey_hex: &str) -> Result<(), EldError> {
        let public_key_bytes = hex::decode(pubkey_hex).map_err(|e| EldError::ValidationError {
            field: "public_key".to_string(),
            value: pubkey_hex.to_string(),
            details: format!("Invalid public key hex format: {e}"),
        })?;

        let public_key: [u8; PublicKey::LEN] =
            public_key_bytes
                .as_slice()
                .try_into()
                .map_err(|_| EldError::ValidationError {
                    field: "public_key".to_string(),
                    value: pubkey_hex.to_string(),
                    details: format!("Public key must be exactly {} bytes", PublicKey::LEN),
                })?;

        if self.is_from_public_key(&public_key) {
            Ok(())
        } else {
            Err(EldError::ValidationError {
                field: "address".to_string(),
                value: self.hex_with_prefix(),
                details: "Address does not derive from the provided public key".to_string(),
            })
        }
    }

    fn from_slice(bytes: &[u8]) -> Result<Self, EldError> {
        if bytes.len() != Self::LENGTH {
            return Err(EldError::ValidationError {
                field: "address".to_string(),
                value: format!("{} bytes", bytes.len()),
                details: format!("Invalid address length: {}, expected 20", bytes.len()),
            });
        }
        let mut value = [0u8; Self::LENGTH];
        value.copy_from_slice(bytes);
        Ok(Address { value })
    }
}

impl Hash for Address {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.value.hash(state);
    }
}

impl Serialize for Address {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Serialize as hex string with 0x prefix (matches Display)
        serializer.serialize_str(&self.hex_with_prefix())
    }
}

impl<'de> Deserialize<'de> for Address {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct AddressVisitor;

        impl<'de> Visitor<'de> for AddressVisitor {
            type Value = Address;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a hex-encoded address string (with or without 0x prefix)")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: Error,
            {
                Address::parse_hex_str(value).map_err(Error::custom)
            }
        }

        deserializer.deserialize_str(AddressVisitor)
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.hex_with_prefix())
    }
}

impl fmt::Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Address({})", self.hex_with_prefix())
    }
}

impl FromStr for Address {
    type Err = EldError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_hex_str(s)
    }
}

impl TryFrom<&str> for Address {
    type Error = EldError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::parse_hex_str(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::RngCore;

    #[test]
    fn test_derive_pubk_and_address() {
        // Documented throwaway seed (all 0x02). Not a live-network key.
        let signing_key = SigningKey::from_bytes(&[0x02u8; PublicKey::LEN]);
        let verifying_key = signing_key.verifying_key();
        let _address = Address::from_public_key(&verifying_key).expect("Address from pubk");
    }

    #[test]
    fn test_derivation_from_public_key() {
        let mut rng = rand::rng(); // Thread-local RNG
        let mut secret_bytes = [0u8; PublicKey::LEN]; // Ed25519 secret key length
        rng.fill_bytes(&mut secret_bytes); // Fill with random bytes
        let signing_key = SigningKey::from_bytes(&secret_bytes); // Construct SigningKey
        let verifying_key = signing_key.verifying_key();
        let address = Address::from_public_key(&verifying_key).expect("Address from pubk");
        assert!(address.is_from_public_key(verifying_key.as_bytes()));
        assert!(address.is_from_public_key(&verifying_key.to_bytes()));
    }

    #[test]
    fn test_derive_pubk_and_address_with_osrng() {
        let mut rng = rand::rng();
        let mut secret_bytes = [0u8; PublicKey::LEN];
        rng.fill_bytes(&mut secret_bytes);
        let signing_key = SigningKey::from_bytes(&secret_bytes);
        let verifying_key = signing_key.verifying_key();
        let address = Address::from_public_key(&verifying_key).expect("Address from pubk");
        assert_eq!(address.value.len(), 20);
    }

    #[test]
    fn test_derivation_from_public_key_with_osrng() {
        let mut rng = rand::rng();
        let mut secret_bytes = [0u8; PublicKey::LEN];
        rng.fill_bytes(&mut secret_bytes);
        let signing_key = SigningKey::from_bytes(&secret_bytes);
        let verifying_key = signing_key.verifying_key();
        let address = Address::from_public_key(&verifying_key).expect("Address from pubk");
        assert!(address.is_from_public_key(&verifying_key.to_bytes()));
        assert!(address.is_from_public_key(verifying_key.as_bytes()));
    }

    #[test]
    fn test_address_consistency_with_ed25519_dalek_2_2_0() {
        // Test that Address::from_public_key works consistently with ed25519-dalek 2.2.0
        let mut rng = rand::rng();
        let mut secret_bytes = [0u8; PublicKey::LEN];
        rng.fill_bytes(&mut secret_bytes);
        let signing_key = SigningKey::from_bytes(&secret_bytes);
        let verifying_key = signing_key.verifying_key();

        // Test that to_bytes() method works correctly
        let pk_bytes = verifying_key.to_bytes();
        assert_eq!(pk_bytes.len(), PublicKey::LEN);

        // Test address derivation
        let address1 = Address::from_public_key(&verifying_key).expect("Address from pubk");
        let address2 = Address::from_public_key(&verifying_key).expect("Address from pubk");

        // Addresses should be consistent for the same public key
        assert_eq!(address1, address2);
        assert_eq!(address1.value.len(), 20);
    }

    #[test]
    fn test_verifying_key_compatibility() {
        // Test that VerifyingKey methods are compatible with ed25519-dalek 2.2.0
        let mut rng = rand::rng();
        let mut secret_bytes = [0u8; PublicKey::LEN];
        rng.fill_bytes(&mut secret_bytes);
        let signing_key = SigningKey::from_bytes(&secret_bytes);
        let verifying_key = signing_key.verifying_key();

        // Test to_bytes() method
        let pk_bytes = verifying_key.to_bytes();
        assert_eq!(pk_bytes.len(), PublicKey::LEN);

        // Test as_bytes() method (if available)
        let pk_bytes_alt = verifying_key.as_bytes();
        assert_eq!(&pk_bytes, pk_bytes_alt);

        // Test address derivation works with both methods
        let address1 = Address::from_public_key(&verifying_key).expect("Address from pubk");
        let hash = Sha256::digest(pk_bytes);
        let hash_prefix: [u8; 20] = hash[..20]
            .try_into()
            .expect("hash prefix should always be 20 bytes");
        let address2 = Address::from_bytes(hash_prefix);

        assert_eq!(address1, address2);
    }

    #[test]
    fn test_address_serialization() {
        // Test serialization to JSON
        let address = Address::parse_hex_str("0x1234567890abcdef1234567890abcdef12345678")
            .expect("Valid address");

        let json = serde_json::to_string(&address).expect("Should serialize");
        assert_eq!(json, "\"0x1234567890abcdef1234567890abcdef12345678\"");

        // Test deserialization from JSON with 0x prefix
        let deserialized: Address = serde_json::from_str(&json).expect("Should deserialize");
        assert_eq!(address, deserialized);

        // Test deserialization from JSON without 0x prefix
        let json_no_prefix = "\"1234567890abcdef1234567890abcdef12345678\"";
        let deserialized_no_prefix: Address =
            serde_json::from_str(json_no_prefix).expect("Should deserialize without prefix");
        assert_eq!(address, deserialized_no_prefix);
    }

    #[test]
    fn test_address_hash_eq_ord() {
        let addr1 = Address::parse_hex_str("0x1234567890abcdef1234567890abcdef12345678")
            .expect("Valid address");
        let addr2 = Address::parse_hex_str("0x1234567890abcdef1234567890abcdef12345678")
            .expect("Valid address");
        let addr3 = Address::parse_hex_str("0xabcdef1234567890abcdef1234567890abcdef12")
            .expect("Valid address");

        // Test Eq
        assert_eq!(addr1, addr2);
        assert_ne!(addr1, addr3);

        // Test Hash
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(addr1);
        assert!(set.contains(&addr2));
        assert!(!set.contains(&addr3));

        // Test Ord
        assert!(addr1 <= addr2);
        assert!(addr1 < addr3 || addr3 < addr1); // One must be less than the other
    }

    #[test]
    fn test_address_parse_hex_str_and_matches_str() {
        let address =
            Address::parse_hex_str("0x1234567890abcdef1234567890abcdef12345678").expect("valid");
        assert!(address.matches_str("1234567890abcdef1234567890abcdef12345678"));
        assert!(address.matches_str("0x1234567890abcdef1234567890abcdef12345678"));
        assert!(!address.matches_str("0x1234567890abcdef1234567890abcdef12345679"));
    }

    #[test]
    fn test_address_parse_hex_str_accepts_optional_prefix() {
        let with_prefix =
            Address::parse_hex_str("0x1234567890abcdef1234567890abcdef12345678").expect("valid");
        let without_prefix =
            Address::parse_hex_str("1234567890abcdef1234567890abcdef12345678").expect("valid");
        let uppercase_prefix =
            Address::parse_hex_str("0X1234567890abcdef1234567890abcdef12345678").expect("valid");

        assert_eq!(with_prefix, without_prefix);
        assert_eq!(with_prefix, uppercase_prefix);
    }

    #[test]
    fn test_address_parse_hex_str_error_messages() {
        let too_short =
            Address::parse_hex_str("0x").expect_err("too-short address should fail early");
        assert!(matches!(
            too_short,
            EldError::ValidationError { field, details, .. }
                if field == "address" && details == "address hex cannot be empty"
        ));

        let prefixed_length_without_prefix =
            Address::parse_hex_str("1234567890abcdef1234567890abcdef1234567890")
                .expect_err("42-char unprefixed address should fail");
        assert!(matches!(
            prefixed_length_without_prefix,
            EldError::ValidationError { field, details, .. }
                if field == "address" && details.contains("must be 20 bytes")
        ));

        let short = Address::parse_hex_str("0x1234567890abcdef1234567890abcdef123456")
            .expect_err("short address should fail");
        assert!(matches!(
            short,
            EldError::ValidationError { field, details, .. }
                if field == "address"
                    && (details.contains("must be 20 bytes") || details.contains("valid hex"))
        ));

        let invalid_hex = Address::parse_hex_str("0x1234567890abcdef1234567890abcdef1234567g")
            .expect_err("invalid hex should fail");
        assert!(matches!(
            invalid_hex,
            EldError::ValidationError { field, details, .. }
                if field == "address" && details.contains("valid hex")
        ));
    }

    #[test]
    fn test_address_parse_hex_str_matches_from_str_and_try_from() {
        let value = "0x1234567890abcdef1234567890abcdef12345678";
        let parsed_by_hex = Address::parse_hex_str(value).expect("parse_hex_str should parse");
        let parsed_by_from_str: Address = value.parse().expect("FromStr should parse");
        let parsed_by_try_from = Address::try_from(value).expect("TryFrom<&str> should parse");

        assert_eq!(parsed_by_hex, parsed_by_from_str);
        assert_eq!(parsed_by_hex, parsed_by_try_from);

        let without_prefix = "1234567890abcdef1234567890abcdef12345678";
        assert_eq!(
            Address::parse_hex_str(without_prefix).expect("optional prefix"),
            parsed_by_hex
        );
    }

    #[test]
    fn test_address_verify_derives_from_pubkey_hex() {
        let mut rng = rand::rng();
        let mut secret_bytes = [0u8; PublicKey::LEN];
        rng.fill_bytes(&mut secret_bytes);
        let signing_key = SigningKey::from_bytes(&secret_bytes);
        let verifying_key = signing_key.verifying_key();
        let pubkey_hex = hex::encode(verifying_key.to_bytes());
        let address = Address::from_public_key(&verifying_key).expect("Address from pubk");

        assert!(address.verify_derives_from_pubkey_hex(&pubkey_hex).is_ok());
        assert!(address
            .verify_derives_from_pubkey_hex(
                "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
            )
            .is_err());
    }
}
