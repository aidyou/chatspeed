//! Final-boundary diagnostic construction for the proxy engine.
//!
//! Handlers own the request/model/provider context at the point where an error
//! stops a request. This module keeps that context in explicit frames while the
//! existing `ErrorDiagnostic` type owns sanitization and serialization.

use std::error::Error;

use crate::error_diagnostics::{ErrorDiagnostic, ErrorFrame, ErrorProjection, FailureMessage};

type Frame = (&'static str, Option<String>);

/// Builds a diagnostic from a typed error before any caller stringifies it.
pub(crate) fn from_error(
    module: &'static str,
    operation: &'static str,
    error: &(dyn Error + 'static),
    frames: impl IntoIterator<Item = Frame>,
) -> ErrorDiagnostic {
    build(
        FailureMessage::from_error(error).diagnostic(),
        module,
        operation,
        frames,
    )
}

/// Builds a diagnostic from an already-carried failure without dropping origin.
pub(crate) fn from_failure(
    module: &'static str,
    operation: &'static str,
    failure: &FailureMessage,
    frames: impl IntoIterator<Item = Frame>,
) -> ErrorDiagnostic {
    build(failure.diagnostic(), module, operation, frames)
}

/// Builds a diagnostic for boundaries that only have a legacy message.
pub(crate) fn from_message(
    module: &'static str,
    operation: &'static str,
    message: impl Into<String>,
    frames: impl IntoIterator<Item = Frame>,
) -> ErrorDiagnostic {
    build(
        FailureMessage::leaf(message).diagnostic(),
        module,
        operation,
        frames,
    )
}

fn build(
    mut diagnostic: ErrorDiagnostic,
    module: &'static str,
    operation: &'static str,
    frames: impl IntoIterator<Item = Frame>,
) -> ErrorDiagnostic {
    diagnostic.module = module.to_string();
    diagnostic.operation = operation.to_string();
    diagnostic.context_chain.extend(
        frames
            .into_iter()
            .map(|(operation, context)| ErrorFrame::new("ccproxy", operation, context)),
    );
    diagnostic
}

/// Projects an upstream rejection for logs without changing its protocol response.
pub(crate) fn from_upstream(
    operation: &'static str,
    protocol: &crate::ccproxy::types::ChatProtocol,
    status: http::StatusCode,
    headers: &reqwest::header::HeaderMap,
    body: &[u8],
    frames: impl IntoIterator<Item = Frame>,
) -> ErrorDiagnostic {
    let normalized =
        crate::ccproxy::adapter::error::normalize_backend_error(protocol, status, headers, body);
    let mut diagnostic = from_message(
        "ccproxy",
        operation,
        crate::error_diagnostics::sanitize_user_message(&normalized.message),
        frames,
    );
    diagnostic.http_status = Some(status.as_u16());
    diagnostic.code = normalized.code.and_then(|code| match code {
        serde_json::Value::String(value) => Some(value),
        serde_json::Value::Number(value) => Some(value.to_string()),
        _ => None,
    });
    diagnostic.request_id = normalized.request_id;
    diagnostic
}

/// Emits one sanitized JSON line at the caller's original log level.
pub(crate) fn emit(level: log::Level, diagnostic: &ErrorDiagnostic) {
    log::log!(target: "error_diagnostic", level, "{}", diagnostic.to_json_line());
}

/// Shared request context for final error boundaries.
pub(crate) struct RequestContext<'a> {
    pub request_id: &'a str,
    pub client_protocol: &'a str,
    pub backend_protocol: &'a str,
    pub client_model: &'a str,
    pub backend_model: &'a str,
    pub provider_id: i64,
    pub provider: &'a str,
    pub group: Option<&'a str>,
    pub path: &'static str,
    pub compat: bool,
}

pub(crate) fn request_frames(context: RequestContext<'_>) -> Vec<Frame> {
    vec![
        ("request", Some(format!("id={}", context.request_id))),
        (
            "model",
            Some(format!(
                "client={} backend={}",
                context.client_model, context.backend_model
            )),
        ),
        (
            "provider",
            Some(format!(
                "id={} name={}",
                context.provider_id, context.provider
            )),
        ),
        (
            "protocol",
            Some(format!(
                "client={} backend={}",
                context.client_protocol, context.backend_protocol
            )),
        ),
        (
            "route",
            Some(format!(
                "path={} compat={}",
                context.path,
                if context.compat { "on" } else { "off" }
            )),
        ),
        ("group", context.group.map(str::to_string)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, thiserror::Error)]
    #[error("outer operation")]
    struct Outer {
        #[source]
        source: std::io::Error,
    }

    #[test]
    fn typed_source_is_captured_before_stringification() {
        let error = Outer {
            source: std::io::Error::other("stream closed: peer"),
        };
        let diagnostic = from_error(
            "ccproxy",
            "stream_read",
            &error,
            [("route", Some("path=unified".to_string()))],
        );

        assert_eq!(diagnostic.module, "ccproxy");
        assert_eq!(diagnostic.operation, "stream_read");
        assert_eq!(diagnostic.error, "stream closed: peer");
        assert_eq!(diagnostic.source_chain, vec!["outer operation"]);
        assert_eq!(diagnostic.context_chain[0].module, "ccproxy");
        assert_eq!(diagnostic.context_chain[0].operation, "route");
    }

    #[test]
    fn request_frames_keep_explicit_endpoint_context() {
        let frames = request_frames(RequestContext {
            request_id: "message-id",
            client_protocol: "openai",
            backend_protocol: "claude",
            client_model: "alias",
            backend_model: "backend-model",
            provider_id: 7,
            provider: "provider",
            group: Some("group"),
            path: "unified",
            compat: true,
        });
        let diagnostic = from_message("ccproxy", "backend_error", "overloaded", frames);
        let names = diagnostic
            .context_chain
            .iter()
            .map(|frame| frame.operation.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec!["request", "model", "provider", "protocol", "route", "group"]
        );
        assert!(diagnostic.context_chain[4]
            .context
            .as_deref()
            .is_some_and(|context| context.contains("path=unified compat=on")));
    }

    #[test]
    fn upstream_diagnostic_keeps_code_status_and_request_id_without_body_payload() {
        let body = br#"{"error":{"message":"overloaded token=CANARY","code":"busy"},"unrelated":"PRIVATE_PAYLOAD"}"#;
        let before = body.to_vec();
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("x-request-id", "req-42".parse().unwrap());
        let diagnostic = from_upstream(
            "backend_error",
            &crate::ccproxy::types::ChatProtocol::OpenAI,
            http::StatusCode::SERVICE_UNAVAILABLE,
            &headers,
            body,
            [],
        );
        assert_eq!(diagnostic.http_status, Some(503));
        assert_eq!(diagnostic.code.as_deref(), Some("busy"));
        assert_eq!(diagnostic.request_id.as_deref(), Some("req-42"));
        assert!(diagnostic.error.starts_with("overloaded"));
        let line = diagnostic.to_json_line();
        assert!(!line.contains("CANARY"));
        assert!(!line.contains("PRIVATE_PAYLOAD"));
        assert_eq!(body.as_slice(), before.as_slice());
    }

    #[test]
    fn diagnostic_lines_redact_frames_and_messages() {
        let diagnostic = from_message(
            "ccproxy",
            "backend_error",
            "upstream rejected token=CANARY",
            [("request", Some("authorization: Bearer CANARY".to_string()))],
        );
        let line = diagnostic.to_json_line();

        assert!(!line.contains("CANARY"));
        assert!(!line.contains('\n'));
        assert!(line.contains("\"event\":\"error_diagnostic\""));
    }
}
