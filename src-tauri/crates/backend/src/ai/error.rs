use rust_i18n::t;
use serde::Serialize;
use thiserror::Error;

use crate::error_diagnostics::{ErrorDiagnostic, ErrorOrigin, ErrorProjection, FailureMessage};
use chatspeed_contracts::error::{ErrorReason, UserErrorPresentation};

#[derive(Error, Debug, Serialize)]
#[serde(tag = "kind", content = "message", rename_all = "camelCase")]
pub enum AiError {
    #[error("{}",t!("chat.error.api_request_failed",provider=provider,details=details,status_code=status_code))]
    ApiRequestFailed {
        status_code: u16,
        provider: String,
        details: FailureMessage,
    },

    #[error("{details}")]
    RawApiRequestFailed {
        status_code: u16,
        provider: String,
        details: String,
    },

    #[error("{0}")]
    InitFailed(FailureMessage),

    #[error("{}", t!("chat.error.invalid_input", details =.0))]
    InvalidInput(String),

    #[error("{}", t!("chat.error.response_parse_failed", provider = .provider, details = .details))]
    ResponseParseFailed {
        provider: String,
        details: FailureMessage,
    },

    #[error("{}", t!("chat.error.stream_processing_failed", provider = .provider, details = .details))]
    StreamProcessingFailed {
        provider: String,
        details: FailureMessage,
    },

    #[error("{}", t!("chat.error.tool_call_serialization_failed", details = .details))]
    ToolCallSerializationFailed { details: FailureMessage },

    #[error("{}", t!("chat.error.failed_to_get_or_create_window_channel", error = .0))]
    FailedToGetOrCreateWindowChannel(FailureMessage),

    /// The runtime is quiescing and refuses to start new chat work, so a
    /// canonical chat entry reached without the HTTP transport is still fenced.
    /// The message is already localized by the admission gate.
    #[error("{0}")]
    RuntimeStopping(String),
}

impl AiError {
    /// Keeps structured causes separate from the legacy display and wire representation.
    pub fn error_origin(&self) -> ErrorOrigin {
        let (failure, operation, status) = match self {
            Self::ApiRequestFailed {
                details,
                status_code,
                ..
            } => (details, "api_request", Some(*status_code)),
            Self::InitFailed(details) => (details, "initialize", None),
            Self::ResponseParseFailed { details, .. } => (details, "parse_response", None),
            Self::StreamProcessingFailed { details, .. } => (details, "process_stream", None),
            Self::ToolCallSerializationFailed { details } => (details, "serialize_tool_call", None),
            Self::FailedToGetOrCreateWindowChannel(details) => (details, "window_channel", None),
            Self::RawApiRequestFailed {
                details,
                status_code,
                ..
            } => {
                return ErrorOrigin::leaf(details.clone(), None, None, valid_status(*status_code))
                    .context("ai", "api_request", None);
            }
            Self::InvalidInput(details) => {
                return ErrorOrigin::leaf(details.clone(), None, None, None).context(
                    "ai",
                    "validate_input",
                    None,
                );
            }
            Self::RuntimeStopping(message) => {
                return ErrorOrigin::leaf(
                    message.clone(),
                    Some(ErrorReason::RuntimeStopping),
                    None,
                    None,
                )
                .context("ai", "admit_chat", None);
            }
        };
        let mut origin = failure
            .origin()
            .cloned()
            .unwrap_or_else(|| ErrorOrigin::leaf(failure.as_str(), None, None, None));
        origin.status = origin.status.and_then(valid_status);
        if origin.status.is_none() {
            origin.status = status.and_then(valid_status);
        }
        origin.context("ai", operation, None)
    }
}

