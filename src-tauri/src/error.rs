use rust_i18n::t;
use serde::Serialize;
use thiserror::Error;

/// The single, unified error type for the entire application.
///
/// This enum wraps all module-specific errors, providing a consistent structure
/// for error handling across the backend and for serialization to the frontend.
/// The `#[serde(tag = "module", content = "details")]` attribute ensures that
/// the JSON output is clean and predictable.
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

    /// Errors originating from the HTTP module.
    #[error(transparent)]
    Http(#[from] crate::http::error::HttpError),
    /// Errors originating from the CCProxy module.
    #[error(transparent)]
    Ccproxy(#[from] crate::ccproxy::CCProxyError),

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

impl<T> From<std::sync::PoisonError<T>> for AppError {
    fn from(err: std::sync::PoisonError<T>) -> Self {
        AppError::Db(crate::db::StoreError::LockError(
            t!("db.failed_to_lock_main_store", error = err.to_string()).to_string(),
        ))
    }
}

/// A universal Result type for Tauri commands and other fallible functions.
pub type Result<T> = std::result::Result<T, AppError>;
