//! Node process runtime settings: P2P, capacity storage, and indexer.

use eld_common::error::EldError;
use eld_common::validation::validate_port;
use serde::{Deserialize, Serialize};
use url::Url;

const DEFAULT_HTTP_CORS_ORIGINS: [&str; 4] = [
    "https://explorer.eld.network",
    "https://www.eld.network",
    "https://eld.network",
    "https://docs.eld.network",
];

/// Browser CORS allow-list for the Axum content API (`app_port`).
///
/// CLI and faucet clients are not browsers and do not need to be listed.
#[derive(Deserialize, Debug, Clone, Serialize)]
pub struct HttpCorsConfig {
    #[serde(default = "default_http_cors_origins")]
    pub allow_origins: Vec<String>,
}

fn default_http_cors_origins() -> Vec<String> {
    DEFAULT_HTTP_CORS_ORIGINS
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

impl Default for HttpCorsConfig {
    fn default() -> Self {
        Self {
            allow_origins: default_http_cors_origins(),
        }
    }
}

impl HttpCorsConfig {
    pub fn validate(&self) -> Result<(), EldError> {
        if self.allow_origins.is_empty() {
            EldError::validation_error(
                "http_cors.allow_origins",
                "",
                "must contain at least one origin",
            )?;
        }
        for origin in &self.allow_origins {
            validate_http_cors_origin(origin)?;
        }
        Ok(())
    }
}

fn validate_http_cors_origin(origin: &str) -> Result<(), EldError> {
    let trimmed = origin.trim();
    if trimmed.is_empty() {
        return EldError::validation_error("http_cors.allow_origins", origin, "cannot be empty");
    }
    if trimmed == "*" {
        return EldError::validation_error(
            "http_cors.allow_origins",
            origin,
            "wildcard * is not allowed",
        );
    }

    let url = Url::parse(trimmed).map_err(|e| {
        EldError::make_validation_error(
            "http_cors.allow_origins",
            origin,
            format!("must be an http(s) origin: {e}"),
        )
    })?;

    if url.scheme() != "http" && url.scheme() != "https" {
        return EldError::validation_error(
            "http_cors.allow_origins",
            origin,
            "scheme must be http or https",
        );
    }

    let host = match url.host_str() {
        Some(h) => h,
        None => {
            return EldError::validation_error(
                "http_cors.allow_origins",
                origin,
                "must include a host",
            );
        }
    };
    if host.contains('*') {
        return EldError::validation_error(
            "http_cors.allow_origins",
            origin,
            "wildcard hosts (for example *.eld.network) are not allowed",
        );
    }

    let path_ok = url.path().is_empty() || url.path() == "/";
    if !path_ok || url.query().is_some() || url.fragment().is_some() {
        return EldError::validation_error(
            "http_cors.allow_origins",
            origin,
            "must be an origin only (no path, query, or fragment)",
        );
    }

    if !url.username().is_empty() || url.password().is_some() {
        return EldError::validation_error(
            "http_cors.allow_origins",
            origin,
            "must not include userinfo",
        );
    }

    Ok(())
}

#[derive(Deserialize, Debug, Clone, Serialize)]
pub struct NodeRuntimeConfig {
    #[serde(default)]
    pub p2p_tcp_port: Option<String>,
    #[serde(default)]
    pub p2p_udp_port: Option<String>,
    #[serde(default)]
    pub single_node: Option<bool>,
    pub capacity_size_mb: Option<u64>,
    pub capacity_storage_path: Option<String>,
    #[serde(default)]
    pub indexer: bool,
    #[serde(default)]
    pub http_cors: HttpCorsConfig,
}

impl NodeRuntimeConfig {
    pub fn from_file(file: &str) -> Result<Self, EldError> {
        <Self as crate::config::loader::ConfigLoadable>::from_file(file)
    }

    pub fn validate(&self) -> Result<(), EldError> {
        if let Some(port) = &self.p2p_tcp_port {
            validate_port(port)?;
        }
        if let Some(port) = &self.p2p_udp_port {
            validate_port(port)?;
        }

        match self.capacity_size_mb {
            Some(mb) if mb > 0 => {}
            Some(mb) => {
                EldError::validation_error(
                    "capacity_size_mb",
                    &mb.to_string(),
                    "must be greater than 0",
                )?;
            }
            None => {
                EldError::validation_error("capacity_size_mb", "", "must be set in config.json")?;
            }
        }

        match self.capacity_storage_path.as_deref().map(str::trim) {
            Some(path) if !path.is_empty() => {}
            Some(_) => {
                EldError::validation_error("capacity_storage_path", "", "cannot be empty")?;
            }
            None => {
                EldError::validation_error(
                    "capacity_storage_path",
                    "",
                    "must be set in config.json",
                )?;
            }
        }

        self.http_cors.validate()?;

        Ok(())
    }
}

impl crate::config::loader::ConfigValidator for NodeRuntimeConfig {
    fn validate(&self) -> Result<(), EldError> {
        self.validate()
    }
}

impl crate::config::loader::ConfigLoadable for NodeRuntimeConfig {}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_config() -> NodeRuntimeConfig {
        NodeRuntimeConfig {
            p2p_tcp_port: Some("4001".to_string()),
            p2p_udp_port: Some("4002".to_string()),
            single_node: Some(false),
            capacity_size_mb: Some(50),
            capacity_storage_path: Some("./data/capacity".to_string()),
            indexer: true,
            http_cors: HttpCorsConfig::default(),
        }
    }

    #[test]
    fn validate_accepts_sample_runtime_fields() {
        assert!(valid_config().validate().is_ok());
    }

    #[test]
    fn validate_rejects_invalid_p2p_port() {
        let mut cfg = valid_config();
        cfg.p2p_tcp_port = Some("0".to_string());
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_missing_capacity() {
        let mut cfg = valid_config();
        cfg.capacity_size_mb = None;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn http_cors_default_list_is_valid() {
        assert!(HttpCorsConfig::default().validate().is_ok());
        assert_eq!(
            HttpCorsConfig::default().allow_origins,
            default_http_cors_origins()
        );
    }

    #[test]
    fn http_cors_rejects_star() {
        let cfg = HttpCorsConfig {
            allow_origins: vec!["*".to_string()],
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn http_cors_rejects_wildcard_host() {
        let cfg = HttpCorsConfig {
            allow_origins: vec!["https://*.eld.network".to_string()],
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn http_cors_rejects_origin_with_path() {
        let cfg = HttpCorsConfig {
            allow_origins: vec!["https://eld.network/docs".to_string()],
        };
        assert!(cfg.validate().is_err());
    }
}
