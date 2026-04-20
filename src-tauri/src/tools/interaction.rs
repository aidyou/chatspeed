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
        "Ask the user for a blocking decision that is required before you can continue.\n\n\
        Use ask_user only when the next step genuinely depends on user input, such as choosing between mutually exclusive implementation paths, clarifying an ambiguous requirement, or confirming a risky change. \
        Do NOT use ask_user for routine status updates, final answers, progress reports, generic feedback surveys, or plan approval in Planning Mode; use submit_plan for plan approval and answer_user/finish_task for reporting.\n\n\
        Usage rules:\n\
        - Pass grouped choices using an `items` array.\n\
        - Prefer one focused group. Use multiple groups only when each group is an independent blocking decision.\n\
        - Each item MUST use the shape {\"title\": \"...\", \"options\": [\"...\", \"...\"]}.\n\
        - Titles must be direct questions or decision labels, not vague headings like \"Any thoughts?\".\n\
        - Options must be concise, mutually exclusive, and actionable. Do not include placeholder options like \"I want to input...\" because the UI already provides custom text input.\n\
        - If you recommend a specific option, make it the first option and add \"(Recommended)\" at the end of the label.\n\
        - Include the concrete consequence in the option text when it matters, for example \"Proceed with safe change; skip data backfill\"."
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
                    "items": {
                        "type": "array",
                        "description": "Grouped blocking decisions. Prefer one focused item; use multiple items only for independent decisions that all block progress.",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": {
                                "title": {
                                    "type": "string",
                                    "description": "A direct question or decision label. Avoid vague headings like 'Any thoughts?' or status-only text."
                                },
                                "options": {
                                    "type": "array",
                                    "items": { "type": "string" },
                                    "minItems": 1,
                                    "description": "Concise, mutually exclusive, actionable options. Do not include a custom-input placeholder; the UI already provides custom text input. Put the recommended option first and suffix it with '(Recommended)' when applicable."
                                }
                            },
                            "required": ["title", "options"],
                            "additionalProperties": false
                        }
                    }
                }
                ,
                "required": ["items"],
                "additionalProperties": false
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

pub struct SubmitPlan;

#[async_trait]
impl ToolDefinition for SubmitPlan {
    fn name(&self) -> &str {
        crate::tools::TOOL_SUBMIT_PLAN
    }
    fn description(&self) -> &str {
        "Submits a proposed plan for user review. This tool is only available in Planning Mode. \
        The plan should be a detailed Markdown document outlining the research findings and implementation steps you intend to take. \
        Once submitted, the session will enter an 'Awaiting Approval' state where the user can review and approve your plan before you begin execution."
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
                    "plan": { "type": "string", "description": "The detailed Markdown plan." }
                },
                "required": ["plan"]
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }
    async fn call(&self, _params: Value) -> NativeToolResult {
        Ok(ToolCallResult::success(
            Some("Plan submitted for review. Entering 'Awaiting Approval' state.".into()),
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
            "items": [
                {
                    "title": "Choose a strategy",
                    "options": ["Fast", "Safe"]
                }
            ]
        });

        let result = tool.call(params).await.unwrap();
        assert_eq!(result.content.unwrap(), "Waiting for user response");
    }

    #[tokio::test]
    async fn test_ask_user_with_empty_params() {
        let tool = AskUser;
        // Tool doesn't validate, so empty params should work
        let params = json!([]);

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
        let tools: Vec<Box<dyn ToolDefinition>> = vec![Box::new(AskUser), Box::new(FinishTask)];

        for tool in tools {
            let params = json!({});
            let result = tool.call(params).await.unwrap();
            assert!(result.content.is_some());
            assert!(result.structured_content.is_none());
        }
    }
}
