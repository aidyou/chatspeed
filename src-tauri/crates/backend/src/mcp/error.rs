// Server-side session errors only exist when the runtime MCP server is
// compiled; the desktop keeps just the client/desktop error surface.
#[cfg(not(feature = "desktop"))]
use crate::error_diagnostics::ReconnectDisposition;
use crate::error_diagnostics::{ErrorDiagnostic, ErrorOrigin, ErrorProjection, FailureMessage};
#[cfg(not(feature = "desktop"))]
use crate::mcp::server::persistent_session::{EventIdParseError, SessionError};
use chatspeed_contracts::error::UserErrorPresentation;
use rmcp::model::{ErrorCode, ErrorData};
#[cfg(not(feature = "desktop"))]
use rmcp::service::ServiceError;
#[cfg(not(feature = "desktop"))]
use rmcp::transport::streamable_http_client::StreamableHttpError;
use rust_i18n::t;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "kind", content = "message", rename_all = "camelCase")]
pub enum McpError {
    // Client-side errors
    #[error("{}", t!("mcp.error.client_call_failed", error = _0))]
    ClientCallError(FailureMessage),
    #[error("{}", t!("mcp.error.client_config_failed", error = _0))]
    ClientConfigError(FailureMessage),
    #[error("{}", t!("mcp.error.client_start_failed", error = _0))]
    ClientStartError(FailureMessage),
    #[error("{}", t!("mcp.error.client_stop_failed", error = _0))]
    ClientStopError(FailureMessage),
    #[error("{}", t!("mcp.error.client_status_failed", error = _0))]
    ClientStatusError(String),

    // Server-side errors (from rmcp::ErrorData or internal server issues)
    #[error("{}", t!("mcp.error.server_initialization_failed", error = _0))]
    ServerInitializationError(FailureMessage),
    #[error("{}", t!("mcp.error.server_tool_not_found", tool_name = _0))]
    ServerToolNotFound(String),
    #[error("{}", t!("mcp.error.server_tool_execution_failed", error = _0))]
    ServerToolExecutionError(String),
    #[error("{}", t!("mcp.error.server_internal_error", error = _0))]
    ServerInternalError(String),
    #[error("{}", t!("mcp.error.server_unknown_error", error = _0))]
    ServerUnknownError(String),

    // Common errors
    #[error("{}", t!("mcp.error.io_error", error = _0))]
    Io(FailureMessage),

    #[error("{}", t!("mcp.error.not_found_error", name = _0))]
    NotFound(String),

    #[error("{}", t!("mcp.error.serialization_error", error = _0))]
    Serialization(FailureMessage),
    #[error("{}", t!("mcp.error.store_error", error = _0))]
    Store(String),
    #[error("{}", t!("mcp.error.state_change_failed", error = _0))]
    StateChangeFailed(String),
    #[error("{}", t!("mcp.error.timeout_error", error = _0))]
    Timeout(FailureMessage),

    #[error("{0}")]
    General(FailureMessage),
}

impl McpError {
    pub fn error_origin(&self) -> ErrorOrigin {
        match self {
            Self::ClientCallError(message)
            | Self::ClientConfigError(message)
            | Self::ClientStartError(message)
            | Self::ClientStopError(message)
            | Self::ServerInitializationError(message)
            | Self::Io(message)
            | Self::Serialization(message)
            | Self::Timeout(message)
            | Self::General(message) => message
                .origin()
                .cloned()
                .unwrap_or_else(|| ErrorOrigin::leaf(message.as_str(), None, None, None)),
            // Parameter variants need their complete localized descriptions.
            _ => ErrorOrigin::leaf(self.to_string(), None, None, None),
        }
    }
}

