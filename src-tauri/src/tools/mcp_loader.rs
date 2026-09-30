//! MCP Tool Expander
//!
//! This module provides on-demand expansion of MCP tool schemas.
//! Instead of injecting all MCP tool schemas into the context upfront,
//! only tool descriptions are shown, and the full schema is expanded when needed.

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::ai::traits::chat::MCPToolDeclaration;
use crate::tools::tool_manager::ToolManager;
use crate::tools::{
    NativeToolResult, ToolCallResult, ToolCategory, ToolDefinition, ToolError, ToolScope,
};
use std::collections::HashSet;
use std::sync::Arc;

/// MCP Tool Expander
///
/// Loads detailed parameter schemas for MCP tools on demand.
/// This reduces context token usage by not including full schemas upfront.
pub struct McpToolExpand {
    pub tool_manager: Arc<ToolManager>,
    pub allowed_tools: Option<HashSet<String>>,
}

#[async_trait]
impl ToolDefinition for McpToolExpand {
    fn name(&self) -> &str {
        crate::tools::TOOL_MCP_TOOL_EXPAND
    }

    fn description(&self) -> &str {
        "Auto-expanded MCP tools already appear directly with their full definitions. For a folded MCP tool, provide its public name to expand it before calling mcp_tool_execute. This only loads the definition; it does NOT execute the MCP tool."
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }

    fn scope(&self) -> ToolScope {
        ToolScope::Both
    }

    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "tool_name": {
                        "type": "string",
                        "description": "The public short name of an MCP tool listed in the available tool declarations"
                    }
                },
                "required": ["tool_name"]
            }),
            output_schema: None,
            disabled: false,
            scope: Some(ToolScope::Both),
        }
    }

    async fn call(&self, params: Value) -> NativeToolResult {
        let tool_name = params["tool_name"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidParams("tool_name is required".to_string()))?;

        let canonical_name = self.tool_manager.resolve_mcp_tool_name(tool_name).await;

        if self.allowed_tools.as_ref().is_some_and(|tools| {
            !tools.contains(tool_name)
                && !canonical_name
                    .as_ref()
                    .is_some_and(|canonical_name| tools.contains(canonical_name))
        }) {
            return Err(ToolError::Security(format!(
                "MCP tool '{}' is not available in this workflow",
                tool_name
            )));
        }

        let canonical_name = canonical_name
            .ok_or_else(|| ToolError::InvalidParams("Not an MCP tool".to_string()))?;

        let declaration = self
            .tool_manager
            .get_mcp_tool_declaration(&canonical_name)
            .await?;

        let declaration_json =
            serde_json::to_string_pretty(&declaration).unwrap_or_else(|_| declaration.name.clone());
        Ok(ToolCallResult::success(
            Some(format!(
                "Loaded the complete definition for folded MCP tool '{}'. This lookup did not execute the MCP tool. Call `mcp_tool_execute` in your next tool action with `tool_name` set to '{}' and `arguments` matching this authoritative declaration; do not call mcp_tool_expand again while this definition is still visible in the current context. If a new work segment starts, context is manually cleared or compressed, or the definition is no longer visible, load it again.\n\nFull MCP tool definition:\n{}",
                tool_name, declaration.name, declaration_json
            )),
            Some(serde_json::to_value(declaration).unwrap_or_default()),
        ))
    }
}

/// Stable-schema declaration for folded MCP tool execution.
///
/// The workflow engine unwraps calls to this control tool before approval and execution, so all
/// permissions apply to the canonical target. `call` remains a defensive fallback for non-workflow
/// callers and repeats target authorization before dispatch.
pub struct McpToolExecute {
    pub tool_manager: Arc<ToolManager>,
    pub allowed_tools: Option<HashSet<String>>,
}

