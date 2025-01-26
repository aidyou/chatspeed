use rust_i18n::t;
use std::fmt;
use thiserror::Error;
use crate::plugins::PluginError;

/// Workflow error types
#[derive(Debug, Error)]
pub enum WorkflowError {
    /// Configuration error
    #[error("{}", t!("workflow.error.config", msg = .0))]
    Config(String),

    /// Plugin error
    #[error("{}", t!("workflow.error.plugin", msg = .0))]
    Plugin(String),

    /// Execution error
    #[error("{}", t!("workflow.error.execution", msg = .0))]
    Execution(String),

    /// Validation error
    #[error("{}", t!("workflow.error.validation", msg = .0))]
    Validation(String),

    /// IO error
    #[error("{}", t!("workflow.error.io", msg = .0))]
    Io(std::io::Error),

    /// Serialization error
    #[error("{}", t!("workflow.error.serialization", msg = .0))]
    Serialization(serde_json::Error),

    /// Invalid state error
    #[error("{}", t!("workflow.error.invalid_state", reason = .0))]
    InvalidState(String),

    /// Other error
    #[error("{}", t!("workflow.error.other", msg = .0))]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

impl From<std::io::Error> for WorkflowError {
    fn from(err: std::io::Error) -> Self {
        WorkflowError::Io(err)
    }
}

impl From<serde_json::Error> for WorkflowError {
    fn from(err: serde_json::Error) -> Self {
        WorkflowError::Serialization(err)
    }
}

impl From<String> for WorkflowError {
    fn from(err: String) -> Self {
        WorkflowError::Other(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            err,
        )))
    }
}

impl From<PluginError> for WorkflowError {
    fn from(error: PluginError) -> Self {
        WorkflowError::Plugin(error.to_string())
    }
}
