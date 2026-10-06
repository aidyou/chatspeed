//! The desktop Tauri-command error contract.
//!
//! This is the desktop's own error union: it is the wire shape a `#[tauri::command]`
//! returns to the frontend, so it must be able to name the desktop-only error
//! sources the runtime must never link — the legacy HTTP server
//! (`crate::http::error::HttpError`), the updater (`crate::updater::UpdateError`)
//! and the Tauri/Wry host errors. The desktop-free runtime keeps its own
//! transport-neutral `AppError` in `chatspeed_runtime_backend::error`; the two
//! enums share the same `#[serde(tag = "module", content = "details")]` shape and
//! the same variant set for the shared domains, so the frontend parses both
//! identically.
//!
//! The two definitions cannot be unified: `From<tauri::Error>`/`From<wry::Error>`
//! are inherent conversions on this enum, and an inherent impl may only be written
//! where the type is defined, while the runtime backend must not depend on Tauri,
//! Wry or an updater error type.

use rust_i18n::t;
use serde::Serialize;
use thiserror::Error;

/// The single, unified error type for the desktop Tauri surface.
///
/// The enum wraps every module-specific error, providing a consistent structure
/// for error handling across the desktop commands and for serialization to the
/// frontend. The `#[serde(tag = "module", content = "details")]` attribute keeps
/// the JSON output clean and predictable.
#[derive(Error, Debug, Serialize)]
#[serde(tag = "module", content = "details")]
pub enum AppError {
    #[error(transparent)]
    Ai(#[from] crate::ai::error::AiError),

    #[error(transparent)]
    Db(#[from] crate::db::error::StoreError),

    #[error(transparent)]
    Tool(#[from] crate::tools::ToolError),

    #[error(transparent)]
    Workflow(#[from] crate::workflow::error::WorkflowError),

    /// Errors originating from the desktop HTTP server module.
    #[error(transparent)]
    Http(#[from] crate::http::error::HttpError),
    /// Errors originating from the CCProxy module.
    #[error(transparent)]
    Ccproxy(#[from] crate::ccproxy::CCProxyError),

    /// Errors originating from the updater, which only the desktop client runs.
    #[error(transparent)]
    Updater(#[from] crate::updater::UpdateError),

    #[error(transparent)]
    Mcp(#[from] crate::mcp::McpError),

    #[error(transparent)]
    Sensitive(#[from] crate::sensitive::error::SensitiveError),

    #[error("{message}")]
    General { message: String },
}

// This allows Tauri commands to return AppError directly.
impl From<AppError> for String {
    fn from(error: AppError) -> Self {
        let error_message = error.to_string();

        match serde_json::to_value(&error) {
            Ok(mut value) => {
                if let Some(obj) = value.as_object_mut() {
                    obj.insert(
                        "message".to_string(),
                        serde_json::Value::String(error_message),
                    );
                }
                // This final serialization should ideally not fail if `to_value` succeeded.
                serde_json::to_string(&value).unwrap_or_else(|e| {
                    serde_json::json!({
                        "module": "Internal",
                        "details": {
                            "kind": "SerializationFailed",
                            "message": format!("Failed to re-serialize error value: {}", e)
                        },
                        "message": "An unexpected error occurred during error handling.".to_string()
                    })
                    .to_string()
                })
            }
            Err(e) => {
                // Fallback if the initial serialization to `Value` fails.
                serde_json::json!({
                    "module": "Internal",
                    "details": {
                        "kind": "SerializationFailed",
                        "message": format!("Failed to serialize error: {}", e)
                    },
                    "message": error_message
                })
                .to_string()
            }
        }
    }
}

impl From<tauri::Error> for AppError {
    fn from(err: tauri::Error) -> Self {
        AppError::General {
            message: err.to_string(),
        }
    }
}

/// Errors reported by the `wry` webview that carries the ChatHub page.
impl From<wry::Error> for AppError {
    fn from(err: wry::Error) -> Self {
        AppError::General {
            message: err.to_string(),
        }
    }
}

impl<T> From<std::sync::PoisonError<T>> for AppError {
    fn from(err: std::sync::PoisonError<T>) -> Self {
        AppError::Db(crate::db::StoreError::LockError(
            t!("db.failed_to_lock_main_store", error = err.to_string()).to_string(),
        ))
    }
}

/// A universal Result type for Tauri commands and other fallible functions.
pub type Result<T> = std::result::Result<T, AppError>;