impl McpToolExecute {
    async fn resolve_target(&self, tool_name: &str) -> Result<String, ToolError> {
        let canonical_name = self
            .tool_manager
            .resolve_mcp_tool_name(tool_name)
            .await
            .ok_or_else(|| {
                ToolError::InvalidParams(format!("MCP tool '{}' was not found", tool_name))
            })?;

        if self
            .allowed_tools
            .as_ref()
            .is_some_and(|tools| !tools.contains(&canonical_name))
        {
            return Err(ToolError::Security(format!(
                "MCP tool '{}' is not available in this workflow",
                tool_name
            )));
        }

        // This rejects disabled tools even though they remain canonically addressable for MCP
        // configuration and fail-closed authorization checks.
        self.tool_manager
            .get_mcp_tool_declaration(&canonical_name)
            .await?;

        let server_name = canonical_name
            .split_once(crate::tools::MCP_TOOL_NAME_SPLIT)
            .map(|(server_name, _)| server_name)
            .ok_or_else(|| {
                ToolError::InvalidParams("Invalid canonical MCP tool name".to_string())
            })?;
        let server = self.tool_manager.get_mcp_server(server_name).await?;
        match server.status().await {
            crate::mcp::client::McpStatus::Connected | crate::mcp::client::McpStatus::Running => {
                Ok(canonical_name)
            }
            status => Err(ToolError::ExecutionFailed(format!(
                "MCP server '{}' is not available (status: {})",
                server_name, status
            ))),
        }
    }
}

#[async_trait]
impl ToolDefinition for McpToolExecute {
    fn name(&self) -> &str {
        crate::tools::TOOL_MCP_TOOL_EXECUTE
    }

    fn description(&self) -> &str {
        "Executes one available MCP tool through a stable interface. Use the target's public name from the available MCP declarations and pass arguments matching the definition returned by mcp_tool_expand. Authorization and approval are evaluated for the resolved target MCP tool, not for this dispatcher."
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }

    fn scope(&self) -> ToolScope {
        ToolScope::Both
    }

    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "tool_name": {
                        "type": "string",
                        "description": "The public short name or canonical name of the target MCP tool"
                    },
                    "arguments": {
                        "type": "object",
                        "description": "Arguments matching the target MCP tool definition"
                    }
                },
                "required": ["tool_name", "arguments"],
                "additionalProperties": false
            }),
            output_schema: None,
            disabled: false,
            scope: Some(ToolScope::Both),
        }
    }

    async fn call(&self, params: Value) -> NativeToolResult {
        let tool_name = params
            .get("tool_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| ToolError::InvalidParams("tool_name is required".to_string()))?;
        let arguments = params
            .get("arguments")
            .filter(|arguments| arguments.is_object())
            .cloned()
            .ok_or_else(|| ToolError::InvalidParams("arguments must be an object".to_string()))?;
        let canonical_name = self.resolve_target(tool_name).await?;

        let result = self
            .tool_manager
            .native_tool_call(&canonical_name, arguments)
            .await?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn description_requires_immediate_direct_execution_after_loading() {
        let loader = McpToolExpand {
            tool_manager: Arc::new(ToolManager::new()),
            allowed_tools: None,
        };
        let description = loader.description();

        assert_eq!(loader.name(), crate::tools::TOOL_MCP_TOOL_EXPAND);
        assert!(!loader.description().contains("mcp_tool_load"));
        assert!(description.contains("full definitions"));
        assert!(description.contains("folded MCP tool"));
        assert!(description.contains("does NOT execute the MCP tool"));
    }

    #[test]
    fn executor_has_a_stable_target_agnostic_schema() {
        let executor = McpToolExecute {
            tool_manager: Arc::new(ToolManager::new()),
            allowed_tools: None,
        };
        let declaration = executor.tool_calling_spec();

        assert_eq!(declaration.name, crate::tools::TOOL_MCP_TOOL_EXECUTE);
        assert_eq!(
            declaration.input_schema["required"],
            json!(["tool_name", "arguments"])
        );
        assert_eq!(
            declaration.input_schema["properties"]["arguments"]["type"],
            "object"
        );
        assert_eq!(declaration.input_schema["additionalProperties"], false);
    }

    #[tokio::test]
    async fn executor_rejects_invalid_arguments_before_target_resolution() {
        let executor = McpToolExecute {
            tool_manager: Arc::new(ToolManager::new()),
            allowed_tools: None,
        };

        let error = executor
            .call(json!({ "tool_name": "server__MCP__tool", "arguments": "{}" }))
            .await
            .expect_err("executor arguments must be a JSON object");

        assert!(matches!(error, ToolError::InvalidParams(_)));
    }

    #[tokio::test]
    async fn rejects_mcp_tool_outside_allowed_list() {
        let loader = McpToolExpand {
            tool_manager: Arc::new(ToolManager::new()),
            allowed_tools: Some(HashSet::from(["server__MCP__allowed".to_string()])),
        };

        let error = loader
            .call(json!({ "tool_name": "server__MCP__blocked" }))
            .await
            .expect_err("blocked MCP tools must not expose their declaration");

        assert!(matches!(error, ToolError::Security(_)));
    }
}