#[cfg(not(feature = "desktop"))]
impl McpError {
    /// Wraps a typed rmcp service failure without inspecting its display text.
    ///
    /// The origin snapshot owns the reconnect decision, so a translated or
    /// randomized message can never change recovery behavior.
    pub(crate) fn from_service_error(error: &ServiceError) -> Self {
        let origin = ErrorOrigin::from_error(error).with_reconnect(reconnect_disposition(error));
        let text = error.to_string();
        Self::ClientCallError(FailureMessage::leaf(text).with_origin(origin))
    }

    /// Only a typed client-call failure may ask the caller to rebuild its
    /// transport; tool-reported and legacy string errors never do.
    pub fn should_reconnect(&self) -> bool {
        matches!(
            self,
            Self::ClientCallError(message)
                if message
                    .origin()
                    .is_some_and(|origin| {
                        origin.reconnect_disposition() == ReconnectDisposition::ReconnectOnce
                    })
        )
    }
}

/// Classifies only typed rmcp failures. Unknown variants deliberately do not
/// retry: a wrong reconnect is more damaging than a surfaced error.
#[cfg(not(feature = "desktop"))]
fn reconnect_disposition(error: &ServiceError) -> ReconnectDisposition {
    match error {
        ServiceError::TransportClosed => ReconnectDisposition::ReconnectOnce,
        ServiceError::TransportSend(error) => transport_disposition(error.error.as_ref()),
        ServiceError::McpError(data) => error_code_disposition(data.code.0),
        ServiceError::Cancelled { .. } => ReconnectDisposition::DoNotReconnect,
        _ => ReconnectDisposition::Unknown,
    }
}

#[cfg(not(feature = "desktop"))]
fn error_code_disposition(code: i32) -> ReconnectDisposition {
    match code {
        404 | 410 => ReconnectDisposition::ReconnectOnce,
        401 | 403 => ReconnectDisposition::DoNotReconnect,
        _ => ReconnectDisposition::Unknown,
    }
}

#[cfg(not(feature = "desktop"))]
fn http_status_disposition(status: Option<u16>) -> ReconnectDisposition {
    match status {
        Some(404 | 410) => ReconnectDisposition::ReconnectOnce,
        Some(401 | 403) => ReconnectDisposition::DoNotReconnect,
        _ => ReconnectDisposition::Unknown,
    }
}

#[cfg(not(feature = "desktop"))]
fn io_disposition(kind: std::io::ErrorKind) -> ReconnectDisposition {
    use std::io::ErrorKind;
    match kind {
        ErrorKind::BrokenPipe
        | ErrorKind::ConnectionRefused
        | ErrorKind::ConnectionReset
        | ErrorKind::UnexpectedEof => ReconnectDisposition::ReconnectOnce,
        ErrorKind::PermissionDenied => ReconnectDisposition::DoNotReconnect,
        _ => ReconnectDisposition::Unknown,
    }
}

