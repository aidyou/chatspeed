use crate::ai::traits::chat::MCPToolDeclaration;
use crate::db::MainStore;
use crate::tools::{NativeToolResult, ToolCallResult, ToolCategory, ToolDefinition, ToolError};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

fn compact_user_message_payload(message: crate::db::WorkflowMessage) -> Value {
    json!({
        "message_id": message.id,
        "role": message.role,
        "message": message.message,
        "attached_context": message.attached_context,
    })
}

/// Reads one authoritative user-authored workflow message by database ID.
///
/// The workflow engine owns the session ID, so callers cannot read messages from
/// another workflow even when they know its message ID. The role is an explicit
/// input contract and is currently restricted to user messages.
pub struct ReadHistoryMessage {
    pub session_id: String,
    pub main_store: Arc<MainStore>,
}

#[async_trait]
impl ToolDefinition for ReadHistoryMessage {
    fn name(&self) -> &str {
        crate::tools::TOOL_READ_HISTORY_MESSAGE
    }

    fn description(&self) -> &str {
        "Read one authoritative user-authored workflow message by message_id from the workflow database. Use this when a compressed user-context reference points to an unavailable external file. Pass role=user explicitly. The result only contains message_id, role, message, and attached_context; large results are automatically reduced and saved to a temporary file by the workflow observation stage."
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::System
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
                    "message_id": {
                        "type": "integer",
                        "description": "The durable workflow message ID to retrieve."
                    },
                    "role": {
                        "type": "string",
                        "enum": ["user"],
                        "description": "The expected message role. Only user messages are available."
                    }
                },
                "required": ["message_id", "role"]
            }),
            output_schema: None,
            disabled: false,
            scope: Some(self.scope()),
        }
    }

    async fn call(&self, params: Value) -> NativeToolResult {
        let message_id = params
            .get("message_id")
            .and_then(Value::as_i64)
            .ok_or_else(|| ToolError::InvalidParams("message_id must be an integer".to_string()))?;
        if message_id <= 0 {
            return Err(ToolError::InvalidParams(
                "message_id must be positive".to_string(),
            ));
        }

        let role = params
            .get("role")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidParams("role is required".to_string()))?;
        if role != "user" {
            return Err(ToolError::InvalidParams(
                "role must be user; other workflow roles are not available through this tool"
                    .to_string(),
            ));
        }

        let message = self
            .main_store
            .get_workflow_message_by_id_and_role(&self.session_id, message_id, role)
            .map_err(|error| {
                ToolError::ExecutionFailed(format!("Failed to read workflow message: {error}"))
            })?
            .ok_or_else(|| {
                ToolError::ExecutionFailed(format!(
                    "User workflow message {message_id} was not found in this session"
                ))
            })?;
        let payload = compact_user_message_payload(message);
        let serialized = serde_json::to_string_pretty(&payload)
            .map_err(|error| ToolError::Serialization(error.to_string()))?;
        Ok(ToolCallResult::success(Some(serialized), Some(payload)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_tool_is_workflow_scoped_and_user_only() {
        let tool = ReadHistoryMessage {
            session_id: "session".to_string(),
            main_store: Arc::new(MainStore::new(":memory:").expect("store")),
        };
        let spec = tool.tool_calling_spec();
        assert_eq!(tool.scope(), crate::tools::ToolScope::Workflow);
        assert_eq!(spec.input_schema["required"], json!(["message_id", "role"]));
        assert_eq!(
            spec.input_schema["properties"]["message_id"]["type"],
            "integer"
        );
        assert_eq!(
            spec.input_schema["properties"]["role"]["enum"],
            json!(["user"])
        );
    }

    #[test]
    fn compact_payload_omits_runtime_metadata() {
        let payload = compact_user_message_payload(crate::db::WorkflowMessage {
            id: Some(42),
            session_id: "session".to_string(),
            role: "user".to_string(),
            message: "request".to_string(),
            reasoning: Some("private reasoning".to_string()),
            message_kind: "message".to_string(),
            message_subtype: None,
            segment_id: 1,
            source_event_type: Some("runtime".to_string()),
            metadata: Some(json!({"large": "metadata"})),
            attached_context: Some("attachment".to_string()),
            step_type: Some("thinking".to_string()),
            step_index: 3,
            is_error: false,
            error_type: None,
            created_at: Some("now".to_string()),
        });
        let keys = payload
            .as_object()
            .expect("payload object")
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            keys,
            ["attached_context", "message", "message_id", "role"]
                .into_iter()
                .map(str::to_string)
                .collect()
        );
    }

    #[tokio::test]
    async fn history_tool_rejects_non_user_role_before_database_lookup() {
        let tool = ReadHistoryMessage {
            session_id: "session".to_string(),
            main_store: Arc::new(MainStore::new(":memory:").expect("store")),
        };
        let error = tool
            .call(json!({"message_id": 1, "role": "tool"}))
            .await
            .expect_err("tool role must be rejected");
        assert!(matches!(error, ToolError::InvalidParams(_)));
    }
}
