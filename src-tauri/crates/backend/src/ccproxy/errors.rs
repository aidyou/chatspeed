use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use rust_i18n::t;
use serde::Serialize;
use serde_json::json;
use thiserror::Error;

use crate::ccproxy::diagnostic;
use crate::error_diagnostics::ErrorDiagnostic;

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
    /// The client request body does not satisfy the endpoint protocol.
    #[error("{}", t!("proxy.error.invalid_request_format", error = _0))]
    InvalidRequestBody(String),
    /// Failed to acquire lock on the MainStore.
    #[error("{}", t!("proxy.error.store_lock_failed", error = _0))]
    StoreLockError(String),
}

impl CCProxyError {
    fn status_and_type(&self) -> (StatusCode, &'static str) {
        match self {
            CCProxyError::InvalidToken | CCProxyError::MissingToken => {
                (StatusCode::UNAUTHORIZED, "Authentication Error")
            }
            CCProxyError::NoBackendTargets(_) => (StatusCode::BAD_REQUEST, "Configuration Error"),
            CCProxyError::NoKeysConfigured => (StatusCode::UNAUTHORIZED, "Configuration Error"),
            CCProxyError::InvalidProtocolError(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "Configuration Error")
            }
            CCProxyError::InvalidRequestBody(_) => (StatusCode::BAD_REQUEST, "Invalid Request"),
            CCProxyError::InternalError(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error")
            }
            CCProxyError::BackendRequestError(_) => {
                (StatusCode::BAD_GATEWAY, "Upstream Connection Error")
            }
            CCProxyError::ModelAliasNotFound(_) => (StatusCode::NOT_FOUND, "Model Not Found"),
            CCProxyError::ModelDetailsFetchError(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "Database Error")
            }
            CCProxyError::StoreLockError(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Store Error"),
        }
    }

    fn diagnostic_code(&self) -> &'static str {
        match self {
            CCProxyError::InvalidToken => "invalid_token",
            CCProxyError::InternalError(_) => "internal_error",
            CCProxyError::BackendRequestError(_) => "backend_request",
            CCProxyError::MissingToken => "missing_token",
            CCProxyError::ModelAliasNotFound(_) => "model_alias_not_found",
            CCProxyError::NoBackendTargets(_) => "no_backend_targets",
            CCProxyError::NoKeysConfigured => "no_keys_configured",
            CCProxyError::ModelDetailsFetchError(_) => "model_details_fetch",
            CCProxyError::InvalidProtocolError(_) => "invalid_protocol",
            CCProxyError::InvalidRequestBody(_) => "invalid_request",
            CCProxyError::StoreLockError(_) => "store_lock",
        }
    }

    pub(crate) fn diagnostic(&self) -> ErrorDiagnostic {
        let (status, error_type) = self.status_and_type();
        let mut diagnostic = diagnostic::from_error(
            "ccproxy",
            "proxy_response",
            self,
            [
                ("category", Some(error_type.to_string())),
                ("response", Some(format!("status={}", status.as_u16()))),
            ],
        );
        diagnostic.code = Some(self.diagnostic_code().to_string());
        diagnostic.http_status = Some(status.as_u16());
        diagnostic
    }
}

impl IntoResponse for CCProxyError {
    fn into_response(self) -> Response {
        let (status, error_type) = self.status_and_type();
        let message = self.to_string();

        diagnostic::emit(log::Level::Error, &self.diagnostic());

        let error_response = json!({ "error": { "message": message, "type": error_type }});
        (status, Json(error_response)).into_response()
    }
}

#[cfg(not(feature = "desktop"))]
pub type ProxyResult<T> = std::result::Result<T, CCProxyError>;

#[cfg(test)]
mod tests {
    use super::CCProxyError;
    use axum::{body::to_bytes, response::IntoResponse};
    use http::StatusCode;
    use serde_json::{json, Value};

    #[test]
    fn diagnostic_keeps_typed_cause_code_and_status() {
        let error = CCProxyError::InternalError("upstream token=CANARY failed".to_string());
        let diagnostic = error.diagnostic();
        let line = diagnostic.to_json_line();

        assert_eq!(diagnostic.module, "ccproxy");
        assert_eq!(diagnostic.operation, "proxy_response");
        assert_eq!(diagnostic.code.as_deref(), Some("internal_error"));
        assert_eq!(diagnostic.http_status, Some(500));
        assert_eq!(diagnostic.context_chain[0].module, "ccproxy");
        assert_eq!(diagnostic.context_chain[1].operation, "response");
        assert!(!line.contains("CANARY"));
    }

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
