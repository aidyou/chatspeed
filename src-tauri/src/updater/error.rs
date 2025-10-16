//! Error types for the update process
//!
//! Defines the error types that can occur during the update process.

use rust_i18n::t;
use serde::Serialize;
use thiserror::Error;

/// Errors that can occur during the update process
#[derive(Error, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum UpdateError {
    /// Error parsing a version string.
    #[error("{}", t!("updater.errors.version_parse", version = version, error = error))]
    VersionParseError { version: String, error: String },

    /// Error during the update check request.
    #[error("{}", t!("updater.errors.update_request", error = _0))]
    UpdateRequestError(String),

    /// Error during the download and installation process.
    #[error("{}", t!("updater.errors.download", error = _0))]
    DownloadError(String),

    /// Configuration error, e.g., updater not properly initialized.
    #[error("{}", t!("updater.errors.config", error = _0))]
    ConfigError(String),

    /// Error during the update installation process.
    #[error("{}", t!("updater.errors.install", error = _0))]
    InstallError(String),

    /// IO error.
    #[error("{}", t!("updater.errors.io_error", error = _0))]
    IoError(String),

    /// Error acquiring a mutex lock.
    #[error("{}", t!("updater.errors.lock_error", error = _0))]
    LockError(String),

    /// The version from the server does not match the version the user intended to install.
    #[error("{}", t!("updater.errors.version_mismatch"))]
    VersionMismatch,

    /// No update was found on the server when one was expected.
    #[error("{}", t!("updater.errors.update_not_found"))]
    UpdateNotFound,
}

impl From<std::io::Error> for UpdateError {
    fn from(err: std::io::Error) -> Self {
        UpdateError::IoError(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, UpdateError>;
