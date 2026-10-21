//! Bounded, local error context and diagnostic event types.

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn carrier_preserves_legacy_serde_but_projects_only_origin() {
        let origin = ErrorOrigin::leaf(
            "overloaded: https://example.test/a\nretry later",
            Some(ErrorReason::SessionStopping),
            Some("internal_error".into()),
            Some(503),
        );
        let failure = FailureMessage::leaf("Compression failed: AI error: legacy")
            .with_origin(origin)
            .context("workflow", "compress", None);
        assert_eq!(failure.to_string(), "Compression failed: AI error: legacy");
        assert_eq!(
            serde_json::to_value(&failure).unwrap(),
            json!(failure.legacy)
        );
        let restored: FailureMessage = serde_json::from_value(json!(failure.legacy)).unwrap();
        assert!(restored.origin.is_none());
        assert_eq!(restored.to_string(), failure.to_string());
        assert_eq!(
            failure.user_error().message,
            "overloaded: https://example.test/a\nretry later"
        );
        assert_eq!(
            failure.user_error().reason,
            Some(ErrorReason::SessionStopping)
        );
        assert_eq!(failure.user_error().upstream_status, Some(503));
        let diagnostic = failure.diagnostic();
        assert_eq!(diagnostic.code.as_deref(), Some("internal_error"));
        assert_eq!(diagnostic.http_status, Some(503));
        assert_eq!(diagnostic.context_chain[0].operation, "compress");
        assert!(!serde_json::to_string(&failure.user_error())
            .unwrap()
            .contains("context_chain"));
    }

    #[test]
    fn reconnect_disposition_is_internal_and_never_projected() {
        let origin = ErrorOrigin::leaf("root cause", None, None, None)
            .with_reconnect(ReconnectDisposition::ReconnectOnce);
        assert_eq!(
            origin.reconnect_disposition(),
            ReconnectDisposition::ReconnectOnce
        );

        let failure = FailureMessage::leaf("legacy display").with_origin(origin);
        assert_eq!(failure.to_string(), "legacy display");
        assert_eq!(
            serde_json::to_value(&failure).unwrap(),
            json!("legacy display")
        );

        let projected = failure.user_error();
        assert_eq!(projected.message, "root cause");
        assert!(!serde_json::to_string(&projected)
            .unwrap()
            .contains("reconnect"));

        let diagnostic = failure.diagnostic();
        assert_eq!(diagnostic.error, "root cause");
        assert!(!serde_json::to_string(&diagnostic)
            .unwrap()
            .contains("reconnect"));

        let contextual = failure.context("mcp", "call", None);
        assert_eq!(
            contextual.origin().unwrap().reconnect_disposition(),
            ReconnectDisposition::ReconnectOnce
        );
    }

    #[derive(Debug, thiserror::Error)]
    #[error("outer operation")]
    struct Outer {
        #[source]
        source: std::io::Error,
    }

    #[test]
    fn typed_source_is_captured_before_display_loss() {
        let error = Outer {
            source: std::io::Error::other("socket closed: peer"),
        };
        let failure = FailureMessage::from_error(&error);
        assert_eq!(failure.to_string(), "outer operation");
        assert_eq!(failure.user_error().message, "socket closed: peer");
        assert_eq!(failure.origin().unwrap().sources, vec!["outer operation"]);
        assert_eq!(failure.user_error().reason, None);
    }

    #[derive(Debug)]
    struct Cycle;
    impl fmt::Display for Cycle {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("cycle")
        }
    }
    impl Error for Cycle {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            Some(self)
        }
    }

    #[test]
    fn source_cycles_and_explicit_frame_overflow_are_bounded_and_marked() {
        let failure = FailureMessage::from_error(&Cycle);
        assert!(failure.diagnostic().truncated);
        let mut origin = ErrorOrigin::leaf("cause", None, None, None);
        for _ in 0..100 {
            origin = origin.context("module", "operation", None);
        }
        assert_eq!(origin.frames.len(), 16);
        assert!(
            FailureMessage::leaf("legacy")
                .with_origin(origin)
                .diagnostic()
                .truncated
        );
    }

    #[test]
    fn diagnostic_is_redacted_single_line_structured_and_byte_bounded() {
        let secret = "token=CANARY";
        let mut diagnostic = FailureMessage::leaf(secret).diagnostic();
        diagnostic.module = secret.into();
        diagnostic.operation = secret.into();
        diagnostic.code = Some(secret.into());
        diagnostic.request_id = Some(secret.into());
        diagnostic.session_id = Some(secret.into());
        diagnostic.context_chain = vec![ErrorFrame::new(secret, secret, Some(secret.into())); 40];
        diagnostic.source_chain = vec![format!("{secret}{}", "\"\\\n𠀀".repeat(4096)); 40];
        let line = diagnostic.to_json_line();
        assert!(line.len() <= 32768);
        assert!(!line.contains('\n'));
        assert!(!line.contains("CANARY"));
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["event"], "error_diagnostic");
        assert_eq!(value["truncated"], true);
        assert!(value["context_chain"].as_array().unwrap().len() <= 16);
        assert!(value["source_chain"].as_array().unwrap().len() <= 16);
        let plain: serde_json::Value =
            serde_json::from_str(&FailureMessage::leaf("cause").diagnostic().to_json_line())
                .unwrap();
        assert_eq!(plain["truncated"], false);
    }
}
use std::{error::Error, fmt};

