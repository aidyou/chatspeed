use warp::{filters::cors::CorsForbidden, http::StatusCode, Rejection, Reply};

/// Custom error types for the ccproxy module.
#[derive(Debug)]
pub enum ProxyAuthError {
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

impl warp::reject::Reject for ProxyAuthError {}

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
            "not_found",
            "Endpoint not found.".to_string(),
        )
    } else if err.find::<CorsForbidden>().is_some() {
        log::debug!(
            "handle_proxy_rejection: Received 'CorsForbidden': {:?}",
            err
        );
        (
            StatusCode::FORBIDDEN, // More appropriate status for CORS rejections
            "cors_error",
            "Request blocked by CORS policy.".to_string(),
        )
    } else if let Some(auth_error) = err.find::<ProxyAuthError>() {
        let (status, err_type_str, msg_str_slice) = match auth_error {
            ProxyAuthError::InvalidToken => (
                StatusCode::UNAUTHORIZED,
                "authentication_error",
                "Invalid API key (token).",
            ),
            ProxyAuthError::InternalError(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_server_error",
                &*format!("Internal server error: {}", e),
            ),
            ProxyAuthError::MissingToken => (
                StatusCode::UNAUTHORIZED,
                "authentication_error",
                "Authorization header is missing or invalid.",
            ),
            ProxyAuthError::ModelAliasNotFound(alias) => (
                StatusCode::NOT_FOUND,
                "not_found",
                &*format!("Model alias '{}' not found.", alias),
            ),
            ProxyAuthError::NoBackendTargets(alias) => (
                StatusCode::BAD_REQUEST,
                "configuration_error",
                &*format!("No backend targets configured for model alias '{}'.", alias),
            ),
            ProxyAuthError::NoKeysConfigured => (
                StatusCode::UNAUTHORIZED,
                "configuration_error",
                "Proxy access keys are not configured on the server.",
            ),
            ProxyAuthError::ModelDetailsFetchError(id_str) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "database_error",
                &*format!("Failed to fetch model details for provider_id: {}", id_str),
            ),
            ProxyAuthError::InvalidProtocolError(protocol_str) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "configuration_error",
                &*format!("Invalid protocol configured for model: {}", protocol_str),
            ),

            ProxyAuthError::StoreLockError(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "store_error",
                &*format!("Failed to access store: {}", e),
            ),
        };
        (status, err_type_str, msg_str_slice.to_string()) // Convert message to String
    } else {
        log::warn!("handle_proxy_rejection: Unhandled rejection: {:?}", err);
        // Consider making this log message more generic if the handler is truly global
        log::error!("Unhandled rejection routed to error handler: {:?}", err);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_server_error",
            "An unexpected error occurred.".to_string(),
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
