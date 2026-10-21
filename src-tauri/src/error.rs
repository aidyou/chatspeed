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

use chatspeed_contracts::UserErrorPresentation;
use chatspeed_runtime_backend::error_diagnostics::{
    ErrorDiagnostic, ErrorOrigin, ErrorProjection, FailureMessage,
};
use rust_i18n::t;
use serde::{Serialize, Serializer};
use thiserror::Error;

/// The single, unified error type for the desktop Tauri surface.
///
/// The enum wraps every module-specific error, providing a consistent structure
/// for error handling across the desktop commands and for serialization to the
/// frontend. The `#[serde(tag = "module", content = "details")]` attribute keeps
/// the JSON output clean and predictable.
#[derive(Error, Debug)]
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
    General { message: FailureMessage },
}

// A borrowed view preserves the legacy payload without recursively serializing AppError.
#[derive(Serialize)]
#[serde(tag = "module", content = "details")]
enum LegacyAppError<'a> {
    Ai(&'a crate::ai::error::AiError),
    Db(&'a crate::db::error::StoreError),
    Tool(&'a crate::tools::ToolError),
    Workflow(&'a crate::workflow::error::WorkflowError),
    Http(&'a crate::http::error::HttpError),
    Ccproxy(&'a crate::ccproxy::CCProxyError),
    Updater(&'a crate::updater::UpdateError),
    Mcp(&'a crate::mcp::McpError),
    Sensitive(&'a crate::sensitive::error::SensitiveError),
    General { message: &'a str },
}

impl AppError {
    pub fn error_origin(&self) -> ErrorOrigin {
        match self {
            Self::Ai(error) => error.error_origin(),
            Self::Db(error) => error.error_origin(),
            Self::Tool(error) => error.error_origin(),
            Self::Http(error) => error.error_origin(),
            Self::Updater(error) => error.error_origin(),
            Self::Mcp(error) => error.error_origin(),
            Self::Sensitive(error) => error.error_origin(),
            Self::General { message } => message
                .origin()
                .cloned()
                .unwrap_or_else(|| ErrorOrigin::leaf(message.as_str(), None, None, None)),
            // Legacy DAG and proxy protocol mappings remain outside this projection.
            Self::Workflow(_) | Self::Ccproxy(_) => {
                ErrorOrigin::leaf(self.to_string(), None, None, None)
            }
        }
    }

    fn legacy(&self) -> LegacyAppError<'_> {
        match self {
            Self::Ai(error) => LegacyAppError::Ai(error),
            Self::Db(error) => LegacyAppError::Db(error),
            Self::Tool(error) => LegacyAppError::Tool(error),
            Self::Workflow(error) => LegacyAppError::Workflow(error),
            Self::Http(error) => LegacyAppError::Http(error),
            Self::Ccproxy(error) => LegacyAppError::Ccproxy(error),
            Self::Updater(error) => LegacyAppError::Updater(error),
            Self::Mcp(error) => LegacyAppError::Mcp(error),
            Self::Sensitive(error) => LegacyAppError::Sensitive(error),
            Self::General { message } => LegacyAppError::General {
                message: message.as_str(),
            },
        }
    }
}

impl ErrorProjection for AppError {
    fn user_error(&self) -> UserErrorPresentation {
        FailureMessage::leaf(self.to_string())
            .with_origin(self.error_origin())
            .user_error()
    }

    fn diagnostic(&self) -> ErrorDiagnostic {
        FailureMessage::leaf(self.to_string())
            .with_origin(self.error_origin())
            .diagnostic()
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Wire<'a> {
            #[serde(flatten)]
            legacy: LegacyAppError<'a>,
            message: String,
            presentation: UserErrorPresentation,
        }
        Wire {
            legacy: self.legacy(),
            message: self.to_string(),
            presentation: self.user_error(),
        }
        .serialize(serializer)
    }
}

impl From<AppError> for String {
    fn from(error: AppError) -> Self {
        serde_json::to_string(&error).unwrap_or_else(|_| {
            serde_json::json!({
                "module": "Internal",
                "details": { "kind": "SerializationFailed", "message": error.to_string() },
                "message": error.to_string(),
                "presentation": error.user_error()
            })
            .to_string()
        })
    }
}

impl From<tauri::Error> for AppError {
    fn from(err: tauri::Error) -> Self {
        AppError::General {
            message: FailureMessage::from_error(&err),
        }
    }
}

/// Errors reported by the `wry` webview that carries the ChatHub page.
impl From<wry::Error> for AppError {
    fn from(err: wry::Error) -> Self {
        AppError::General {
            message: FailureMessage::from_error(&err),
        }
    }
}

impl<T> From<std::sync::PoisonError<T>> for AppError {
    fn from(err: std::sync::PoisonError<T>) -> Self {
        AppError::Db(crate::db::StoreError::LockError(
            FailureMessage::leaf(
                t!("db.failed_to_lock_main_store", error = err.to_string()).to_string(),
            )
            .with_origin(ErrorOrigin::leaf(err.to_string(), None, None, None)),
        ))
    }
}

/// A universal Result type for Tauri commands and other fallible functions.
pub type Result<T> = std::result::Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::error::HttpError;
    use crate::updater::UpdateError;
    use serde_json::json;

    #[test]
    fn all_desktop_variants_preserve_legacy_shape_and_share_both_exits() {
        let errors: Vec<AppError> = vec![
            crate::ai::error::AiError::RawApiRequestFailed {
                status_code: 503,
                provider: "p".into(),
                details: r#"{"error":{"message":"overloaded"}}"#.into(),
            }
            .into(),
            crate::db::StoreError::RuntimeClosed.into(),
            crate::tools::ToolError::FunctionNotFound("missing".into()).into(),
            crate::workflow::error::WorkflowError::Other("legacy".into()).into(),
            HttpError::Request("request failed".into()).into(),
            crate::ccproxy::CCProxyError::MissingToken.into(),
            UpdateError::VersionMismatch.into(),
            crate::mcp::McpError::NotFound("missing".into()).into(),
            crate::sensitive::error::SensitiveError::InvalidRule {
                message: "invalid".into(),
            }
            .into(),
            AppError::General {
                message: "root: https://example.test/a\nnext".into(),
            },
        ];
        for error in errors {
            let legacy = serde_json::to_value(error.legacy()).unwrap();
            let display = error.to_string();
            let direct = serde_json::to_value(&error).unwrap();
            assert_eq!(direct["module"], legacy["module"]);
            assert_eq!(direct["details"], legacy["details"]);
            assert_eq!(direct["message"], display);
            assert_eq!(
                direct["presentation"],
                serde_json::to_value(error.user_error()).unwrap()
            );
            assert!(direct.get("source_chain").is_none());
            let converted: String = error.into();
            assert_eq!(
                direct,
                serde_json::from_str::<serde_json::Value>(&converted).unwrap()
            );
        }
    }

    #[derive(Debug, thiserror::Error)]
    #[error("outer operation")]
    struct Outer {
        #[source]
        source: std::io::Error,
    }

    #[test]
    fn desktop_domains_keep_string_payloads_and_typed_sources() {
        let source = Outer {
            source: std::io::Error::other("socket closed: peer"),
        };
        let failure = FailureMessage::from_error(&source);
        let http = HttpError::Response(failure.clone());
        assert_eq!(
            serde_json::to_value(&http).unwrap(),
            json!({"kind":"response","message":"outer operation"})
        );
        assert_eq!(http.user_error().message, "socket closed: peer");
        assert_eq!(http.diagnostic().source_chain, vec!["outer operation"]);
        let update = UpdateError::DownloadError(failure);
        assert_eq!(
            serde_json::to_value(&update).unwrap(),
            json!({"kind":"downloadError","message":"outer operation"})
        );
        assert_eq!(update.user_error().message, "socket closed: peer");
        let io = std::io::Error::other("disk unavailable");
        assert_eq!(HttpError::from(io).user_error().message, "disk unavailable");
        let io = std::io::Error::other("disk unavailable");
        assert_eq!(
            UpdateError::from(io).user_error().message,
            "disk unavailable"
        );
    }

    #[test]
    fn desktop_general_preserves_typed_origin_and_legacy_message() {
        #[derive(Debug, thiserror::Error)]
        #[error("outer operation")]
        struct Outer {
            #[source]
            source: std::io::Error,
        }

        let error = AppError::General {
            message: FailureMessage::from_error(&Outer {
                source: std::io::Error::other("socket closed: peer"),
            }),
        };
        assert_eq!(error.to_string(), "outer operation");
        assert_eq!(error.user_error().message, "socket closed: peer");
        assert_eq!(error.error_origin().sources, vec!["outer operation"]);
        assert_eq!(
            serde_json::to_value(error).unwrap()["details"],
            json!({"message":"outer operation"})
        );
    }

    #[test]
    fn updater_parameter_and_unit_variants_keep_localized_descriptions() {
        let parse = UpdateError::VersionParseError {
            version: "invalid-version".into(),
            error: "bad version".into(),
        };
        assert_eq!(parse.user_error().message, parse.to_string());
        assert_eq!(
            serde_json::to_value(&parse).unwrap(),
            json!({"kind":"versionParseError", "message":{"version":"invalid-version", "error":"bad version"}})
        );
        for error in [UpdateError::VersionMismatch, UpdateError::UpdateNotFound] {
            assert_eq!(error.user_error().message, error.to_string());
            assert!(serde_json::to_value(&error)
                .unwrap()
                .get("message")
                .is_none());
        }
        let poison: AppError = std::sync::PoisonError::new(()).into();
        assert_eq!(
            poison.user_error().message,
            std::sync::PoisonError::new(()).to_string()
        );
    }
}
