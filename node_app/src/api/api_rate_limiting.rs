use super::error::ApiError;
use axum::http::HeaderMap;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::warn;

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

/// Simple rate limiting state for tracking requests per client
#[derive(Debug)]
pub struct RateLimitState {
    clients: HashMap<String, Vec<Instant>>,
    config: ApiRateLimitConfig,
}

impl RateLimitState {
    pub fn new(config: ApiRateLimitConfig) -> Self {
        Self {
            clients: HashMap::new(),
            config,
        }
    }

    /// Check if a request is allowed for the given client and endpoint type
    pub fn is_allowed(&mut self, client_id: &str, endpoint_type: ApiEndpointType) -> bool {
        if !self.config.enabled {
            return true;
        }

        let max_requests = match endpoint_type {
            ApiEndpointType::General => self.config.general_requests_per_minute,
            ApiEndpointType::Upload => self.config.upload_requests_per_minute,
            ApiEndpointType::Cado => self.config.cado_requests_per_minute,
            ApiEndpointType::Health => self.config.health_requests_per_minute,
        };

        let now = Instant::now();
        let window = Duration::from_secs(60);

        let client_requests = self.clients.entry(client_id.to_string()).or_default();

        // Remove old requests outside the time window
        client_requests.retain(|&time| now.duration_since(time) <= window);

        // Check if we're under the limit
        if client_requests.len() < max_requests as usize {
            client_requests.push(now);
            true
        } else {
            warn!(
                "Rate limit exceeded for client {} on endpoint type {:?}: {} requests in the last minute",
                client_id, endpoint_type, client_requests.len()
            );
            false
        }
    }
}

/// Extract client identifier from request headers
pub(crate) fn extract_client_id(headers: &HeaderMap) -> String {
    // Try to get X-Forwarded-For header first (for proxied requests)
    if let Some(forwarded_for) = headers.get("X-Forwarded-For") {
        if let Ok(forwarded_for_str) = forwarded_for.to_str() {
            // Take the first IP in the chain
            if let Some(first_ip) = forwarded_for_str.split(',').next() {
                return first_ip.trim().to_string();
            }
        }
    }

    // Fall back to X-Real-IP header
    if let Some(real_ip) = headers.get("X-Real-IP") {
        if let Ok(real_ip_str) = real_ip.to_str() {
            return real_ip_str.to_string();
        }
    }

    // Default to a placeholder (in production, you'd want to extract the actual IP)
    "unknown".to_string()
}

/// Check rate limit for a request
pub(crate) async fn check_rate_limit(
    rate_limit_state: &Arc<RwLock<RateLimitState>>,
    endpoint_type: ApiEndpointType,
    headers: &HeaderMap,
) -> Result<(), ApiError> {
    let client_id = extract_client_id(headers);

    let rate_limit_exceeded = {
        let mut state = rate_limit_state.write().await;
        !state.is_allowed(&client_id, endpoint_type)
    };

    if rate_limit_exceeded {
        return Err(ApiError::ServiceUnavailable {
            message: "Rate limit exceeded. Please try again later.".to_string(),
            details: Some("Too many requests from this client".to_string()),
        });
    }

    Ok(())
}

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
