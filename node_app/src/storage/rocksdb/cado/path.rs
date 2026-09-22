use eld_common::cado::CadoPath;
use eld_common::constants::cado::{
    ALLOWED_SCOPES, MAX_PATH_LENGTH, MAX_PREFIX_LENGTH, TYPE_ACCOUNT, TYPE_STAKING_ACCOUNT,
    TYPE_STORAGE_STAKING_ACCOUNT, VALID_TYPES,
};
use eld_common::error::EldError;
use tracing::{debug, warn};

use super::super::RocksDBStorage;

impl RocksDBStorage {
    /// Validates CADO path for security issues
    /// This function checks for path traversal attacks and other malicious patterns
    pub(crate) fn validate_cado_path_security(path: &CadoPath) -> Result<(), EldError> {
        Self::validate_cado_path_security_enhanced(path)
    }

    /// Enhanced CADO path validation with comprehensive security checks
    /// This function implements a whitelist approach and checks for various attack vectors
    pub(crate) fn validate_cado_path_security_enhanced(path: &CadoPath) -> Result<(), EldError> {
        match Self::validate_cado_path_internal(path) {
            Ok(()) => {
                debug!("CADO path validation passed: {}", path.as_str());
                Ok(())
            }
            Err(e) => {
                warn!("CADO path validation failed: {} - {}", path.as_str(), e);
                Err(e)
            }
        }
    }

    /// Internal CADO path validation implementation
    pub(super) fn validate_cado_path_internal(path: &CadoPath) -> Result<(), EldError> {
        let path_str = path.as_str();

        // 1. Path length limits (prevent resource exhaustion)
        if path_str.len() > MAX_PATH_LENGTH {
            return Err(EldError::ValidationError {
                field: "cado_path_length".to_string(),
                value: path_str.to_string(),
                details: format!("Path exceeds maximum length of {MAX_PATH_LENGTH} characters"),
            });
        }

        // 2. Null byte and control character validation
        if path_str.contains('\0') {
            return Err(EldError::ValidationError {
                field: "cado_path_null_bytes".to_string(),
                value: path_str.to_string(),
                details: "Path contains null bytes".to_string(),
            });
        }

        if path_str
            .chars()
            .any(|c| c.is_control() && c != '\t' && c != '\n' && c != '\r')
        {
            return Err(EldError::ValidationError {
                field: "cado_path_control_chars".to_string(),
                value: path_str.to_string(),
                details: "Path contains forbidden control characters".to_string(),
            });
        }

        // 3. Comprehensive path traversal detection
        let traversal_patterns = [
            "..",
            "//",
            "\\",
            "~",
            "..\\",
            "../",
            "\\..",
            "/..",
            "..\\",
            "..//",
            "//..",
            "\\..\\",
            "....",
            "..../",
            "....\\",
            "..%2f",
            "..%5c",
            "%2e%2e",
            "%2e%2e%2f",
            "%2e%2e%5c",
            "..%252f",
            "..%255c",
            "%252e%252e",
            "%252e%252e%252f",
            "%252e%252e%255c",
        ];

        for pattern in traversal_patterns.iter() {
            if path_str.to_lowercase().contains(pattern) {
                return Err(EldError::ValidationError {
                    field: "cado_path_traversal".to_string(),
                    value: path_str.to_string(),
                    details: format!("Path contains forbidden traversal pattern: {pattern}"),
                });
            }
        }

        // 4. Whitelist approach for path structure
        let path_parts: Vec<&str> = path_str.split('/').filter(|s| !s.is_empty()).collect();
        if path_parts.len() < 3 {
            return Err(EldError::ValidationError {
                field: "cado_path_structure".to_string(),
                value: path_str.to_string(),
                details: "Path must have at least 3 parts: /@scope/type/name".to_string(),
            });
        }

        // 5. Scope validation with whitelist
        let scope = path.scope();
        if !ALLOWED_SCOPES.contains(&scope) {
            return Err(EldError::ValidationError {
                field: "cado_path_scope".to_string(),
                value: scope.to_string(),
                details: format!("Scope '{scope}' is not allowed"),
            });
        }

        // 6. Type validation with whitelist
        let type_ = path.type_();
        if !VALID_TYPES.contains(&type_) {
            return Err(EldError::ValidationError {
                field: "cado_path_type".to_string(),
                value: type_.to_string(),
                details: format!("Invalid CADO type: {type_}"),
            });
        }

        // 7. Name format validation based on type
        let name = path.name();
        if name.is_empty() {
            return Err(EldError::ValidationError {
                field: "cado_path_name".to_string(),
                value: name.to_string(),
                details: "Name cannot be empty".to_string(),
            });
        }

        if type_ == TYPE_ACCOUNT
            || type_ == TYPE_STAKING_ACCOUNT
            || type_ == TYPE_STORAGE_STAKING_ACCOUNT
        {
            if !name.starts_with("0x")
                || hex::decode(&name[2..])
                    .map_err(|_| EldError::ValidationError {
                        field: "cado_path_name_hex".to_string(),
                        value: name.to_string(),
                        details: "Invalid hex format".to_string(),
                    })?
                    .len()
                    != 20
            {
                return Err(EldError::ValidationError {
                    field: "cado_path_name_address".to_string(),
                    value: name.to_string(),
                    details: format!("Invalid address format for {type_}"),
                });
            }
        } else if type_ == eld_common::constants::cado::TYPE_NAMESPACE {
            eld_common::namespace::validate_namespace_slug(name)?;
        } else {
            // For other types, validate as hex
            if !name.starts_with("0x")
                || hex::decode(&name[2..])
                    .map_err(|_| EldError::ValidationError {
                        field: "cado_path_name_hex".to_string(),
                        value: name.to_string(),
                        details: "Invalid hex format".to_string(),
                    })?
                    .len()
                    != 32
            {
                return Err(EldError::ValidationError {
                    field: "cado_path_name_hex".to_string(),
                    value: name.to_string(),
                    details: format!("Invalid hex format for {type_}"),
                });
            }
        }

        // 8. Additional security checks
        if path_str.contains("//") || path_str.contains("\\\\") {
            return Err(EldError::ValidationError {
                field: "cado_path_separators".to_string(),
                value: path_str.to_string(),
                details: "Path contains consecutive separators".to_string(),
            });
        }

        if path_str.ends_with('/') || path_str.ends_with('\\') {
            return Err(EldError::ValidationError {
                field: "cado_path_ending".to_string(),
                value: path_str.to_string(),
                details: "Path cannot end with separator".to_string(),
            });
        }

        Ok(())
    }

