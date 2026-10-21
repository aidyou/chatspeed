// Only `McpProtocolType`, `McpServerConfig` and `McpStatus` are shared with the
// desktop build; the client traits and their supporting items below are
// runtime-only, so their imports carry the same gate.
#[cfg(not(feature = "desktop"))]
use rmcp::model::{CallToolRequestParams, InitializeRequestParams};
#[cfg(not(feature = "desktop"))]
use rmcp::service::RunningService;
#[cfg(not(feature = "desktop"))]
use rmcp::RoleClient;
#[cfg(not(feature = "desktop"))]
use rust_i18n::t;
use serde::{Deserialize, Serialize};
#[cfg(not(feature = "desktop"))]
use serde_json::{Map, Value};
#[cfg(not(feature = "desktop"))]
use tokio::sync::RwLock;
#[cfg(not(feature = "desktop"))]
use tokio::time::{timeout, Duration};

use std::collections::HashSet;
use std::fmt::{Display, Formatter};
#[cfg(not(feature = "desktop"))]
use std::sync::Arc;

#[cfg(not(feature = "desktop"))]
use crate::ai::traits::chat::MCPToolDeclaration;
#[cfg(not(feature = "desktop"))]
use crate::error_diagnostics::{ErrorProjection, FailureMessage};
#[cfg(not(feature = "desktop"))]
use crate::mcp::McpError; // Ensure this is the correct path

#[cfg(not(feature = "desktop"))]
use super::util::get_tools;

/// MCP protocol type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum McpProtocolType {
    /// Server-Sent Events protocol
    Sse,
    /// Standard I/O protocol
    Stdio,
    /// Streamable HTTP protocol
    #[serde(rename = "streamable_http")]
    StreamableHttp,
}
impl From<&str> for McpProtocolType {
    fn from(value: &str) -> Self {
        match value {
            "sse" => McpProtocolType::Sse,
            "stdio" => McpProtocolType::Stdio,
            "streamable_http" => McpProtocolType::StreamableHttp,
            _ => McpProtocolType::Stdio,
        }
    }
}
impl From<String> for McpProtocolType {
    fn from(value: String) -> Self {
        value.as_str().into()
    }
}

impl From<McpProtocolType> for &str {
    fn from(value: McpProtocolType) -> Self {
        match value {
            McpProtocolType::Sse => "sse",
            McpProtocolType::Stdio => "stdio",
            McpProtocolType::StreamableHttp => "streamable_http",
        }
    }
}

impl From<McpProtocolType> for String {
    fn from(value: McpProtocolType) -> Self {
        value.into()
    }
}

impl Display for McpProtocolType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            McpProtocolType::Sse => write!(f, "sse"),
            McpProtocolType::Stdio => write!(f, "stdio"),
            McpProtocolType::StreamableHttp => write!(f, "streamable_http"),
        }
    }
}

/// Configuration for MCP servers
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Mcp server name
    pub name: String,
    /// Protocol type
    #[serde(rename = "type")]
    pub protocol_type: McpProtocolType,

    /// URL for SSE protocol
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub bearer_token: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,

    /// Command for Stdio protocol
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,

    /// Arguments for Stdio protocol
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,

    /// Environment variables for Stdio protocol
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<Vec<(String, String)>>,

    /// Disabled tools
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled_tools: Option<HashSet<String>>,

    /// Timeout in seconds for operations like list_tools
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            name: Default::default(),
            protocol_type: McpProtocolType::Stdio,
            url: Default::default(),
            bearer_token: Default::default(),
            proxy: Default::default(),
            command: Default::default(),
            args: Default::default(),
            env: Default::default(),
            disabled_tools: Default::default(),
            timeout: Some(60),
        }
    }
}

#[cfg(not(feature = "desktop"))]
pub type McpClientResult<T> = Result<T, McpError>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpStatus {
    Starting,
    Connected,
    Running,
    Stopped,
    #[serde(rename = "error")]
    Error(String),
}

