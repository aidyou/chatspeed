//! Streamable HTTP client implementation for ModelScope Control Protocol (MCP)
use std::sync::Arc;
use std::time::Duration;

use reqwest::{header, Client};
use rmcp::{
    model::{ClientCapabilities, ClientConfig, Implementation, InitializeRequestParams},
    service::RunningService,
    transport::{
        common::client_side_sse::ExponentialBackoff,
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    RoleClient, ServiceExt as _,
};
use rust_i18n::t;
use tokio::sync::RwLock;

use crate::mcp::McpError;

use super::core::McpClientCore;
use super::{
    types::{McpClientInternal, McpStatus, StatusChangeCallback},
    McpClient, McpClientResult, McpProtocolType, McpServerConfig,
};

/// Handles connection lifecycle and provides methods for:
/// - Establishing HTTP connections
/// - Executing remote tool calls
pub struct StreamableHttpClient {
    core: McpClientCore,
    /// Upper bound on transport reconnect attempts for this client.
    ///
    /// Ordinary servers keep the historical long retry budget; the dedicated
    /// desktop Web MCP provider uses a single attempt so a dead provider fails
    /// its in-flight call immediately instead of silently reconnecting.
    max_retries: usize,
    /// Base backoff between reconnect attempts.
    retry_base_duration: Duration,
}

impl StreamableHttpClient {
    /// Creates a new HTTP Protocol of MCP client instance with given configuration
    pub fn new(config: McpServerConfig) -> McpClientResult<Self> {
        Self::with_retry(config, 120, Duration::from_secs(2))
    }

    /// Creates a streamable HTTP client with an explicit, bounded retry budget.
    ///
    /// The dedicated desktop Web MCP provider uses this so a dead or released
    /// provider is never silently reconnected: it is reached once and then fails
    /// closed as unavailable.
    pub fn with_retry(
        config: McpServerConfig,
        max_retries: usize,
        retry_base_duration: Duration,
    ) -> McpClientResult<Self> {
        if config.protocol_type != McpProtocolType::StreamableHttp {
            return Err(McpError::ClientConfigError(
                t!(
                    "mcp.client.config_mismatch",
                    client = "StreamableHttpClient",
                    protocol_type = config.protocol_type.to_string()
                )
                .to_string(),
            ));
        }

        if config.url.as_deref().unwrap_or_default().is_empty() {
            return Err(McpError::ClientConfigError(
                t!("mcp.client.http_url_cant_be_empty").to_string(),
            ));
        }

        Ok(StreamableHttpClient {
            core: McpClientCore::new(config),
            max_retries,
            retry_base_duration,
        })
    }

    async fn build_http_client_async(&self) -> McpClientResult<reqwest::Client> {
        let mut client_builder = Client::builder();
        let current_config = self.core.get_config().await;

        let connect_timeout = current_config
            .timeout
            .map(|t| Duration::from_secs(t))
            .unwrap_or(Duration::from_secs(15));
        if !connect_timeout.is_zero() {
            client_builder = client_builder.connect_timeout(connect_timeout);
        }

        if let Some(token) = current_config.bearer_token.as_ref() {
            if !token.trim().is_empty() {
                let mut headers = header::HeaderMap::new();
                headers.insert(
                    header::AUTHORIZATION,
                    header::HeaderValue::from_str(&format!("Bearer {}", token))
                        .map_err(|e| McpError::ClientConfigError(e.to_string()))?,
                );

                client_builder = client_builder.default_headers(headers);
            }
        }

        if let Some(proxy) = current_config.proxy.as_ref() {
            if !proxy.trim().is_empty() {
                let proxy = reqwest::Proxy::all(proxy)
                    .map_err(|e| McpError::ClientConfigError(e.to_string()))?;
                client_builder = client_builder.proxy(proxy);
            }
        }

        let http_client = client_builder
            .build()
            .map_err(|e| McpError::ClientConfigError(e.to_string()))?;
        Ok(http_client)
    }
}

#[async_trait::async_trait]
impl McpClientInternal for StreamableHttpClient {
    async fn set_status(&self, status: McpStatus) {
        self.core.set_status(status).await;
    }

    async fn notify_status_change(&self, name: String, status: McpStatus) {
        self.core.notify_status_change(name, status).await;
    }
}

#[async_trait::async_trait]
impl McpClient for StreamableHttpClient {
    async fn perform_connect(
        &self,
    ) -> McpClientResult<RunningService<RoleClient, InitializeRequestParams>> {
        let config = self.core.get_config().await;
        let url_str = config.url.as_deref().filter(|s| !s.is_empty());

        let url = match url_str {
            Some(u) => u,
            None => {
                let err_msg = t!("mcp.client.http_url_cant_be_empty").to_string();
                return Err(McpError::ClientConfigError(err_msg));
            }
        };

        let http_client = self.build_http_client_async().await?;
        let mut retry_config = ExponentialBackoff::default();
        retry_config.max_times = Some(self.max_retries);
        retry_config.base_duration = self.retry_base_duration;

        let mut transport_config = StreamableHttpClientTransportConfig::with_uri(url);
        transport_config.retry_config = Arc::new(retry_config);
        transport_config.auth_header = config.bearer_token.clone();
        let transport = StreamableHttpClientTransport::with_client(http_client, transport_config);

        let mut client_info = ClientConfig::default();
        client_info.protocol_version = Default::default();
        client_info.capabilities = ClientCapabilities::default();
        client_info.client_info =
            Implementation::new("Chatspeed MCP Client", env!("CARGO_PKG_VERSION"))
                .with_title("Chatspeed")
                .with_website_url("https://chatspeed.aidyou.ai");
        let client_service_result = client_info
            .serve(transport)
            .await
            .inspect_err(|e| log::error!("MCP StreamableHttp client error: {}", e.to_string()));

        let client_service = match client_service_result {
            Ok(cs) => cs,
            Err(e) => {
                let detailed_error = e.to_string();
                log::error!("Start HttpClient error: {}", detailed_error);
                return Err(McpError::ClientStartError(
                    t!(
                        "mcp.client.http_service_start_failed",
                        url = url,
                        error = detailed_error
                    )
                    .to_string(),
                ));
            }
        };
        Ok(client_service)
    }

    fn client(&self) -> Arc<RwLock<Option<RunningService<RoleClient, InitializeRequestParams>>>> {
        self.core.get_client_instance_arc()
    }

    async fn name(&self) -> String {
        self.core.get_name().await
    }

    async fn config(&self) -> McpServerConfig {
        self.core.get_config().await
    }

    async fn update_disabled_tools(
        &self,
        tool_name: &str,
        is_disabled: bool,
    ) -> McpClientResult<()> {
        self.core
            .update_disabled_tools(tool_name, is_disabled)
            .await;
        Ok(())
    }

    async fn status(&self) -> McpStatus {
        self.core.get_status().await
    }

    async fn on_status_change(&self, callback: StatusChangeCallback) {
        self.core.set_on_status_change_callback(callback).await;
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;
    use std::time::Duration;

    use axum::Router;
    use rmcp::model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    };
    use rmcp::service::{RequestContext, RoleServer};
    use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService,
    };
    use rmcp::{ErrorData, ServerHandler};

    use crate::mcp::{
        client::{
            streamable_http::StreamableHttpClient, McpClient as _, McpProtocolType,
            McpServerConfig, McpStatus,
        },
        McpError,
    };

    /// A deterministic, loopback MCP server. It speaks the real streamable HTTP
    /// transport, so the client exercises the full protocol without any
    /// external service.
    #[derive(Clone)]
    struct LoopbackMcpServer;

    impl ServerHandler for LoopbackMcpServer {
        fn get_info(&self) -> ServerConfig {
            let mut info = ServerConfig::default();
            info.capabilities = ServerCapabilities::builder().enable_tools().build();
            info.server_info = Implementation::new("loopback-fixture", "1.0.0");
            info
        }

        async fn list_tools(
            &self,
            _request: Option<PaginatedRequestParams>,
            _context: RequestContext<RoleServer>,
        ) -> Result<ListToolsResult, ErrorData> {
            Ok(ListToolsResult::with_all_items(vec![Tool::new(
                "fixture_echo",
                "echo the requested payload back",
                Arc::new(serde_json::Map::new()),
            )]))
        }

        async fn call_tool(
            &self,
            request: CallToolRequestParams,
            _context: RequestContext<RoleServer>,
        ) -> Result<CallToolResponse, ErrorData> {
            Ok(CallToolResponse::Complete(CallToolResult::structured(
                serde_json::json!({ "tool": request.name, "ok": true }),
            )))
        }
    }

    /// Binds the fixture to an ephemeral loopback port and returns its MCP
    /// endpoint plus the serving task so the test can shut it down.
    async fn spawn_loopback_server() -> (String, tokio::task::JoinHandle<()>) {
        let factory = move || Ok(LoopbackMcpServer);
        let service = StreamableHttpService::new(
            factory,
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default(),
        );
        let router = Router::new().nest_service("/mcp", service);
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind the loopback MCP server");
        let port = listener.local_addr().expect("server address").port();
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        (format!("http://127.0.0.1:{port}/mcp"), task)
    }

    /// The full initialize -> list -> call -> stop lifecycle against the local
    /// fixture. `start()` must complete the handshake before it returns.
    #[tokio::test]
    async fn http_start_lists_calls_and_stops_against_a_local_server() -> Result<(), McpError> {
        let (url, task) = spawn_loopback_server().await;
        let client = StreamableHttpClient::new(McpServerConfig {
            name: "loopback-fixture".into(),
            protocol_type: McpProtocolType::StreamableHttp,
            url: Some(url),
            timeout: Some(10),
            ..Default::default()
        })?;

        client.start().await?;
        assert_eq!(client.status().await, McpStatus::Connected);

        let tools = client.list_tools().await?;
        assert_eq!(client.status().await, McpStatus::Running);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "fixture_echo");

        let result = client
            .call("fixture_echo", serde_json::json!({ "value": 7 }))
            .await?;
        assert!(
            result.to_string().contains("fixture_echo"),
            "unexpected call result: {result}"
        );

        client.stop().await?;
        assert_eq!(client.status().await, McpStatus::Stopped);
        task.abort();
        Ok(())
    }

    /// A dead endpoint must be reported as an honest start failure, never a
    /// fabricated connection.
    #[tokio::test]
    async fn http_start_classifies_an_unreachable_endpoint_as_a_start_error() -> Result<(), McpError>
    {
        // Reserve a loopback port and release it, so nothing is listening there.
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("reserve an unused port");
        let port = listener.local_addr().expect("reserved address").port();
        drop(listener);

        let client = StreamableHttpClient::with_retry(
            McpServerConfig {
                name: "unreachable-fixture".into(),
                protocol_type: McpProtocolType::StreamableHttp,
                url: Some(format!("http://127.0.0.1:{port}/mcp")),
                timeout: Some(2),
                ..Default::default()
            },
            0,
            Duration::from_millis(1),
        )?;

        let error = client.start().await.expect_err("no server is listening");
        assert!(
            matches!(error, McpError::ClientStartError(_)),
            "connection failures must be classified as start errors, got: {error:?}"
        );
        assert!(
            matches!(client.status().await, McpStatus::Error(_)),
            "a failed start must surface an error status"
        );
        Ok(())
    }
}
