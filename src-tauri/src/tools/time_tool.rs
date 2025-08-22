use async_trait::async_trait;
use chrono::Local;
use serde_json::{json, Value};

use crate::{
    ai::traits::chat::MCPToolDeclaration,
    tools::{NativeToolResult, ToolCallResult, ToolDefinition},
};

pub struct TimeTool;

impl TimeTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl ToolDefinition for TimeTool {
    fn name(&self) -> &str {
        "getCurrentTime"
    }

    fn description(&self) -> &str {
        "Get the current local time in Y-m-d H:M:S %Z format"
    }

    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
            output_schema: None,
            disabled: false,
        }
    }

    /// Gets the current local time in Y-m-d H:M:S %Z format.
    ///
    /// # Returns
    /// Returns a `ToolResult` containing the current time as a simple string.
    async fn call(&self, _params: Value) -> NativeToolResult {
        let local_time = Local::now();
        let formatted_time = local_time.format("%Y-%m-%d %H:%M:%S %Z").to_string();

        Ok(ToolCallResult::success(
            Some(formatted_time.clone()),
            Some(json!({"current_time":formatted_time})),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_time_tool() {
        let tool = TimeTool::new();

        let params = json!({});
        let result = tool.call(params).await;

        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(response.content.is_some());

        // Verify the time format is correct (YYYY-MM-DD HH:MM:SS TZ)
        let time_str = response.content.as_ref().unwrap();
        log::debug!("time_str: {}", time_str);
        assert!(time_str.len() > 19); // At least contains date and time part
        assert!(time_str.contains("-")); // Contains date separator
        assert!(time_str.contains(":")); // Contains time separator
        assert!(time_str.contains(" ")); // Contains space separator
    }
}
