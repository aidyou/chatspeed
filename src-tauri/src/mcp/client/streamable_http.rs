//! Streamable HTTP client implementation for ModelScope Control Protocol (MCP)
use std::sync::Arc;
use std::time::Duration;

use reqwest::{header, Client};
use rmcp::{
    model::{ClientCapabilities, ClientInfo, Implementation, InitializeRequestParam},
    service::RunningService,
    transport::{
        common::client_side_sse::ExponentialBackoff,
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    RoleClient, ServiceExt as _,
};
use rust_i18n::t;
use tokio::sync::RwLock;

use super::core::McpClientCore;
use super::{
    types::{McpClientInternal, McpStatus, StatusChangeCallback},
    McpClient, McpClientError, McpClientResult, McpProtocolType, McpServerConfig,
};

/// Handles connection lifecycle and provides methods for:
/// - Establishing HTTP connections
/// - Executing remote tool calls
pub struct StreamableHttpClient {
    core: McpClientCore,
}

impl StreamableHttpClient {
    /// Creates a new HTTP Protocol of MCP client instance with given configuration
    pub fn new(config: McpServerConfig) -> McpClientResult<Self> {
        if config.protocol_type != McpProtocolType::StreamableHttp {
            return Err(McpClientError::ConfigError(
                t!(
                    "mcp.client.config_mismatch",
                    client = "StreamableHttpClient",
                    protocol_type = config.protocol_type.to_string()
                )
                .to_string(),
            ));
        }

        if config.url.as_deref().unwrap_or_default().is_empty() {
            return Err(McpClientError::ConfigError(
                t!("mcp.client.http_url_cant_be_empty").to_string(),
            ));
        }

        Ok(StreamableHttpClient {
            core: McpClientCore::new(config),
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
                        .map_err(|e| McpClientError::ConfigError(e.to_string()))?,
                );

                client_builder = client_builder.default_headers(headers);
            }
        }

        if let Some(proxy) = current_config.proxy.as_ref() {
            if !proxy.trim().is_empty() {
                let proxy = reqwest::Proxy::all(proxy)
                    .map_err(|e| McpClientError::ConfigError(e.to_string()))?;
                client_builder = client_builder.proxy(proxy);
            }
        }

        let http_client = client_builder
            .build()
            .map_err(|e| McpClientError::ConfigError(e.to_string()))?;
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
    ) -> McpClientResult<RunningService<RoleClient, InitializeRequestParam>> {
        let config = self.core.get_config().await;
        let url_str = config.url.as_deref().filter(|s| !s.is_empty());

        let url = match url_str {
            Some(u) => u,
            None => {
                let err_msg = t!("mcp.client.http_url_cant_be_empty").to_string();
                return Err(McpClientError::ConfigError(err_msg));
            }
        };

        let http_client = self.build_http_client_async().await?;
        let retry_config = ExponentialBackoff {
            max_times: Some(120),
            base_duration: Duration::from_secs(2),
        };
        let transport_config = StreamableHttpClientTransportConfig {
            uri: Arc::from(url),
            retry_config: Arc::new(retry_config),
            auth_header: config.bearer_token.clone(),
            ..Default::default()
        };
        let transport = StreamableHttpClientTransport::with_client(http_client, transport_config);

        let client_info = ClientInfo {
            protocol_version: Default::default(),
            capabilities: ClientCapabilities::default(),
            client_info: Implementation {
                name: "Chatspeed MCP Client".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                title: Some("Chatspeed".to_string()),
                website_url: Some("https://chatspeed.aidyou.ai".to_string()),
                icons: None,
            },
        };
        let client_service_result = client_info
            .serve(transport)
            .await
            .inspect_err(|e| log::error!("MCP StreamableHttp client error: {}", e.to_string()));

        let client_service = match client_service_result {
            Ok(cs) => cs,
            Err(e) => {
                let detailed_error = e.to_string();
                log::error!("Start HttpClient error: {}", detailed_error);
                return Err(McpClientError::StartError(
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

    fn client(&self) -> Arc<RwLock<Option<RunningService<RoleClient, InitializeRequestParam>>>> {
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
    use crate::mcp::client::{
        streamable_http::StreamableHttpClient, McpClient as _, McpClientError, McpProtocolType,
        McpServerConfig,
    };

    #[tokio::test]
    async fn http_test() -> Result<(), McpClientError> {
        let client = StreamableHttpClient::new(McpServerConfig {
            protocol_type: McpProtocolType::StreamableHttp,
            url: Some("http://127.0.0.1:8000/tools/inference/sse".to_string()),
            ..Default::default()
        })?;
        client.start().await?;

        // List tools
        let tools = client.list_tools().await?;
        log::info!(
            "{}",
            format!(
                "Available tools: {}",
                serde_json::to_string_pretty(&tools).expect("Failed to serialize tools")
            )
        );

        let tool_result = client
            .call(
                "bing_search".into(),
                serde_json::json!({ "query": "deepseek v2" }),
            )
            .await?;
        log::info!("Tool result: {}", tool_result);

        client.stop().await?;
        Ok(())
    }
}
