//! Decode JSON number arrays used on CADO wire responses.

use eld_common::error::EldError;
use serde_json::Value;

pub(crate) fn json_number_array_as_bytes(
    values: &[Value],
    field: &str,
) -> Result<Vec<u8>, EldError> {
    values
        .iter()
        .map(|v| {
            v.as_u64()
                .and_then(|n| u8::try_from(n).ok())
                .ok_or_else(|| EldError::ValidationError {
                    field: field.to_string(),
                    value: v.to_string(),
                    details: "CADO JSON array element is not a byte (0-255)".to_string(),
                })
        })
        .collect()
}
