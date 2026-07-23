use crate::ai::traits::chat::MCPToolDeclaration;
use crate::tools::{NativeToolResult, ToolCallResult, ToolCategory, ToolDefinition, ToolError};
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
        Do NOT use ask_user for routine status updates, final answers, progress reports, generic feedback surveys, or plan approval in Planning Mode; use submit_plan for plan approval and answer_user/complete_workflow for reporting.\n\n\
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
    async fn call(&self, params: Value) -> NativeToolResult {
        let items = params
            .get("items")
            .and_then(|value| value.as_array())
            .ok_or_else(|| ToolError::InvalidParams("items must be a non-empty array".into()))?;
        if items.is_empty() {
            return Err(ToolError::InvalidParams(
                "items must be a non-empty array".into(),
            ));
        }
        for item in items {
            let title = item.get("title").and_then(|value| value.as_str());
            let options = item.get("options").and_then(|value| value.as_array());
            if title
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
                || options.map_or(true, |values| values.is_empty())
            {
                return Err(ToolError::InvalidParams(
                    "Each ask_user item must include a non-empty title and at least one option"
                        .into(),
                ));
            }
        }
        Ok(ToolCallResult::success(
            Some("Waiting for user response".into()),
            Some(json!({
                "status": "waiting_for_user",
                "items_count": items.len()
            })),
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
        "Submits a proposed plan for user review before implementation begins. \
        The authoritative approval payload MUST be provided in the structured `plan` argument. Do not rely on surrounding assistant text as the plan source. \
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
                    "plan": { "type": "string", "description": "The complete detailed Markdown plan that the user should approve. This field is required and is the only authoritative plan payload." }
                },
                "required": ["plan"]
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }
    async fn call(&self, params: Value) -> NativeToolResult {
        let plan = params
            .get("plan")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ToolError::InvalidParams("plan must be a non-empty string".into()))?;
        Ok(ToolCallResult::success(
            Some("Plan submitted for review. Entering 'Awaiting Approval' state.".into()),
            Some(json!({
                "status": "awaiting_approval",
                "plan_length": plan.len()
            })),
        ))
    }
}

pub struct FinishTask;
pub struct SubmitResult;

#[async_trait]
impl ToolDefinition for FinishTask {
    fn name(&self) -> &str {
        crate::tools::TOOL_COMPLETE_WORKFLOW
    }
    fn description(&self) -> &str {
        "Signals that the current task has been fully addressed and is now complete.\n\n\
        Strict usage rules:\n\
        - Call this only after the requested work is actually complete or you have reached a clear stopping point accepted by the user.\n\
        - A completion report is required. Put the full report in `summary` unless a valid report is already present in this assistant response or the runtime explicitly says it captured a pending report.\n\
        - `summary` is optional only when the runtime can use that current or pending report. Otherwise it must be a non-empty, complete report.\n\
        - If the runtime says it captured a pending completion report, omit `summary` and visible report text to commit that exact draft without repeating it.\n\
        - Equivalent report sources are deduplicated, but conflicting reports are rejected.\n\
        - A completion report must explicitly cover: 1) what was completed, 2) what was verified, and 3) any important remaining notes or limitations.\n\
        - Reasoning/thinking text does not count as a completion report.\n\
        - Do not use placeholder reports such as 'done', 'completed', or hidden reasoning.\n\
        - If completion is rejected, resolve the reported issue before calling this tool again."
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
                    "summary": {
                        "type": "string",
                        "minLength": 1,
                        "description": "The complete user-visible completion report. Required unless a valid report is already present in the current assistant response or was explicitly captured by the runtime."
                    }
                },
                "additionalProperties": false
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }
    async fn call(&self, params: Value) -> NativeToolResult {
        let object = params
            .as_object()
            .ok_or_else(|| ToolError::InvalidParams("arguments must be an object".into()))?;
        let report_source = object.get("report_source").and_then(Value::as_str);
        let summary_is_non_empty = object
            .get("summary")
            .and_then(Value::as_str)
            .is_some_and(|summary| !summary.trim().is_empty());
        let valid_shape = match object.len() {
            0 => true,
            1 => {
                summary_is_non_empty
                    || (report_source == Some("assistant_message")
                        && !object.contains_key("summary"))
            }
            2 => report_source == Some("tool_argument") && summary_is_non_empty,
            _ => false,
        };
        if !valid_shape {
            return Err(ToolError::InvalidParams(
                "complete_workflow accepts an optional non-empty summary; only historical report_source payloads are additionally supported for compatibility".into(),
            ));
        }

        Ok(ToolCallResult::success(
            Some("Task finished".into()),
            Some(json!({ "status": "completed" })),
        ))
    }
}

