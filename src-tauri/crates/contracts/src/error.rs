//! Optional user-facing error data, independent of application error implementations.

use serde::{Deserialize, Serialize};

/// Validated classifications supplement existing machine codes; text is never authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorReason {
    SessionAlreadyExists,
    SessionStopping,
    NoActiveSession,
    FileTooLarge,
    FileChangedOnDisk,
    UnsupportedListModels,
    RuntimeStopping,
    #[serde(other)]
    Unknown,
}

/// Safe display-only projection. Diagnostic chains remain local to the log sink.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserErrorPresentation {
    pub schema_version: u8,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<ErrorReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_status: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ErrorEnvelope {
    pub error: ErrorDetail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ErrorDetail {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation: Option<UserErrorPresentation>,
}

impl ErrorEnvelope {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error: ErrorDetail {
                code: code.into(),
                message: message.into(),
                presentation: None,
            },
        }
    }

    pub fn with_presentation(mut self, presentation: UserErrorPresentation) -> Self {
        self.error.presentation = Some(presentation);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn presentation_roundtrip_preserves_legacy_code_and_message() {
        let presentation = UserErrorPresentation {
            schema_version: 1,
            message: "overloaded: https://example.test/a\nretry later".to_string(),
            reason: Some(ErrorReason::SessionStopping),
            upstream_status: Some(503),
        };
        let envelope = ErrorEnvelope::new("invalid_state", "legacy text")
            .with_presentation(presentation.clone());
        let value = serde_json::to_value(&envelope).expect("encode");
        assert_eq!(value["error"]["code"], "invalid_state");
        assert_eq!(value["error"]["message"], "legacy text");
        assert_eq!(value["error"]["presentation"]["reason"], "session_stopping");
        assert_eq!(
            serde_json::from_value::<ErrorEnvelope>(value).expect("decode"),
            envelope
        );
        // Older clients ignore the additional optional field.
        #[derive(Deserialize)]
        struct OldDetail {
            code: String,
            message: String,
        }
        let old: OldDetail =
            serde_json::from_value(serde_json::to_value(&envelope.error).expect("encode"))
                .expect("old reader");
        assert_eq!(old.code, "invalid_state");
        assert_eq!(old.message, "legacy text");
        assert_eq!(envelope.error.presentation, Some(presentation));
    }

    #[test]
    fn presentation_unknown_reason_is_not_a_control_classification() {
        let projection: UserErrorPresentation = serde_json::from_value(json!({
            "schema_version": 1,
            "message": "session already exists",
            "reason": "future_reason",
            "future_field": "ignored"
        }))
        .expect("future reader");
        assert_eq!(projection.reason, Some(ErrorReason::Unknown));
        assert_eq!(projection.upstream_status, None);
        let old: ErrorEnvelope = serde_json::from_value(json!({
            "error": {"code": "conflict", "message": "old"}
        }))
        .expect("legacy reader");
        assert_eq!(old.error.presentation, None);
        assert_eq!(
            serde_json::to_value(old).expect("encode"),
            json!({"error": {"code":"conflict", "message":"old"}})
        );
    }
}
