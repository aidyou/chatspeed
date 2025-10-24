use crate::mcp::server::persistent_session::{EventIdParseError, SessionError};
use rmcp::model::{ErrorCode, ErrorData};
use rust_i18n::t;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "kind", content = "message", rename_all = "camelCase")]
pub enum McpError {
    // Client-side errors
    #[error("{}", t!("mcp.error.client_call_failed", error = _0))]
    ClientCallError(String),
    #[error("{}", t!("mcp.error.client_config_failed", error = _0))]
    ClientConfigError(String),
    #[error("{}", t!("mcp.error.client_start_failed", error = _0))]
    ClientStartError(String),
    #[error("{}", t!("mcp.error.client_stop_failed", error = _0))]
    ClientStopError(String),
    #[error("{}", t!("mcp.error.client_status_failed", error = _0))]
    ClientStatusError(String),

    // Server-side errors (from rmcp::ErrorData or internal server issues)
    #[error("{}", t!("mcp.error.server_initialization_failed", error = _0))]
    ServerInitializationError(String),
    #[error("{}", t!("mcp.error.server_tool_not_found", tool_name = _0))]
    ServerToolNotFound(String),
    #[error("{}", t!("mcp.error.server_tool_execution_failed", error = _0))]
    ServerToolExecutionError(String),
    #[error("{}", t!("mcp.error.server_internal_error", error = _0))]
    ServerInternalError(String),
    #[error("{}", t!("mcp.error.server_unknown_error", error = _0))]
    ServerUnknownError(String),

    // Common errors
    #[error("{}", t!("mcp.error.io_error", error = _0))]
    Io(String),

    #[error("{}", t!("mcp.error.not_found_error", name = _0))]
    NotFound(String),

    #[error("{}", t!("mcp.error.serialization_error", error = _0))]
    Serialization(String),
    #[error("{}", t!("mcp.error.store_error", error = _0))]
    Store(String),
    #[error("{}", t!("mcp.error.state_change_failed", error = _0))]
    StateChangeFailed(String),
    #[error("{}", t!("mcp.error.timeout_error", error = _0))]
    Timeout(String),

    #[error("{0}")]
    General(String),
}

impl From<McpError> for ErrorData {
    fn from(error: McpError) -> Self {
        ErrorData::new(ErrorCode::INTERNAL_ERROR, error.to_string(), None)
    }
}

impl From<SessionError> for McpError {
    fn from(error: SessionError) -> Self {
        McpError::General(error.to_string())
    }
}

impl From<serde_json::Error> for McpError {
    fn from(error: serde_json::Error) -> Self {
        McpError::Serialization(error.to_string())
    }
}

impl From<sled::Error> for McpError {
    fn from(error: sled::Error) -> Self {
        McpError::Store(error.to_string())
    }
}

impl From<std::io::Error> for McpError {
    fn from(error: std::io::Error) -> Self {
        McpError::Io(error.to_string())
    }
}

impl From<tokio::time::error::Elapsed> for McpError {
    fn from(error: tokio::time::error::Elapsed) -> Self {
        McpError::Timeout(error.to_string())
    }
}

impl From<crate::tools::ToolError> for McpError {
    fn from(error: crate::tools::ToolError) -> Self {
        McpError::General(error.to_string())
    }
}

impl From<EventIdParseError> for McpError {
    fn from(error: EventIdParseError) -> Self {
        McpError::General(error.to_string())
    }
}

impl From<crate::mcp::server::persistent_session::LocalSessionWorkerError> for McpError {
    fn from(error: crate::mcp::server::persistent_session::LocalSessionWorkerError) -> Self {
        McpError::General(error.to_string())
    }
}

impl From<anyhow::Error> for McpError {
    fn from(error: anyhow::Error) -> Self {
        McpError::General(error.to_string())
    }
}
