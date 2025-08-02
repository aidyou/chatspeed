//! MCP Proxy Handler
//!
//! This module implements the MCP server handler that proxies tool calls to the internal tool manager.

use crate::ai::traits::chat::MCPToolDeclaration;
use crate::{ai::interaction::chat_completion::ChatState, workflow::MCP_TOOL_NAME_SPLIT};
use rmcp::{
    model::{
        CallToolRequestParam, CallToolResult, Content, Implementation, InitializeRequestParam,
        InitializeResult, ListToolsResult, PaginatedRequestParam, ProtocolVersion,
        ServerCapabilities, ServerInfo, Tool,
    },
    service::{RequestContext, RoleServer},
    ErrorData as McpError, ServerHandler,
};
use rust_i18n::t;
use serde_json::Value;
use std::{collections::HashSet, sync::Arc};

/// Converts MCPToolDeclaration to rmcp Tool
impl From<MCPToolDeclaration> for Tool {
    fn from(tool: MCPToolDeclaration) -> Self {
        // Convert serde_json::Value to Arc<JsonObject>
        let input_schema = match tool.input_schema {
            Value::Object(obj) => Arc::new(obj),
            _ => Arc::new(serde_json::Map::new()), // Fallback to empty object
        };

        Tool {
            name: tool.name.into(),
            description: Some(tool.description.into()),
            input_schema,
            // output_schema: None,
            annotations: None,
        }
    }
}

/// MCP Proxy Handler
///
/// This handler implements the MCP ServerHandler trait and proxies tool calls
/// to the internal tool manager.
pub struct McpProxyHandler {
    /// Chat state for accessing the tool manager
    chat_state: Arc<ChatState>,
}

impl McpProxyHandler {
    /// Creates a new MCP proxy handler
    ///
    /// # Arguments
    /// * `chat_state` - Chat state instance for accessing the tool manager
    ///
    /// # Returns
    /// Returns a new MCP proxy handler instance
    pub fn new(chat_state: Arc<ChatState>) -> Self {
        Self { chat_state }
    }
}

impl ServerHandler for McpProxyHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::default(),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "Chatspeed MCP Proxy".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
            instructions: Some(t!("mcp.proxy.service_description").to_string()),
        }
    }

    async fn initialize(
        &self,
        _request: InitializeRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        Ok(self.get_info())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        // exclude tools
        let exclude_tools = HashSet::from(["chat_completion".to_string()]);
        let tools: Vec<Tool> = self
            .chat_state
            .tool_manager
            .get_tool_calling_spec(Some(exclude_tools))
            .await
            .map(|tool_calling_spec| {
                tool_calling_spec
                    .into_iter()
                    .filter(|t| t.name.split(MCP_TOOL_NAME_SPLIT).count() <= 2) //disable the tool of mcp proxy itself
                    .map(|t| t.into())
                    .collect()
            })
            .unwrap_or(vec![]);

        Ok(ListToolsResult {
            tools: tools,
            next_cursor: None,
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let tool_name = if request.name.contains(MCP_TOOL_NAME_SPLIT) {
            let parts: Vec<&str> = request.name.split(MCP_TOOL_NAME_SPLIT).collect();
            parts[parts.len().saturating_sub(2)..].join(MCP_TOOL_NAME_SPLIT)
        } else {
            request.name.to_string()
        };

        let arguments = request.arguments.unwrap_or_default();

        log::debug!(
            "MCP client call mcp tool name: {}, arguments: {:?}",
            tool_name,
            arguments
        );

        // Call the tool manager to execute the tool
        let result = self
            .chat_state
            .tool_manager
            .tool_call(&tool_name, Value::Object(arguments))
            .await;

        match result {
            Ok(tool_result) => Ok(CallToolResult {
                content: vec![Content::text(tool_result.to_string())].into(),
                // structured_content: Some(tool_result),
                is_error: Some(false),
            }),
            Err(e) => Ok(CallToolResult {
                content: vec![Content::text(t!(
                    "mcp.proxy.tool_execution_error",
                    error = e.to_string()
                ))]
                .into(),
                // structured_content: Some(serde_json::json!({"text":t!(
                //     "mcp.proxy.tool_execution_error",
                //     error = e.to_string()
                // )})),
                is_error: Some(true),
            }),
        }
    }
}
