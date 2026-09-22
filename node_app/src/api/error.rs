//! Standardized HTTP API error types.

use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

/// Standardized API error response structure
#[derive(Debug, Serialize, Deserialize)]
pub struct ApiErrorResponse {
    /// Error code for programmatic handling
    pub code: String,
    /// Human-readable error message
    pub message: String,
    /// Detailed error information
    pub details: Option<String>,
    /// Timestamp of the error
    pub timestamp: String,
}

/// API error types with corresponding HTTP status codes
#[derive(Debug)]
pub enum ApiError {
    /// 400 Bad Request - Invalid input parameters
    BadRequest {
        message: String,
        details: Option<String>,
    },
    /// 404 Not Found - Resource not found
    NotFound {
        resource_type: String,
        identifier: String,
        details: Option<String>,
    },
    /// 413 Payload Too Large - Request too large
    PayloadTooLarge {
        max_size: usize,
        actual_size: usize,
        details: Option<String>,
    },
    /// 500 Internal Server Error - Server error
    InternalServerError {
        message: String,
        details: Option<String>,
    },
    /// 503 Service Unavailable - Service temporarily unavailable
    ServiceUnavailable {
        message: String,
        details: Option<String>,
    },
}

impl ApiError {
    /// Get the HTTP status code for this error
    pub fn status_code(&self) -> StatusCode {
        match self {
            ApiError::BadRequest { .. } => StatusCode::BAD_REQUEST,
            ApiError::NotFound { .. } => StatusCode::NOT_FOUND,
            ApiError::PayloadTooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE,
            ApiError::InternalServerError { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            ApiError::ServiceUnavailable { .. } => StatusCode::SERVICE_UNAVAILABLE,
        }
    }

    /// Get the error code for programmatic handling
    pub fn error_code(&self) -> &'static str {
        match self {
            ApiError::BadRequest { .. } => "BAD_REQUEST",
            ApiError::NotFound { .. } => "NOT_FOUND",
            ApiError::PayloadTooLarge { .. } => "PAYLOAD_TOO_LARGE",
            ApiError::InternalServerError { .. } => "INTERNAL_SERVER_ERROR",
            ApiError::ServiceUnavailable { .. } => "SERVICE_UNAVAILABLE",
        }
    }

    /// Get the error message
    pub fn message(&self) -> String {
        match self {
            ApiError::BadRequest { message, .. } => message.clone(),
            ApiError::NotFound {
                resource_type,
                identifier,
                ..
            } => {
                format!("{resource_type} not found: {identifier}")
            }
            ApiError::PayloadTooLarge {
                max_size,
                actual_size,
                ..
            } => {
                format!(
                    "Request too large. Maximum size: {max_size} bytes, actual size: {actual_size} bytes"
                )
            }
            ApiError::InternalServerError { message, .. } => message.clone(),
            ApiError::ServiceUnavailable { message, .. } => message.clone(),
        }
    }

