use serde::Serialize;
use thiserror::Error;

/// Defines the custom error types for the sensitive data filtering module.
#[derive(Error, Debug, Serialize)]
#[serde(tag = "kind", content = "details")]
pub enum SensitiveError {
    /// An error that occurs when a regular expression fails to compile.
    #[error("Failed to compile regex pattern: {pattern}")]
    RegexCompilationFailed {
        pattern: String,
        message: String,
    },

    /// An error for an invalid filter rule configuration.
    #[error("Invalid filter rule configuration: {message}")]
    InvalidRule { message: String },

    /// Represents I/O errors that might occur, e.g., reading a filter configuration file.
    #[error("I/O error during filter operation: {message}")]
    IoError { message: String },
}