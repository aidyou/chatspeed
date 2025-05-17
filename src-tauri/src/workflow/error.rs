use rust_i18n::t;
use thiserror::Error;

/// Workflow error types
#[derive(Debug, Error, Clone)]
pub enum WorkflowError {
    /// Cancelled error
    #[error("{0}")]
    Cancelled(String),

    /// Circular dependency error
    #[error("{}", t!("workflow.circular_dependency", nodes = .0))]
    CircularDependency(String),

    /// Configuration error
    #[error("{0}")]
    Config(String),

    /// Context error
    #[error("{0}")]
    Context(String),

    /// Execution error
    #[error("{0}")]
    Execution(String),

    /// Function error
    #[error("{}", t!("workflow.function", msg = .0))]
    Function(String),

    /// Function not found
    #[error("{}", t!("workflow.function_not_found", name = .0))]
    FunctionNotFound(String),

    /// Function already exists
    #[error("{}", t!("workflow.function_already_exists", name = .0))]
    FunctionAlreadyExists(String),

    /// Function parameter error
    #[error("{0}")]
    FunctionParamError(String),

    #[error("{}", t!("workflow.mcp_server_not_found", name = .0))]
    McpServerNotFound(String),

    /// Max retries exceeded
    #[error("{}", t!("workflow.max_retries_exceeded", item = .0))]
    // Parameter name 'item' might be more generic
    MaxRetriesExceeded(String),

    #[error("{0}")]
    Initialization(String),

    /// Invalid state error
    #[error("{0}")]
    InvalidState(String),

    /// Invalid graph error
    #[error("{0}")]
    InvalidGraph(String),

    /// IO error
    #[error("{0}")]
    Io(String),

    /// Serialization error
    #[error("{0}")]
    Serialization(String),

    /// State change failed
    #[error("{0}")]
    StateChangeFailed(String),

    /// Store error
    #[error("{0}")]
    Store(String),

    /// Timeout error
    #[error("{}", t!("workflow.timeout", operation = .0))]
    // Parameter name 'operation' might be more generic
    Timeout(String),

    /// Validation error
    #[error("{}", t!("workflow.validation", msg = .0))]
    Validation(String),

    /// Other error
    #[error("{0}")]
    Other(String),
}

impl WorkflowError {
    /// Determines if the error is retriable
    pub fn is_retriable(&self) -> bool {
        match self {
            // Consider which execution errors are truly retriable.
            // Network-related or transient Execution errors might be.
            // Errors from Function execution might depend on the function.
            Self::Io(_) | Self::Timeout(_) | Self::Execution(_) => true,
            Self::Function(details) => !details.contains("parameter error"), // Example: make parameter errors non-retriable
            _ => false,
        }
    }
}

impl From<std::io::Error> for WorkflowError {
    fn from(err: std::io::Error) -> Self {
        // Construct the full message here
        WorkflowError::Io(t!("workflow.io_error_details", details = err.to_string()).to_string())
    }
}

impl From<serde_json::Error> for WorkflowError {
    fn from(err: serde_json::Error) -> Self {
        // Construct the full message here
        WorkflowError::Serialization(
            t!(
                "workflow.serialization_error_details",
                details = err.to_string()
            )
            .to_string(),
        )
    }
}

// This From<String> implementation can be too broad and might hide more specific errors.
// It's often better to require more explicit error wrapping.
// Changed to WorkflowError::Other to signify a less specific origin.
impl From<String> for WorkflowError {
    fn from(err: String) -> Self {
        WorkflowError::Other(err)
    }
}

// Implement the From trait for a specific type instead of using a generic implementation
// This avoids conflicts with the standard library's impl<T> From<T> for T
impl From<Box<dyn std::error::Error + Send + Sync>> for WorkflowError {
    fn from(err: Box<dyn std::error::Error + Send + Sync>) -> Self {
        WorkflowError::Other(err.to_string()) // .to_string() is usually preferred over debug format
    }
}