impl Display for McpStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            McpStatus::Starting => write!(f, "Starting"),
            &McpStatus::Connected => write!(f, "Connected"),
            McpStatus::Running => write!(f, "Running"),
            McpStatus::Stopped => write!(f, "Stopped"),
            McpStatus::Error(err) => write!(f, "Error: {}", err),
        }
    }
}

/// Status change callback type
/// This callback is invoked when the status of an MCP client changes.
///
/// # Arguments
/// * `id` - The ID of the MCP server ID whose status has changed.
/// * `status` - The new status of the MCP server.
#[cfg(not(feature = "desktop"))]
pub type StatusChangeCallback = Box<dyn Fn(String, McpStatus) -> () + Send + Sync>;

/// Default budget for an explicit [`McpClient::stop`].
///
/// Generous enough that a healthy server's `cancel` completes long before it
/// elapses, while still bounding a stuck transport so a stop can never hang the
/// caller forever.
#[cfg(not(feature = "desktop"))]
pub const MCP_STOP_DEFAULT_BUDGET: Duration = Duration::from_secs(30);

/// The bounded result of stopping one MCP client.
///
/// rmcp's `RunningService::cancel` is the canonical teardown of an MCP server:
/// it stops the service task and drops the child-process transport, but it
/// exposes no child *tree* handle. A server that spawned its own descendants is
/// not tracked, so a completed cancel proves the transport and the direct child
/// were released — never that every descendant was reaped. These outcomes are
/// therefore deliberately conservative: they never claim a tree-wide reap.
#[cfg(not(feature = "desktop"))]
#[derive(Debug, Clone)]
pub enum McpStopOutcome {
    /// No live service was held (already stopped, or never started). Nothing was
    /// leaked, so this is a satisfied, idempotent stop.
    AlreadyStopped,
    /// The transport was cancelled within the budget and the direct child was
    /// released. Descendant reaping is not provable, so it is not claimed.
    TransportStopped,
    /// The transport could not be confirmed released within the budget, or the
    /// cancel reported an error. The caller must treat the teardown as unproven
    /// instead of recording a clean stop.
    Unproven(McpError),
}

#[cfg(not(feature = "desktop"))]
#[async_trait::async_trait]
pub trait McpClientInternal: Send + Sync {
    /// Internal method to set MCP status
    async fn set_status(&self, status: McpStatus);

    /// Internal method to notify status change
    async fn notify_status_change(&self, name: String, status: McpStatus);
}

/// Main trait containing methods for an MCP client.
/// This trait is designed to be object-safe for use with `dyn McpClient`.
#[cfg(not(feature = "desktop"))]
#[async_trait::async_trait]
pub(crate) trait McpClient: Send + Sync + McpClientInternal {
    /// Gets the name of the MCP client
    async fn name(&self) -> String;

    /// Gets the MCP server configuration
    async fn config(&self) -> McpServerConfig;

    /// Updates the disabled status of a tool in the client's internal configuration.
    async fn update_disabled_tools(
        &self,
        tool_name: &str,
        is_disabled: bool,
    ) -> McpClientResult<()>;

    /// Gets the running service instance
    /// Implementors should return a clone of their `Arc<RwLock<Option<RunningService<...>>>>`.
    fn client(&self) -> Arc<RwLock<Option<RunningService<RoleClient, InitializeRequestParams>>>>;

    async fn status(&self) -> McpStatus;

    /// Set callback for status changes
    async fn on_status_change(&self, callback: StatusChangeCallback);

    /// Performs the client-specific connection logic.
    /// This method should establish the connection and return the running service instance
    /// upon success, or an error upon failure. It should NOT set McpStatus itself,
    /// nor should it store the running service in the shared client field.
    async fn perform_connect(
        &self,
    ) -> McpClientResult<RunningService<RoleClient, InitializeRequestParams>>;

    /// Starts the MCP client connection.
    /// This default implementation calls `perform_connect` and manages status updates
    /// and storage of the running service.
    async fn start(&self) -> McpClientResult<()> {
        match self.perform_connect().await {
            Ok(running_service) => {
                *self.client().write().await = Some(running_service);
                self.set_status(McpStatus::Connected).await;
                Ok(())
            }
            Err(e) => {
                self.set_status(McpStatus::Error(e.to_string())).await;
                Err(e)
            }
        }
    }

