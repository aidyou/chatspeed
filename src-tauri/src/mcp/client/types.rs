use rmcp::model::CallToolRequestParam;
use rmcp::service::RunningService;
use rmcp::RoleClient;
use rust_i18n::t;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;
use tokio::sync::RwLock;

use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::sync::Arc;

use crate::ai::traits::chat::MCPToolDeclaration; // Ensure this is the correct path

use super::util::get_tools;

/// MCP protocol type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum McpProtocolType {
    /// Server-Sent Events protocol
    Sse,
    /// Standard I/O protocol
    Stdio,
}
impl From<&str> for McpProtocolType {
    fn from(value: &str) -> Self {
        match value {
            "sse" => McpProtocolType::Sse,
            "stdio" => McpProtocolType::Stdio,
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
    pub disabled_tools: Option<Vec<String>>,
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
        }
    }
}

/// Collection of MCP servers
pub type McpServers = HashMap<String, McpServerConfig>;

#[derive(Debug, Error, Clone)]
pub enum McpClientError {
    /// Call error
    #[error("{}", t!("mcp.client.call_error", error =.0))]
    CallError(String),
    /// Configuration error
    #[error("{}", t!("mcp.client.config_error", error =.0))]
    ConfigError(String),
    /// Start error
    #[error("{}", t!("mcp.client.failed_to_start", error = .0))]
    StartError(String),
    /// Stop error
    #[error("{}", t!("mcp.client.failed_to_stop", error =.0))]
    StopError(String),

    #[error("{}", t!("mcp.client.failed_to_get_status", error =.0))]
    StatusError(String),
}

pub type McpClientResult<T> = Result<T, McpClientError>;

#[derive(Debug, Clone, PartialEq)]
pub enum McpStatus {
    Running,
    Stopped,
    Error(String),
}
/// Main trait containing methods for an MCP client.
/// This trait is designed to be object-safe for use with `dyn McpClient`.
#[async_trait::async_trait]
pub trait McpClient: Send + Sync {
    /// Gets the name of the MCP client
    fn name(&self) -> String;

    /// Gets the MCP server configuration
    fn config(&self) -> McpServerConfig;

    /// Gets the running service instance
    /// Implementors should return a clone of their `Arc<RwLock<Option<RunningService<...>>>>`.
    fn client(&self) -> Arc<RwLock<Option<RunningService<RoleClient, ()>>>>;

    async fn status(&self) -> McpStatus;

    async fn set_status(&self, status: McpStatus);

    /// Performs the client-specific connection logic.
    /// This method should establish the connection and return the running service instance
    /// upon success, or an error upon failure. It should NOT set McpStatus itself,
    /// nor should it store the running service in the shared client field.
    async fn perform_connect(&self) -> McpClientResult<RunningService<RoleClient, ()>>;

    /// Starts the MCP client connection.
    /// This default implementation calls `perform_connect` and manages status updates
    /// and storage of the running service.
    async fn start(&self) -> McpClientResult<()> {
        match self.perform_connect().await {
            Ok(running_service) => {
                *self.client().write().await = Some(running_service);
                self.set_status(McpStatus::Running).await;
                Ok(())
            }
            Err(e) => {
                self.set_status(McpStatus::Error(e.to_string())).await;
                Err(e)
            }
        }
    }

    /// Stops the running MCP client.
    /// This is a default implementation.
    async fn stop(&self) -> McpClientResult<()> {
        let client_arc = self.client();
        let mut guard = client_arc.write().await;
        if let Some(service_instance) = guard.take() {
            match service_instance.cancel().await {
                Ok(_) => {
                    self.set_status(McpStatus::Stopped).await;
                    Ok(())
                }
                Err(e) => {
                    let err_msg = e.to_string();
                    // Even if stopping fails, the service instance has been taken.
                    // The status should reflect the error.
                    self.set_status(McpStatus::Error(format!("Failed to stop: {}", err_msg)))
                        .await;
                    Err(McpClientError::StopError(err_msg))
                }
            }
        } else {
            // If client was already None (e.g., already stopped or never started)
            self.set_status(McpStatus::Stopped).await; // Ensure status is Stopped
            Ok(()) // Considered successful as it's already in a stopped state
        }
    }

    /// Lists all available tools from the connected MCP server.
    /// This is a default implementation.
    async fn list_tools(&self) -> McpClientResult<Vec<MCPToolDeclaration>> {
        let client_arc = self.client();
        let guard = client_arc.read().await; // Use read lock
        if let Some(service_instance) = guard.as_ref() {
            let tools = service_instance
                .list_tools(Default::default())
                .await
                .map_err(|e| McpClientError::StatusError(e.to_string()))?;
            Ok(get_tools(&tools))
        } else {
            Err(McpClientError::StatusError(
                t!("mcp.client.no_running", client = self.name()).to_string(),
            ))
        }
    }

    /// Calls a specific tool with given arguments.
    /// This is a default implementation.
    async fn call(&self, tool_name: &str, args: Value) -> McpClientResult<Value> {
        let client_arc = self.client();
        let guard = client_arc.read().await; // Use read lock
        if let Some(service_instance) = guard.as_ref() {
            let call_tool_result = service_instance
                .call_tool(CallToolRequestParam {
                    name: tool_name.to_string().into(),
                    arguments: self.arg_parser(args),
                })
                .await
                .map_err(|e| McpClientError::CallError(e.to_string()))?;

            // Check the `is_error` field from rmcp::model::CallToolResult
            // If `is_error` is Some(true), it indicates a tool execution error.
            if call_tool_result.is_error.unwrap_or(false) {
                // Serialize the content as the error message if an error occurred.
                // This assumes `Content` can be serialized.
                let error_content_str = serde_json::to_string(&call_tool_result.content)
                    .unwrap_or_else(|e| format!("Failed to serialize error content: {}", e));
                return Err(McpClientError::CallError(error_content_str));
            }

            // If not an error (is_error is Some(false) or None),
            // serialize the content as the successful result.
            // This assumes `Content` can be serialized to `Value`.
            // We'll serialize the Vec<Content> into a JSON Value (likely an array).
            serde_json::to_value(call_tool_result.content).map_err(|e| {
                McpClientError::CallError(format!("Failed to serialize successful content: {}", e))
            })
        } else {
            Err(McpClientError::StatusError(
                t!("mcp.client.no_running", client = self.name()).to_string(),
            ))
        }
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
