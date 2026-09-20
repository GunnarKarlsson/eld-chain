// ============================================================================
// RATE LIMITING CONFIGURATION
// ============================================================================

/// Rate limiting configuration for different endpoint types
#[derive(Debug, Clone)]
pub struct ApiRateLimitConfig {
    /// Maximum requests per minute for general endpoints
    pub general_requests_per_minute: u32,
    /// Maximum requests per minute for content upload endpoints
    pub upload_requests_per_minute: u32,
    /// Maximum requests per minute for CADO endpoints
    pub cado_requests_per_minute: u32,
    /// Maximum requests per minute for health check endpoints
    pub health_requests_per_minute: u32,
    /// Whether to enable rate limiting (can be disabled for testing)
    pub enabled: bool,
}

impl Default for ApiRateLimitConfig {
    fn default() -> Self {
        Self {
            general_requests_per_minute: 1000,
            upload_requests_per_minute: 100,
            cado_requests_per_minute: 2000,
            health_requests_per_minute: 5000,
            enabled: true,
        }
    }
}

// ============================================================================
// ENDPOINT TYPE CLASSIFICATION
// ============================================================================

/// Classification of endpoint types for different rate limiting rules
#[derive(Debug, Clone, Copy)]
pub enum ApiEndpointType {
    General,
    Upload,
    Cado,
    Health,
}

// ============================================================================
// CONFIGURATION HELPERS
// ============================================================================

/// Create rate limiting configuration from environment variables
pub fn create_api_rate_limit_config_from_env() -> ApiRateLimitConfig {
    let general_requests_per_minute = std::env::var("ELD_API_RATE_LIMIT_GENERAL")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1000);

    let upload_requests_per_minute = std::env::var("ELD_API_RATE_LIMIT_UPLOAD")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);

    let cado_requests_per_minute = std::env::var("ELD_API_RATE_LIMIT_CADO")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2000);

    let health_requests_per_minute = std::env::var("ELD_API_RATE_LIMIT_HEALTH")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5000);

    let enabled = std::env::var("ELD_API_RATE_LIMIT_ENABLED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(true);

    ApiRateLimitConfig {
        general_requests_per_minute,
        upload_requests_per_minute,
        cado_requests_per_minute,
        health_requests_per_minute,
        enabled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limit_config_default() {
        let config = ApiRateLimitConfig::default();
        assert_eq!(config.general_requests_per_minute, 1000);
        assert_eq!(config.upload_requests_per_minute, 100);
        assert_eq!(config.cado_requests_per_minute, 2000);
        assert_eq!(config.health_requests_per_minute, 5000);
        assert!(config.enabled);
    }

    #[test]
    fn test_rate_limit_config_from_env() {
        // Test with environment variables
        std::env::set_var("ELD_API_RATE_LIMIT_GENERAL", "500");
        std::env::set_var("ELD_API_RATE_LIMIT_UPLOAD", "50");
        std::env::set_var("ELD_API_RATE_LIMIT_CADO", "1000");
        std::env::set_var("ELD_API_RATE_LIMIT_HEALTH", "2500");
        std::env::set_var("ELD_API_RATE_LIMIT_ENABLED", "false");

        let config = create_api_rate_limit_config_from_env();
        assert_eq!(config.general_requests_per_minute, 500);
        assert_eq!(config.upload_requests_per_minute, 50);
        assert_eq!(config.cado_requests_per_minute, 1000);
        assert_eq!(config.health_requests_per_minute, 2500);
        assert!(!config.enabled);

        // Clean up environment variables
        std::env::remove_var("ELD_API_RATE_LIMIT_GENERAL");
        std::env::remove_var("ELD_API_RATE_LIMIT_UPLOAD");
        std::env::remove_var("ELD_API_RATE_LIMIT_CADO");
        std::env::remove_var("ELD_API_RATE_LIMIT_HEALTH");
        std::env::remove_var("ELD_API_RATE_LIMIT_ENABLED");
    }
}
