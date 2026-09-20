//! HTTP base URL resolution for CLI/node client configuration.

use eld_common::error::EldError;
use eld_common::validation::{validate_ip_or_hostname, validate_port};
use url::Url;

const DEFAULT_SCHEME: &str = "http";

/// Parse and normalize an explicit `http`/`https` base URL (trailing `/`).
pub fn normalize_http_base_url(raw: &str) -> Result<String, EldError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(EldError::ValidationError {
            field: "url".to_string(),
            value: raw.to_string(),
            details: "URL cannot be empty".to_string(),
        });
    }
    let mut url = Url::parse(trimmed).map_err(|e| EldError::ValidationError {
        field: "url".to_string(),
        value: raw.to_string(),
        details: format!("Invalid URL: {e}"),
    })?;
    match url.scheme() {
        "http" | "https" => {}
        scheme => {
            return Err(EldError::ValidationError {
                field: "url".to_string(),
                value: raw.to_string(),
                details: format!("URL scheme must be http or https, got {scheme}"),
            });
        }
    }
    if url.host().is_none() {
        return Err(EldError::ValidationError {
            field: "url".to_string(),
            value: raw.to_string(),
            details: "URL must include a host".to_string(),
        });
    }
    if url.path() == "/" && url.query().is_none() && url.fragment().is_none() {
        // already a bare origin
    } else if url.path().is_empty() || url.path() == "/" {
        url.set_path("/");
    }
    let mut normalized = url.to_string();
    if !normalized.ends_with('/') {
        normalized.push('/');
    }
    Ok(normalized)
}

/// Resolve RPC base URL from optional full URL or host + port.
pub fn resolve_node_base_url(
    node_url: Option<&str>,
    node_host: &str,
    node_port: &str,
) -> Result<String, EldError> {
    if let Some(url) = node_url {
        return normalize_http_base_url(url);
    }
    validate_ip_or_hostname(node_host)?;
    validate_port(node_port)?;
    Ok(format!("{DEFAULT_SCHEME}://{node_host}:{node_port}/"))
}

/// Resolve app REST API base URL from optional full URL or RPC host + app port.
pub fn resolve_app_base_url(
    app_url: Option<&str>,
    node_url: Option<&str>,
    node_host: &str,
    node_port: &str,
    app_port: &str,
) -> Result<String, EldError> {
    if let Some(url) = app_url {
        return normalize_http_base_url(url);
    }
    validate_port(app_port)?;
    if let Some(node_url) = node_url {
        let parsed = Url::parse(node_url.trim()).map_err(|e| EldError::ValidationError {
            field: "node_url".to_string(),
            value: node_url.to_string(),
            details: format!("Invalid node_url: {e}"),
        })?;
        let host = parsed.host_str().ok_or_else(|| EldError::ValidationError {
            field: "node_url".to_string(),
            value: node_url.to_string(),
            details: "node_url must include a host".to_string(),
        })?;
        let scheme = parsed.scheme();
        return Ok(format!("{scheme}://{host}:{app_port}/"));
    }
    validate_ip_or_hostname(node_host)?;
    validate_port(node_port)?;
    Ok(format!("{DEFAULT_SCHEME}://{node_host}:{app_port}/"))
}

fn normalize_endpoint_path(endpoint: &str) -> String {
    if endpoint.starts_with('/') {
        endpoint.to_string()
    } else {
        format!("/{endpoint}")
    }
}

/// Resolve the full faucet POST URL from optional `faucet_url` or host/port + endpoint path.
pub fn resolve_faucet_request_url(
    faucet_url: Option<&str>,
    faucet_host: &str,
    faucet_port: &str,
    faucet_end_point: &str,
) -> Result<String, EldError> {
    let path = normalize_endpoint_path(faucet_end_point);

    if let Some(base) = faucet_url {
        let trimmed = base.trim();
        if trimmed.is_empty() {
            return Err(EldError::ValidationError {
                field: "faucet_url".to_string(),
                value: base.to_string(),
                details: "faucet_url cannot be empty".to_string(),
            });
        }
        let mut url = Url::parse(trimmed).map_err(|e| EldError::ValidationError {
            field: "faucet_url".to_string(),
            value: trimmed.to_string(),
            details: format!("Invalid faucet_url: {e}"),
        })?;
        match url.scheme() {
            "http" | "https" => {}
            scheme => {
                return Err(EldError::ValidationError {
                    field: "faucet_url".to_string(),
                    value: trimmed.to_string(),
                    details: format!("faucet_url scheme must be http or https, got {scheme}"),
                });
            }
        }
        if url.host().is_none() {
            return Err(EldError::ValidationError {
                field: "faucet_url".to_string(),
                value: trimmed.to_string(),
                details: "faucet_url must include a host".to_string(),
            });
        }
        if url.path().is_empty() || url.path() == "/" {
            url.set_path(&path);
        }
        return Ok(url.to_string());
    }

    validate_ip_or_hostname(faucet_host)?;
    validate_port(faucet_port)?;
    Ok(format!("http://{faucet_host}:{faucet_port}{path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_adds_trailing_slash() {
        assert_eq!(
            normalize_http_base_url("https://node-api.eld.network").unwrap(),
            "https://node-api.eld.network/"
        );
    }

    #[test]
    fn resolve_node_from_host_port() {
        assert_eq!(
            resolve_node_base_url(None, "127.0.0.1", "26657").unwrap(),
            "http://127.0.0.1:26657/"
        );
    }

    #[test]
    fn resolve_app_from_urls() {
        assert_eq!(
            resolve_app_base_url(
                Some("https://node-api.eld.network"),
                Some("https://node-rpc.eld.network"),
                "",
                "",
                "9001"
            )
            .unwrap(),
            "https://node-api.eld.network/"
        );
    }

    #[test]
    fn resolve_app_from_node_url_and_port() {
        assert_eq!(
            resolve_app_base_url(None, Some("https://node-rpc.eld.network"), "", "", "9001")
                .unwrap(),
            "https://node-rpc.eld.network:9001/"
        );
    }

    #[test]
    fn resolve_faucet_from_host_port() {
        assert_eq!(
            resolve_faucet_request_url(None, "127.0.0.1", "8080", "/faucet/request").unwrap(),
            "http://127.0.0.1:8080/faucet/request"
        );
    }

    #[test]
    fn resolve_faucet_from_https_base_url() {
        assert_eq!(
            resolve_faucet_request_url(
                Some("https://faucet.eld.network"),
                "127.0.0.1",
                "8080",
                "/request"
            )
            .unwrap(),
            "https://faucet.eld.network/request"
        );
    }

    #[test]
    fn resolve_faucet_full_url_keeps_path() {
        assert_eq!(
            resolve_faucet_request_url(
                Some("https://faucet.eld.network/request"),
                "127.0.0.1",
                "8080",
                "/ignored"
            )
            .unwrap(),
            "https://faucet.eld.network/request"
        );
    }

    #[test]
    fn resolve_faucet_rejects_invalid_scheme() {
        assert!(resolve_faucet_request_url(
            Some("ftp://faucet.example.com"),
            "127.0.0.1",
            "8080",
            "/request"
        )
        .is_err());
    }
}
