//! Error types for the update process
//!
//! Defines the error types that can occur during the update process.

use rust_i18n::t;

/// Errors that can occur during the update process
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    /// Error during version check
    #[error("{}", t!("updater.version_check_failed", "error" = _0))]
    VersionCheckError(String),

    /// Error during download
    #[error("{}", t!("updater.download_failed", "error" = _0))]
    DownloadError(String),

    /// Configuration error
    #[error("{}", t!("updater.config_error", "error" = _0))]
    ConfigError(String),

    /// Error during version check
    #[error("{}", t!("updater.check_failed", "error" = _0))]
    CheckError(String),

    /// Version mismatch
    #[error("{}", t!("updater.version_mismatch"))]
    VersionMismatch,

    /// Update not found
    #[error("{}", t!("updater.update_not_found"))]
    UpdateNotFound,
}

pub type Result<T> = std::result::Result<T, UpdateError>;
