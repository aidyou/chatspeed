use crate::ai::traits::chat::MCPToolDeclaration;
use crate::tools::{NativeToolResult, ToolCallResult, ToolCategory, ToolDefinition};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct AnswerUser;

#[async_trait]
impl ToolDefinition for AnswerUser {
    fn name(&self) -> &str {
        crate::tools::TOOL_ANSWER_USER
    }
    fn description(&self) -> &str {
        "Delivers a partial or final answer to the user. Use this whenever you want to provide information, \
        give a summary, or simply talk to the user. This tool ensures your response is properly formatted and delivered."
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
                    "text": { "type": "string", "description": "The message content to deliver to the user. Supports Markdown." }
                },
                "required": ["text"]
            }),
            output_schema: None,
            disabled: false,
        }
    }
    async fn call(&self, _params: Value) -> NativeToolResult {
        Ok(ToolCallResult::success(
            Some("Message delivered".into()),
            None,
        ))
    }
}

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
        Provide a comprehensive summary of the work performed and any final conclusions."
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
                    "summary": { "type": "string", "description": "A comprehensive summary of the work performed." }
                },
                "required": ["summary"]
            }),
            output_schema: None,
            disabled: false,
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
    async fn test_answer_user() {
        let tool = AnswerUser;
        let params = json!({
            "text": "Hello, this is a test message."
        });

        let result = tool.call(params).await.unwrap();
        assert_eq!(result.content.unwrap(), "Message delivered");
    }

    #[tokio::test]
    async fn test_answer_user_empty_params() {
        let tool = AnswerUser;
        // The tool doesn't validate params, but should still work
        let params = json!({});

        let result = tool.call(params).await.unwrap();
        assert_eq!(result.content.unwrap(), "Message delivered");
    }

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
        let params = json!({
            "summary": "Completed all work successfully."
        });

        let result = tool.call(params).await.unwrap();
        assert_eq!(result.content.unwrap(), "Task finished");
    }

    #[tokio::test]
    async fn test_finish_task_empty_summary() {
        let tool = FinishTask;
        let params = json!({});

        let result = tool.call(params).await.unwrap();
        assert_eq!(result.content.unwrap(), "Task finished");
    }

    // Test edge cases for input types
    #[tokio::test]
    async fn test_answer_user_with_non_string_text() {
        let tool = AnswerUser;
        let params = json!({
            "text": 12345  // Non-string value
        });

        // Should still work since tool doesn't validate type
        let result = tool.call(params).await.unwrap();
        assert_eq!(result.content.unwrap(), "Message delivered");
    }

    #[tokio::test]
    async fn test_ask_user_with_complex_question() {
        let tool = AskUser;
        let long_question = "A".repeat(1000);
        let params = json!({
            "question": long_question
        });

        let result = tool.call(params).await.unwrap();
        assert_eq!(result.content.unwrap(), "Waiting for user response");
    }

    // Test that all tools return ToolCallResult with expected structure
    #[tokio::test]
    async fn test_tool_result_structure() {
        let tools: Vec<Box<dyn ToolDefinition>> = vec![
            Box::new(AnswerUser),
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
