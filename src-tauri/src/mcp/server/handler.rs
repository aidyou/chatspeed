//! MCP Proxy Handler
//!
//! This module implements the MCP server handler that proxies tool calls to the internal tool manager.

use crate::ai::traits::chat::MCPToolDeclaration;
use crate::mcp::McpError;
use crate::tools::ToolCallResult;
use crate::{ai::interaction::chat_completion::ChatState, tools::MCP_TOOL_NAME_SPLIT};

use rmcp::model::IntoContents;
use rmcp::{
    model::{
        CallToolRequestParam, CallToolResult, Implementation, InitializeRequestParam,
        InitializeResult, ListToolsResult, PaginatedRequestParam, ProtocolVersion,
        ServerCapabilities, ServerInfo, Tool,
    },
    service::{RequestContext, RoleServer},
    ServerHandler,
};
use rust_i18n::t;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tokio::sync::RwLock;

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
            output_schema: None,
            annotations: None,
            title: None,
            icons: None,
        }
    }
}

#[derive(Clone)]
struct ToolReference {
    /// MCP server name if this is an MCP tool, None for native tools
    server_name: Option<String>,
    /// The actual tool name
    tool_name: String,
}

impl ToolReference {
    fn new(server_name: Option<String>, tool_name: String) -> Self {
        Self {
            server_name,
            tool_name,
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
    /// Tool map for mapping tool names
    /// Key: display name
    /// Value: ToolReference
    tool_map: Arc<RwLock<HashMap<String, ToolReference>>>,
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
        Self {
            chat_state,
            tool_map: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Ensures the tool map is loaded, reloads if empty
    async fn ensure_tool_map_loaded(&self) -> Result<(), McpError> {
        let tool_map_guard = self.tool_map.read().await;
        if !tool_map_guard.is_empty() {
            return Ok(());
        }
        drop(tool_map_guard);

        log::debug!("Tool map is empty, reloading tools...");

        // Rebuild tool map using the same logic as list_tools
        let exclude_tools = HashSet::from(["chat_completion".to_string()]);
        let all_tools = self
            .chat_state
            .tool_manager
            .get_tool_calling_spec(Some(exclude_tools))
            .await
            .unwrap_or_default();

        let mut new_tool_map = HashMap::new();
        let mut used_short_names = HashSet::new();

        for tool_spec in all_tools {
            // Built-in tools
            if !tool_spec.name.contains(MCP_TOOL_NAME_SPLIT) {
                let tool_name = tool_spec.name.clone();
                new_tool_map.insert(tool_name.clone(), ToolReference::new(None, tool_name));
                continue;
            }

            // mcp tools
            // disable the tool of mcp proxy itself
            if tool_spec.name.matches(MCP_TOOL_NAME_SPLIT).count() > 1 {
                continue;
            }

            let parts: Vec<&str> = tool_spec.name.splitn(2, MCP_TOOL_NAME_SPLIT).collect();
            if parts.len() == 2 {
                let server_name = parts[0].to_string();
                let original_tool_name = parts[1].to_string();

                let display_name = if !used_short_names.contains(&original_tool_name) {
                    original_tool_name.clone()
                } else {
                    format!("{}_{}", server_name, original_tool_name)
                };

                if new_tool_map.contains_key(&display_name) {
                    continue;
                }

                used_short_names.insert(display_name.clone());
                new_tool_map.insert(
                    display_name.clone(),
                    ToolReference::new(Some(server_name.clone()), original_tool_name.clone()),
                );
            }
        }

        // Update the shared tool map
        let mut tool_map = self.tool_map.write().await;
        *tool_map = new_tool_map;

        Ok(())
    }
}

impl ServerHandler for McpProxyHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::default(),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "Chatspeed MCP Hub".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                title: Some("Chatspeed".to_string()),
                website_url: Some("https://chatspeed.aidyou.ai".to_string()),
                icons: None,
            },
            instructions: Some(t!("mcp.proxy.service_description").to_string()),
        }
    }

    async fn initialize(
        &self,
        _request: InitializeRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, rmcp::model::ErrorData> {
        Ok(self.get_info())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::model::ErrorData> {
        let exclude_tools = HashSet::from(["chat_completion".to_string()]);
        let all_tools = self
            .chat_state
            .tool_manager
            .get_tool_calling_spec(Some(exclude_tools))
            .await
            .unwrap_or_default();

        let mut display_tools = Vec::new();
        let mut new_tool_map = HashMap::new();
        let mut used_short_names = HashSet::new();

        for tool_spec in all_tools {
            // Built-in tools
            if !tool_spec.name.contains(MCP_TOOL_NAME_SPLIT) {
                let tool_name = tool_spec.name.clone();
                display_tools.push(tool_spec.into());
                new_tool_map.insert(tool_name.clone(), ToolReference::new(None, tool_name));
                continue;
            }

            // mcp tools

            // disable the tool of mcp proxy itself
            if tool_spec.name.matches(MCP_TOOL_NAME_SPLIT).count() > 1 {
                continue;
            }

            let parts: Vec<&str> = tool_spec.name.splitn(2, MCP_TOOL_NAME_SPLIT).collect();
            if parts.len() == 2 {
                let server_name = parts[0].to_string();
                let original_tool_name = parts[1].to_string();

                let display_name = if !used_short_names.contains(&original_tool_name) {
                    original_tool_name.clone()
                } else {
                    format!("{}_{}", server_name, original_tool_name)
                };

                if new_tool_map.contains_key(&display_name) {
                    // This means the prefixed name also conflicts, which implies a duplicate tool from the same server.
                    // As per the user's request, we skip it.
                    continue;
                }

                used_short_names.insert(display_name.clone());
                new_tool_map.insert(
                    display_name.clone(),
                    ToolReference::new(Some(server_name.clone()), original_tool_name.clone()),
                );

                let mut new_tool: Tool = tool_spec.into();
                new_tool.name = display_name.into();
                display_tools.push(new_tool);
            }
        }

        // Update the shared tool map
        let mut tool_map = self.tool_map.write().await;
        *tool_map = new_tool_map;

        Ok(ListToolsResult {
            tools: display_tools,
            next_cursor: None,
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, rmcp::model::ErrorData> {
        // Ensure tool map is loaded
        self.ensure_tool_map_loaded().await?;

        let tool_map_guard = self.tool_map.read().await;
        let tool_ref = match tool_map_guard.get(request.name.as_ref()) {
            Some(tool) => tool.clone(),
            None => {
                let error = json!({"error":t!("mcp.proxy.tool_not_found", tool_name = request.name).to_string()});
                return Ok(CallToolResult {
                    content: error.to_string().into_contents(),
                    structured_content: Some(error),
                    is_error: Some(true),
                    meta: None,
                });
            }
        };
        drop(tool_map_guard); // 显式释放锁

        let arguments = request.arguments.unwrap_or_default();

        log::debug!(
            "MCP client calling tool '{}', which maps to internal tool ('{}', '{}'), with arguments: {:?}",
            request.name,
            tool_ref.server_name.as_deref().unwrap_or_default(),
             &tool_ref.tool_name,
            arguments
        );

        // Call the tool manager to execute the tool
        let result = if let Some(server_name) = tool_ref.server_name.as_deref() {
            self.chat_state
                .tool_manager
                .mcp_tool_call(server_name, &tool_ref.tool_name, Value::Object(arguments))
                .await
        } else {
            self.chat_state
                .tool_manager
                .native_tool_call(&tool_ref.tool_name, Value::Object(arguments))
                .await
                .map(|r| r.into())
        };

        match result {
            Ok(tool_result) => {
                let (content, structured_content) = if let Ok(result) =
                    serde_json::from_value::<CallToolResult>(tool_result.clone())
                {
                    (result.content, result.structured_content)
                } else if let Ok(result) =
                    serde_json::from_value::<ToolCallResult>(tool_result.clone())
                {
                    (
                        result
                            .content
                            .map(|s| s.into_contents())
                            .unwrap_or_default(),
                        // result.structured_content,
                        None,
                    )
                } else {
                    (
                        tool_result.to_string().into_contents(),
                        Some(tool_result.clone()),
                    )
                };
                Ok(CallToolResult {
                    content,
                    structured_content,
                    is_error: Some(false),
                    meta: None,
                })
            }
            Err(e) => {
                let error = json!({"error":t!("mcp.proxy.tool_execution_error", error = e.to_string())
                .to_string()});
                Ok(CallToolResult {
                    content: error.to_string().into_contents(),
                    structured_content: Some(error),
                    is_error: Some(true),
                    meta: None,
                })
            }
        }
    }
}
