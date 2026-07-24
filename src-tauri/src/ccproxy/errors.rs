use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use rust_i18n::t;
use serde::Serialize;
use serde_json::json;
use thiserror::Error;

/// Custom error types for the ccproxy module.
#[derive(Error, Debug, Serialize)]
#[serde(tag = "kind", content = "message", rename_all = "camelCase")]
pub enum CCProxyError {
    /// The provided token is invalid.
    #[error("{}", t!("proxy.error.invalid_api_key"))]
    InvalidToken,
    /// An internal server error occurred.
    #[error("{}", t!("proxy.error.internal_server_error", error = _0))]
    InternalError(String),
    /// A backend request failed before an HTTP response was received.
    #[error("{0}")]
    BackendRequestError(String),
    /// Authorization header is missing or malformed.
    #[error("{}", t!("proxy.error.missing_auth_header"))]
    MissingToken,
    /// The requested proxy model alias was not found.
    #[error("{}", t!("proxy.error.model_alias_not_found", alias = _0))]
    ModelAliasNotFound(String),
    /// No backend targets configured for the given model alias.
    #[error("{}", t!("proxy.error.no_backend_targets", alias = _0))]
    NoBackendTargets(String),
    /// No proxy access keys are configured on the server.
    #[error("{}", t!("proxy.error.no_keys_configured"))]
    NoKeysConfigured,
    /// Failed to retrieve AI model details from the database.
    #[error("{}", t!("proxy.error.model_details_fetch_failed", id = _0))]
    ModelDetailsFetchError(String),
    /// Invalid protocol string from AI model details.
    #[error("{}", t!("proxy.error.invalid_protocol", protocol = _0))]
    InvalidProtocolError(String),
    /// Failed to acquire lock on the MainStore.
    #[error("{}", t!("proxy.error.store_lock_failed", error = _0))]
    StoreLockError(String),
}

impl IntoResponse for CCProxyError {
    fn into_response(self) -> Response {
        let (status, error_type, message) = match self {
            CCProxyError::InvalidToken => (
                StatusCode::UNAUTHORIZED,
                "Authentication Error",
                t!("proxy.error.invalid_api_key").to_string(),
            ),
            CCProxyError::MissingToken => (
                StatusCode::UNAUTHORIZED,
                "Authentication Error",
                t!("proxy.error.missing_auth_header").to_string(),
            ),
            CCProxyError::NoBackendTargets(alias) => (
                StatusCode::BAD_REQUEST,
                "Configuration Error",
                t!("proxy.error.no_backend_targets", alias = alias).to_string(),
            ),
            CCProxyError::NoKeysConfigured => (
                StatusCode::UNAUTHORIZED,
                "Configuration Error",
                t!("proxy.error.no_keys_configured").to_string(),
            ),
            CCProxyError::InvalidProtocolError(protocol) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Configuration Error",
                t!("proxy.error.invalid_protocol", protocol = protocol).to_string(),
            ),
            CCProxyError::InternalError(message) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal Server Error",
                t!("proxy.error.internal_server_error", error = message).to_string(),
            ),
            CCProxyError::BackendRequestError(message) => (
                StatusCode::BAD_GATEWAY,
                "Upstream Connection Error",
                message,
            ),
            CCProxyError::ModelAliasNotFound(alias) => (
                StatusCode::NOT_FOUND,
                "Model Not Found",
                t!("proxy.error.model_alias_not_found", alias = alias).to_string(),
            ),
            CCProxyError::ModelDetailsFetchError(id) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database Error",
                t!("proxy.error.model_details_fetch_failed", id = id).to_string(),
            ),
            CCProxyError::StoreLockError(message) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Store Error",
                t!("proxy.error.store_lock_failed", error = message).to_string(),
            ),
        };

        log::error!("CCProxyError: type={}, message={}", error_type, &message);

        let error_response = json!({ "error": { "message": message, "type": error_type }});
        (status, Json(error_response)).into_response()
    }
}

pub type ProxyResult<T> = std::result::Result<T, CCProxyError>;

#[cfg(test)]
mod tests {
    use super::CCProxyError;
    use axum::{body::to_bytes, response::IntoResponse};
    use http::StatusCode;
    use serde_json::{json, Value};

    #[tokio::test]
    async fn backend_request_error_preserves_original_message() {
        let message = "Request to backend failed: error sending request for url (https://api.example.com/v1/chat/completions)";
        let response = CCProxyError::BackendRequestError(message.to_string()).into_response();

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body should be readable");
        let body: Value =
            serde_json::from_slice(&body).expect("response body should be valid JSON");

        assert_eq!(
            body,
            json!({
                "error": {
                    "message": message,
                    "type": "Upstream Connection Error"
                }
            })
        );
        assert!(!body["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("内部服务器错误"));
    }
}
