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
        CallToolRequestParams, CallToolResult, Implementation, InitializeRequestParams,
        InitializeResult, ListToolsResult, PaginatedRequestParams, ProtocolVersion,
        ServerCapabilities, ServerInfo, Tool,
    },
    service::{NotificationContext, Peer, RequestContext, RoleServer},
    ServerHandler,
};
use rust_i18n::t;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tokio::sync::{broadcast, Mutex, RwLock};

/// Converts MCPToolDeclaration to rmcp Tool
impl From<MCPToolDeclaration> for Tool {
    fn from(tool: MCPToolDeclaration) -> Self {
        // Convert serde_json::Value to Arc<JsonObject>
        let input_schema = match tool.input_schema {
            Value::Object(obj) => Arc::new(obj),
            _ => Arc::new(serde_json::Map::new()), // Fallback to empty object
        };

        Tool::new(tool.name, tool.description, input_schema)
    }
}

#[derive(Clone)]
struct ToolReference {
    /// Canonical registry name for a native or MCP tool.
    registry_name: String,
}

impl ToolReference {
    fn new(registry_name: String) -> Self {
        Self { registry_name }
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
    /// Receives changes to the externally visible MCP tool list.
    tool_change_receiver: Mutex<Option<broadcast::Receiver<()>>>,
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
        let tool_change_receiver = chat_state.tool_manager.subscribe_mcp_tool_change_events();
        Self {
            chat_state,
            tool_map: Arc::new(RwLock::new(HashMap::new())),
            tool_change_receiver: Mutex::new(Some(tool_change_receiver)),
        }
    }

    async fn start_tool_change_notifier(&self, peer: Peer<RoleServer>) {
        let Some(mut receiver) = self.tool_change_receiver.lock().await.take() else {
            return;
        };
        let tool_map = self.tool_map.clone();

        tokio::spawn(async move {
            loop {
                match receiver.recv().await {
                    Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {
                        tool_map.write().await.clear();
                        if let Err(error) = peer.notify_tool_list_changed().await {
                            log::debug!(
                                "Failed to notify MCP client about a tool list change: {}",
                                error
                            );
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    async fn load_tool_map(&self) -> Result<Vec<Tool>, McpError> {
        let exclude_tools = HashSet::from(["chat_completion".to_string()]);
        let all_tools = self
            .chat_state
            .tool_manager
            .get_tool_calling_spec(None, Some(exclude_tools))
            .await
            .unwrap_or_default();
        let mut display_tools = Vec::new();
        let mut new_tool_map = HashMap::new();

        for tool_spec in all_tools {
            if tool_spec.disabled
                || !matches!(
                    tool_spec.scope,
                    Some(crate::tools::ToolScope::Chat) | Some(crate::tools::ToolScope::Both)
                )
            {
                continue;
            }
            let registry_name = self
                .chat_state
                .tool_manager
                .resolve_tool_name(&tool_spec.name)
                .await;
            // Keep the existing nested-proxy guard, but apply it to canonical identity.
            if registry_name.matches(MCP_TOOL_NAME_SPLIT).count() > 1 {
                continue;
            }
            let public_name = tool_spec.name.clone();
            display_tools.push(tool_spec.into());
            new_tool_map.insert(public_name, ToolReference::new(registry_name));
        }

        *self.tool_map.write().await = new_tool_map;
        Ok(display_tools)
    }

    /// Ensures the tool map is loaded, reloads if empty
    async fn ensure_tool_map_loaded(&self) -> Result<(), McpError> {
        let tool_map_guard = self.tool_map.read().await;
        if !tool_map_guard.is_empty() {
            return Ok(());
        }
        drop(tool_map_guard);

        log::debug!("Tool map is empty, reloading tools...");
        self.load_tool_map().await.map(|_| ())
    }
}

impl ServerHandler for McpProxyHandler {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.protocol_version = ProtocolVersion::default();
        info.capabilities = ServerCapabilities::builder()
            .enable_tools()
            .enable_tool_list_changed()
            .build();
        info.server_info = Implementation::new("Chatspeed MCP Hub", env!("CARGO_PKG_VERSION"))
            .with_title("Chatspeed")
            .with_website_url("https://chatspeed.aidyou.ai");
        info.instructions = Some(t!("mcp.proxy.service_description").to_string());
        info
    }

    async fn initialize(
        &self,
        _request: InitializeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, rmcp::model::ErrorData> {
        Ok(self.get_info())
    }

    async fn on_initialized(&self, context: NotificationContext<RoleServer>) {
        self.start_tool_change_notifier(context.peer).await;
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::model::ErrorData> {
        let display_tools = self.load_tool_map().await?;
        Ok(ListToolsResult::with_all_items(display_tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, rmcp::model::ErrorData> {
        // Ensure tool map is loaded
        self.ensure_tool_map_loaded().await?;

        let tool_map_guard = self.tool_map.read().await;
        let tool_name: &str = request.name.as_ref();
        let tool_ref = match tool_map_guard.get(tool_name) {
            Some(tool) => tool.clone(),
            None => {
                let error = json!({"error":t!("mcp.proxy.tool_not_found", tool_name = request.name).to_string()});
                return Ok(CallToolResult::structured_error(error));
            }
        };
        drop(tool_map_guard); // Explicitly release lock

        let arguments = request.arguments.unwrap_or_default();

        log::debug!(
            "MCP client calling public tool '{}', which maps to registry '{}', with arguments: {:?}",
            request.name,
            &tool_ref.registry_name,
            arguments
        );

        let result = self
            .chat_state
            .tool_manager
            .tool_call(&tool_ref.registry_name, Value::Object(arguments))
            .await;

        match result {
            Ok(tool_result) => {
                let (content, structured_content, is_error) = if let Ok(result) =
                    serde_json::from_value::<CallToolResult>(tool_result.clone())
                {
                    (
                        result.content,
                        result.structured_content,
                        result.is_error.unwrap_or(false),
                    )
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
                        false,
                    )
                } else {
                    (
                        tool_result.to_string().into_contents(),
                        Some(tool_result.clone()),
                        false,
                    )
                };
                let call_result = if let Some(structured_content) = structured_content {
                    let mut result = if is_error {
                        CallToolResult::structured_error(structured_content)
                    } else {
                        CallToolResult::structured(structured_content)
                    };
                    if !content.is_empty() {
                        result.content = content;
                    }
                    result
                } else if is_error {
                    CallToolResult::error(content)
                } else {
                    CallToolResult::success(content)
                };
                Ok(call_result)
            }
            Err(e) => {
                let error = json!({"error":t!("mcp.proxy.tool_execution_error", error = e.to_string())
                .to_string()});
                Ok(CallToolResult::structured_error(error))
            }
        }
    }
}