fn valid_status(status: u16) -> Option<u16> {
    (100..=599).contains(&status).then_some(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn variants_keep_legacy_display_and_serialized_shapes() {
        let provider = "provider".to_string();
        let cases = [
            (
                AiError::ApiRequestFailed {
                    status_code: 0,
                    provider: provider.clone(),
                    details: "cause".into(),
                },
                "apiRequestFailed",
                json!({"status_code":0,"provider":"provider","details":"cause"}),
                t!(
                    "chat.error.api_request_failed",
                    provider = "provider",
                    details = "cause",
                    status_code = 0
                )
                .to_string(),
            ),
            (
                AiError::RawApiRequestFailed {
                    status_code: 429,
                    provider: provider.clone(),
                    details: " raw\nbody ".into(),
                },
                "rawApiRequestFailed",
                json!({"status_code":429,"provider":"provider","details":" raw\nbody "}),
                " raw\nbody ".into(),
            ),
            (
                AiError::InitFailed("cause".into()),
                "initFailed",
                json!("cause"),
                "cause".into(),
            ),
            (
                AiError::InvalidInput("cause".into()),
                "invalidInput",
                json!("cause"),
                t!("chat.error.invalid_input", details = "cause").to_string(),
            ),
            (
                AiError::ResponseParseFailed {
                    provider: provider.clone(),
                    details: "cause".into(),
                },
                "responseParseFailed",
                json!({"provider":"provider","details":"cause"}),
                t!(
                    "chat.error.response_parse_failed",
                    provider = "provider",
                    details = "cause"
                )
                .to_string(),
            ),
            (
                AiError::StreamProcessingFailed {
                    provider,
                    details: "cause".into(),
                },
                "streamProcessingFailed",
                json!({"provider":"provider","details":"cause"}),
                t!(
                    "chat.error.stream_processing_failed",
                    provider = "provider",
                    details = "cause"
                )
                .to_string(),
            ),
            (
                AiError::ToolCallSerializationFailed {
                    details: "cause".into(),
                },
                "toolCallSerializationFailed",
                json!({"details":"cause"}),
                t!(
                    "chat.error.tool_call_serialization_failed",
                    details = "cause"
                )
                .to_string(),
            ),
            (
                AiError::FailedToGetOrCreateWindowChannel("cause".into()),
                "failedToGetOrCreateWindowChannel",
                json!("cause"),
                t!(
                    "chat.error.failed_to_get_or_create_window_channel",
                    error = "cause"
                )
                .to_string(),
            ),
            (
                AiError::RuntimeStopping("cause".into()),
                "runtimeStopping",
                json!("cause"),
                "cause".into(),
            ),
        ];
        for (error, kind, message, display) in cases {
            let diagnostic = error.diagnostic();
            assert_eq!(diagnostic.module, "ai");
            assert_eq!(
                diagnostic.operation,
                error.error_origin().frames.last().unwrap().operation
            );
            assert!(!diagnostic.operation.is_empty());
            assert_eq!(error.to_string(), display);
            assert_eq!(
                serde_json::to_value(&error).unwrap(),
                json!({"kind":kind,"message":message})
            );
            assert_eq!(error.error_origin().code, None);
            assert_eq!(
                error.user_error().upstream_status,
                if kind == "rawApiRequestFailed" {
                    Some(429)
                } else {
                    None
                }
            );
            assert_eq!(
                error.user_error().reason,
                if kind == "runtimeStopping" {
                    Some(ErrorReason::RuntimeStopping)
                } else {
                    None
                }
            );
        }
    }

    #[test]
    fn raw_projection_uses_known_message_without_changing_body_or_classification() {
        let raw = json!({"error":{"message":"overloaded: https://example.test/a\nretry token=CANARY"},"payload":{"message":"hidden"}}).to_string();
        let error = AiError::RawApiRequestFailed {
            status_code: 503,
            provider: "provider".into(),
            details: raw.clone(),
        };
        assert_eq!(error.to_string(), raw);
        let projection = error.user_error();
        assert!(projection
            .message
            .starts_with("overloaded: https://example.test/a\nretry"));
        assert!(!projection.message.contains("CANARY"));
        assert_eq!(projection.upstream_status, Some(503));
        assert_eq!(projection.reason, None);
        for status in [0, 99, 600, u16::MAX] {
            let error = AiError::RawApiRequestFailed {
                status_code: status,
                provider: "provider".into(),
                details: "unsupported list models".into(),
            };
            assert_eq!(error.user_error().upstream_status, None);
            assert_eq!(error.user_error().reason, None);
        }
    }

    #[derive(Debug, Error)]
    #[error("outer operation")]
    struct Outer {
        #[source]
        source: std::io::Error,
    }

    #[test]
    fn upstream_origin_status_takes_priority_without_changing_legacy_classification() {
        let error = AiError::ApiRequestFailed {
            status_code: 500,
            provider: "provider".into(),
            details: FailureMessage::leaf("legacy provider wrapper").with_origin(
                ErrorOrigin::leaf(
                    "method unavailable",
                    Some(ErrorReason::UnsupportedListModels),
                    None,
                    Some(405),
                ),
            ),
        };
        assert_eq!(error.user_error().upstream_status, Some(405));
        assert_eq!(
            error.user_error().reason,
            Some(ErrorReason::UnsupportedListModels)
        );
        assert_eq!(error.user_error().message, "method unavailable");
        assert_eq!(
            serde_json::to_value(&error).unwrap()["message"]["status_code"],
            500
        );
        assert!(matches!(
            error,
            AiError::ApiRequestFailed {
                status_code: 500,
                ..
            }
        ));
    }

    #[test]
    fn carrier_preserves_root_source_and_legacy_display() {
        let source = Outer {
            source: std::io::Error::other("socket closed"),
        };
        let error = AiError::InitFailed(FailureMessage::from_error(&source));
        assert_eq!(error.to_string(), "outer operation");
        assert_eq!(
            serde_json::to_value(&error).unwrap()["message"],
            "outer operation"
        );
        assert_eq!(error.user_error().message, "socket closed");
        assert_eq!(error.error_origin().sources, vec!["outer operation"]);
        assert_eq!(error.diagnostic().context_chain[0].module, "ai");
        let error = AiError::ApiRequestFailed {
            status_code: 500,
            provider: "provider".into(),
            details: FailureMessage::from_error(&source),
        };
        assert_eq!(error.user_error().upstream_status, Some(500));
        assert_eq!(error.user_error().message, "socket closed");
    }
}

impl ErrorProjection for AiError {
    fn user_error(&self) -> UserErrorPresentation {
        FailureMessage::leaf(self.to_string())
            .with_origin(self.error_origin())
            .user_error()
    }

    fn diagnostic(&self) -> ErrorDiagnostic {
        let origin = self.error_origin();
        let operation = origin.frames.last().map(|frame| frame.operation.clone());
        let mut diagnostic = FailureMessage::leaf(self.to_string())
            .with_origin(origin)
            .diagnostic();
        diagnostic.module = "ai".into();
        diagnostic.operation = operation.unwrap_or_else(|| "request".into());
        diagnostic
    }
}
