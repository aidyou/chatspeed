//! MCP Tool Loader
//!
//! This module provides on-demand loading of MCP tool schemas.
//! Instead of injecting all MCP tool schemas into the context upfront,
//! only tool descriptions are shown, and the full schema is loaded when needed.

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::ai::traits::chat::MCPToolDeclaration;
use crate::tools::tool_manager::ToolManager;
use crate::tools::{NativeToolResult, ToolCallResult, ToolCategory, ToolDefinition, ToolError, ToolScope};
use std::sync::Arc;

/// MCP Tool Loader
///
/// Loads detailed parameter schemas for MCP tools on demand.
/// This reduces context token usage by not including full schemas upfront.
pub struct McpToolLoad {
    pub tool_manager: Arc<ToolManager>,
}

#[async_trait]
impl ToolDefinition for McpToolLoad {
    fn name(&self) -> &str {
        "mcp_tool_load"
    }

    fn description(&self) -> &str {
        "Load detailed parameter schema for a specific MCP tool. MUST be called before invoking any MCP tool that doesn't display detailed parameter information. For tools already showing full schemas, this is not needed."
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
                        "description": "The combined name of the MCP tool (format: server__MCP__tool)"
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

        let declaration = self.tool_manager.get_mcp_tool_declaration(tool_name).await?;

        Ok(ToolCallResult::success(
            None,
            Some(serde_json::to_value(declaration).unwrap_or_default()),
        ))
    }
}