    /// Get additional error details
    pub fn details(&self) -> Option<String> {
        match self {
            ApiError::BadRequest { details, .. } => details.clone(),
            ApiError::NotFound { details, .. } => details.clone(),
            ApiError::PayloadTooLarge { details, .. } => details.clone(),
            ApiError::InternalServerError { details, .. } => details.clone(),
            ApiError::ServiceUnavailable { details, .. } => details.clone(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status_code = self.status_code();
        let error_response = ApiErrorResponse {
            code: self.error_code().to_string(),
            message: self.message(),
            details: self.details(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        };

        let mut headers = HeaderMap::new();
        headers.insert(
            "Content-Type",
            "application/json"
                .parse()
                .expect("Hardcoded Content-Type should always parse"),
        );

        (status_code, headers, Json(error_response)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[test]
    fn test_api_error_status_codes() {
        assert_eq!(
            ApiError::BadRequest {
                message: "test".to_string(),
                details: None,
            }
            .status_code(),
            StatusCode::BAD_REQUEST
        );

        assert_eq!(
            ApiError::NotFound {
                resource_type: "test".to_string(),
                identifier: "id".to_string(),
                details: None,
            }
            .status_code(),
            StatusCode::NOT_FOUND
        );

        assert_eq!(
            ApiError::InternalServerError {
                message: "test".to_string(),
                details: None,
            }
            .status_code(),
            StatusCode::INTERNAL_SERVER_ERROR
        );

        assert_eq!(
            ApiError::ServiceUnavailable {
                message: "test".to_string(),
                details: None,
            }
            .status_code(),
            StatusCode::SERVICE_UNAVAILABLE
        );

        assert_eq!(
            ApiError::PayloadTooLarge {
                max_size: 1000,
                actual_size: 2000,
                details: None,
            }
            .status_code(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
    }

    #[test]
    fn test_api_error_codes() {
        assert_eq!(
            ApiError::BadRequest {
                message: "test".to_string(),
                details: None,
            }
            .error_code(),
            "BAD_REQUEST"
        );

        assert_eq!(
            ApiError::NotFound {
                resource_type: "test".to_string(),
                identifier: "id".to_string(),
                details: None,
            }
            .error_code(),
            "NOT_FOUND"
        );

        assert_eq!(
            ApiError::InternalServerError {
                message: "test".to_string(),
                details: None,
            }
            .error_code(),
            "INTERNAL_SERVER_ERROR"
        );

        assert_eq!(
            ApiError::ServiceUnavailable {
                message: "test".to_string(),
                details: None,
            }
            .error_code(),
            "SERVICE_UNAVAILABLE"
        );

        assert_eq!(
            ApiError::PayloadTooLarge {
                max_size: 1000,
                actual_size: 2000,
                details: None,
            }
            .error_code(),
            "PAYLOAD_TOO_LARGE"
        );
    }

    #[test]
    fn test_api_error_messages() {
        let bad_request = ApiError::BadRequest {
            message: "Invalid input".to_string(),
            details: None,
        };
        assert_eq!(bad_request.message(), "Invalid input");

        let not_found = ApiError::NotFound {
            resource_type: "User".to_string(),
            identifier: "123".to_string(),
            details: None,
        };
        assert_eq!(not_found.message(), "User not found: 123");

        let payload_too_large = ApiError::PayloadTooLarge {
            max_size: 1000,
            actual_size: 2000,
            details: None,
        };
        assert!(payload_too_large.message().contains("Request too large"));
        assert!(payload_too_large.message().contains("1000"));
        assert!(payload_too_large.message().contains("2000"));

        let internal_error = ApiError::InternalServerError {
            message: "Database error".to_string(),
            details: Some("Connection timeout".to_string()),
        };
        assert_eq!(internal_error.message(), "Database error");
        assert_eq!(
            internal_error.details(),
            Some("Connection timeout".to_string())
        );
    }

    #[test]
    fn test_api_error_serialization() {
        let error_response = ApiErrorResponse {
            code: "NOT_FOUND".to_string(),
            message: "Resource not found".to_string(),
            details: Some("No such resource".to_string()),
            timestamp: "2024-01-01T12:00:00Z".to_string(),
        };

        let json = serde_json::to_string(&error_response).unwrap();
        assert!(json.contains("NOT_FOUND"));
        assert!(json.contains("Resource not found"));
        assert!(json.contains("No such resource"));
        assert!(json.contains("2024-01-01T12:00:00Z"));
    }

    #[test]
    fn test_api_error_deserialization() {
        let json = r#"{
        "code": "BAD_REQUEST",
        "message": "Invalid input",
        "details": "Missing required field",
        "timestamp": "2024-01-01T12:00:00Z"
    }"#;

        let error_response: ApiErrorResponse = serde_json::from_str(json).unwrap();

        assert_eq!(error_response.code, "BAD_REQUEST");
        assert_eq!(error_response.message, "Invalid input");
        assert_eq!(
            error_response.details,
            Some("Missing required field".to_string())
        );
        assert_eq!(error_response.timestamp, "2024-01-01T12:00:00Z");
    }
}
