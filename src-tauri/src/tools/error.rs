use rust_i18n::t;
use serde::Serialize;
use thiserror::Error;

#[derive(Error, Debug, Serialize)]
#[serde(tag = "kind", content = "message", rename_all = "camelCase")]
pub enum ToolError {
    /// Errors related to initial configuration or setup.
    #[error("{}", t!("tools.error.config", details = .0))]
    Config(String),

    /// Errors during the initialization process of a tool or service.
    #[error("{}", t!("tools.error.initialization", details = .0))]
    Initialization(String),

    /// A required tool or function was not found in the registry.
    #[error("{}", t!("tools.error.function_not_found", name = .0))]
    FunctionNotFound(String),

    /// Attempted to register a tool that already exists.
    #[error("{}", t!("tools.error.function_already_exists", name = .0))]
    FunctionAlreadyExists(String),

    /// Invalid parameters were provided to a tool.
    #[error("{}", t!("tools.error.invalid_params", details = .0))]
    InvalidParams(String),

    /// A network operation timed out.
    #[error("{}", t!("tools.error.timeout", details = .0))]
    Timeout(String),

    /// A general network error occurred.
    #[error("{}", t!("tools.error.network_error", details = .0))]
    NetworkError(String),

    /// An I/O error occurred (e.g., file not found, permission denied).
    #[error("{}", t!("tools.error.io_error", details = .0))]
    IoError(String),

    /// Authentication or authorization failed.
    #[error("{}", t!("tools.error.auth_error", details = .0))]
    AuthError(String),

    /// A generic error during tool execution that doesn't fit other categories.
    #[error("{}", t!("tools.error.execution_failed", details = .0))]
    ExecutionFailed(String),

    /// A non-recoverable error that should terminate the workflow.
    #[error("{}", t!("tools.error.fatal", details = .0))]
    Fatal(String),

    /// An error occurred with an MCP (Model Control Protocol) server.
    #[error("{}", t!("tools.error.mcp_server_not_found", name = .0))]
    McpServerNotFound(String),

    /// Error during serialization or deserialization.
    #[error("{}", t!("tools.error.serialization", details = .0))]
    Serialization(String),

    /// Failed to change the state of a resource.
    #[error("{}", t!("tools.error.state_change_failed", details = .0))]
    StateChangeFailed(String),

    /// Error accessing the underlying data store.
    #[error("{}", t!("tools.error.store", details = .0))]
    Store(String),
}

impl From<String> for ToolError {
    fn from(s: String) -> Self {
        // Default to a generic execution failure for simple string conversions.
        ToolError::ExecutionFailed(s)
    }
}