#[async_trait]
impl ToolDefinition for SubmitResult {
    fn name(&self) -> &str {
        crate::tools::TOOL_SUBMIT_RESULT
    }
    fn description(&self) -> &str {
        "Submits the final result of a child agent and ends the delegated task. \
        This tool is only for child-agent workflows.\n\n\
        Required fields:\n\
        - `result`: The complete final result the parent agent should consume.\n\
        - `summary`: A short summary for notifications and task lists.\n\n\
        Strict usage rules:\n\
        - Call this exactly once when the delegated task is complete or cannot proceed further.\n\
        - Put the full parent-facing output in `result`; do not rely on previous assistant messages to carry the final answer.\n\
        - Keep `summary` concise and specific."
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
                    "result": {
                        "type": "string",
                        "description": "The complete final result for the parent agent."
                    },
                    "summary": {
                        "type": "string",
                        "description": "A concise summary of the delegated result."
                    }
                },
                "required": ["result", "summary"],
                "additionalProperties": false
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }
    async fn call(&self, params: Value) -> NativeToolResult {
        let result = params
            .get("result")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ToolError::InvalidParams("result must be a non-empty string".into()))?
            .to_string();
        let summary = params
            .get("summary")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ToolError::InvalidParams("summary must be a non-empty string".into()))?
            .to_string();

        Ok(ToolCallResult::success(
            Some(result.clone()),
            Some(json!({
                "status": "completed",
                "result": result,
                "summary": summary
            })),
        ))
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
        let params = json!({});

        let result = tool.call(params).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_finish_task_uses_optional_summary_schema() {
        let tool = FinishTask;
        let declaration = tool.tool_calling_spec();

        assert_eq!(declaration.input_schema["type"], "object");
        assert_eq!(
            declaration.input_schema["properties"]["summary"]["type"],
            "string"
        );
        assert_eq!(
            declaration.input_schema["properties"]["summary"]["minLength"],
            1
        );
        assert_eq!(declaration.input_schema["additionalProperties"], false);
        assert!(declaration.input_schema.get("required").is_none());

        for params in [
            json!({}),
            json!({ "summary": "Completed the requested work and verified it." }),
        ] {
            let result = tool.call(params).await.unwrap();
            assert_eq!(result.content.unwrap(), "Task finished");
            assert_eq!(result.structured_content.unwrap()["status"], "completed");
        }
    }

    #[tokio::test]
    async fn test_finish_task_accepts_legacy_arguments_at_runtime_boundary() {
        let tool = FinishTask;

        for params in [
            json!({ "summary": "Legacy single-argument completion report." }),
            json!({ "report_source": "assistant_message" }),
            json!({
                "report_source": "tool_argument",
                "summary": "Legacy multi-argument completion report."
            }),
        ] {
            assert!(tool.call(params).await.is_ok());
        }
    }

    #[tokio::test]
    async fn test_finish_task_rejects_invalid_legacy_argument_shapes() {
        let tool = FinishTask;

        for params in [
            json!({ "summary": "" }),
            json!({ "summary": "   " }),
            json!({ "summary": 42 }),
            json!({ "report_source": "assistant_message", "summary": "duplicate" }),
            json!({ "report_source": "tool_argument" }),
            json!({ "report_source": "unknown", "summary": "report" }),
            json!({ "unexpected": "value" }),
        ] {
            assert!(tool.call(params).await.is_err());
        }
    }

    // Test that all tools return ToolCallResult with expected structure
    #[tokio::test]
    async fn test_tool_result_structure() {
        let tools: Vec<Box<dyn ToolDefinition>> = vec![
            Box::new(AskUser),
            Box::new(FinishTask),
            Box::new(SubmitResult),
        ];
        let params_by_name = |name: &str| match name {
            crate::tools::TOOL_ASK_USER => json!({
                "items": [{"title": "Choose", "options": ["A", "B"]}]
            }),
            crate::tools::TOOL_COMPLETE_WORKFLOW => json!({}),
            crate::tools::TOOL_SUBMIT_RESULT => json!({
                "result": "Result",
                "summary": "Summary"
            }),
            _ => json!({}),
        };

        for tool in tools {
            let params = params_by_name(tool.name());
            let result = tool.call(params).await.unwrap();
            assert!(result.content.is_some());
            assert!(result.structured_content.is_some());
        }
    }

    #[tokio::test]
    async fn test_submit_result() {
        let tool = SubmitResult;
        let params = json!({
            "result": "OCR module uses provider X",
            "summary": "OCR module analyzed"
        });

        let result = tool.call(params).await.unwrap();
        assert_eq!(result.content.unwrap(), "OCR module uses provider X");
        assert_eq!(
            result.structured_content.unwrap()["summary"]
                .as_str()
                .unwrap(),
            "OCR module analyzed"
        );
    }
}
