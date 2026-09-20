//! Helpers to redact sensitive values in log fields.
//!
//! Process-wide subscriber setup lives in `eld_client::logging::init_default_logging`.

use std::fmt;

/// Sanitizes sensitive data for logging by truncating or masking sensitive information
pub struct LogSanitizer;

impl LogSanitizer {
    /// Sanitizes a CADO key by showing only the first 8 and last 4 characters
    /// Example: "abc123def456" -> "abc123...def456"
    pub fn sanitize_cado_key(key: &str) -> String {
        if key.len() <= 12 {
            return key.to_string();
        }
        format!("{}...{}", &key[..8], &key[key.len() - 4..])
    }

    /// Sanitizes a file path by showing only the directory structure and filename
    /// Example: "/home/user/.eld/config.json" -> "/.../config.json"
    pub fn sanitize_path(path: &str) -> String {
        if path.is_empty() {
            return path.to_string();
        }

        let path_components: Vec<&str> = path.split('/').collect();
        if path_components.len() <= 2 {
            return path.to_string();
        }

        // Show only the last component (filename) and indicate directory structure
        if let Some(filename) = path_components.last() {
            format!("/.../{filename}")
        } else {
            path.to_string()
        }
    }

    /// Sanitizes an address by showing only the first 6 and last 4 characters
    /// Example: "0x1234567890abcdef1234567890abcdef12345678" -> "0x1234...5678"
    pub fn sanitize_address(address: &str) -> String {
        if !address.starts_with("0x") || address.len() <= 10 {
            return address.to_string();
        }

        let hex_part = &address[2..]; // Remove 0x prefix
        if hex_part.len() <= 8 {
            return address.to_string();
        }

        format!("0x{}...{}", &hex_part[..4], &hex_part[hex_part.len() - 4..])
    }

    /// Sanitizes a hash by showing only the first 8 and last 4 characters
    /// Example: "a1b2c3d4e5f6789012345678901234567890abcdef" -> "a1b2c3d4...cdef"
    pub fn sanitize_hash(hash: &str) -> String {
        if hash.len() <= 12 {
            return hash.to_string();
        }
        format!("{}...{}", &hash[..8], &hash[hash.len() - 4..])
    }

    /// Sanitizes a public key by showing only the first 8 and last 4 characters
    /// Example: "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef" -> "12345678...cdef"
    pub fn sanitize_public_key(pubkey: &str) -> String {
        if pubkey.len() <= 12 {
            return pubkey.to_string();
        }
        format!("{}...{}", &pubkey[..8], &pubkey[pubkey.len() - 4..])
    }

    /// Sanitizes a signature by showing only the first 8 and last 4 characters
    /// Example: "sig1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef" -> "sig12345...cdef"
    pub fn sanitize_signature(signature: &str) -> String {
        if signature.len() <= 12 {
            return signature.to_string();
        }
        format!(
            "{}...{}",
            &signature[..8],
            &signature[signature.len() - 4..]
        )
    }

    /// Sanitizes a manifest ID by showing only the first 8 and last 4 characters
    /// Example: "manifest_1234567890abcdef1234567890abcdef" -> "manifest...cdef"
    pub fn sanitize_manifest_id(manifest_id: &str) -> String {
        if manifest_id.len() <= 12 {
            return manifest_id.to_string();
        }
        format!(
            "{}...{}",
            &manifest_id[..8],
            &manifest_id[manifest_id.len() - 4..]
        )
    }

    /// Generic sanitization that attempts to identify the type of data and sanitize accordingly
    pub fn sanitize_generic(data: &str) -> String {
        if data.starts_with("0x") && data.len() > 10 {
            // Likely an address or hash
            Self::sanitize_address(data)
        } else if data.contains('/') && data.len() > 20 {
            // Likely a path
            Self::sanitize_path(data)
        } else if data.len() > 12 {
            // Generic truncation for other sensitive data
            format!("{}...{}", &data[..8], &data[data.len() - 4..])
        } else {
            data.to_string()
        }
    }
}

/// Wrapper for logging sensitive data with automatic sanitization
pub struct SanitizedLog {
    sanitized: String,
}

impl SanitizedLog {
    pub fn new<T: ToString>(data: T) -> Self {
        let sanitized = LogSanitizer::sanitize_generic(&data.to_string());
        Self { sanitized }
    }

    pub fn as_cado_key<T: ToString>(data: T) -> Self {
        let sanitized = LogSanitizer::sanitize_cado_key(&data.to_string());
        Self { sanitized }
    }

    pub fn as_path<T: ToString>(data: T) -> Self {
        let sanitized = LogSanitizer::sanitize_path(&data.to_string());
        Self { sanitized }
    }

