//! Error types for the update process
//!
//! Defines the error types that can occur during the update process.

use chatspeed_contracts::UserErrorPresentation;
use chatspeed_runtime_backend::error_diagnostics::{
    ErrorDiagnostic, ErrorOrigin, ErrorProjection, FailureMessage,
};
use rust_i18n::t;
use serde::Serialize;
use thiserror::Error;

/// Errors that can occur during the update process
#[derive(Error, Debug, Serialize)]
#[serde(tag = "kind", content = "message", rename_all = "camelCase")]
pub enum UpdateError {
    /// Error parsing a version string.
    #[error("{}", t!("updater.errors.version_parse", version = version, error = error))]
    VersionParseError {
        version: String,
        error: FailureMessage,
    },

    /// Error during the update check request.
    #[error("{}", t!("updater.errors.update_request", error = _0))]
    UpdateRequestError(FailureMessage),

    /// Error during the download and installation process.
    #[error("{}", t!("updater.errors.download", error = _0))]
    DownloadError(FailureMessage),

    /// Configuration error, e.g., updater not properly initialized.
    #[error("{}", t!("updater.errors.config", error = _0))]
    ConfigError(FailureMessage),

    /// Error during the update installation process.
    #[error("{}", t!("updater.errors.install", error = _0))]
    InstallError(FailureMessage),

    /// IO error.
    #[error("{}", t!("updater.errors.io_error", error = _0))]
    IoError(FailureMessage),

    /// Error acquiring a mutex lock.
    #[error("{}", t!("updater.errors.lock_error", error = _0))]
    LockError(FailureMessage),

    /// The version from the server does not match the version the user intended to install.
    #[error("{}", t!("updater.errors.version_mismatch"))]
    VersionMismatch,

    /// No update was found on the server when one was expected.
    #[error("{}", t!("updater.errors.update_not_found"))]
    UpdateNotFound,
}

impl UpdateError {
    pub fn error_origin(&self) -> ErrorOrigin {
        let message = match self {
            Self::VersionParseError { error, .. } => {
                return error
                    .origin()
                    .cloned()
                    .unwrap_or_else(|| ErrorOrigin::leaf(self.to_string(), None, None, None))
            }
            Self::UpdateRequestError(message)
            | Self::DownloadError(message)
            | Self::ConfigError(message)
            | Self::InstallError(message)
            | Self::IoError(message)
            | Self::LockError(message) => message,
            Self::VersionMismatch | Self::UpdateNotFound => {
                return ErrorOrigin::leaf(self.to_string(), None, None, None)
            }
        };
        message
            .origin()
            .cloned()
            .unwrap_or_else(|| ErrorOrigin::leaf(message.as_str(), None, None, None))
    }
}

impl ErrorProjection for UpdateError {
    fn user_error(&self) -> UserErrorPresentation {
        FailureMessage::leaf(self.to_string())
            .with_origin(self.error_origin())
            .user_error()
    }
    fn diagnostic(&self) -> ErrorDiagnostic {
        let mut diagnostic = FailureMessage::leaf(self.to_string())
            .with_origin(self.error_origin())
            .diagnostic();
        diagnostic.module = "updater".into();
        diagnostic
    }
}

impl From<std::io::Error> for UpdateError {
    fn from(err: std::io::Error) -> Self {
        UpdateError::IoError(FailureMessage::from_error(&err))
    }
}

pub type Result<T> = std::result::Result<T, UpdateError>;