    /// Validates CADO prefix for security issues in search operations
    pub(crate) fn validate_cado_prefix_security(prefix: &str) -> Result<(), EldError> {
        // Similar validation as path but for prefixes
        if prefix.len() > MAX_PREFIX_LENGTH {
            return Err(EldError::ValidationError {
                field: "cado_prefix_length".to_string(),
                value: prefix.to_string(),
                details: format!("Prefix exceeds maximum length of {MAX_PREFIX_LENGTH} characters"),
            });
        }

        // Check for traversal patterns in prefix
        let traversal_patterns = ["..", "//", "\\", "~"];
        for pattern in traversal_patterns.iter() {
            if prefix.to_lowercase().contains(pattern) {
                return Err(EldError::ValidationError {
                    field: "cado_prefix_traversal".to_string(),
                    value: prefix.to_string(),
                    details: format!("Prefix contains forbidden traversal pattern: {pattern}"),
                });
            }
        }

        // Validate prefix format
        if !prefix.starts_with('/') {
            return Err(EldError::ValidationError {
                field: "cado_prefix_format".to_string(),
                value: prefix.to_string(),
                details: "Prefix must start with '/'".to_string(),
            });
        }

        // Check for control characters
        if prefix
            .chars()
            .any(|c| c.is_control() && c != '\t' && c != '\n' && c != '\r')
        {
            return Err(EldError::ValidationError {
                field: "cado_prefix_control_chars".to_string(),
                value: prefix.to_string(),
                details: "Prefix contains forbidden control characters".to_string(),
            });
        }

        // Check for null bytes
        if prefix.contains('\0') {
            return Err(EldError::ValidationError {
                field: "cado_prefix_null_bytes".to_string(),
                value: prefix.to_string(),
                details: "Prefix contains null bytes".to_string(),
            });
        }

        // Validate scope in prefix (similar to path validation)
        let prefix_parts: Vec<&str> = prefix.split('/').filter(|s| !s.is_empty()).collect();
        if !prefix_parts.is_empty() {
            let scope = prefix_parts[0];
            if scope.starts_with('@') && !ALLOWED_SCOPES.contains(&scope) {
                return Err(EldError::ValidationError {
                    field: "cado_prefix_scope".to_string(),
                    value: scope.to_string(),
                    details: format!("Scope '{scope}' is not allowed in prefix"),
                });
            }
        }

        Ok(())
    }
}
