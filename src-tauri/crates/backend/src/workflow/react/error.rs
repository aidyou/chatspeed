// The engine error and its terminal projection are runtime-only: the desktop
// reaches the runtime over the control plane and never constructs or matches the
// engine error, so this module is runtime-only.
#![cfg(not(feature = "desktop"))]

use crate::ai::error::AiError;
use crate::db::error::StoreError;
use crate::error_diagnostics::{ErrorDiagnostic, ErrorOrigin, ErrorProjection, FailureMessage};
use crate::tools::ToolError;

use chatspeed_contracts::UserErrorPresentation;
use serde::Serialize;
use serde_json::{json, Value};
use thiserror::Error;

/// A compression precondition failure.
///
/// This aliases the shared failure carrier so a compression failure keeps one
/// legacy display/serde string plus an optional structured origin, exactly like
/// every other diagnostic-aware error.
pub type CompressionFailure = FailureMessage;

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowTerminalError {
    pub content: String,
    pub error_type: &'static str,
    pub metadata: Value,
}

/// Classifies upstream failures into stable, UI-safe snake_case categories.
pub fn classify_ai_error(error: &AiError) -> &'static str {
    match error {
        AiError::ApiRequestFailed { status_code, .. }
        | AiError::RawApiRequestFailed { status_code, .. } => match status_code {
            401 => "llm_authentication",
            402 => "llm_billing",
            403 | 404 => "engine",
            429 => "rate_limited",
            500..=599 => "upstream_server",
            _ => "upstream_http",
        },
        AiError::StreamProcessingFailed { .. } => "network",
        _ => "unknown",
    }
}

/// Produces a bounded summary without upstream bodies, headers, or credentials.
pub fn retry_error_summary(error: &AiError) -> String {
    match error {
        AiError::ApiRequestFailed {
            status_code,
            provider,
            ..
        }
        | AiError::RawApiRequestFailed {
            status_code,
            provider,
            ..
        } => {
            format!(
                "{} returned HTTP {}",
                provider.chars().take(64).collect::<String>(),
                status_code
            )
        }
        AiError::StreamProcessingFailed { provider, .. } => {
            format!(
                "{} stream/network failure",
                provider.chars().take(64).collect::<String>()
            )
        }
        _ => "LLM request failed".to_string(),
    }
}

#[derive(Error, Debug, Serialize)]
#[serde(tag = "type", content = "details")]
pub enum WorkflowEngineError {
    #[error("{0}")]
    Cancelled(String),

