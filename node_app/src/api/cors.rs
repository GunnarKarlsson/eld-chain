//! Browser CORS for the Axum content API.

use axum::http::{header, HeaderValue, Method};
use eld_common::error::EldError;
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::config::HttpCorsConfig;

/// Build CORS for the content API from [`HttpCorsConfig`].
pub fn cors_layer(cfg: &HttpCorsConfig) -> Result<CorsLayer, EldError> {
    cfg.validate()?;
    let origins: Vec<HeaderValue> = cfg
        .allow_origins
        .iter()
        .map(|origin| {
            HeaderValue::from_str(origin.trim()).map_err(|e| {
                EldError::make_validation_error(
                    "http_cors.allow_origins",
                    origin,
                    format!("invalid Origin header value: {e}"),
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([Method::GET, Method::POST, Method::HEAD, Method::OPTIONS])
        .allow_headers([header::CONTENT_TYPE]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cors_layer_accepts_default_origins() {
        let cfg = HttpCorsConfig::default();
        assert!(cors_layer(&cfg).is_ok());
    }
}
