//! Shared helpers for flow implementations.

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use tendermint::abci::EventAttribute;

pub(crate) fn decode_event_attribute(attribute: EventAttribute) -> (String, String) {
    let decoded_key_bytes = BASE64_STANDARD
        .decode(attribute.key_str().expect("Failed to get key string"))
        .expect("Failed to decode key bytes");
    let key = String::from_utf8(decoded_key_bytes.to_vec())
        .expect("Failed to convert key bytes to UTF-8");
    let decoded_value_bytes = BASE64_STANDARD
        .decode(attribute.value_str().expect("Failed to get value string"))
        .expect("Failed to decode value bytes");
    let value = String::from_utf8(decoded_value_bytes.to_vec())
        .expect("Failed to convert value bytes to UTF-8");
    (key, value)
}
