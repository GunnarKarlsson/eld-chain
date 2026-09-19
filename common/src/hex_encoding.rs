//! Shared hex parse rules for ID types.
//!
//! Input accepts an optional `0x` / `0X` prefix and case-insensitive hex digits.
//! Canonical *output* (Display / serde) is defined by each type so existing clients
//! keep seeing the same wire form they already depend on.

use crate::error::EldError;

/// Strips a leading `0x` or `0X` if present. `x` is not a hex digit, so this is unambiguous.
#[must_use]
pub(crate) fn strip_optional_hex_prefix(s: &str) -> &str {
    match s.as_bytes() {
        [b'0', b'x' | b'X', ..] => &s[2..],
        _ => s,
    }
}

/// Decodes exactly `N` bytes from hex. Prefix is optional; digits are case-insensitive.
///
/// # Errors
///
/// Returns [`EldError::ValidationError`] if the string is empty after an optional prefix,
/// is not valid hex, or is not exactly `N` bytes.
pub(crate) fn decode_fixed_hex<const N: usize>(s: &str, field: &str) -> Result<[u8; N], EldError> {
    let hex_body = strip_optional_hex_prefix(s);
    if hex_body.is_empty() {
        return Err(EldError::ValidationError {
            field: field.to_string(),
            value: s.to_string(),
            details: format!("{field} hex cannot be empty"),
        });
    }

    let decoded = hex::decode(hex_body).map_err(|e| EldError::ValidationError {
        field: field.to_string(),
        value: s.to_string(),
        details: format!("{field} must be valid hex (optional 0x prefix): {e}"),
    })?;

    let bytes: [u8; N] = decoded
        .as_slice()
        .try_into()
        .map_err(|_| EldError::ValidationError {
            field: field.to_string(),
            value: s.to_string(),
            details: format!(
                "{field} must be {N} bytes ({} hex digits), got {} bytes",
                N * 2,
                decoded.len()
            ),
        })?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE32: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const SAMPLE20: &str = "1234567890abcdef1234567890abcdef12345678";

    #[test]
    fn strip_optional_hex_prefix_accepts_0x_0x_upper_and_bare() {
        assert_eq!(strip_optional_hex_prefix("0xabc"), "abc");
        assert_eq!(strip_optional_hex_prefix("0Xabc"), "abc");
        assert_eq!(strip_optional_hex_prefix("abc"), "abc");
        assert_eq!(strip_optional_hex_prefix(""), "");
    }

    #[test]
    fn decode_fixed_hex_accepts_prefixed_and_bare() {
        let prefixed = decode_fixed_hex::<32>(&format!("0x{SAMPLE32}"), "id").expect("prefixed");
        let upper_prefix =
            decode_fixed_hex::<32>(&format!("0X{SAMPLE32}"), "id").expect("0X prefix");
        let bare = decode_fixed_hex::<32>(SAMPLE32, "id").expect("bare");
        assert_eq!(prefixed, bare);
        assert_eq!(upper_prefix, bare);
        assert_eq!(hex::encode(bare), SAMPLE32);
    }

    #[test]
    fn decode_fixed_hex_address_length() {
        let bytes = decode_fixed_hex::<20>(SAMPLE20, "address").expect("20-byte hex");
        assert_eq!(hex::encode(bytes), SAMPLE20);
        assert!(decode_fixed_hex::<20>(&format!("0x{SAMPLE20}"), "address").is_ok());
    }

    #[test]
    fn decode_fixed_hex_rejects_empty_wrong_length_and_invalid() {
        assert!(decode_fixed_hex::<32>("", "id").is_err());
        assert!(decode_fixed_hex::<32>("0x", "id").is_err());
        assert!(decode_fixed_hex::<32>("00", "id").is_err());
        assert!(decode_fixed_hex::<32>(&format!("0x{SAMPLE32}gg"), "id").is_err());
    }
}