    /// Stops the running MCP client.
    ///
    /// Delegates to [`Self::stop_bounded`] with a generous default budget so an
    /// explicit stop still waits for the transport teardown, mapping an unproven
    /// outcome back to the same `ClientStopError` a raw cancel failure used to
    /// produce.
    async fn stop(&self) -> McpClientResult<()> {
        match self.stop_bounded(MCP_STOP_DEFAULT_BUDGET).await {
            McpStopOutcome::AlreadyStopped | McpStopOutcome::TransportStopped => Ok(()),
            McpStopOutcome::Unproven(error) => Err(error),
        }
    }

    /// Stops the running client with a bounded wait and reports whether the
    /// teardown was confirmed.
    ///
    /// This is the canonical bounded stop the coordinated shutdown uses. It
    /// takes the live service under a short write lock, then awaits the cancel
    /// *without* holding it, so a slow `cancel` can neither block a concurrent
    /// registration nor exceed `budget`. An elapsed budget or a reported cancel
    /// error is surfaced as [`McpStopOutcome::Unproven`] instead of being
    /// hidden, and a stop with no live service is the idempotent
    /// [`McpStopOutcome::AlreadyStopped`].
    async fn stop_bounded(&self, budget: Duration) -> McpStopOutcome {
        let service_instance = self.client().write().await.take();
        let Some(service_instance) = service_instance else {
            // Already stopped or never started: nothing to leak.
            self.set_status(McpStatus::Stopped).await;
            return McpStopOutcome::AlreadyStopped;
        };
        match timeout(budget, service_instance.cancel()).await {
            Ok(Ok(_)) => {
                self.set_status(McpStatus::Stopped).await;
                McpStopOutcome::TransportStopped
            }
            Ok(Err(e)) => {
                let err_msg = e.to_string();
                self.set_status(McpStatus::Error(format!("Failed to stop: {}", err_msg)))
                    .await;
                McpStopOutcome::Unproven(McpError::ClientStopError(
                    crate::error_diagnostics::FailureMessage::leaf(err_msg)
                        .with_origin(crate::error_diagnostics::ErrorOrigin::from_error(&e)),
                ))
            }
            Err(_) => {
                let err_msg = format!("MCP stop exceeded its {:?} budget", budget);
                self.set_status(McpStatus::Error(err_msg.clone())).await;
                McpStopOutcome::Unproven(McpError::ClientStopError(err_msg.into()))
            }
        }
    }

    /// Lists all available tools from the connected MCP server.
    /// This is a default implementation.
    async fn list_tools(&self) -> McpClientResult<Vec<MCPToolDeclaration>> {
        let timeout_secs = self.config().await.timeout.unwrap_or(60);
        let result = {
            let peer = {
                let client = self.client();
                let guard = client.read().await;
                match guard.as_ref() {
                    Some(service_instance) => service_instance.peer().clone(),
                    None => {
                        return Err(McpError::ClientStatusError(
                            t!("mcp.client.no_running", client = self.name().await).to_string(),
                        ));
                    }
                }
            };
            timeout(
                Duration::from_secs(timeout_secs),
                peer.list_tools(Default::default()),
            )
            .await
        };

        match result {
            Ok(result) => {
                let tools = result.map_err(|e| McpError::from_service_error(&e))?;
                self.set_status(McpStatus::Running).await;
                Ok(get_tools(&tools))
            }
            Err(_) => {
                let err = McpError::ClientStatusError(
                    t!(
                        "mcp.client.list_tools_timeout",
                        name = self.config().await.name
                    )
                    .to_string(),
                );
                self.set_status(McpStatus::Error(err.to_string())).await;
                Err(err)
            }
        }
    }

