use crate::error_diagnostics::{ErrorDiagnostic, ErrorOrigin, ErrorProjection, FailureMessage};
use chatspeed_contracts::UserErrorPresentation;
use rust_i18n::t;
use serde::{Serialize, Serializer};
use thiserror::Error;

/// The single, unified error type for the entire application.
///
/// This enum wraps all module-specific errors, providing a consistent structure
/// for error handling across the backend and for serialization to the frontend.
/// The `#[serde(tag = "module", content = "details")]` attribute ensures that
/// the JSON output is clean and predictable.
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

    /// Errors originating from the CCProxy module.
    #[error(transparent)]
    Ccproxy(#[from] crate::ccproxy::CCProxyError),

    #[error(transparent)]
    Mcp(#[from] crate::mcp::McpError),

    #[error(transparent)]
    Sensitive(#[from] crate::sensitive::error::SensitiveError),

    #[error("{message}")]
    General { message: FailureMessage },
}

// The borrowed view keeps legacy enum payloads unchanged while one serializer owns both exits.
#[derive(Serialize)]
#[serde(tag = "module", content = "details")]
enum LegacyAppError<'a> {
    Ai(&'a crate::ai::error::AiError),
    Db(&'a crate::db::error::StoreError),
    Tool(&'a crate::tools::ToolError),
    Workflow(&'a crate::workflow::error::WorkflowError),
    Ccproxy(&'a crate::ccproxy::CCProxyError),
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
            Self::Mcp(error) => error.error_origin(),
            Self::Sensitive(error) => error.error_origin(),
            Self::General { message } => message
                .origin()
                .cloned()
                .unwrap_or_else(|| ErrorOrigin::leaf(message.as_str(), None, None, None)),
            // Legacy workflow and proxy protocol errors keep their own public formatting.
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
            Self::Ccproxy(error) => LegacyAppError::Ccproxy(error),
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

impl<T> From<std::sync::PoisonError<T>> for AppError {
    fn from(err: std::sync::PoisonError<T>) -> Self {
        AppError::Db(crate::db::StoreError::LockError(
            FailureMessage::leaf(
                t!("db.failed_to_lock_main_store", error = err.to_string()).to_string(),
            )
            .with_origin(
                ErrorOrigin::leaf(err.to_string(), None, None, None).context(
                    "db",
                    "lock_main_store",
                    None,
                ),
            ),
        ))
    }
}

/// A universal Result type for Tauri commands and other fallible functions.
pub type Result<T> = std::result::Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn both_error_exits_keep_legacy_shape_and_identical_projection() {
        let variants: Vec<AppError> = vec![
            crate::ai::error::AiError::RawApiRequestFailed {
                status_code: 503,
                provider: "p".into(),
                details: r#"{"error":{"message":"overloaded: try later"}}"#.into(),
            }
            .into(),
            crate::db::StoreError::RuntimeClosed.into(),
            crate::tools::ToolError::FunctionNotFound("missing_tool".into()).into(),
            crate::workflow::error::WorkflowError::Other("legacy only".into()).into(),
            crate::ccproxy::CCProxyError::MissingToken.into(),
            crate::mcp::McpError::NotFound("missing_server".into()).into(),
            crate::sensitive::error::SensitiveError::InvalidRule {
                message: "invalid rule".into(),
            }
            .into(),
            AppError::General {
                message: FailureMessage::leaf("root: https://example.test:123/a\nnext"),
            },
        ];
        for error in variants {
            let legacy = serde_json::to_value(error.legacy()).unwrap();
            let display = error.to_string();
            let direct = serde_json::to_value(&error).unwrap();
            assert_eq!(direct["module"], legacy["module"]);
            assert_eq!(direct["details"], legacy["details"]);
            assert_eq!(direct["message"], display);
            assert_eq!(direct["presentation"]["schema_version"], 1);
            assert!(direct["presentation"]["message"].is_string());
            assert!(direct["presentation"].get("source_chain").is_none());
            let string: String = error.into();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&string).unwrap(),
                direct
            );
        }
    }

    #[test]
    fn nested_ai_payload_keeps_legacy_body_but_exposes_safe_cause() {
        let raw = r#"{"error":{"message":"overloaded: https://example.test/path\nretry","api_key":"CANARY"}}"#;
        let error = AppError::Ai(crate::ai::error::AiError::RawApiRequestFailed {
            status_code: 503,
            provider: "p".into(),
            details: raw.into(),
        });
        let value = serde_json::to_value(error).unwrap();
        assert_eq!(value["details"]["message"]["details"], raw);
        assert_eq!(
            value["presentation"]["message"],
            "overloaded: https://example.test/path\nretry"
        );
        assert_eq!(value["presentation"]["upstream_status"], 503);
        assert_eq!(value["details"]["kind"], "rawApiRequestFailed");
    }

    #[test]
    fn general_preserves_typed_origin_while_legacy_output_stays_string() {
        #[derive(Debug, thiserror::Error)]
        #[error("outer operation")]
        struct Outer {
            #[source]
            source: std::io::Error,
        }
        let error = AppError::General {
            message: FailureMessage::from_error(&Outer {
                source: std::io::Error::other("socket closed"),
            }),
        };
        assert_eq!(error.to_string(), "outer operation");
        assert_eq!(error.user_error().message, "socket closed");
        assert_eq!(error.error_origin().sources, vec!["outer operation"]);
        assert_eq!(
            serde_json::to_value(error).unwrap()["details"],
            json!({"message":"outer operation"})
        );
    }
    #[test]
    fn general_legacy_structure_and_colons_remain_unchanged() {
        let value = serde_json::to_value(AppError::General {
            message: FailureMessage::leaf("bad: reason"),
        })
        .unwrap();
        assert_eq!(value["module"], "General");
        assert_eq!(value["details"], json!({"message":"bad: reason"}));
        assert_eq!(value["presentation"]["message"], "bad: reason");
    }
}