#[cfg(not(feature = "desktop"))]
fn transport_disposition(error: &(dyn std::error::Error + 'static)) -> ReconnectDisposition {
    if let Some(error) = error.downcast_ref::<StreamableHttpError<reqwest::Error>>() {
        return streamable_http_disposition(error);
    }
    if let Some(error) = error.downcast_ref::<std::io::Error>() {
        return io_disposition(error.kind());
    }
    ReconnectDisposition::Unknown
}

#[cfg(not(feature = "desktop"))]
fn streamable_http_disposition(
    error: &StreamableHttpError<reqwest::Error>,
) -> ReconnectDisposition {
    match error {
        StreamableHttpError::Io(error) => io_disposition(error.kind()),
        StreamableHttpError::Client(error) => {
            http_status_disposition(error.status().map(|status| status.as_u16()))
        }
        StreamableHttpError::UnexpectedEndOfStream
        | StreamableHttpError::TransportChannelClosed
        | StreamableHttpError::SessionExpired => ReconnectDisposition::ReconnectOnce,
        StreamableHttpError::AuthRequired(_) | StreamableHttpError::InsufficientScope(_) => {
            ReconnectDisposition::DoNotReconnect
        }
        _ => ReconnectDisposition::Unknown,
    }
}

impl ErrorProjection for McpError {
    fn user_error(&self) -> UserErrorPresentation {
        FailureMessage::leaf(self.to_string())
            .with_origin(self.error_origin())
            .user_error()
    }

    fn diagnostic(&self) -> ErrorDiagnostic {
        let mut diagnostic = FailureMessage::leaf(self.to_string())
            .with_origin(self.error_origin())
            .diagnostic();
        diagnostic.module = "mcp".into();
        diagnostic
    }
}

impl From<McpError> for ErrorData {
    fn from(error: McpError) -> Self {
        ErrorData::new(ErrorCode::INTERNAL_ERROR, error.user_error().message, None)
    }
}

#[cfg(not(feature = "desktop"))]
impl From<SessionError> for McpError {
    fn from(error: SessionError) -> Self {
        McpError::General(FailureMessage::from_error(&error))
    }
}

impl From<serde_json::Error> for McpError {
    fn from(error: serde_json::Error) -> Self {
        McpError::Serialization(FailureMessage::from_error(&error))
    }
}

impl From<std::io::Error> for McpError {
    fn from(error: std::io::Error) -> Self {
        McpError::Io(FailureMessage::from_error(&error))
    }
}

impl From<tokio::time::error::Elapsed> for McpError {
    fn from(error: tokio::time::error::Elapsed) -> Self {
        McpError::Timeout(FailureMessage::from_error(&error))
    }
}

impl From<crate::tools::ToolError> for McpError {
    fn from(error: crate::tools::ToolError) -> Self {
        let origin = error.error_origin();
        McpError::General(FailureMessage::leaf(error.to_string()).with_origin(origin))
    }
}

#[cfg(not(feature = "desktop"))]
impl From<EventIdParseError> for McpError {
    fn from(error: EventIdParseError) -> Self {
        McpError::General(FailureMessage::from_error(&error))
    }
}

#[cfg(not(feature = "desktop"))]
impl From<crate::mcp::server::persistent_session::LocalSessionWorkerError> for McpError {
    fn from(error: crate::mcp::server::persistent_session::LocalSessionWorkerError) -> Self {
        McpError::General(FailureMessage::from_error(&error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(feature = "desktop"))]
    use crate::error_diagnostics::ReconnectDisposition;
    use crate::tools::ToolError;
    #[cfg(not(feature = "desktop"))]
    use rmcp::service::ServiceError;
    #[cfg(not(feature = "desktop"))]
    use rmcp::transport::streamable_http_client::StreamableHttpError;
    #[cfg(not(feature = "desktop"))]
    use rmcp::transport::DynamicTransportError;
    use serde_json::json;
    #[cfg(not(feature = "desktop"))]
    use std::any::TypeId;
    #[cfg(not(feature = "desktop"))]
    use std::io::ErrorKind;

    #[test]
    fn legacy_serde_defaults_to_no_origin() {
        for kind in [
            "io",
            "serialization",
            "timeout",
            "general",
            "clientCallError",
            "clientConfigError",
            "clientStartError",
            "clientStopError",
            "serverInitializationError",
        ] {
            let value = json!({"kind": kind, "message": "legacy detail"});
            let error: McpError = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(error.clone()).unwrap(), value);
            assert!(error.error_origin().frames.is_empty());
            assert!(error.error_origin().sources.is_empty());
        }
        let error = McpError::General(FailureMessage::leaf("legacy").with_origin(
            ErrorOrigin::leaf("root", None, None, None).context("tools", "call", None),
        ));
        assert_eq!(
            serde_json::to_value(error).unwrap(),
            json!({"kind":"general", "message":"legacy"})
        );
    }

    #[derive(Debug, thiserror::Error)]
    #[error("operation wrapper")]
    struct Wrapped {
        #[source]
        source: std::io::Error,
    }

    #[test]
    fn tool_mcp_tool_preserves_typed_origin_once() {
        let source = Wrapped {
            source: std::io::Error::other("socket closed"),
        };
        let tool = ToolError::ExecutionFailed(
            FailureMessage::from_error(&source).context("tools", "execute", None),
        );
        let origin = tool.error_origin();
        let mcp = McpError::from(tool);
        assert_eq!(mcp.error_origin(), origin);
        let tool = ToolError::from(mcp);
        assert_eq!(tool.error_origin(), origin);
        assert_eq!(tool.user_error().message, "socket closed");
        assert_eq!(tool.diagnostic().source_chain, vec!["operation wrapper"]);
        assert_eq!(tool.diagnostic().context_chain.len(), 1);
    }

    #[test]
    fn parameter_errors_project_full_localized_descriptions() {
        for error in [
            McpError::NotFound("missing".into()),
            McpError::ServerToolNotFound("missing".into()),
        ] {
            assert_eq!(error.user_error().message, error.to_string());
            assert_ne!(error.user_error().message, "missing");
        }
        for error in [
            ToolError::FunctionNotFound("missing".into()),
            ToolError::McpServerNotFound("missing".into()),
        ] {
            assert_eq!(error.user_error().message, error.to_string());
            assert_ne!(error.user_error().message, "missing");
        }
    }

    #[test]
    fn jsonrpc_retains_internal_code_without_local_context() {
        let error = McpError::General(
            FailureMessage::leaf("legacy wrapper token=CANARY").with_origin(
                ErrorOrigin::leaf("socket closed token=CANARY", None, None, None).context(
                    "mcp",
                    "connect",
                    Some("private path".into()),
                ),
            ),
        );
        let projected = error.user_error().message;
        let rpc: ErrorData = error.into();
        assert_eq!(rpc.code, ErrorCode::INTERNAL_ERROR);
        assert_eq!(rpc.message, projected);
        assert!(!rpc.message.contains("CANARY"));
        assert!(!rpc.message.contains("wrapper"));
        assert!(rpc.data.is_none());
    }

    #[cfg(not(feature = "desktop"))]
    fn transport_error(error: impl std::error::Error + Send + Sync + 'static) -> ServiceError {
        ServiceError::TransportSend(DynamicTransportError::from_parts(
            "test-transport",
            TypeId::of::<()>(),
            Box::new(error),
        ))
    }

    #[cfg(not(feature = "desktop"))]
    #[test]
    fn typed_reconnect_decisions_ignore_display_text() {
        let translated =
            ServiceError::McpError(ErrorData::new(ErrorCode(404), "服务暂不可用", None));
        let random = ServiceError::McpError(ErrorData::new(ErrorCode(410), "alpha-7", None));

        for source in [&translated, &random] {
            let error = McpError::from_service_error(source);
            assert!(error.should_reconnect());
            assert_eq!(
                error.error_origin().reconnect_disposition(),
                ReconnectDisposition::ReconnectOnce
            );
            match &error {
                McpError::ClientCallError(message) => {
                    assert_eq!(message.as_str(), source.to_string());
                }
                _ => panic!("expected a client call error"),
            }
        }
    }

    #[cfg(not(feature = "desktop"))]
    #[test]
    fn typed_transport_failures_map_to_bounded_dispositions() {
        let reconnecting = [
            ServiceError::TransportClosed,
            transport_error(StreamableHttpError::<reqwest::Error>::UnexpectedEndOfStream),
            transport_error(StreamableHttpError::<reqwest::Error>::TransportChannelClosed),
            transport_error(StreamableHttpError::<reqwest::Error>::SessionExpired),
            transport_error(StreamableHttpError::<reqwest::Error>::Io(
                std::io::Error::new(ErrorKind::BrokenPipe, "closed"),
            )),
            transport_error(StreamableHttpError::<reqwest::Error>::Io(
                std::io::Error::new(ErrorKind::ConnectionRefused, "refused"),
            )),
            transport_error(StreamableHttpError::<reqwest::Error>::Io(
                std::io::Error::new(ErrorKind::ConnectionReset, "reset"),
            )),
            transport_error(StreamableHttpError::<reqwest::Error>::Io(
                std::io::Error::new(ErrorKind::UnexpectedEof, "eof"),
            )),
            transport_error(std::io::Error::new(ErrorKind::ConnectionReset, "raw reset")),
        ];

        for source in reconnecting {
            let error = McpError::from_service_error(&source);
            assert!(
                error.should_reconnect(),
                "expected reconnect for {}",
                source
            );
            assert_eq!(
                error.error_origin().reconnect_disposition(),
                ReconnectDisposition::ReconnectOnce
            );
        }

        let not_reconnecting = [
            transport_error(StreamableHttpError::<reqwest::Error>::Io(
                std::io::Error::new(ErrorKind::PermissionDenied, "denied"),
            )),
            transport_error(std::io::Error::new(
                ErrorKind::Other,
                "channel closed connection reset 404 EOF",
            )),
            ServiceError::Cancelled {
                reason: Some("connection reset 404 EOF".into()),
            },
            ServiceError::McpError(ErrorData::new(ErrorCode(401), "unauthorized", None)),
            ServiceError::McpError(ErrorData::new(ErrorCode(403), "forbidden", None)),
            ServiceError::Timeout {
                timeout: std::time::Duration::from_secs(1),
            },
            ServiceError::UnexpectedResponse,
        ];

        for source in not_reconnecting {
            let error = McpError::from_service_error(&source);
            assert!(
                !error.should_reconnect(),
                "unexpected reconnect for {}",
                source
            );
            assert_ne!(
                error.error_origin().reconnect_disposition(),
                ReconnectDisposition::ReconnectOnce
            );
        }

        assert_eq!(
            McpError::from_service_error(&ServiceError::Cancelled { reason: None })
                .error_origin()
                .reconnect_disposition(),
            ReconnectDisposition::DoNotReconnect
        );
    }

    #[cfg(not(feature = "desktop"))]
    #[test]
    fn http_status_disposition_only_reconnects_on_gone_statuses() {
        assert_eq!(
            http_status_disposition(Some(404)),
            ReconnectDisposition::ReconnectOnce
        );
        assert_eq!(
            http_status_disposition(Some(410)),
            ReconnectDisposition::ReconnectOnce
        );
        assert_eq!(
            http_status_disposition(Some(401)),
            ReconnectDisposition::DoNotReconnect
        );
        assert_eq!(
            http_status_disposition(Some(403)),
            ReconnectDisposition::DoNotReconnect
        );
        assert_eq!(
            http_status_disposition(Some(500)),
            ReconnectDisposition::Unknown
        );
        assert_eq!(http_status_disposition(None), ReconnectDisposition::Unknown);
    }

    #[cfg(not(feature = "desktop"))]
    #[test]
    fn legacy_client_call_text_restores_unknown_without_retry() {
        let legacy: McpError = serde_json::from_value(
            json!({"kind": "clientCallError", "message": " 404 connection reset channel closed EOF"}),
        )
        .unwrap();

        assert!(!legacy.should_reconnect());
        assert_eq!(
            legacy.error_origin().reconnect_disposition(),
            ReconnectDisposition::Unknown
        );
    }

    #[cfg(not(feature = "desktop"))]
    #[test]
    fn reconnect_disposition_survives_context() {
        let typed = McpError::from_service_error(&ServiceError::TransportClosed);
        let McpError::ClientCallError(message) = typed else {
            panic!("expected a client call error");
        };

        let contextual = message.context("mcp", "call", Some("server=demo".into()));

        assert_eq!(
            contextual.origin().unwrap().reconnect_disposition(),
            ReconnectDisposition::ReconnectOnce
        );
    }
}

impl From<anyhow::Error> for McpError {
    fn from(error: anyhow::Error) -> Self {
        McpError::General(FailureMessage::from_error(error.as_ref()))
    }
}
