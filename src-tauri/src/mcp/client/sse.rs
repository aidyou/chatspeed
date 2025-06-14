//! Server-Sent Events (SSE) client implementation for ModelScope Control Protocol (MCP)
//!
//! Provides persistent bi-directional communication channel using SSE as transport layer.
//!
//! # Features
//! - Auto-reconnection with exponential backoff
//! - Bearer token authentication support
//! - Proxy configuration support
//! - Thread-safe connection sharing
//!
//! # Usage Example
//! ```no_run
//! use mcp::client::{McpServerConfig, McpProtocolType};
//!
//! #[tokio::main]
//! async fn main() {
//!     let config = McpServerConfig {
//!         protocol_type: McpProtocolType::Sse,
//!         url: Some("https://api.example.com/sse".into()),
//!         bearer_token: Some("your_token".into()),
//!         ..Default::default()
//!     };
//!
//!     let client = SseClient::new(config).unwrap();
//!     client.start().await.unwrap();
//!
//!     // Get available tools
//!     let tools = client.list_tools().await.unwrap();
//!
//!     // Call remote tool
//!     let result = client.call("tool_name", serde_json::json!({...})).await;
//! }
//! ```
//!
//! # Reconnection Strategy
//! Implements hybrid retry policy combining:
//! 1. Server-suggested retry interval (from `retry` field in SSE events)
//! 2. Client-configured minimum interval (default 3 seconds)
//!
//! Maximum retry attempts default to 10 times (configurable)

use std::{sync::Arc, time::Duration};

use reqwest::{header, Client};
use rmcp::{
    service::RunningService,
    transport::{
        common::client_side_sse::FixedInterval, sse_client::SseClientConfig, SseClientTransport,
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
/// - Establishing SSE connections
/// - Managing automatic reconnections
/// - Executing remote tool calls
pub struct SseClient {
    core: McpClientCore,
}

impl SseClient {
    /// Creates a new SSE Protocol of MCP client instance with given configuration
    ///
    /// # Arguments
    /// * `config` - Server configuration parameters
    ///
    /// # Returns
    /// `McpClientResult<Self>` - New client instance or validation error
    ///
    /// # Errors
    /// Returns `McpClientError::ConfigError` if:
    /// - Protocol type mismatch
    /// - URL is empty or not provided
    pub fn new(config: McpServerConfig) -> McpClientResult<Self> {
        if config.protocol_type != McpProtocolType::Sse {
            return Err(McpClientError::ConfigError(
                t!(
                    "mcp.client.config_mismatch",
                    client = "SseClient",
                    protocol_type = config.protocol_type.to_string()
                )
                .to_string(),
            ));
        }

        if config.url.as_deref().unwrap_or_default().is_empty() {
            return Err(McpClientError::ConfigError(
                t!("mcp.client.sse_url_cant_be_empty").to_string(),
            ));
        }

        Ok(SseClient {
            core: McpClientCore::new(config),
        })
    }

    /// Builds a configured HTTP client for SSE connections
    ///
    /// # Arguments
    /// * `&self` - Current SseClient instance
    ///
    /// # Returns
    /// `McpClientResult<reqwest::Client>` - Configured HTTP client or error
    ///
    /// # Errors
    /// Returns `McpClientError::ConfigError` if:
    /// - Bearer token format is invalid
    /// - Proxy configuration is invalid
    async fn build_http_client_async(&self) -> McpClientResult<reqwest::Client> {
        // Renamed and made async
        let mut client_builder = Client::builder().timeout(Duration::from_secs(30));
        let current_config = self.core.get_config().await;
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

        // Set proxy
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
impl McpClientInternal for SseClient {
    async fn set_status(&self, status: McpStatus) {
        self.core.set_status(status).await;
    }

    async fn notify_status_change(&self, name: String, status: McpStatus) {
        self.core.notify_status_change(name, status).await;
    }
}

#[async_trait::async_trait]
impl McpClient for SseClient {
    /// Performs the actual SSE connection logic.
    /// This method is called by the default `start` implementation in the `McpClient` trait.
    ///
    /// # Returns
    /// `McpClientResult<RunningService<RoleClient, ()>>` - The running service instance
    /// on success, or an error.
    ///
    /// # Errors
    /// Returns `McpClientError::StartError` if:
    /// - Connection establishment fails
    /// - Transport initialization fails
    async fn perform_connect(&self) -> McpClientResult<RunningService<RoleClient, ()>> {
        let config = self.core.get_config().await; // Use the async getter
        let url_str = config.url.as_deref().filter(|s| !s.is_empty());

        let url = match url_str {
            Some(u) => u,
            None => {
                let err_msg = t!("mcp.client.sse_url_cant_be_empty").to_string();
                return Err(McpClientError::ConfigError(err_msg));
            }
        };

        let http_client = self.build_http_client_async().await?; // Call the async version
        let retry_config = FixedInterval {
            max_times: Some(10),
            duration: Duration::from_secs(3),
        };
        let transport_config = SseClientConfig {
            retry_policy: Arc::new(retry_config),
            ..SseClientConfig::default()
        };
        let transport_result =
            SseClientTransport::start_with_client(http_client, transport_config).await;

        let transport = match transport_result {
            Ok(t) => t,
            Err(e) => {
                return Err(McpClientError::StartError(
                    t!(
                        "mcp.client.sse_transport_start_failed",
                        url = url,
                        error = e.to_string()
                    )
                    .to_string(),
                ));
            }
        };

        let client_service_result = ().serve(transport).await;
        let client_service = match client_service_result {
            Ok(cs) => cs,
            Err(e) => {
                // Optional: Wrap with t!
                let detailed_error = e.to_string();
                log::error!("Start SseClient error: {}", detailed_error);
                return Err(McpClientError::StartError(
                    t!(
                        "mcp.client.sse_service_start_failed",
                        url = url,
                        error = detailed_error
                    )
                    .to_string(),
                ));
            }
        };
        Ok(client_service)
    }

    /// Provides access to the underlying client instance
    ///
    /// # Returns
    /// Thread-safe reference to the running service
    fn client(&self) -> Arc<RwLock<Option<RunningService<RoleClient, ()>>>> {
        self.core.get_client_instance_arc()
    }

    /// Returns the client type identifier
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
        sse::SseClient, McpClient as _, McpClientError, McpProtocolType, McpServerConfig,
    };

    #[tokio::test]
    async fn sse_test() -> Result<(), McpClientError> {
        let client = SseClient::new(McpServerConfig {
            protocol_type: McpProtocolType::Sse,
            url: Some("https://mcp.api-inference.modelscope.cn/sse/56e15cc0adab45".to_string()),
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
                serde_json::json!({"query":"deepseek r2"}),
            )
            .await?;
        log::info!("Tool result: {}", tool_result);

        client.stop().await?;
        Ok(())
    }
}
