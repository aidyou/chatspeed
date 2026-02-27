use async_trait::async_trait;
use serde_json::{json, Value};
use crate::tools::{ToolDefinition, NativeToolResult, ToolCallResult, ToolCategory};
use crate::ai::traits::chat::MCPToolDeclaration;

pub struct AnswerUser;

#[async_trait]
impl ToolDefinition for AnswerUser {
    fn name(&self) -> &str { "answer_user" }
    fn description(&self) -> &str { 
        "Delivers a partial or final answer to the user. Use this whenever you want to provide information, \
        give a summary, or simply talk to the user. This tool ensures your response is properly formatted and delivered." 
    }
    fn category(&self) -> ToolCategory { ToolCategory::System }
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
        Ok(ToolCallResult::success(Some("Message delivered".into()), None))
    }
}

pub struct AskUser;

#[async_trait]
impl ToolDefinition for AskUser {
    fn name(&self) -> &str { "ask_user" }
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
    fn category(&self) -> ToolCategory { ToolCategory::System }
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
        Ok(ToolCallResult::success(Some("Waiting for user response".into()), None))
    }
}

pub struct FinishTask;

#[async_trait]
impl ToolDefinition for FinishTask {
    fn name(&self) -> &str { "finish_task" }
    fn description(&self) -> &str { 
        "Signals that the current task has been fully addressed and is now complete. \
        Provide a comprehensive summary of the work performed and any final conclusions." 
    }
    fn category(&self) -> ToolCategory { ToolCategory::System }
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
