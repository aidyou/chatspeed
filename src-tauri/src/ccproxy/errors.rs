use rust_i18n::t;
use warp::{filters::cors::CorsForbidden, http::StatusCode, Rejection, Reply};

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
    Forbidden(String),
}

impl warp::reject::Reject for CCProxyError {}

/// Handles rejections specific to the ccproxy module, particularly authentication errors.
pub async fn handle_proxy_rejection(
    err: Rejection,
) -> Result<impl Reply, std::convert::Infallible> {
    let (code, error_type, message_string) = if err.is_not_found() {
        log::debug!(
            "handle_proxy_rejection: Received a 'not_found' rejection: {:?}",
            err
        );
        (
            StatusCode::NOT_FOUND,
            "Not Found",
            t!("proxy.error.endpoint_not_found").to_string(),
        )
    } else if err.find::<warp::reject::MethodNotAllowed>().is_some() {
        log::debug!(
            "handle_proxy_rejection: Received 'MethodNotAllowed': {:?}",
            err
        );
        (
            StatusCode::METHOD_NOT_ALLOWED,
            "Method Not Allowed",
            "Method Not Allowed".to_string(),
        )
    } else if err.find::<CorsForbidden>().is_some() {
        log::debug!(
            "handle_proxy_rejection: Received 'CorsForbidden': {:?}",
            err
        );
        (
            StatusCode::FORBIDDEN, // More appropriate status for CORS rejections
            "Cors Error",
            t!("proxy.error.cors_blocked").to_string(),
        )
    } else if let Some(customer_error) = err.find::<CCProxyError>() {
        let (status, err_type_str, msg_str_slice) = match customer_error {
            CCProxyError::Forbidden(s) => {
                (StatusCode::FORBIDDEN, "Authentication Error", s.to_string())
            }
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
                t!("proxy.error.no_backend_targets", alias = alias.to_string()).to_string(),
            ),
            CCProxyError::NoKeysConfigured => (
                StatusCode::UNAUTHORIZED,
                "Configuration Error",
                t!("proxy.error.no_keys_configured").to_string(),
            ),
            CCProxyError::InvalidProtocolError(protocol_str) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Configuration Error",
                t!(
                    "proxy.error.invalid_protocol",
                    protocol = protocol_str.to_string()
                )
                .to_string(),
            ),
            CCProxyError::InternalError(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal Server Error",
                t!("proxy.error.internal_server_error", error = e.to_string()).to_string(),
            ),
            CCProxyError::ModelAliasNotFound(alias) => (
                StatusCode::NOT_FOUND,
                "Not Found",
                t!(
                    "proxy.error.model_alias_not_found",
                    alias = alias.to_string()
                )
                .to_string(),
            ),
            CCProxyError::ModelDetailsFetchError(id_str) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database Error",
                t!(
                    "proxy.error.model_details_fetch_failed",
                    id = id_str.to_string()
                )
                .to_string(),
            ),
            CCProxyError::StoreLockError(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Store Error",
                t!("proxy.error.store_lock_failed", error = e.to_string()).to_string(),
            ),
        };
        (status, err_type_str, msg_str_slice) // msg_str_slice is already a String
    } else {
        log::warn!("handle_proxy_rejection: Unhandled rejection: {:?}", err);
        log::error!("Unhandled rejection routed to error handler: {:?}", err);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal Server Error",
            t!("proxy.error.unexpected_error").to_string(),
        )
    };

    let error_response =
        serde_json::json!({ "error": { "message": message_string, "type": error_type, }});
    Ok(warp::reply::with_status(
        warp::reply::json(&error_response),
        code,
    ))
}

pub type ProxyResult<T> = std::result::Result<T, Rejection>;
