use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use rust_i18n::t;
use serde_json::json;

/// Custom error types for the ccproxy module.
#[derive(Debug)]
pub enum CCProxyError {
    /// The provided token is invalid.
    InvalidToken,
    /// An internal server error occurred.
    InternalError(String),
    /// Authorization header is missing or malformed.
    MissingToken,
    /// The requested proxy model alias was not found.
    ModelAliasNotFound(String),
    /// No backend targets configured for the given model alias.
    NoBackendTargets(String),
    /// No proxy access keys are configured on the server.
    NoKeysConfigured,
    /// Failed to retrieve AI model details from the database.
    ModelDetailsFetchError(String),
    /// Invalid protocol string from AI model details.
    InvalidProtocolError(String),
    /// Failed to acquire lock on the MainStore.
    StoreLockError(String),
}

impl IntoResponse for CCProxyError {
    fn into_response(self) -> Response {
        let (status, error_type, message_string) = match self {
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
            CCProxyError::InvalidProtocolError(protocol_str) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Configuration Error",
                t!("proxy.error.invalid_protocol", protocol = protocol_str).to_string(),
            ),
            CCProxyError::InternalError(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal Server Error",
                t!("proxy.error.internal_server_error", error = e).to_string(),
            ),
            CCProxyError::ModelAliasNotFound(alias) => (
                StatusCode::NOT_FOUND,
                "Model Not Found",
                t!("proxy.error.model_alias_not_found", alias = alias).to_string(),
            ),
            CCProxyError::ModelDetailsFetchError(id_str) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database Error",
                t!("proxy.error.model_details_fetch_failed", id = id_str).to_string(),
            ),
            CCProxyError::StoreLockError(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Store Error",
                t!("proxy.error.store_lock_failed", error = e).to_string(),
            ),
        };

        log::error!(
            "CCProxyError: type={}, message={}",
            error_type,
            message_string
        );

        let error_response = json!({ "error": { "message": message_string, "type": error_type }});
        (status, Json(error_response)).into_response()
    }
}

pub type ProxyResult<T> = std::result::Result<T, CCProxyError>;
