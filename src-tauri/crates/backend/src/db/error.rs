use crate::error_diagnostics::{ErrorDiagnostic, ErrorOrigin, ErrorProjection, FailureMessage};
use chatspeed_contracts::UserErrorPresentation;
use rust_i18n::t;
use serde::Serialize;
use thiserror::Error;

/// Represents errors that can occur in the configuration store.
#[derive(Error, Debug, Serialize)]
#[serde(tag = "kind", content = "message", rename_all = "camelCase")]
pub enum StoreError {
    /// Error variant for already exists errors.
    #[error("{}", t!("db.already_exists", error = _0))]
    AlreadyExists(String),

    #[error("{0}")]
    InvalidData(String),

    /// Error variant for JSON-related errors.
    #[error("{0}")]
    JsonError(FailureMessage),

    #[error("{}",t!("db.failed_to_lock_main_store",error = _0))]
    LockError(FailureMessage),

    /// Error variant for I/O-related errors.
    #[error("{0}")]
    IoError(FailureMessage),

    /// Error variant for not found errors.
    #[error("{0}")]
    NotFound(String),

    /// Error variant for database-related errors.
    #[error("{0}")]
    Query(FailureMessage),

    #[error("Database runtime is closed")]
    RuntimeClosed,

    #[error("Database runtime is in maintenance mode")]
    RuntimeMaintenance,

    #[error("Database worker failed: {0}")]
    WorkerFailed(FailureMessage),

    /// Error variant for Tauri-related errors.
    #[error("{0}")]
    TauriError(String),
}

impl StoreError {
    /// Returns an in-memory typed origin for transfer across application error layers.
    pub fn error_origin(&self) -> ErrorOrigin {
        let (message, origin) = match self {
            Self::JsonError(message)
            | Self::LockError(message)
            | Self::IoError(message)
            | Self::Query(message)
            | Self::WorkerFailed(message) => {
                (message.as_str().to_string(), message.origin().cloned())
            }
            // Parameter variants use their complete legacy descriptions rather than exposing
            // only the caller-supplied value.
            _ => (self.to_string(), None),
        };
        origin
            .unwrap_or_else(|| ErrorOrigin::leaf(message, None, None, None))
            .context("db", self.error_operation(), None)
    }

    fn error_operation(&self) -> &'static str {
        match self {
            Self::AlreadyExists(_) => "already_exists",
            Self::InvalidData(_) => "validate_data",
            Self::JsonError(_) => "json",
            Self::LockError(_) => "lock_store",
            Self::IoError(_) => "io",
            Self::NotFound(_) => "lookup",
            Self::Query(_) => "query",
            Self::RuntimeClosed => "runtime_closed",
            Self::RuntimeMaintenance => "runtime_maintenance",
            Self::WorkerFailed(_) => "worker",
            Self::TauriError(_) => "tauri",
        }
    }
}

impl ErrorProjection for StoreError {
    fn user_error(&self) -> UserErrorPresentation {
        FailureMessage::leaf(self.to_string())
            .with_origin(self.error_origin())
            .user_error()
    }

    fn diagnostic(&self) -> ErrorDiagnostic {
        let mut diagnostic = FailureMessage::leaf(self.to_string())
            .with_origin(self.error_origin())
            .diagnostic();
        diagnostic.module = "db".into();
        diagnostic.operation = self.error_operation().into();
        diagnostic
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(err: rusqlite::Error) -> Self {
        StoreError::Query(FailureMessage::from_error(&err))
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(err: serde_json::Error) -> Self {
        StoreError::JsonError(FailureMessage::from_error(&err))
    }
}

impl From<std::io::Error> for StoreError {
    fn from(err: std::io::Error) -> Self {
        StoreError::IoError(FailureMessage::from_error(&err))
    }
}

impl From<StoreError> for rusqlite::Error {
    fn from(err: StoreError) -> Self {
        match err {
            StoreError::NotFound(_) => rusqlite::Error::QueryReturnedNoRows,
            _ => rusqlite::Error::InvalidParameterName(err.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn all_variants_preserve_legacy_display_and_serde_shape() {
        let cases = [
            (
                StoreError::AlreadyExists("duplicate".into()),
                json!({"kind":"alreadyExists","message":"duplicate"}),
            ),
            (
                StoreError::InvalidData("invalid".into()),
                json!({"kind":"invalidData","message":"invalid"}),
            ),
            (
                StoreError::JsonError("invalid json".into()),
                json!({"kind":"jsonError","message":"invalid json"}),
            ),
            (
                StoreError::LockError("poisoned".into()),
                json!({"kind":"lockError","message":"poisoned"}),
            ),
            (
                StoreError::IoError("disk unavailable".into()),
                json!({"kind":"ioError","message":"disk unavailable"}),
            ),
            (
                StoreError::NotFound("record".into()),
                json!({"kind":"notFound","message":"record"}),
            ),
            (
                StoreError::Query("query failed".into()),
                json!({"kind":"query","message":"query failed"}),
            ),
            (StoreError::RuntimeClosed, json!({"kind":"runtimeClosed"})),
            (
                StoreError::RuntimeMaintenance,
                json!({"kind":"runtimeMaintenance"}),
            ),
            (
                StoreError::WorkerFailed("worker failed".into()),
                json!({"kind":"workerFailed","message":"worker failed"}),
            ),
            (
                StoreError::TauriError("tauri failed".into()),
                json!({"kind":"tauriError","message":"tauri failed"}),
            ),
        ];

        for (error, serialized) in cases {
            assert_eq!(serde_json::to_value(&error).unwrap(), serialized);
            assert!(!error.to_string().is_empty());
        }
        assert_eq!(
            StoreError::RuntimeClosed.to_string(),
            "Database runtime is closed"
        );
        assert_eq!(
            StoreError::RuntimeMaintenance.to_string(),
            "Database runtime is in maintenance mode"
        );
        assert_eq!(
            StoreError::WorkerFailed("worker failed".into()).to_string(),
            "Database worker failed: worker failed"
        );
    }

    #[test]
    fn typed_io_and_json_errors_retain_origin_and_projection_redacts_secrets() {
        let io_error = StoreError::from(std::io::Error::other("token=CANARY"));
        assert_eq!(io_error.error_origin().message, "token=CANARY");
        assert_eq!(io_error.error_origin().frames[0].module, "db");
        assert_eq!(io_error.user_error().message, "token=\"[redacted]\"");

        let json_error = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
        let json_error = StoreError::from(json_error);
        assert!(!json_error.error_origin().message.is_empty());
        assert_eq!(json_error.error_origin().frames[0].operation, "json");
    }

    #[test]
    fn parameter_descriptions_keep_legacy_display_and_serialization() {
        let already_exists = StoreError::AlreadyExists("name".into());
        assert_eq!(
            already_exists.to_string(),
            rust_i18n::t!("db.already_exists", error = "name").to_string()
        );
        assert_eq!(
            serde_json::to_value(&already_exists).unwrap(),
            json!({"kind":"alreadyExists","message":"name"})
        );

        let invalid = StoreError::InvalidData("caller description".into());
        assert_eq!(invalid.to_string(), "caller description");
        assert_eq!(
            serde_json::to_value(&invalid).unwrap()["message"],
            "caller description"
        );
    }
}