    #[error("Database error: {0}")]
    Db(#[from] StoreError),

    #[error("Tool error: {0}")]
    Tool(#[from] ToolError),

    #[error("AI Model error: {0}")]
    Ai(#[from] AiError),

    #[error("LLM retry attempts exhausted ({attempt}/{max_attempts}): {details}")]
    LlmRetryExhausted {
        #[source]
        source: Option<AiError>,
        details: String,
        attempt: u32,
        max_attempts: u32,
    },

    #[error("Security violation: {0}")]
    Security(String),

    /// A compression precondition failed, so the workflow cannot continue.
    /// Goal tracking runs before the handoff is written, and a handoff without a
    /// tracked goal would silently drift, so this is a direct termination rather
    /// than a recoverable step error.
    #[error("Compression failed: {0}")]
    CompressionFailed(CompressionFailure),

    #[error("Gateway error: {0}")]
    Gateway(String),

    #[error("Gateway input channel closed")]
    GatewayInputChannelClosed,

    #[error("Gateway input channel missing")]
    GatewayInputChannelMissing,

    #[error("General error: {0}")]
    General(String),

    #[error("Dispatcher channel full")]
    DispatcherChannelFull,

    #[error("Dispatcher closed")]
    DispatcherClosed,
}

impl WorkflowEngineError {
    /// Returns the structured cause for the failures that can carry one.
    ///
    /// Raw upstream bodies, provider status codes, and compression precondition
    /// origins stay typed here instead of being re-derived from display text.
    pub fn error_origin(&self) -> Option<ErrorOrigin> {
        match self {
            Self::Ai(error) => Some(error.error_origin()),
            Self::Db(error) => Some(error.error_origin()),
            Self::Tool(error) => Some(error.error_origin()),
            Self::Cancelled(message)
            | Self::Security(message)
            | Self::Gateway(message)
            | Self::General(message) => Some(ErrorOrigin::leaf(message, None, None, None)),
            Self::LlmRetryExhausted {
                source: Some(source),
                ..
            } => Some(source.error_origin()),
            Self::CompressionFailed(failure) => failure.origin().cloned(),
            _ => None,
        }
    }

    fn ai_terminal_error(
        error: &AiError,
        retry_attempts: Option<(u32, u32)>,
    ) -> WorkflowTerminalError {
        let (content, upstream) = match error {
            AiError::ApiRequestFailed {
                status_code,
                provider,
                ..
            } => (error.to_string(), Some((*status_code, provider.clone()))),
            AiError::RawApiRequestFailed {
                status_code,
                provider,
                details,
                ..
            } => {
                let content = if details.trim().is_empty() {
                    format!(
                        "{} returned HTTP {} without an error body",
                        provider, status_code
                    )
                } else {
                    details.clone()
                };
                (content, Some((*status_code, provider.clone())))
            }
            _ => (error.to_string(), None),
        };
        let error_type = if retry_attempts.is_some() {
            "llm_retry_exhausted"
        } else {
            classify_ai_error(error)
        };
        let mut metadata = json!({
            "error_type": error_type
        });
        if let Some((status_code, provider)) = upstream {
            metadata["upstream_status_code"] = json!(status_code);
            metadata["provider"] = json!(provider);
        }
        if let Some((attempt, max_attempts)) = retry_attempts {
            metadata["retry_exhausted"] = json!(true);
            metadata["retry_attempt"] = json!(attempt);
            metadata["retry_max_attempts"] = json!(max_attempts);
        }

        WorkflowTerminalError {
            content,
            error_type,
            metadata,
        }
    }

    pub fn terminal_error(&self) -> WorkflowTerminalError {
        let mut terminal = match self {
            Self::Ai(error) => Self::ai_terminal_error(error, None),
            Self::LlmRetryExhausted {
                source,
                details,
                attempt,
                max_attempts,
            } => {
                if let Some(source) = source {
                    let mut terminal =
                        Self::ai_terminal_error(source, Some((*attempt, *max_attempts)));
                    if terminal.content.trim().is_empty() {
                        terminal.content = details.clone();
                    }
                    terminal
                } else {
                    WorkflowTerminalError {
                        content: details.clone(),
                        error_type: "llm_retry_exhausted",
                        metadata: json!({
                            "error_type": "llm_retry_exhausted",
                            "retry_exhausted": true,
                            "retry_attempt": attempt,
                            "retry_max_attempts": max_attempts
                        }),
                    }
                }
            }
            Self::CompressionFailed(details) => WorkflowTerminalError {
                content: format!(
                    "Compression failed: {details}\n<SYSTEM_REMINDER>Compression is a precondition for continuing this task: without a valid handoff the context would drift. This is a fatal error, not a retryable tool-argument problem. Inform the user that the workflow stopped because context compression could not be prepared, and do not attempt to continue the task.</SYSTEM_REMINDER>"
                ),
                error_type: "compression_failed",
                metadata: json!({ "error_type": "compression_failed" }),
            },
            _ => WorkflowTerminalError {
                content: format!(
                    "Critical Error: {}\n<SYSTEM_REMINDER>A fatal error occurred in the execution engine. If this error is related to invalid tool arguments, please correct your parameters and retry. If it is a system-level issue, please inform the user about the failure.</SYSTEM_REMINDER>",
                    self
                ),
                error_type: "engine",
                metadata: json!({ "error_type": "engine" }),
            },
        };
        let presentation = self.user_error();
        terminal.metadata["user_error"] = serde_json::to_value(&presentation)
            .unwrap_or_else(|_| json!({ "schema_version": 1, "message": presentation.message }));
        terminal
    }
}

impl ErrorProjection for WorkflowEngineError {
    fn user_error(&self) -> UserErrorPresentation {
        FailureMessage {
            legacy: self.to_string(),
            origin: self.error_origin(),
        }
        .user_error()
    }

    fn diagnostic(&self) -> ErrorDiagnostic {
        let mut diagnostic = FailureMessage {
            legacy: self.to_string(),
            origin: self.error_origin(),
        }
        .diagnostic();
        diagnostic.module = "workflow".into();
        diagnostic.operation = "engine".into();
        diagnostic
    }
}

#[cfg(test)]
mod tests {
    use super::{CompressionFailure, WorkflowEngineError};
    use crate::ai::error::AiError;
    use crate::error_diagnostics::{ErrorOrigin, ErrorProjection, FailureMessage};

    #[test]
    fn typed_domain_and_leaf_causes_do_not_gain_engine_prefixes_in_ui() {
        for error in [
            WorkflowEngineError::Tool(crate::tools::ToolError::ExecutionFailed(
                FailureMessage::leaf("legacy tool wrapper").with_origin(ErrorOrigin::leaf(
                    "tool refused token=CANARY",
                    None,
                    None,
                    Some(503),
                )),
            )),
            WorkflowEngineError::Db(crate::db::StoreError::IoError(
                FailureMessage::leaf("legacy store wrapper").with_origin(ErrorOrigin::leaf(
                    "store refused token=CANARY",
                    None,
                    None,
                    Some(503),
                )),
            )),
            WorkflowEngineError::General("leaf refused token=CANARY".to_string()),
        ] {
            let legacy = error.to_string();
            let terminal = error.terminal_error();
            assert!(terminal.content.contains(&legacy));
            let presentation = error.user_error();
            assert!(!presentation.message.contains("CANARY"));
            assert!(!presentation.message.contains("legacy"));
            assert!(!presentation.message.contains("General error:"));
            assert!(!presentation.message.contains("Tool error:"));
            assert!(!presentation.message.contains("Database error:"));
            assert_eq!(
                terminal.metadata["user_error"]["message"],
                presentation.message
            );
        }
    }

    #[test]
    fn compression_failure_keeps_legacy_display_and_serialized_string() {
        let error = WorkflowEngineError::CompressionFailed(CompressionFailure::leaf(
            "Goal tracking failed after 5 attempts",
        ));

        assert_eq!(
            error.to_string(),
            "Compression failed: Goal tracking failed after 5 attempts"
        );
        assert_eq!(
            serde_json::to_value(&error).expect("serialize compression failure"),
            serde_json::json!({
                "type": "CompressionFailed",
                "details": "Goal tracking failed after 5 attempts"
            })
        );
    }

    #[test]
    fn terminal_ai_error_appends_safe_user_error_to_existing_metadata() {
        let raw_body = r#"{"error":{"message":"quota exhausted"}}"#;
        let terminal = WorkflowEngineError::Ai(AiError::RawApiRequestFailed {
            status_code: 429,
            provider: "provider".to_string(),
            details: raw_body.to_string(),
        })
        .terminal_error();

        assert_eq!(terminal.content, raw_body);
        assert_eq!(terminal.error_type, "rate_limited");
        assert_eq!(terminal.metadata["error_type"], "rate_limited");
        assert_eq!(terminal.metadata["upstream_status_code"], 429);
        assert_eq!(terminal.metadata["provider"], "provider");
        assert_eq!(terminal.metadata["user_error"]["schema_version"], 1);
        assert_eq!(
            terminal.metadata["user_error"]["message"],
            "quota exhausted"
        );
        assert_eq!(terminal.metadata["user_error"]["upstream_status"], 429);
    }

    #[test]
    fn compression_terminal_keeps_exact_content_and_projects_source_user_error() {
        let cause = AiError::RawApiRequestFailed {
            status_code: 503,
            provider: "provider".to_string(),
            details: r#"{"error":{"message":"overloaded"}}"#.to_string(),
        };
        let last_error = format!("Goal tracking failed after 5 attempts: {cause}");
        let failure =
            CompressionFailure::leaf(last_error.clone()).with_origin(cause.error_origin());
        let terminal = WorkflowEngineError::CompressionFailed(failure).terminal_error();

        assert_eq!(
            terminal.content,
            format!(
                "Compression failed: {last_error}\n<SYSTEM_REMINDER>Compression is a precondition for continuing this task: without a valid handoff the context would drift. This is a fatal error, not a retryable tool-argument problem. Inform the user that the workflow stopped because context compression could not be prepared, and do not attempt to continue the task.</SYSTEM_REMINDER>"
            )
        );
        assert_eq!(terminal.error_type, "compression_failed");
        assert_eq!(terminal.metadata["error_type"], "compression_failed");
        assert_eq!(terminal.metadata["user_error"]["message"], "overloaded");
        assert_eq!(terminal.metadata["user_error"]["upstream_status"], 503);
    }

    #[test]
    fn compression_origin_keeps_the_latest_typed_cause() {
        let failure = CompressionFailure::leaf("Goal tracking failed").with_origin(
            ErrorOrigin::leaf("parse failed", None, None, None).context(
                "workflow",
                "parse_goals",
                None,
            ),
        );
        let error = WorkflowEngineError::CompressionFailed(failure);
        let origin = error.error_origin().expect("compression origin");
        assert_eq!(origin.message, "parse failed");
        assert_eq!(origin.frames[0].operation, "parse_goals");

        let without_origin =
            WorkflowEngineError::CompressionFailed(CompressionFailure::leaf("no typed cause"));
        assert!(without_origin.error_origin().is_none());
        assert_eq!(
            without_origin.user_error().message,
            "Compression failed: no typed cause"
        );
    }

    #[test]
    fn terminal_ai_errors_expose_structured_auth_and_billing_types() {
        for (status_code, expected_type) in [
            (401, "llm_authentication"),
            (402, "llm_billing"),
            (403, "engine"),
            (404, "engine"),
        ] {
            let terminal = WorkflowEngineError::Ai(AiError::RawApiRequestFailed {
                status_code,
                provider: "provider".to_string(),
                details: "upstream error".to_string(),
            })
            .terminal_error();

            assert_eq!(terminal.error_type, expected_type);
            assert_eq!(terminal.metadata["error_type"], expected_type);
            assert_eq!(terminal.metadata["upstream_status_code"], status_code);
            assert_eq!(terminal.metadata["provider"], "provider");
            assert_eq!(terminal.content, "upstream error");
        }
    }

    #[test]
    fn retry_exhaustion_preserves_server_error_and_attempt_metadata() {
        let terminal = WorkflowEngineError::LlmRetryExhausted {
            source: Some(AiError::RawApiRequestFailed {
                status_code: 429,
                provider: "provider".to_string(),
                details: r#"{"error":{"message":"quota exhausted"}}"#.to_string(),
            }),
            details: "quota exhausted".to_string(),
            attempt: 10,
            max_attempts: 10,
        }
        .terminal_error();

        assert_eq!(terminal.error_type, "llm_retry_exhausted");
        assert_eq!(
            terminal.content,
            r#"{"error":{"message":"quota exhausted"}}"#
        );
        assert_eq!(terminal.metadata["error_type"], "llm_retry_exhausted");
        assert_eq!(terminal.metadata["upstream_status_code"], 429);
        assert_eq!(terminal.metadata["provider"], "provider");
        assert_eq!(terminal.metadata["retry_exhausted"], true);
        assert_eq!(terminal.metadata["retry_attempt"], 10);
        assert_eq!(terminal.metadata["retry_max_attempts"], 10);
    }

    #[test]
    fn compression_failures_terminate_with_an_explicit_precondition_message() {
        let terminal = WorkflowEngineError::CompressionFailed(CompressionFailure::leaf(
            "Goal tracking failed after 5 attempts",
        ))
        .terminal_error();

        assert_eq!(terminal.error_type, "compression_failed");
        assert_eq!(terminal.metadata["error_type"], "compression_failed");
        assert!(terminal
            .content
            .contains("Goal tracking failed after 5 attempts"));
        assert!(terminal.content.contains("fatal error, not a retryable"));
        assert!(!terminal
            .content
            .contains("correct your parameters and retry"));
    }

    #[test]
    fn terminal_error_fixture_keeps_legacy_engine_content_and_tagged_serialization() {
        let error = WorkflowEngineError::General("legacy engine detail".to_string());
        assert_eq!(
            serde_json::to_value(&error).expect("serialize engine error"),
            serde_json::json!({"type": "General", "details": "legacy engine detail"})
        );
        let terminal = error.terminal_error();
        assert_eq!(terminal.error_type, "engine");
        assert_eq!(
            terminal.metadata,
            serde_json::json!({
                "error_type": "engine",
                "user_error": {
                    "schema_version": 1,
                    "message": "legacy engine detail"
                }
            })
        );
        assert_eq!(
            terminal.content,
            "Critical Error: General error: legacy engine detail\n<SYSTEM_REMINDER>A fatal error occurred in the execution engine. If this error is related to invalid tool arguments, please correct your parameters and retry. If it is a system-level issue, please inform the user about the failure.</SYSTEM_REMINDER>"
        );
    }

    #[test]
    fn empty_upstream_error_body_gets_a_visible_fallback() {
        let terminal = WorkflowEngineError::Ai(AiError::RawApiRequestFailed {
            status_code: 503,
            provider: "provider".to_string(),
            details: "  ".to_string(),
        })
        .terminal_error();

        assert_eq!(
            terminal.content,
            "provider returned HTTP 503 without an error body"
        );
    }
}
