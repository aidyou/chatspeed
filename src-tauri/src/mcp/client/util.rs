use std::sync::Arc;

use rmcp::model::ListToolsResult;
use serde_json::Value;

use crate::ai::traits::chat::MCPToolDeclaration;

/// get_tools converts ListToolsResult to Vec<MCPToolDeclaration>
///
/// # Arguments
/// * `list_tools_result` - The ListToolsResult to convert
///
/// # Returns
/// * `Vec<MCPToolDeclaration>` - The converted Vec<MCPToolDeclaration>
pub fn get_tools(list_tools_result: &ListToolsResult) -> Vec<MCPToolDeclaration> {
    let mut tools = vec![];
    for tool in list_tools_result.tools.iter() {
        tools.push(MCPToolDeclaration {
            name: tool.name.to_string(),
            description: tool.description.to_string(),
            input_schema: Value::Object(
                Arc::try_unwrap(tool.input_schema.clone()).unwrap_or_else(|arc| (*arc).clone()),
            ),
            disabled: false,
        });
    }
    tools
}
