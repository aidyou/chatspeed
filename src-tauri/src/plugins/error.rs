use rust_i18n::t;
use std::error::Error;
use thiserror::Error;

/// Unified error type for all plugin-related operations
#[derive(Debug, Error)]
pub enum PluginError {
    // Plugin Manager Errors
    #[error("{}", t!("plugin.errors.not_found", "id" = _0))]
    NotFound(String),

    #[error("{}", t!("plugin.errors.already_exists", "id" = _0))]
    AlreadyExists(String),

    #[error("{}", t!("plugin.errors.init_failed", "id" = _0, "error" = _1))]
    InitializationFailed(String, #[source] Box<dyn Error + Send + Sync>),

    #[error("{}", t!("plugin.errors.destroy_failed", "id" = _0, "error" = _1))]
    DestroyFailed(String, #[source] Box<dyn Error + Send + Sync>),

    #[error("{}", t!("plugin.errors.execution_failed", "id" = _0, "error" = _1))]
    ExecutionFailed(String, #[source] Box<dyn Error + Send + Sync>),

    // Selector Plugin Errors
    #[error("{}", t!("plugin.selector.invalid_selector", "selector" = _0))]
    InvalidSelector(String),

    #[error("{}", t!("plugin.errors.io", "error" = _0))]
    IoError(String),

    #[error("{}", t!("plugin.selector.element_not_found", "selector" = _0))]
    ElementNotFound(String),

    #[error("{}", t!("plugin.selector.parse_failed", "error" = _0))]
    ParseFailed(String),

    #[error("{}", t!("plugin.errors.plugin_type_error", "type" = _0))]
    PluginTypeError(String),

    // Common Errors
    #[error("{}", t!("plugin.errors.invalid_input", "msg" = _0))]
    InvalidInput(String),

    #[error("{}", t!("plugin.errors.invalid_output", "msg" = _0))]
    InvalidOutput(String),

    #[error("{}", t!("plugin.errors.runtime_error", "msg" = _0))]
    RuntimeError(String),

    #[error("{0}")]
    StringError(String),

    #[error(transparent)]
    Other(#[from] Box<dyn Error + Send + Sync>),
}

/// Convenience trait for converting errors to PluginError
pub trait IntoPluginError {
    fn into_plugin_error(self, context: &str) -> PluginError;
}

impl<E: Error + Send + Sync + 'static> IntoPluginError for E {
    fn into_plugin_error(self, context: &str) -> PluginError {
        PluginError::StringError(format!("{}: {}", context, self))
    }
}