use chatspeed_contracts::error::{ErrorReason, UserErrorPresentation};
use serde::{Deserialize, Serialize};

use crate::capability::redaction::redact_text;
use crate::error_diagnostics::traits::ErrorProjection;

const MAX_FIELD: usize = 2048;
const MAX_FRAMES: usize = 16;
const MAX_JSON_LINE: usize = 32_768;

/// Whether a typed failure may rebuild its transport, and how often.
///
/// This is snapshot metadata for recovery decisions only. It never changes the
/// legacy display text, the serialized carrier, or the projected diagnostic.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ReconnectDisposition {
    #[default]
    Unknown,
    DoNotReconnect,
    ReconnectOnce,
}

fn bound(value: &str, max: usize) -> (String, bool) {
    let mut chars = value.chars();
    let output: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        (format!("{output}…"), true)
    } else {
        (output, false)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorFrame {
    pub module: String,
    pub operation: String,
    pub context: Option<String>,
}

impl ErrorFrame {
    pub fn new(
        module: impl Into<String>,
        operation: impl Into<String>,
        context: Option<String>,
    ) -> Self {
        Self {
            module: module.into(),
            operation: operation.into(),
            context,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorOrigin {
    pub message: String,
    pub reason: Option<ErrorReason>,
    pub code: Option<String>,
    pub status: Option<u16>,
    pub frames: Vec<ErrorFrame>,
    pub sources: Vec<String>,
    pub(crate) reconnect: ReconnectDisposition,
}

impl ErrorOrigin {
    pub fn leaf(
        message: impl Into<String>,
        reason: Option<ErrorReason>,
        code: Option<String>,
        status: Option<u16>,
    ) -> Self {
        Self {
            message: message.into(),
            reason,
            code,
            status,
            frames: Vec::new(),
            sources: Vec::new(),
            reconnect: ReconnectDisposition::Unknown,
        }
    }

    /// Captures a bounded standard-error chain; never infers causes from display text.
    pub fn from_error(error: &(dyn Error + 'static)) -> Self {
        let mut chain = Vec::new();
        let mut current: &(dyn Error + 'static) = error;
        let mut seen: Vec<*const dyn Error> = Vec::new();
        let mut truncated = false;
        loop {
            let ptr = current as *const dyn Error;
            if seen.iter().any(|previous| std::ptr::eq(*previous, ptr)) {
                truncated = true;
                break;
            }
            seen.push(ptr);
            chain.push(current.to_string());
            match current.source() {
                Some(source) if chain.len() < MAX_FRAMES => current = source,
                Some(_) => {
                    truncated = true;
                    break;
                }
                None => break,
            }
        }
        let message = chain.last().cloned().unwrap_or_else(|| error.to_string());
        chain.pop();
        Self {
            message,
            reason: None,
            code: None,
            status: None,
            frames: Vec::new(),
            sources: chain,
            reconnect: ReconnectDisposition::Unknown,
        }
        .with_cycle(truncated)
    }

    pub(crate) fn with_reconnect(mut self, disposition: ReconnectDisposition) -> Self {
        self.reconnect = disposition;
        self
    }

    pub(crate) fn reconnect_disposition(&self) -> ReconnectDisposition {
        self.reconnect
    }

    fn with_cycle(mut self, cycle: bool) -> Self {
        if cycle {
            self.sources.push("[source chain truncated]".to_string());
        }
        self
    }

    pub fn context(
        mut self,
        module: impl Into<String>,
        operation: impl Into<String>,
        context: Option<String>,
    ) -> Self {
        if self.frames.len() < MAX_FRAMES {
            self.frames
                .push(ErrorFrame::new(module, operation, context));
        } else if !self
            .sources
            .iter()
            .any(|source| source == "[source chain truncated]")
        {
            self.sources.push("[source chain truncated]".to_string());
        }
        self
    }
}

#[derive(Debug, Clone)]
pub struct FailureMessage {
    pub legacy: String,
    pub origin: Option<ErrorOrigin>,
}

impl FailureMessage {
    pub fn leaf(legacy: impl Into<String>) -> Self {
        Self {
            legacy: legacy.into(),
            origin: None,
        }
    }

    pub fn from_error(error: &(dyn Error + 'static)) -> Self {
        Self {
            legacy: error.to_string(),
            origin: Some(ErrorOrigin::from_error(error)),
        }
    }

    pub fn context(
        mut self,
        module: impl Into<String>,
        operation: impl Into<String>,
        context: Option<String>,
    ) -> Self {
        let origin = self
            .origin
            .take()
            .unwrap_or_else(|| ErrorOrigin::leaf(self.legacy.clone(), None, None, None));
        self.origin = Some(origin.context(module, operation, context));
        self
    }

    pub fn with_origin(mut self, origin: ErrorOrigin) -> Self {
        self.origin = Some(origin);
        self
    }

    pub fn as_str(&self) -> &str {
        &self.legacy
    }
    pub fn origin(&self) -> Option<&ErrorOrigin> {
        self.origin.as_ref()
    }
}

impl From<String> for FailureMessage {
    fn from(value: String) -> Self {
        Self::leaf(value)
    }
}
impl From<&str> for FailureMessage {
    fn from(value: &str) -> Self {
        Self::leaf(value)
    }
}
impl fmt::Display for FailureMessage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.legacy)
    }
}
impl Serialize for FailureMessage {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.legacy.serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for FailureMessage {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::leaf(String::deserialize(deserializer)?))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorDiagnostic {
    pub event: &'static str,
    pub module: String,
    pub operation: String,
    pub code: Option<String>,
    pub http_status: Option<u16>,
    pub error: String,
    pub context_chain: Vec<ErrorFrame>,
    pub source_chain: Vec<String>,
    pub request_id: Option<String>,
    pub session_id: Option<String>,
    pub truncated: bool,
}

impl ErrorDiagnostic {
    pub fn sanitized(&self) -> Self {
        let mut output = self.clone();
        output.module = redact_text(&output.module);
        output.operation = redact_text(&output.operation);
        output.code = output.code.map(|value| redact_text(&value));
        output.error = redact_text(&output.error);
        output.source_chain = output
            .source_chain
            .iter()
            .map(|value| redact_text(value))
            .collect();
        output.request_id = output.request_id.map(|value| redact_text(&value));
        output.session_id = output.session_id.map(|value| redact_text(&value));
        output.context_chain = output
            .context_chain
            .into_iter()
            .map(|frame| ErrorFrame {
                module: redact_text(&frame.module),
                operation: redact_text(&frame.operation),
                context: frame.context.map(|value| redact_text(&value)),
            })
            .collect();
        output.event = "error_diagnostic";
        output
    }

    /// Produces one sanitized JSON line, clamping fields and dropping optional data to fit.
    pub fn to_json_line(&self) -> String {
        let mut output = self.sanitized();
        output.truncated = self.truncated;
        output.event = "error_diagnostic";
        output.truncated |=
            output.context_chain.len() > MAX_FRAMES || output.source_chain.len() > MAX_FRAMES;
        output.context_chain.truncate(MAX_FRAMES);
        output.source_chain.truncate(MAX_FRAMES);
        for value in [
            &mut output.code,
            &mut output.request_id,
            &mut output.session_id,
        ] {
            if let Some(text) = value {
                let (bounded, changed) = bound(text, MAX_FIELD);
                *text = bounded;
                output.truncated |= changed;
            }
        }
        for value in [&mut output.module, &mut output.operation, &mut output.error] {
            let (bounded, changed) = bound(value, MAX_FIELD);
            *value = bounded;
            output.truncated |= changed;
        }
        for frame in &mut output.context_chain {
            let (value, changed) = bound(&frame.module, MAX_FIELD);
            frame.module = value;
            output.truncated |= changed;
            let (value, changed) = bound(&frame.operation, MAX_FIELD);
            frame.operation = value;
            output.truncated |= changed;
            if let Some(context) = &frame.context {
                let (value, changed) = bound(context, MAX_FIELD);
                frame.context = Some(value);
                output.truncated |= changed;
            }
        }
        for source in &mut output.source_chain {
            let (value, changed) = bound(source, MAX_FIELD);
            *source = value;
            output.truncated |= changed;
        }
        let mut encoded = serde_json::to_string(&output)
            .unwrap_or_else(|_| "{\"event\":\"error_diagnostic\",\"truncated\":true}".to_string());
        while encoded.len() > MAX_JSON_LINE {
            output.truncated = true;
            if output.source_chain.pop().is_some() || output.context_chain.pop().is_some() {
                encoded = serde_json::to_string(&output).unwrap_or_default();
            } else if output.session_id.take().is_some()
                || output.request_id.take().is_some()
                || output.code.take().is_some()
            {
                encoded = serde_json::to_string(&output).unwrap_or_default();
            } else {
                output.error = "[truncated]".to_string();
                encoded = serde_json::to_string(&output).unwrap_or_default();
                break;
            }
        }
        encoded
    }
}

impl ErrorProjection for FailureMessage {
    fn user_error(&self) -> UserErrorPresentation {
        let origin = self.origin.as_ref();
        let message = origin
            .map(|value| value.message.as_str())
            .unwrap_or(self.legacy.as_str());
        UserErrorPresentation {
            schema_version: 1,
            message: crate::error_diagnostics::projection::sanitize_user_message(message),
            reason: origin.and_then(|value| value.reason),
            upstream_status: origin.and_then(|value| value.status),
        }
    }

    fn diagnostic(&self) -> ErrorDiagnostic {
        let origin = self.origin.as_ref();
        ErrorDiagnostic {
            event: "error_diagnostic",
            module: String::new(),
            operation: String::new(),
            code: origin.and_then(|value| value.code.clone()),
            http_status: origin.and_then(|value| value.status),
            error: origin
                .map(|value| value.message.clone())
                .unwrap_or_else(|| self.legacy.clone()),
            context_chain: origin.map(|value| value.frames.clone()).unwrap_or_default(),
            source_chain: origin
                .map(|value| value.sources.clone())
                .unwrap_or_default(),
            request_id: None,
            session_id: None,
            truncated: origin.is_some_and(|value| {
                value
                    .sources
                    .iter()
                    .any(|source| source == "[source chain truncated]")
            }),
        }
    }
}