    pub fn as_address<T: ToString>(data: T) -> Self {
        let sanitized = LogSanitizer::sanitize_address(&data.to_string());
        Self { sanitized }
    }

    pub fn as_hash<T: ToString>(data: T) -> Self {
        let sanitized = LogSanitizer::sanitize_hash(&data.to_string());
        Self { sanitized }
    }

    pub fn as_public_key<T: ToString>(data: T) -> Self {
        let sanitized = LogSanitizer::sanitize_public_key(&data.to_string());
        Self { sanitized }
    }

    pub fn as_signature<T: ToString>(data: T) -> Self {
        let sanitized = LogSanitizer::sanitize_signature(&data.to_string());
        Self { sanitized }
    }

    pub fn as_manifest_id<T: ToString>(data: T) -> Self {
        let sanitized = LogSanitizer::sanitize_manifest_id(&data.to_string());
        Self { sanitized }
    }
}

impl fmt::Display for SanitizedLog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.sanitized)
    }
}

impl fmt::Debug for SanitizedLog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.sanitized)
    }
}

/// Trait for types that can provide sanitized logging
pub trait SanitizedLoggable {
    fn sanitized_log(&self) -> String;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Example home config path for path-sanitizer tests (not a real on-disk location).
    const EXAMPLE_USER_CONFIG_PATH: &str = "/home/user/.eld/config.json";

    #[test]
    fn test_sanitize_cado_key() {
        assert_eq!(
            LogSanitizer::sanitize_cado_key("abc123def456"),
            "abc123def456"
        );
        assert_eq!(
            LogSanitizer::sanitize_cado_key("abc123def456789"),
            "abc123de...6789"
        );
        assert_eq!(LogSanitizer::sanitize_cado_key("short"), "short");
    }

    #[test]
    fn test_sanitize_path() {
        assert_eq!(
            LogSanitizer::sanitize_path(EXAMPLE_USER_CONFIG_PATH),
            "/.../config.json"
        );
        assert_eq!(LogSanitizer::sanitize_path("/config.json"), "/config.json");
    }

    #[test]
    fn test_sanitize_address() {
        assert_eq!(
            LogSanitizer::sanitize_address("0x1234567890abcdef1234567890abcdef12345678"),
            "0x1234...5678"
        );
        assert_eq!(LogSanitizer::sanitize_address("0x12345678"), "0x12345678");
    }

    #[test]
    fn test_sanitize_hash() {
        assert_eq!(
            LogSanitizer::sanitize_hash("a1b2c3d4e5f6789012345678901234567890abcdef"),
            "a1b2c3d4...cdef"
        );
        assert_eq!(LogSanitizer::sanitize_hash("short"), "short");
    }

    #[test]
    fn test_sanitize_public_key() {
        assert_eq!(
            LogSanitizer::sanitize_public_key(
                "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
            ),
            "12345678...cdef"
        );
        assert_eq!(LogSanitizer::sanitize_public_key("short"), "short");
    }

    #[test]
    fn test_sanitize_signature() {
        assert_eq!(
            LogSanitizer::sanitize_signature(
                "sig1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
            ),
            "sig12345...cdef"
        );
        assert_eq!(LogSanitizer::sanitize_signature("short"), "short");
    }

    #[test]
    fn test_sanitize_manifest_id() {
        assert_eq!(
            LogSanitizer::sanitize_manifest_id("manifest_1234567890abcdef1234567890abcdef"),
            "manifest...cdef"
        );
        assert_eq!(LogSanitizer::sanitize_manifest_id("short"), "short");
    }

    #[test]
    fn test_sanitize_generic() {
        assert_eq!(
            LogSanitizer::sanitize_generic("0x1234567890abcdef1234567890abcdef12345678"),
            "0x1234...5678"
        );
        assert_eq!(
            LogSanitizer::sanitize_generic(EXAMPLE_USER_CONFIG_PATH),
            "/.../config.json"
        );
        assert_eq!(
            LogSanitizer::sanitize_generic("very_long_sensitive_data_that_should_be_truncated"),
            "very_lon...ated"
        );
    }

    #[test]
    fn test_sanitized_log_wrapper() {
        let sanitized = SanitizedLog::as_cado_key("abc123def456");
        assert_eq!(sanitized.to_string(), "abc123def456");

        let sanitized = SanitizedLog::as_cado_key("abc123def456789");
        assert_eq!(sanitized.to_string(), "abc123de...6789");

        let sanitized = SanitizedLog::as_address("0x1234567890abcdef1234567890abcdef12345678");
        assert_eq!(sanitized.to_string(), "0x1234...5678");

        let sanitized = SanitizedLog::as_path(EXAMPLE_USER_CONFIG_PATH);
        assert_eq!(sanitized.to_string(), "/.../config.json");
    }
}
