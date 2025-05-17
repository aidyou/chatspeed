use rust_i18n::t;
use serde::Serialize;
use thiserror::Error;

/// Represents errors that can occur in the configuration store.
#[derive(Error, Debug, Serialize)]
pub enum StoreError {
    /// Error variant for Tauri-related errors.
    #[error("{0}")]
    TauriError(String),

    /// Error variant for database-related errors.
    #[error("{0}")]
    DatabaseError(String),

    #[error("{0}")]
    InvalidData(String),

    /// Error variant for JSON-related errors.
    #[error("{0}")]
    JsonError(String),

    /// Error variant for I/O-related errors.
    #[error("{0}")]
    IoError(String),

    /// Error variant for not found errors.
    #[error("{0}")]
    NotFound(String),

    #[error("{0}")]
    StringError(String),

    /// Error variant for already exists errors.
    #[error("{}", t!("db.already_exists", error = _0))]
    AlreadyExists(String),
}

/// Macro to implement the `From` trait for converting specific error types into `StoreError`.
///
/// # Arguments
///
/// - `$variant`: The variant of `StoreError` to use.
/// - `$error_type`: The type of the error to convert from.
///
/// This macro reduces boilerplate by automating the implementation of the `From` trait
/// for different error types, allowing for seamless error conversion.
macro_rules! impl_from_error {
    ($variant:ident, $error_type:ty, $t_key:literal) => {
        impl From<$error_type> for StoreError {
            /// Converts the given error into a `StoreError` variant.
            ///
            /// It now uses the provided t! macro key to format the error message.
            /// # Arguments
            ///
            /// - `err`: The error to convert.
            ///
            /// # Returns
            ///
            /// A `StoreError` variant containing the error message.
            fn from(err: $error_type) -> Self {
                StoreError::$variant(t!($t_key, error = err.to_string()).to_string())
            }
        }
    };
}

impl From<StoreError> for rusqlite::Error {
    fn from(err: StoreError) -> Self {
        match err {
            StoreError::DatabaseError(msg) => rusqlite::Error::InvalidParameterName(msg),
            StoreError::NotFound(_) => rusqlite::Error::QueryReturnedNoRows,
            _ => rusqlite::Error::InvalidParameterName(err.to_string()),
        }
    }
}

// Implement `From` trait for specific error types using the `impl_from_error` macro.
impl_from_error!(DatabaseError, rusqlite::Error, "db.database_error");
impl_from_error!(TauriError, tauri::Error, "db.tauri_error");
impl_from_error!(JsonError, serde_json::Error, "db.json_error");
impl_from_error!(IoError, std::io::Error, "db.io_error");
