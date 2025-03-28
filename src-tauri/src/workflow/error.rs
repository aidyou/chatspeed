use rust_i18n::t;
use thiserror::Error;

/// Workflow error types
#[derive(Debug, Error, Clone)]
pub enum WorkflowError {
    /// Cancelled error
    #[error("{}", t!("workflow.cancelled", msg = .0))]
    Cancelled(String),

    /// Circular dependency error
    #[error("{}", t!("workflow.circular_dependency", nodes = .0))]
    CircularDependency(String),

    /// Configuration error
    #[error("{}", t!("workflow.config", msg = .0))]
    Config(String),

    /// Context error
    #[error("{}", t!("workflow.context", msg = .0))]
    Context(String),

    /// Execution error
    #[error("{}", t!("workflow.execution", msg = .0))]
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
    #[error("{}", t!("workflow.function_param_error", name = .0))]
    FunctionParamError(String),

    /// Max retries exceeded
    #[error("{}", t!("workflow.max_retries_exceeded", node = .0))]
    MaxRetriesExceeded(String),

    /// Invalid state error
    #[error("{}", t!("workflow.invalid_state", reason = .0))]
    InvalidState(String),

    /// Invalid graph error
    #[error("{}", t!("workflow.invalid_graph", reason = .0))]
    InvalidGraph(String),

    /// IO error
    #[error("{}", t!("workflow.io", msg = .0))]
    Io(String),

    /// Serialization error
    #[error("{}", t!("workflow.serialization", msg = .0))]
    Serialization(String),

    /// State change failed
    #[error("{}", t!("workflow.state_change_failed", reason = .0))]
    StateChangeFailed(String),

    /// Store error
    #[error("{}", t!("workflow.store", msg = .0))]
    Store(String),

    /// Timeout error
    #[error("{}", t!("workflow.timeout", node = .0))]
    Timeout(String),

    /// Validation error
    #[error("{}", t!("workflow.validation", msg = .0))]
    Validation(String),

    /// Other error
    #[error("{}", t!("workflow.other", msg = .0))]
    Other(String),
}

impl WorkflowError {
    /// 判断错误是否可以重试
    pub fn is_retriable(&self) -> bool {
        match self {
            Self::Function(_) | Self::Io(_) | Self::Timeout(_) => true,
            _ => false,
        }
    }
}

impl From<std::io::Error> for WorkflowError {
    fn from(err: std::io::Error) -> Self {
        WorkflowError::Io(err.to_string())
    }
}

impl From<serde_json::Error> for WorkflowError {
    fn from(err: serde_json::Error) -> Self {
        WorkflowError::Serialization(err.to_string())
    }
}

impl From<String> for WorkflowError {
    fn from(err: String) -> Self {
        WorkflowError::Execution(err)
    }
}

// 实现特定类型的 From 特征，而不是使用泛型实现
// 这样可以避免与标准库中的 impl<T> From<T> for T 冲突
impl From<Box<dyn std::error::Error + Send + Sync>> for WorkflowError {
    fn from(err: Box<dyn std::error::Error + Send + Sync>) -> Self {
        WorkflowError::Other(format!("{:?}", err))
    }
}
