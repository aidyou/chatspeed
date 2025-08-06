//! Error types for the update process
//!
//! Defines the error types that can occur during the update process.

use rust_i18n::t;

/// Errors that can occur during the update process
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    /// Error parsing a version string.
    #[error("{}", t!("updater.errors.version_parse", version = .0, error = .1))]
    VersionParseError(String, String),

    /// Error during the update check request.
    #[error("{}", t!("updater.errors.update_request", error = .0))]
    UpdateRequestError(String),

    /// Error during the download and installation process.
    #[error("{}", t!("updater.errors.download", error = .0))]
    DownloadError(String),

    /// Configuration error, e.g., updater not properly initialized.
    #[error("{}", t!("updater.errors.config", error = .0))]
    ConfigError(String),

    /// The version from the server does not match the version the user intended to install.
    #[error("{}", t!("updater.errors.version_mismatch"))]
    VersionMismatch,

    /// No update was found on the server when one was expected.
    #[error("{}", t!("updater.errors.update_not_found"))]
    UpdateNotFound,
}

pub type Result<T> = std::result::Result<T, UpdateError>;
