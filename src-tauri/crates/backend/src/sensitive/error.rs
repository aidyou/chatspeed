use crate::error_diagnostics::{ErrorDiagnostic, ErrorOrigin, ErrorProjection, FailureMessage};
use chatspeed_contracts::UserErrorPresentation;
use serde::Serialize;
use thiserror::Error;

/// Defines the custom error types for the sensitive data filtering module.
#[derive(Error, Debug, Serialize)]
#[serde(tag = "kind", content = "details")]
pub enum SensitiveError {
    /// An error that occurs when a regular expression fails to compile.
    #[error("Failed to compile regex pattern: {pattern}")]
    RegexCompilationFailed {
        pattern: String,
        message: FailureMessage,
    },

    /// An error for an invalid filter rule configuration.
    #[error("Invalid filter rule configuration: {message}")]
    InvalidRule { message: String },

    /// Represents I/O errors that might occur, e.g., reading a filter configuration file.
    #[error("I/O error during filter operation: {message}")]
    IoError { message: FailureMessage },
}

impl SensitiveError {
    /// Returns an in-memory typed origin for transfer across application error layers.
    pub fn error_origin(&self) -> ErrorOrigin {
        let (message, origin, operation) = match self {
            Self::RegexCompilationFailed { message, .. } => (
                message.as_str().to_string(),
                message.origin().cloned(),
                "compile_regex",
            ),
            Self::InvalidRule { .. } => (self.to_string(), None, "validate_rule"),
            Self::IoError { message } => (
                message.as_str().to_string(),
                message.origin().cloned(),
                "io",
            ),
        };
        origin
            .unwrap_or_else(|| ErrorOrigin::leaf(message, None, None, None))
            .context("sensitive", operation, None)
    }
}

impl ErrorProjection for SensitiveError {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn all_variants_preserve_legacy_display_and_serde_shape() {
        let errors = [
            SensitiveError::RegexCompilationFailed {
                pattern: "broken".into(),
                message: "unclosed group".into(),
            },
            SensitiveError::InvalidRule {
                message: "missing expression".into(),
            },
            SensitiveError::IoError {
                message: "file unavailable".into(),
            },
        ];
        let expected = [
            (
                "Failed to compile regex pattern: broken",
                json!({"kind":"RegexCompilationFailed","details":{"pattern":"broken","message":"unclosed group"}}),
            ),
            (
                "Invalid filter rule configuration: missing expression",
                json!({"kind":"InvalidRule","details":{"message":"missing expression"}}),
            ),
            (
                "I/O error during filter operation: file unavailable",
                json!({"kind":"IoError","details":{"message":"file unavailable"}}),
            ),
        ];
        for (error, (display, serialized)) in errors.iter().zip(expected) {
            assert_eq!(error.to_string(), display);
            assert_eq!(serde_json::to_value(error).unwrap(), serialized);
        }
    }

    #[test]
    fn typed_io_origin_and_user_projection_redact_secrets() {
        let io_error = std::io::Error::other("token=CANARY");
        let error = SensitiveError::IoError {
            message: FailureMessage::from_error(&io_error),
        };
        assert_eq!(error.error_origin().message, "token=CANARY");
        assert_eq!(error.error_origin().frames[0].module, "sensitive");
        assert_eq!(error.user_error().message, "token=\"[redacted]\"");
        assert!(!error.diagnostic().to_json_line().contains("CANARY"));
    }

    #[test]
    fn typed_regex_cause_is_retained_and_invalid_rule_description_is_semantic() {
        let regex_error = regex::Regex::new("(").unwrap_err();
        let regex_message = FailureMessage::from_error(&regex_error);
        let error = SensitiveError::RegexCompilationFailed {
            pattern: "regex_rule".into(),
            message: regex_message.clone(),
        };
        assert_eq!(
            error.error_origin().message,
            regex_message.origin().unwrap().message
        );
        assert_eq!(error.error_origin().frames[0].operation, "compile_regex");

        let invalid = SensitiveError::InvalidRule {
            message: "custom rule is empty".into(),
        };
        assert_eq!(
            invalid.error_origin().message,
            "Invalid filter rule configuration: custom rule is empty"
        );
    }
}
