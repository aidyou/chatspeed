use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const EVENT_VERSION: &str = "1.0.0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowEventType {
    WorkflowStarted,
    StateChanged,
    WaitEntered,
    UserInputReceived,
    ApprovalRequested,
    ApprovalResolved,
    ToolStarted,
    ToolCompleted,
    ToolFailed,
    SubAgentStarted,
    SubAgentCompleted,
    SubAgentFailed,
    SubAgentInterrupted,
    WorkflowCompleted,
    WorkflowFailed,
    WorkflowCancelled,
}

impl WorkflowEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkflowEventType::WorkflowStarted => "workflow_started",
            WorkflowEventType::StateChanged => "state_changed",
            WorkflowEventType::WaitEntered => "wait_entered",
            WorkflowEventType::UserInputReceived => "user_input_received",
            WorkflowEventType::ApprovalRequested => "approval_requested",
            WorkflowEventType::ApprovalResolved => "approval_resolved",
            WorkflowEventType::ToolStarted => "tool_started",
            WorkflowEventType::ToolCompleted => "tool_completed",
            WorkflowEventType::ToolFailed => "tool_failed",
            WorkflowEventType::SubAgentStarted => "sub_agent_started",
            WorkflowEventType::SubAgentCompleted => "sub_agent_completed",
            WorkflowEventType::SubAgentFailed => "sub_agent_failed",
            WorkflowEventType::SubAgentInterrupted => "sub_agent_interrupted",
            WorkflowEventType::WorkflowCompleted => "workflow_completed",
            WorkflowEventType::WorkflowFailed => "workflow_failed",
            WorkflowEventType::WorkflowCancelled => "workflow_cancelled",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowEvent {
    pub event_type: WorkflowEventType,
    pub session_id: String,
    pub event_data: Value,
    pub version: String,
}

impl WorkflowEvent {
    pub fn new(event_type: WorkflowEventType, session_id: String, event_data: Value) -> Self {
        Self {
            event_type,
            session_id,
            event_data,
            version: EVENT_VERSION.to_string(),
        }
    }

    pub fn workflow_started(session_id: String, agent_id: String) -> Self {
        Self::new(
            WorkflowEventType::WorkflowStarted,
            session_id,
            serde_json::json!({ "agent_id": agent_id }),
        )
    }

    pub fn state_changed(session_id: String, from_state: String, to_state: String) -> Self {
        Self::new(
            WorkflowEventType::StateChanged,
            session_id,
            serde_json::json!({ "from_state": from_state, "to_state": to_state }),
        )
    }

    pub fn wait_entered(
        session_id: String,
        wait_reason: String,
        pending_tools: Vec<Value>,
    ) -> Self {
        Self::new(
            WorkflowEventType::WaitEntered,
            session_id,
            serde_json::json!({ "wait_reason": wait_reason, "pending_tools": pending_tools }),
        )
    }

    pub fn user_input_received(session_id: String, content: String) -> Self {
        Self::new(
            WorkflowEventType::UserInputReceived,
            session_id,
            serde_json::json!({ "content": content }),
        )
    }

    pub fn approval_requested(
        session_id: String,
        tool_call_id: String,
        tool_name: String,
        arguments: Value,
        details: Option<Value>,
        display_type: Option<String>,
    ) -> Self {
        Self::new(
            WorkflowEventType::ApprovalRequested,
            session_id,
            serde_json::json!({
                "tool_call_id": tool_call_id,
                "tool_name": tool_name,
                "arguments": arguments,
                "details": details,
                "display_type": display_type
            }),
        )
    }

    pub fn approval_resolved(
        session_id: String,
        tool_call_id: String,
        approved: bool,
        approve_all: bool,
    ) -> Self {
        Self::new(
            WorkflowEventType::ApprovalResolved,
            session_id,
            serde_json::json!({
                "tool_call_id": tool_call_id,
                "approved": approved,
                "approve_all": approve_all
            }),
        )
    }

    pub fn tool_started(
        session_id: String,
        tool_call_id: String,
        tool_name: String,
        arguments: Value,
    ) -> Self {
        Self::new(
            WorkflowEventType::ToolStarted,
            session_id,
            serde_json::json!({
                "tool_call_id": tool_call_id,
                "tool_name": tool_name,
                "arguments": arguments
            }),
        )
    }

    pub fn tool_completed(
        session_id: String,
        tool_call_id: String,
        tool_name: String,
        result: Option<Value>,
    ) -> Self {
        Self::new(
            WorkflowEventType::ToolCompleted,
            session_id,
            serde_json::json!({
                "tool_call_id": tool_call_id,
                "tool_name": tool_name,
                "result": result
            }),
        )
    }