    /// Calls a specific tool with given arguments, with automatic reconnection and retry logic.
    async fn call(&self, tool_name: &str, args: Value) -> McpClientResult<Value> {
        let result = self.try_call(tool_name, args.clone()).await;

        match result {
            Ok(val) => Ok(val),
            Err(e) => {
                // Only a typed origin may rebuild the transport; the display
                // text is never used as a retry signal.
                if e.should_reconnect() {
                    let server_name = self.name().await;
                    log_retry_diagnostic(&e, &server_name, "retry_call");

                    // Stop the client, ignoring errors as it's likely already dead.
                    if let Err(stop_err) = self.stop().await {
                        log_retry_diagnostic(&stop_err, &server_name, "stop_before_retry");
                    }

                    // Start a new connection. If this fails, propagate the error.
                    self.start().await?;

                    log::info!(
                        "Successfully reconnected to MCP server '{}'. Retrying call.",
                        server_name
                    );

                    // Retry the call once.
                    self.try_call(tool_name, args).await
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Internal method to perform a single tool call attempt without retry logic.
    async fn try_call(&self, tool_name: &str, args: Value) -> McpClientResult<Value> {
        let call_tool_result = {
            let peer = {
                let client = self.client();
                let guard = client.read().await;
                let service_name = self.name().await;
                match guard.as_ref() {
                    Some(service_instance) => service_instance.peer().clone(),
                    None => {
                        return Err(McpError::ClientStatusError(
                            t!("mcp.client.no_running", client = service_name).to_string(),
                        ));
                    }
                }
            };
            peer.call_tool({
                let mut request = CallToolRequestParams::default();
                request.name = tool_name.to_string().into();
                request.arguments = self.arg_parser(args);
                request
            })
            .await
            .map_err(|e| McpError::from_service_error(&e))?
        };

        // Check the `is_error` field from rmcp::model::CallToolResult
        // If `is_error` is Some(true), it indicates a tool execution error.
        if call_tool_result.is_error.unwrap_or(false) {
            return Err(McpError::ClientCallError(
                encode_call_error_message(&call_tool_result).into(),
            ));
        }

        serde_json::to_value(call_tool_result).map_err(|e| {
            McpError::ClientCallError(
                crate::error_diagnostics::FailureMessage::leaf(format!(
                    "Failed to serialize successful content: {}",
                    e
                ))
                .with_origin(crate::error_diagnostics::ErrorOrigin::from_error(&e)),
            )
        })
    }

    /// Parses arguments for tool calls.
    /// This is a default implementation.
    fn arg_parser(&self, args: Value) -> Option<Map<String, Value>> {
        match args.as_object() {
            Some(obj) => Some(obj.clone()),
            None => None,
        }
    }
}

/// Encodes the message carried by `McpError::ClientCallError` for an `is_error`
/// `CallToolResult`.
///
/// A provider that reports a structured error (`{"code","message"}`) has its
/// `structured_content` preserved as a JSON object so the stable provider code
/// survives the string-only error variant. Ordinary MCP servers, which only
/// send `content`, keep the previous array-shaped fallback; the two forms stay
/// distinguishable because `content` always serializes to a JSON array.
#[cfg(not(feature = "desktop"))]
fn encode_call_error_message(result: &rmcp::model::CallToolResult) -> String {
    if let Some(structured) = result.structured_content.as_ref() {
        return serde_json::to_string(structured)
            .unwrap_or_else(|e| format!("Failed to serialize structured error content: {}", e));
    }
    serde_json::to_string(&result.content)
        .unwrap_or_else(|e| format!("Failed to serialize error content: {}", e))
}

#[cfg(not(feature = "desktop"))]
fn log_retry_diagnostic(error: &McpError, server_name: &str, operation: &str) {
    let origin =
        error
            .error_origin()
            .context("mcp", operation, Some(format!("server={server_name}")));
    let mut diagnostic = FailureMessage::leaf(error.to_string())
        .with_origin(origin)
        .diagnostic();
    diagnostic.module = "mcp".into();
    diagnostic.operation = operation.into();
    log::warn!(target: "error_diagnostic", "{}", diagnostic.to_json_line());
}

#[cfg(all(test, not(feature = "desktop")))]
mod tests {
    use super::{
        encode_call_error_message, McpClient, McpClientInternal, McpClientResult, McpServerConfig,
        McpStatus,
    };
    use crate::error_diagnostics::FailureMessage;
    use crate::mcp::McpError;
    use rmcp::model::{CallToolResult, InitializeRequestParams, IntoContents};
    use rmcp::service::{RoleClient, RunningService, ServiceError};
    use serde_json::{json, Value};
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use tokio::sync::RwLock;

    // A provider's structured error must survive `ClientCallError` so the
    // manager can read the stable code; the raw message rides along internally
    // and is never echoed to the caller.
    #[test]
    fn structured_call_error_preserves_provider_code() {
        let result = CallToolResult::structured_error(
            json!({"code": "timeout", "message": "https://user:pass@example.com token=abc"}),
        );
        let message = encode_call_error_message(&result);
        let parsed: serde_json::Value = serde_json::from_str(&message).expect("json object");
        assert_eq!(
            parsed.get("code").and_then(|code| code.as_str()),
            Some("timeout")
        );
        assert!(parsed.get("message").is_some(), "message stays internal");
    }

    // Ordinary MCP servers, which only send `content`, keep the array fallback
    // so their existing error shape is unchanged.
    #[test]
    fn content_only_call_error_keeps_array_fallback() {
        let result = CallToolResult::error("raw provider failure".to_string().into_contents());
        let message = encode_call_error_message(&result);
        let parsed: serde_json::Value = serde_json::from_str(&message).expect("json array");
        assert!(parsed.is_array(), "content fallback stays an array");
        assert!(parsed.get("code").is_none());
    }

    // Tool-reported errors are a successful transport response; old connection
    // keywords in their text must never trigger a reconnect.
    #[test]
    fn tool_reported_error_never_retries() {
        let result = CallToolResult::error(
            "connection reset 404 EOF channel closed"
                .to_string()
                .into_contents(),
        );
        let error = McpError::ClientCallError(encode_call_error_message(&result).into());
        assert!(!error.should_reconnect());
    }

    struct MockClient {
        config: McpServerConfig,
        status: Mutex<McpStatus>,
        attempts: AtomicUsize,
        stops: AtomicUsize,
        starts: AtomicUsize,
        outcomes: Mutex<VecDeque<McpClientResult<Value>>>,
        start_error: Mutex<Option<McpError>>,
        service: Arc<RwLock<Option<RunningService<RoleClient, InitializeRequestParams>>>>,
    }

    impl MockClient {
        fn new(outcomes: Vec<McpClientResult<Value>>) -> Self {
            let mut config = McpServerConfig::default();
            config.name = "mock-server".into();
            Self {
                config,
                status: Mutex::new(McpStatus::Stopped),
                attempts: AtomicUsize::new(0),
                stops: AtomicUsize::new(0),
                starts: AtomicUsize::new(0),
                outcomes: Mutex::new(outcomes.into()),
                start_error: Mutex::new(None),
                service: Arc::new(RwLock::new(None)),
            }
        }

        fn with_start_error(self, error: McpError) -> Self {
            *self.start_error.lock().unwrap() = Some(error);
            self
        }

        fn attempts(&self) -> usize {
            self.attempts.load(Ordering::SeqCst)
        }

        fn stops(&self) -> usize {
            self.stops.load(Ordering::SeqCst)
        }

        fn starts(&self) -> usize {
            self.starts.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl McpClientInternal for MockClient {
        async fn set_status(&self, status: McpStatus) {
            *self.status.lock().unwrap() = status;
        }

        async fn notify_status_change(&self, _name: String, _status: McpStatus) {}
    }

    #[async_trait::async_trait]
    impl McpClient for MockClient {
        async fn name(&self) -> String {
            self.config.name.clone()
        }

        async fn config(&self) -> McpServerConfig {
            self.config.clone()
        }

        async fn update_disabled_tools(
            &self,
            _tool_name: &str,
            _is_disabled: bool,
        ) -> McpClientResult<()> {
            Ok(())
        }

        fn client(
            &self,
        ) -> Arc<RwLock<Option<RunningService<RoleClient, InitializeRequestParams>>>> {
            self.service.clone()
        }

        async fn status(&self) -> McpStatus {
            self.status.lock().unwrap().clone()
        }

        async fn on_status_change(&self, _callback: super::StatusChangeCallback) {}

        async fn perform_connect(
            &self,
        ) -> McpClientResult<RunningService<RoleClient, InitializeRequestParams>> {
            Err(McpError::General(FailureMessage::leaf(
                "mock does not connect",
            )))
        }

        async fn start(&self) -> McpClientResult<()> {
            self.starts.fetch_add(1, Ordering::SeqCst);
            if let Some(error) = self.start_error.lock().unwrap().clone() {
                return Err(error);
            }
            self.set_status(McpStatus::Connected).await;
            Ok(())
        }

        async fn stop(&self) -> McpClientResult<()> {
            self.stops.fetch_add(1, Ordering::SeqCst);
            self.set_status(McpStatus::Stopped).await;
            Ok(())
        }

        async fn try_call(&self, _tool_name: &str, _args: Value) -> McpClientResult<Value> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            self.outcomes
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| {
                    Err(McpError::ClientCallError(FailureMessage::leaf(
                        "mock has no outcome",
                    )))
                })
        }
    }

    #[tokio::test]
    async fn typed_reconnect_once_retries_exactly_once_after_stop_start() {
        let client = MockClient::new(vec![
            Err(McpError::from_service_error(&ServiceError::TransportClosed)),
            Ok(json!({"ok": true})),
        ]);

        let value = client
            .call("tool", json!({}))
            .await
            .expect("retry succeeds");

        assert_eq!(value, json!({"ok": true}));
        assert_eq!(client.attempts(), 2);
        assert_eq!(client.stops(), 1);
        assert_eq!(client.starts(), 1);
    }

    #[tokio::test]
    async fn typed_retry_failure_does_not_recurse() {
        let client = MockClient::new(vec![
            Err(McpError::from_service_error(&ServiceError::TransportClosed)),
            Err(McpError::from_service_error(&ServiceError::TransportClosed)),
        ]);

        assert!(client.call("tool", json!({})).await.is_err());
        assert_eq!(client.attempts(), 2);
        assert_eq!(client.stops(), 1);
        assert_eq!(client.starts(), 1);
    }

    #[tokio::test]
    async fn start_failure_does_not_attempt_a_second_call() {
        let client = MockClient::new(vec![Err(McpError::from_service_error(
            &ServiceError::TransportClosed,
        ))])
        .with_start_error(McpError::ClientStartError(FailureMessage::leaf(
            "start failed",
        )));

        let error = client
            .call("tool", json!({}))
            .await
            .expect_err("start fails");
        assert!(matches!(error, McpError::ClientStartError(_)));
        assert_eq!(client.attempts(), 1);
        assert_eq!(client.stops(), 1);
        assert_eq!(client.starts(), 1);
    }

    #[tokio::test]
    async fn legacy_text_error_never_retries() {
        let client = MockClient::new(vec![Err(McpError::ClientCallError(FailureMessage::leaf(
            "channel closed connection reset 404 EOF",
        )))]);

        assert!(client.call("tool", json!({})).await.is_err());
        assert_eq!(client.attempts(), 1);
        assert_eq!(client.stops(), 0);
        assert_eq!(client.starts(), 0);
    }

    #[tokio::test]
    async fn typed_cancelled_error_never_retries_even_with_old_keywords() {
        let client = MockClient::new(vec![Err(McpError::from_service_error(
            &ServiceError::Cancelled {
                reason: Some("connection reset 404 EOF".into()),
            },
        ))]);

        assert!(client.call("tool", json!({})).await.is_err());
        assert_eq!(client.attempts(), 1);
        assert_eq!(client.stops(), 0);
        assert_eq!(client.starts(), 0);
    }
}
