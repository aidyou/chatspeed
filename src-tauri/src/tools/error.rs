use rust_i18n::t;
use thiserror::Error;

#[derive(Debug, Error, Clone)]
pub enum ToolError {
    // Configuration error
    #[error("{0}")]
    Config(String),

    /// Execution error
    #[error("{0}")]
    Execution(String),

    #[error("{0}")]
    Initialization(String),

    /// Function not found
    #[error("{}", t!("tools.function_not_found", name = .0))]
    FunctionNotFound(String),

    /// Function already exists
    #[error("{}", t!("tools.function_already_exists", name = .0))]
    FunctionAlreadyExists(String),

    /// Function parameter error
    #[error("{0}")]
    FunctionParamError(String),

    #[error("{}", t!("tools.mcp_server_not_found", name = .0))]
    McpServerNotFound(String),

    /// Serialization error
    #[error("{0}")]
    Serialization(String),

    /// State change failed
    #[error("{0}")]
    StateChangeFailed(String),

    /// Store error
    #[error("{0}")]
    Store(String),
}
