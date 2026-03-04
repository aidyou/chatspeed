use crate::ai::traits::chat::MCPToolDeclaration;
use crate::tools::{NativeToolResult, ToolCallResult, ToolCategory, ToolDefinition};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct AskUser;

#[async_trait]
impl ToolDefinition for AskUser {
    fn name(&self) -> &str {
        crate::tools::TOOL_ASK_USER
    }
    fn description(&self) -> &str {
        "Use this tool when you need to ask the user questions during execution. This allows you to:\n\
        1. Gather user preferences or requirements\n\
        2. Clarify ambiguous instructions\n\
        3. Get decisions on implementation choices as you work\n\
        4. Offer choices to the user about what direction to take.\n\n\
        Usage notes:\n\
        - Users will always be able to select \"Other\" to provide custom text input\n\
        - If you recommend a specific option, make that the first option in the list and add \"(Recommended)\" at the end of the label"
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Interaction
    }
    fn scope(&self) -> crate::tools::ToolScope {
        crate::tools::ToolScope::Workflow
    }
    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "question": { "type": "string", "description": "The complete question to ask the user. Should be clear and specific." }
                },
                "required": ["question"]
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }
    async fn call(&self, _params: Value) -> NativeToolResult {
        Ok(ToolCallResult::success(
            Some("Waiting for user response".into()),
            None,
        ))
    }
}

pub struct FinishTask;

#[async_trait]
impl ToolDefinition for FinishTask {
    fn name(&self) -> &str {
        crate::tools::TOOL_FINISH_TASK
    }
    fn description(&self) -> &str {
        "Signals that the current task has been fully addressed and is now complete. \
        This tool takes no arguments. Ensure you have provided a comprehensive summary of your work and conclusions in your plain text response BEFORE calling this tool."
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Interaction
    }
    fn scope(&self) -> crate::tools::ToolScope {
        crate::tools::ToolScope::Workflow
    }
    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }
    async fn call(&self, _params: Value) -> NativeToolResult {
        Ok(ToolCallResult::success(Some("Task finished".into()), None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_ask_user() {
        let tool = AskUser;
        let params = json!({
            "question": "What is your preference?"
        });

        let result = tool.call(params).await.unwrap();
        assert_eq!(result.content.unwrap(), "Waiting for user response");
    }

    #[tokio::test]
    async fn test_ask_user_without_question() {
        let tool = AskUser;
        // Tool doesn't validate, so empty params should work
        let params = json!({});

        let result = tool.call(params).await.unwrap();
        assert_eq!(result.content.unwrap(), "Waiting for user response");
    }

    #[tokio::test]
    async fn test_finish_task() {
        let tool = FinishTask;
        let params = json!({});

        let result = tool.call(params).await.unwrap();
        assert_eq!(result.content.unwrap(), "Task finished");
    }

    // Test that all tools return ToolCallResult with expected structure
    #[tokio::test]
    async fn test_tool_result_structure() {
        let tools: Vec<Box<dyn ToolDefinition>> = vec![
            Box::new(AskUser),
            Box::new(FinishTask),
        ];

        for tool in tools {
            let params = json!({});
            let result = tool.call(params).await.unwrap();
            assert!(result.content.is_some());
            assert!(result.structured_content.is_none());
        }
    }
}
