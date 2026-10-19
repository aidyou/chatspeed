// The engine error and its terminal projection are runtime-only: the desktop
// reaches the runtime over the control plane and never constructs or matches the
// engine error, so this module is runtime-only.
#![cfg(not(feature = "desktop"))]

use crate::ai::error::AiError;
use crate::db::error::StoreError;
use crate::tools::ToolError;

use serde::Serialize;
use serde_json::{json, Value};
use thiserror::Error;

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
    CompressionFailed(String),

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
        match self {
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::WorkflowEngineError;
    use crate::ai::error::AiError;

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
        let terminal = WorkflowEngineError::CompressionFailed(
            "Goal tracking failed after 5 attempts".to_string(),
        )
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
