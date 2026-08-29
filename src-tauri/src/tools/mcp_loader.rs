//! MCP Tool Loader
//!
//! This module provides on-demand loading of MCP tool schemas.
//! Instead of injecting all MCP tool schemas into the context upfront,
//! only tool descriptions are shown, and the full schema is loaded when needed.

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::ai::traits::chat::MCPToolDeclaration;
use crate::tools::tool_manager::ToolManager;
use crate::tools::{
    NativeToolResult, ToolCallResult, ToolCategory, ToolDefinition, ToolError, ToolScope,
};
use std::collections::HashSet;
use std::sync::Arc;

/// MCP Tool Loader
///
/// Loads detailed parameter schemas for MCP tools on demand.
/// This reduces context token usage by not including full schemas upfront.
pub struct McpToolLoad {
    pub tool_manager: Arc<ToolManager>,
    pub allowed_tools: Option<HashSet<String>>,
}

#[async_trait]
impl ToolDefinition for McpToolLoad {
    fn name(&self) -> &str {
        crate::tools::TOOL_MCP_TOOL_LOAD
    }

    fn description(&self) -> &str {
        "Load the complete definition of one folded MCP tool, including its authoritative public name and detailed input schema. This only loads the definition; it does NOT execute the MCP tool. For a tool listed under AVAILABLE MCP TOOLS, call this exactly once immediately before using that tool, then call the returned MCP tool directly as your next tool action using the returned schema. Do not call mcp_tool_load again while the same unchanged definition is still visible in the current context; if the definition has been updated, a new work segment starts, context is manually cleared or compressed, or the definition is no longer visible, load it again. Do not use this for an MCP tool whose full schema is already in the API tool list."
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
                "Loaded the complete definition for folded MCP tool '{}'. This lookup did not execute the MCP tool. Call '{}' directly in your next tool action using this authoritative declaration; do not call mcp_tool_load again while this definition is still visible in the current context. If a new work segment starts, context is manually cleared or compressed, or the definition is no longer visible, load it again.\n\nFull MCP tool definition:\n{}",
                tool_name, declaration.name, declaration_json
            )),
            Some(serde_json::to_value(declaration).unwrap_or_default()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn description_requires_immediate_direct_execution_after_loading() {
        let loader = McpToolLoad {
            tool_manager: Arc::new(ToolManager::new()),
            allowed_tools: None,
        };
        let description = loader.description();

        assert!(description.contains("does NOT execute the MCP tool"));
        assert!(description.contains("next tool action"));
        assert!(description.contains("while the same unchanged definition is still visible"));
        assert!(description.contains("definition has been updated"));
        assert!(description.contains("new work segment starts"));
    }
    #[tokio::test]
    async fn rejects_mcp_tool_outside_allowed_list() {
        let loader = McpToolLoad {
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
