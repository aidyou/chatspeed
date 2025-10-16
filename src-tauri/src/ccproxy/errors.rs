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
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CCProxyError {
    /// The provided token is invalid.
    #[error("{}", t!("proxy.error.invalid_api_key"))]
    InvalidToken,
    /// An internal server error occurred.
    #[error("{}", t!("proxy.error.internal_server_error", error = _0))]
    InternalError(String),
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