    pub fn tool_failed(
        session_id: String,
        tool_call_id: String,
        tool_name: String,
        error: String,
    ) -> Self {
        Self::new(
            WorkflowEventType::ToolFailed,
            session_id,
            serde_json::json!({
                "tool_call_id": tool_call_id,
                "tool_name": tool_name,
                "error": error
            }),
        )
    }

    pub fn workflow_completed(session_id: String, summary: Option<String>) -> Self {
        Self::new(
            WorkflowEventType::WorkflowCompleted,
            session_id,
            serde_json::json!({ "summary": summary }),
        )
    }

    pub fn sub_agent_started(
        session_id: String,
        sub_agent_id: String,
        execution_mode: String,
    ) -> Self {
        Self::new(
            WorkflowEventType::SubAgentStarted,
            session_id,
            serde_json::json!({
                "sub_agent_id": sub_agent_id,
                "execution_mode": execution_mode
            }),
        )
    }

    pub fn sub_agent_completed(
        session_id: String,
        sub_agent_id: String,
        status: String,
        result: Value,
    ) -> Self {
        let event_type = match status.as_str() {
            "failed" | "cancelled" => WorkflowEventType::SubAgentFailed,
            "interrupted" => WorkflowEventType::SubAgentInterrupted,
            _ => WorkflowEventType::SubAgentCompleted,
        };
        Self::new(
            event_type,
            session_id,
            serde_json::json!({
                "sub_agent_id": sub_agent_id,
                "status": status,
                "result": result
            }),
        )
    }

    pub fn sub_agent_interrupted(session_id: String, sub_agent_id: String, reason: String) -> Self {
        Self::new(
            WorkflowEventType::SubAgentInterrupted,
            session_id,
            serde_json::json!({
                "sub_agent_id": sub_agent_id,
                "status": "interrupted",
                "reason": reason
            }),
        )
    }

    pub fn workflow_failed(session_id: String, error: String) -> Self {
        Self::new(
            WorkflowEventType::WorkflowFailed,
            session_id,
            serde_json::json!({ "error": error }),
        )
    }

    pub fn workflow_cancelled(session_id: String) -> Self {
        Self::new(
            WorkflowEventType::WorkflowCancelled,
            session_id,
            serde_json::json!({}),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowEventRecord {
    pub id: i64,
    pub session_id: String,
    pub event_type: String,
    pub event_version: String,
    pub event_data: Value,
    pub created_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_event_creation() {
        let event =
            WorkflowEvent::workflow_started("test-session".to_string(), "agent-001".to_string());
        assert_eq!(event.event_type, WorkflowEventType::WorkflowStarted);
        assert_eq!(event.session_id, "test-session");
        assert_eq!(event.version, EVENT_VERSION);
        assert_eq!(event.event_data["agent_id"], "agent-001");
    }

    #[test]
    fn test_state_changed_event() {
        let event = WorkflowEvent::state_changed(
            "test-session".to_string(),
            "thinking".to_string(),
            "awaiting_approval".to_string(),
        );
        assert_eq!(event.event_type, WorkflowEventType::StateChanged);
        assert_eq!(event.event_data["from_state"], "thinking");
        assert_eq!(event.event_data["to_state"], "awaiting_approval");
    }

    #[test]
    fn test_approval_events() {
        let requested = WorkflowEvent::approval_requested(
            "test-session".to_string(),
            "call_123".to_string(),
            "bash".to_string(),
            serde_json::json!({"command": "ls"}),
            Some(serde_json::json!("List files")),
            Some("text".to_string()),
        );
        assert_eq!(requested.event_type, WorkflowEventType::ApprovalRequested);
        assert_eq!(requested.event_data["tool_call_id"], "call_123");

        let resolved = WorkflowEvent::approval_resolved(
            "test-session".to_string(),
            "call_123".to_string(),
            true,
            false,
        );
        assert_eq!(resolved.event_type, WorkflowEventType::ApprovalResolved);
        assert_eq!(resolved.event_data["approved"], true);
    }

    #[test]
    fn test_event_serialization_roundtrip() {
        let event = WorkflowEvent::wait_entered(
            "test-session".to_string(),
            "approval".to_string(),
            vec![serde_json::json!({"tool_call_id": "call_1", "tool_name": "bash"})],
        );

        let json = serde_json::to_string(&event).unwrap();
        let deserialized: WorkflowEvent = serde_json::from_str(&json).unwrap();

        assert_eq!(event.event_type, deserialized.event_type);
        assert_eq!(event.session_id, deserialized.session_id);
        assert_eq!(event.event_data, deserialized.event_data);
    }
}
