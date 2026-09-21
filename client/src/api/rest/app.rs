use super::namespace::{NamespaceNotRegisteredResponse, NamespaceRegisteredResponse};
use super::pinboard::{PostMessageSubmitRequest, PostMessageSubmitResponse};
use eld_common::cado::CadoBody;
use eld_common::error::EldError;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use urlencoding::encode;

/// Standardized API error response structure (matches the server-side structure)
#[derive(Debug, Serialize, Deserialize)]
pub struct ApiErrorResponse {
    pub code: String,
    pub message: String,
    pub details: Option<String>,
    pub request_id: Option<String>,
    pub timestamp: String,
}

/// HTTP client for the node's app REST API (content, pinboard, CADO, health).
pub struct AppApi {
    client: Client,
    base_url: String,
}

impl AppApi {
    pub fn new(base_url: String) -> Result<Self, EldError> {
        let base_url = if base_url.ends_with('/') {
            base_url
        } else {
            format!("{base_url}/")
        };
        let client = reqwest::ClientBuilder::new()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| EldError::NetworkError {
                operation: "create app HTTP client".to_string(),
                details: format!("Failed to create HTTP client: {e}"),
            })?;

        Ok(Self { client, base_url })
    }

    pub async fn get_cado(&self, cado_path: String) -> Result<Option<CadoBody>, EldError> {
        let url = self.base_url.clone() + "cado/" + &encode(&cado_path);
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| EldError::NetworkError {
                operation: "get_cado".to_string(),
                details: e.to_string(),
            })?;
        response
            .json::<Option<CadoBody>>()
            .await
            .map_err(|e| EldError::ValidationError {
                field: "cado".to_string(),
                value: cado_path,
                details: format!("Failed to parse CADO type: {e}"),
            })
    }

    /// Submit a user-signed pinboard message; the node validates and broadcasts `PostMessage` tx.
    pub async fn submit_pinboard_message(
        &self,
        request: PostMessageSubmitRequest,
    ) -> Result<PostMessageSubmitResponse, EldError> {
        let url = format!("{}v1/pinboard/messages:submit", self.base_url);

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|e| EldError::NetworkError {
                operation: "submit pinboard message".to_string(),
                details: e.to_string(),
            })?;

        let status = response.status();
        if status.is_success() {
            response
                .json()
                .await
                .map_err(|e| EldError::ValidationError {
                    field: "response".to_string(),
                    value: "pinboard submit response".to_string(),
                    details: e.to_string(),
                })
        } else {
            let error_text = response.text().await.map_err(|e| EldError::NetworkError {
                operation: "read pinboard error response".to_string(),
                details: e.to_string(),
            })?;

            if let Ok(error_response) = serde_json::from_str::<ApiErrorResponse>(&error_text) {
                let error_msg = if let Some(details) = error_response.details {
                    format!(
                        "{}: {} (Details: {})",
                        error_response.code, error_response.message, details
                    )
                } else {
                    format!("{}: {}", error_response.code, error_response.message)
                };
                return Err(EldError::NetworkError {
                    operation: "submit pinboard message".to_string(),
                    details: error_msg,
                });
            }

            Err(EldError::NetworkError {
                operation: "submit pinboard message".to_string(),
                details: format!("Pinboard submit failed with status {status}: {error_text}"),
            })
        }
    }

    /// Look up a namespace slug. Returns `Ok(Some(_))` when registered (HTTP 200),
    /// `Ok(None)` when not registered (HTTP 404), or an error for other failures.
    pub async fn get_namespace(
        &self,
        namespace_slug: &str,
    ) -> Result<Option<NamespaceRegisteredResponse>, EldError> {
        let url = format!(
            "{}v1/namespace/{}",
            self.base_url,
            encode(namespace_slug).into_owned()
        );

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| EldError::NetworkError {
                operation: "get namespace".to_string(),
                details: e.to_string(),
            })?;

        let status = response.status();
        if status.is_success() {
            return response
                .json::<NamespaceRegisteredResponse>()
                .await
                .map(Some)
                .map_err(|e| EldError::ValidationError {
                    field: "response".to_string(),
                    value: "namespace lookup response".to_string(),
                    details: e.to_string(),
                });
        }

        if status == reqwest::StatusCode::NOT_FOUND {
            let _not_registered: NamespaceNotRegisteredResponse =
                response
                    .json()
                    .await
                    .map_err(|e| EldError::ValidationError {
                        field: "response".to_string(),
                        value: "namespace not registered response".to_string(),
                        details: e.to_string(),
                    })?;
            return Ok(None);
        }

        let error_text = response.text().await.map_err(|e| EldError::NetworkError {
            operation: "read namespace error response".to_string(),
            details: e.to_string(),
        })?;

        if let Ok(error_response) = serde_json::from_str::<ApiErrorResponse>(&error_text) {
            let error_msg = if let Some(details) = error_response.details {
                format!(
                    "{}: {} (Details: {})",
                    error_response.code, error_response.message, details
                )
            } else {
                format!("{}: {}", error_response.code, error_response.message)
            };
            return Err(EldError::NetworkError {
                operation: "get namespace".to_string(),
                details: error_msg,
            });
        }

        Err(EldError::NetworkError {
            operation: "get namespace".to_string(),
            details: format!("Namespace lookup failed with status {status}: {error_text}"),
        })
    }

    /// Check if the app API is healthy.
    pub async fn health_check(&self) -> Result<bool, EldError> {
        let url = self.base_url.clone();

        match self.client.get(&url).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    let body = response.text().await.map_err(|e| EldError::NetworkError {
                        operation: "read health check response".to_string(),
                        details: e.to_string(),
                    })?;
                    Ok(body.trim() == "OK")
                } else {
                    Ok(false)
                }
            }
            Err(_) => Ok(false),
        }
    }
}
