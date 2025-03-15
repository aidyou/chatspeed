use pyo3::{PyErr, Python};
use rust_i18n::t;
use serde::Deserialize;
use thiserror::Error;

/// Runtime errors that can occur during plugin execution.
/// Represents various error cases that may arise during runtime operations.
#[derive(Debug, Error, Deserialize)]
pub enum RuntimeError {
    /// Attribute errors
    #[error("{}", t!("plugin.runtime.attribute_error", "error" = _0))]
    AttributeError(String),

    /// Channel send error
    #[error("{}", t!("plugin.runtime.channel_send_error", "error" = _0))]
    ChannelSendError(String),

    /// Channel receive error
    #[error("{}", t!("plugin.runtime.channel_receive_error", "error" = _0))]
    ChannelReceiveError(String),

    /// Errors that occur during runtime initialization
    #[error("{}", t!("plugin.runtime.init_error", "error" = _0))]
    InitError(String),

    /// Errors that occur during code execution
    #[error("{}", t!("plugin.runtime.execution_error", "error" = _0))]
    ExecutionError(String),

    /// Errors related to file system operations
    #[error("{}", t!("plugin.runtime.file_error", "error" = _0))]
    FileError(String),

    /// Errors related to I/O operations
    #[error("{}", t!("plugin.runtime.permission_error", "error" = _0))]
    PermissionError(String),

    /// Errors related to module loading and execution
    #[error("{}", t!("plugin.runtime.module_error", "error" = _0))]
    ModuleError(String),

    /// JSON serialization/deserialization errors
    #[error("{}", t!("plugin.runtime.json_error", "error" = _0))]
    JsonError(String),

    /// Input validation errors
    #[error("{}", t!("plugin.runtime.validation_error", "error" = _0))]
    ValidationError(String),

    /// Environment-related errors (e.g., missing dependencies)
    #[error("{}", t!("plugin.runtime.environment_error", "error" = _0))]
    EnvironmentError(String),

    /// Runtime-specific errors that don't fit other categories
    #[error("{}", t!("plugin.runtime.runtime_specific_error", "error" = _0))]
    RuntimeSpecificError(String),

    /// Syntax errors
    #[error("{}", t!("plugin.runtime.syntax_error", "error" = _0))]
    SyntaxError(String),
}

unsafe impl Send for RuntimeError {}
unsafe impl Sync for RuntimeError {}

// Implement From trait for common error types
impl From<std::io::Error> for RuntimeError {
    fn from(err: std::io::Error) -> Self {
        RuntimeError::FileError(err.to_string())
    }
}

impl From<serde_json::Error> for RuntimeError {
    fn from(err: serde_json::Error) -> Self {
        RuntimeError::JsonError(err.to_string())
    }
}

// Convenience methods for creating errors from strings
impl From<String> for RuntimeError {
    fn from(err: String) -> Self {
        RuntimeError::RuntimeSpecificError(err)
    }
}

impl From<&str> for RuntimeError {
    fn from(err: &str) -> Self {
        RuntimeError::RuntimeSpecificError(err.to_string())
    }
}

/// Custom Result type for Python operations that can fail
pub type PyRuntimeResult<T> = Result<T, RuntimeError>;

impl From<PyErr> for RuntimeError {
    fn from(err: PyErr) -> Self {
        Python::with_gil(|py| match err {
            e if e.is_instance_of::<pyo3::exceptions::PyImportError>(py) => {
                RuntimeError::ModuleError(e.to_string())
            }

            e if e.is_instance_of::<pyo3::exceptions::PyValueError>(py)
                || e.is_instance_of::<pyo3::exceptions::PyTypeError>(py) =>
            {
                RuntimeError::ValidationError(e.to_string())
            }

            e if e.is_instance_of::<pyo3::exceptions::PyAttributeError>(py) => {
                RuntimeError::AttributeError(e.to_string())
            }

            e if e.is_instance_of::<pyo3::exceptions::PyFileNotFoundError>(py) => {
                RuntimeError::FileError(e.to_string())
            }

            e if e.is_instance_of::<pyo3::exceptions::PyPermissionError>(py) => {
                RuntimeError::PermissionError(e.to_string())
            }

            e if e.is_instance_of::<pyo3::exceptions::PyEnvironmentError>(py) => {
                RuntimeError::EnvironmentError(e.to_string())
            }

            e if e.is_instance_of::<pyo3::exceptions::PySyntaxError>(py) => {
                RuntimeError::SyntaxError(e.to_string())
            }

            e if e.is_instance_of::<pyo3::exceptions::PyRuntimeError>(py) => {
                RuntimeError::ExecutionError(e.to_string())
            }

            e => RuntimeError::RuntimeSpecificError(e.to_string()),
        })
    }
}

impl From<anyhow::Error> for RuntimeError {
    fn from(err: anyhow::Error) -> Self {
        RuntimeError::RuntimeSpecificError(err.to_string())
    }
}

impl From<RuntimeError> for Box<dyn std::error::Error + Send> {
    fn from(err: RuntimeError) -> Self {
        Box::new(err)
    }
}

impl From<Box<dyn std::error::Error>> for RuntimeError {
    fn from(err: Box<dyn std::error::Error>) -> Self {
        RuntimeError::RuntimeSpecificError(err.to_string())
    }
}
