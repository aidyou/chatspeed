//! Error types for the update process
//!
//! Defines the error types that can occur during the update process.

use rust_i18n::t;

/// Errors that can occur during the update process
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    /// Error during version check
    #[error("{0}")]
    VersionCheckError(String),

    /// Error during download
    #[error("{0}")]
    DownloadError(String),

    /// Configuration error
    #[error("{0}")]
    ConfigError(String),

    /// Error during version check
    #[error("{0}")]
    CheckError(String),

    /// Version mismatch
    #[error("{}", t!("updater.version_mismatch"))]
    VersionMismatch,

    /// Update not found
    #[error("{}", t!("updater.update_not_found"))]
    UpdateNotFound,
}

pub type Result<T> = std::result::Result<T, UpdateError>;
