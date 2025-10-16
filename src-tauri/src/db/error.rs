use rust_i18n::t;
use serde::Serialize;
use thiserror::Error;

/// Represents errors that can occur in the configuration store.
#[derive(Error, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StoreError {
    /// Error variant for already exists errors.
    #[error("{}", t!("db.already_exists", error = _0))]
    AlreadyExists(String),

    #[error("{0}")]
    InvalidData(String),

    /// Error variant for JSON-related errors.
    #[error("{0}")]
    JsonError(String),

    #[error("{}",t!("db.failed_to_lock_main_store",error = _0))]
    LockError(String),

    /// Error variant for I/O-related errors.
    #[error("{0}")]
    IoError(String),

    /// Error variant for not found errors.
    #[error("{0}")]
    NotFound(String),

    /// Error variant for database-related errors.
    #[error("{0}")]
    Query(String),

    /// Error variant for Tauri-related errors.
    #[error("{0}")]
    TauriError(String),
}

impl From<rusqlite::Error> for StoreError {
    fn from(err: rusqlite::Error) -> Self {
        StoreError::Query(err.to_string())
    }
}

impl From<tauri::Error> for StoreError {
    fn from(err: tauri::Error) -> Self {
        StoreError::TauriError(err.to_string())
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(err: serde_json::Error) -> Self {
        StoreError::JsonError(err.to_string())
    }
}

impl From<std::io::Error> for StoreError {
    fn from(err: std::io::Error) -> Self {
        StoreError::IoError(err.to_string())
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
