use rust_i18n::t;
use serde::Serialize;
use thiserror::Error;

/// Workflow error types
#[derive(Debug, Error, Clone, Serialize)]
#[serde(tag = "kind", content = "message", rename_all = "camelCase")]
pub enum WorkflowError {
    /// Cancelled error
    #[error("{}", t!("workflow.error.cancelled", details = .0))]
    Cancelled(String),

    /// Circular dependency error
    #[error("{}", t!("workflow.circular_dependency", nodes = .0))]
    CircularDependency(String),

    /// Configuration error
    #[error("{}", t!("workflow.error.config", details = .0))]
    Config(String),

    /// Context error
    #[error("{}", t!("workflow.error.context", details = .0))]
    Context(String),

    /// Execution error
    #[error("{}", t!("workflow.error.execution", details = .0))]
    Execution(String),

    /// Max retries exceeded
    #[error("{}", t!("tools.max_retries_exceeded", item = .0))]
    MaxRetriesExceeded(String),

    #[error("{}", t!("workflow.error.initialization", details = .0))]
    Initialization(String),

    /// Invalid state error
    #[error("{}", t!("workflow.error.invalid_state", details = .0))]
    InvalidState(String),

    /// Invalid graph error
    #[error("{}", t!("workflow.error.invalid_graph", details = .0))]
    InvalidGraph(String),

    /// IO error
    #[error("{}", t!("workflow.error.io", details = .0))]
    Io(String),

    /// Serialization error
    #[error("{}", t!("workflow.error.serialization", details = .0))]
    Serialization(String),

    /// Store error
    #[error("{}", t!("workflow.error.store", details = .0))]
    Store(String),

    /// Validation error
    #[error("{}", t!("workflow.validation", details = .0))]
    Validation(String),

    /// Other error
    #[error("{}", t!("workflow.error.other", details = .0))]
    Other(String),
}

impl WorkflowError {
    /// Determines if the error is retriable
    pub fn is_retriable(&self) -> bool {
        match self {
            // Consider which execution errors are truly retriable.
            // Network-related or transient Execution errors might be.
            // Errors from Function execution might depend on the function.
            Self::Io(_) => true,
            Self::Execution(details) => !details.contains("parameter error"), // Example: make parameter errors non-retriable
            _ => false,
        }
    }
}

impl From<crate::tools::ToolError> for WorkflowError {
    fn from(err: crate::tools::ToolError) -> Self {
        match err {
            crate::tools::ToolError::Config(msg) => WorkflowError::Config(msg),
            crate::tools::ToolError::Initialization(msg) => WorkflowError::Initialization(msg),
            crate::tools::ToolError::FunctionNotFound(name) => {
                WorkflowError::Execution(format!("Function not found: {}", name))
            }
            crate::tools::ToolError::FunctionAlreadyExists(name) => {
                WorkflowError::Execution(format!("Function already exists: {}", name))
            }
            crate::tools::ToolError::InvalidParams(msg) => WorkflowError::Validation(msg),
            crate::tools::ToolError::Timeout(msg) => {
                WorkflowError::Execution(format!("Timeout: {}", msg))
            }
            crate::tools::ToolError::NetworkError(msg) => {
                WorkflowError::Execution(format!("Network Error: {}", msg))
            }
            crate::tools::ToolError::IoError(msg) => WorkflowError::Io(msg),
            crate::tools::ToolError::AuthError(msg) => {
                WorkflowError::Execution(format!("Auth Error: {}", msg))
            }
            crate::tools::ToolError::ExecutionFailed(msg) => WorkflowError::Execution(msg),
            crate::tools::ToolError::Fatal(msg) => WorkflowError::Execution(msg),
            crate::tools::ToolError::McpServerNotFound(name) => {
                WorkflowError::Execution(format!("MCP server not found: {}", name))
            }
            crate::tools::ToolError::Serialization(error) => WorkflowError::Serialization(error),
            crate::tools::ToolError::StateChangeFailed(msg) => WorkflowError::Other(msg),
            crate::tools::ToolError::Store(msg) => WorkflowError::Store(msg),
        }
    }
}
