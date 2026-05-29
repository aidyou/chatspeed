use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Display, EnumString)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum WorkflowState {
    Pending,
    Thinking,
    Executing,
    Auditing,
    Stopping,
    Paused,
    AwaitingUser,
    AwaitingApproval,
    AwaitingAutoApproval,
    AwaitingSubAgent,
    Completed,
    Error,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Display, EnumString)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum StepType {
    Think,
    Act,
    Observe,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GatewayPayload {
    /// Incremental chunk of text (for streaming content)
    Chunk {
        content: String,
    },
    /// Incremental chunk of reasoning text
    ReasoningChunk {
        content: String,
    },
    /// Full message update
    Message {
        role: String,
        content: String,
        reasoning: Option<String>,
        step_type: Option<StepType>,
        step_index: i32,
        is_error: bool,
        error_type: Option<String>,
        metadata: Option<serde_json::Value>,
    },
    State {
        // Kept for compatibility with existing UI consumers.
        // Newer consumers should prefer `wait_reason` for interaction decisions.
        state: WorkflowState,
        #[serde(skip_serializing_if = "Option::is_none")]
        wait_reason: Option<WaitReason>,
    },
    Confirm {
        id: String,
        action: String,
        details: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        display_type: Option<String>,
    },
    ApprovalResolved {
        tool_call_id: String,
        approved: bool,
        approve_all: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        approval_status: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        execution_status: Option<String>,
    },
    QueuedUserMessageRemoved {
        queued_user_message_id: String,
    },
    ToolStarted {
        tool_call_id: String,
        tool_name: String,
        arguments: serde_json::Value,
    },
    SyncTodo {
        todo_list: serde_json::Value,
    },
    /// Status message for retry logic (e.g., 429 exponential backoff)
    RetryStatus {
        attempt: u32,
        total_attempts: u32,
        next_retry_in_seconds: u32,
    },
    /// Context compression status notification
    CompressionStatus {
        is_compressing: bool,
        message: String,
    },
    /// Compression summary has been persisted and the message projection changed.
    CompressionApplied {
        compressed_until_message_id: i64,
    },
    /// Current runtime context token estimate after compaction/rebuild.
    ContextUsage {
        total_tokens: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        current_context_tokens: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_context_tokens: Option<usize>,
    },
    /// Lightweight projection of a sub-agent for the parent session UI.
    #[serde(rename = "sub_agent_progress")]
    SubAgentProgress {
        sub_agent_id: String,
        parent_session_id: String,
        status: RuntimeState,
        workflow_state: WorkflowState,
        #[serde(skip_serializing_if = "Option::is_none")]
        wait_reason: Option<WaitReason>,
        #[serde(skip_serializing_if = "Option::is_none")]
        agent_name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        task: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        tool_calls_count: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        current_context_tokens: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_context_tokens: Option<usize>,
        is_error: bool,
        updated_at_ms: i64,
    },
    /// Generic notification message for the UI status bar
    Notification {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        category: Option<String>, // e.g., "info", "warning", "error", "fun"
    },
    /// Auto-approved tools list updated
    AutoApprovedToolsUpdated {
        tools: Vec<String>,
    },
    /// Workflow agent configuration changed at runtime.
    AgentConfigUpdated {
        agent_config: serde_json::Value,
    },
    /// Workflow title updated asynchronously in the background.
    WorkflowTitleUpdated {
        title: String,
    },
    /// Shell policy updated
    ShellPolicyUpdated {
        policy: Vec<crate::tools::ShellPolicyRule>,
    },
    /// Tool streaming output
    ToolStream {
        tool_id: String,
        output: String,
        timestamp: u64,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Display, EnumString)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum RuntimeState {
    Pending,
    Running,
    Stopping,
    Waiting,
    Completed,
    Failed,
    Cancelled,
}

impl From<&WorkflowState> for RuntimeState {
    fn from(state: &WorkflowState) -> Self {
        match state {
            WorkflowState::Pending => RuntimeState::Pending,
            WorkflowState::Thinking | WorkflowState::Executing | WorkflowState::Auditing => {
                RuntimeState::Running
            }
            WorkflowState::Stopping => RuntimeState::Stopping,
            WorkflowState::Paused
            | WorkflowState::AwaitingUser
            | WorkflowState::AwaitingApproval
            | WorkflowState::AwaitingAutoApproval
            | WorkflowState::AwaitingSubAgent => RuntimeState::Waiting,
            WorkflowState::Completed => RuntimeState::Completed,
            WorkflowState::Error => RuntimeState::Failed,
            WorkflowState::Cancelled => RuntimeState::Cancelled,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Display, EnumString)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum WaitReason {
    Confirmation,
    UserInput,
    Approval,
    SubAgent,
}

/// Structured signal types for workflow control.
/// Signals are parsed from JSON strings sent by the frontend.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkflowSignal {
    /// User provides text input (for AwaitingUser state)
    UserMessage {
        content: String,
        #[serde(default)]
        attached_context: Option<String>,
        #[serde(default)]
        metadata: Option<serde_json::Value>,
        #[serde(default, alias = "queuedUserMessageId")]
        queued_user_message_id: Option<String>,
    },
    RemoveQueuedUserMessage {
        queued_user_message_id: String,
    },
    /// User approves or rejects a tool call (for AwaitingApproval state)
    /// Frontend legacy format uses "approval" as type and "id" as field.
    #[serde(rename = "approval")]
    ApprovalDecision {
        #[serde(rename = "id")]
        tool_call_id: String,
        approved: bool,
        #[serde(default)]
        approve_all: bool,
        #[serde(default)]
        rejection_message: Option<String>,
    },
    /// Resume execution (for Paused state)
    Continue,
    /// Cancel workflow execution (allowed in all waiting states)
    Stop,
    /// Re-broadcast pending confirmations to frontend
    RebroadcastPending,
    /// Update runtime final audit configuration
    UpdateFinalAudit {
        #[serde(alias = "finalAudit", alias = "audit")]
        final_audit: bool,
    },
    /// Update runtime task-boundary rollup compression configuration
    UpdateAutoCompress {
        #[serde(alias = "autoCompress", alias = "enabled")]
        auto_compress: bool,
    },
    /// Update runtime approval level configuration
    UpdateApprovalLevel {
        #[serde(alias = "approvalLevel", alias = "level")]
        approval_level: String,
    },
    /// Update runtime execution phase configuration
    UpdatePhase {
        phase: String,
    },
    /// Update runtime allowed paths configuration
    UpdateAllowedPaths {
        paths: Vec<String>,
    },
    /// Update runtime model configuration
    UpdateModelConfig {
        configs: serde_json::Value,
    },
    /// Update runtime skills configuration
    UpdateSkillsConfig {
        #[serde(alias = "skillEnabled")]
        skill_enabled: bool,
        #[serde(alias = "selectedSkills", default)]
        selected_skills: Vec<String>,
    },
    /// Remove a tool from auto-approve list
    RemoveAutoApprovedTool {
        tool_name: String,
    },
    /// Remove a pattern from shell policy
    RemoveShellPolicyItem {
        pattern: String,
    },
    /// Sub-agent completed (for SubAgent waiting state)
    #[serde(rename = "sub_agent_complete")]
    SubAgentComplete {
        sub_agent_id: String,
        result: serde_json::Value,
    },
    /// Background context compression completed and is ready to be persisted.
    CompressionReady {
        compressed_until_message_id: i64,
        summary: String,
    },
    CompressionFailed {
        compressed_until_message_id: i64,
        error: String,
    },
}

impl WorkflowSignal {
    /// Parse a JSON string into a WorkflowSignal.
    /// Returns None if parsing fails or the signal type is unknown.
    pub fn parse(json_str: &str) -> Option<Self> {
        serde_json::from_str(json_str).ok()
    }

    /// Returns true if this signal is valid for the given wait reason.
    /// Stop signal is always valid in any waiting state.
    pub fn is_valid_for(&self, wait_reason: Option<&WaitReason>) -> bool {
        match (self, wait_reason) {
            // Stop is always valid
            (WorkflowSignal::Stop, _) => true,
            // Runtime config/control signals are valid regardless of current waiting reason
            (WorkflowSignal::RebroadcastPending, _) => true,
            (WorkflowSignal::UpdateFinalAudit { .. }, _) => true,
            (WorkflowSignal::UpdateAutoCompress { .. }, _) => true,
            (WorkflowSignal::UpdateApprovalLevel { .. }, _) => true,
            (WorkflowSignal::UpdatePhase { .. }, _) => true,
            (WorkflowSignal::UpdateAllowedPaths { .. }, _) => true,
            (WorkflowSignal::UpdateModelConfig { .. }, _) => true,
            (WorkflowSignal::UpdateSkillsConfig { .. }, _) => true,
            (WorkflowSignal::RemoveAutoApprovedTool { .. }, _) => true,
            (WorkflowSignal::RemoveShellPolicyItem { .. }, _) => true,
            (WorkflowSignal::RemoveQueuedUserMessage { .. }, _) => true,
            (WorkflowSignal::CompressionReady { .. }, _) => true,
            (WorkflowSignal::CompressionFailed { .. }, _) => true,
            // UserMessage is valid for UserInput waiting
            (WorkflowSignal::UserMessage { .. }, Some(WaitReason::UserInput)) => true,
            // ApprovalDecision is valid for Approval waiting
            (WorkflowSignal::ApprovalDecision { .. }, Some(WaitReason::Approval)) => true,
            // Continue is valid for Confirmation waiting
            (WorkflowSignal::Continue, Some(WaitReason::Confirmation)) => true,
            // SubAgentComplete is valid for SubAgent waiting
            (WorkflowSignal::SubAgentComplete { .. }, Some(WaitReason::SubAgent)) => true,
            // Everything else is invalid
            _ => false,
        }
    }

    /// Returns the signal type name for logging purposes.
    pub fn type_name(&self) -> &'static str {
        match self {
            WorkflowSignal::UserMessage { .. } => "user_message",
            WorkflowSignal::RemoveQueuedUserMessage { .. } => "remove_queued_user_message",
            WorkflowSignal::ApprovalDecision { .. } => "approval_decision",
            WorkflowSignal::Continue => "continue",
            WorkflowSignal::Stop => "stop",
            WorkflowSignal::RebroadcastPending => "rebroadcast_pending",
            WorkflowSignal::UpdateFinalAudit { .. } => "update_final_audit",
            WorkflowSignal::UpdateAutoCompress { .. } => "update_auto_compress",
            WorkflowSignal::UpdateApprovalLevel { .. } => "update_approval_level",
            WorkflowSignal::UpdatePhase { .. } => "update_phase",
            WorkflowSignal::UpdateAllowedPaths { .. } => "update_allowed_paths",
            WorkflowSignal::UpdateModelConfig { .. } => "update_model_config",
            WorkflowSignal::UpdateSkillsConfig { .. } => "update_skills_config",
            WorkflowSignal::RemoveAutoApprovedTool { .. } => "remove_auto_approved_tool",
            WorkflowSignal::RemoveShellPolicyItem { .. } => "remove_shell_policy_item",
            WorkflowSignal::SubAgentComplete { .. } => "sub_agent_complete",
            WorkflowSignal::CompressionReady { .. } => "compression_ready",
            WorkflowSignal::CompressionFailed { .. } => "compression_failed",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PendingTool {
    pub tool_call_id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub details: Option<serde_json::Value>,
    #[serde(default)]
    pub display_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubAgentCompletion {
    pub sub_agent_id: String,
    pub parent_session_id: String,
    pub status: String,
    #[serde(default)]
    pub result: Option<String>,
    pub summary: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub tool_calls_count: usize,
    pub completed_at_ms: i64,
    #[serde(default)]
    pub consumed: bool,
}

impl SubAgentCompletion {
    pub fn to_signal_result(&self) -> serde_json::Value {
        let mut result = serde_json::json!({
            "status": self.status,
            "task_id": self.sub_agent_id,
            "tool_calls_count": self.tool_calls_count,
        });
        if let Some(content) = &self.result {
            result["result"] = serde_json::json!(content);
        }
        if let Some(summary) = &self.summary {
            result["summary"] = serde_json::json!(summary);
        }
        if let Some(error) = &self.error {
            result["error"] = serde_json::json!(error);
        }
        result
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutionContext {
    pub session_id: String,
    pub state: RuntimeState,
    pub wait_reason: Option<WaitReason>,
    #[serde(default = "default_execution_context_segment_id")]
    pub current_segment_id: i32,
    pub current_step: usize,
    pub max_steps: usize,
    pub pending_tools: Vec<PendingTool>,
    pub last_action_summary: Option<String>,
    #[serde(default)]
    pub current_context_tokens: Option<usize>,
    #[serde(default)]
    pub max_context_tokens: Option<usize>,
    #[serde(default)]
    pub last_event_id: Option<i64>,
    pub version: String,
    #[serde(default)]
    pub waiting_on_sub_agent_id: Option<String>,
    #[serde(default)]
    pub sub_agent_sessions: Vec<String>,
    #[serde(default)]
    pub pending_sub_agent_completions: Vec<SubAgentCompletion>,
}

fn default_execution_context_segment_id() -> i32 {
    1
}

impl ExecutionContext {
    pub const CURRENT_VERSION: &'static str = "1.3.0";

    #[cfg(test)]
    pub fn new(session_id: String) -> Self {
        Self {
            session_id,
            state: RuntimeState::Pending,
            wait_reason: None,
            current_segment_id: default_execution_context_segment_id(),
            current_step: 0,
            max_steps: 100,
            pending_tools: Vec::new(),
            last_action_summary: None,
            current_context_tokens: None,
            max_context_tokens: None,
            last_event_id: None,
            version: Self::CURRENT_VERSION.to_string(),
            waiting_on_sub_agent_id: None,
            sub_agent_sessions: Vec::new(),
            pending_sub_agent_completions: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execution_context_new() {
        let ctx = ExecutionContext::new("test-session".to_string());
        assert_eq!(ctx.session_id, "test-session");
        assert_eq!(ctx.state, RuntimeState::Pending);
        assert!(ctx.wait_reason.is_none());
        assert_eq!(ctx.current_segment_id, 1);
        assert!(ctx.pending_tools.is_empty());
        assert!(ctx.last_event_id.is_none());
        assert_eq!(ctx.version, "1.3.0");
        assert!(ctx.waiting_on_sub_agent_id.is_none());
        assert!(ctx.sub_agent_sessions.is_empty());
        assert!(ctx.pending_sub_agent_completions.is_empty());
    }

    #[test]
    fn test_execution_context_serialization_roundtrip() {
        let mut ctx = ExecutionContext::new("test-session".to_string());
        ctx.state = RuntimeState::Waiting;
        ctx.wait_reason = Some(WaitReason::Approval);
        ctx.current_segment_id = 3;
        ctx.current_step = 5;
        ctx.max_steps = 100;
        ctx.pending_tools.push(PendingTool {
            tool_call_id: "call_123".to_string(),
            tool_name: "bash".to_string(),
            arguments: serde_json::json!({"command": "ls"}),
            details: Some(serde_json::json!("List files")),
            display_type: Some("text".to_string()),
        });

        let json = serde_json::to_string(&ctx).unwrap();
        let deserialized: ExecutionContext = serde_json::from_str(&json).unwrap();

        assert_eq!(ctx, deserialized);
    }

    #[test]
    fn test_pending_tool_roundtrip() {
        let tool = PendingTool {
            tool_call_id: "call_abc".to_string(),
            tool_name: "write_file".to_string(),
            arguments: serde_json::json!({"path": "/tmp/test.txt", "content": "hello"}),
            details: Some(serde_json::json!("Write test file")),
            display_type: Some("diff".to_string()),
        };

        let json = serde_json::to_string(&tool).unwrap();
        let deserialized: PendingTool = serde_json::from_str(&json).unwrap();

        assert_eq!(tool, deserialized);
    }

    #[test]
    fn test_runtime_state_from_workflow_state() {
        assert_eq!(
            RuntimeState::from(&WorkflowState::Pending),
            RuntimeState::Pending
        );
        assert_eq!(
            RuntimeState::from(&WorkflowState::Thinking),
            RuntimeState::Running
        );
        assert_eq!(
            RuntimeState::from(&WorkflowState::Executing),
            RuntimeState::Running
        );
        assert_eq!(
            RuntimeState::from(&WorkflowState::Paused),
            RuntimeState::Waiting
        );
        assert_eq!(
            RuntimeState::from(&WorkflowState::AwaitingUser),
            RuntimeState::Waiting
        );
        assert_eq!(
            RuntimeState::from(&WorkflowState::AwaitingApproval),
            RuntimeState::Waiting
        );
        assert_eq!(
            RuntimeState::from(&WorkflowState::Completed),
            RuntimeState::Completed
        );
        assert_eq!(
            RuntimeState::from(&WorkflowState::Error),
            RuntimeState::Failed
        );
        assert_eq!(
            RuntimeState::from(&WorkflowState::Cancelled),
            RuntimeState::Cancelled
        );
    }

    #[test]
    fn test_execution_context_with_multiple_pending_tools() {
        let mut ctx = ExecutionContext::new("multi-tool-session".to_string());
        ctx.state = RuntimeState::Waiting;
        ctx.wait_reason = Some(WaitReason::Approval);

        for i in 0..3 {
            ctx.pending_tools.push(PendingTool {
                tool_call_id: format!("call_{}", i),
                tool_name: format!("tool_{}", i),
                arguments: serde_json::json!({"arg": i}),
                details: Some(serde_json::json!(format!("Details for tool {}", i))),
                display_type: Some("text".to_string()),
            });
        }

        let json = serde_json::to_string(&ctx).unwrap();
        let deserialized: ExecutionContext = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.pending_tools.len(), 3);
        assert_eq!(deserialized.pending_tools[0].tool_call_id, "call_0");
        assert_eq!(deserialized.pending_tools[2].tool_name, "tool_2");
    }

    #[test]
    fn test_workflow_signal_parse() {
        let json = r#"{"type":"user_message","content":"hello"}"#;
        let signal = WorkflowSignal::parse(json).unwrap();
        assert!(
            matches!(signal, WorkflowSignal::UserMessage { content, .. } if content == "hello")
        );

        // Legacy frontend format: type="approval", field="id"
        let json = r#"{"type":"approval","id":"call_123","approved":true,"approve_all":false}"#;
        let signal = WorkflowSignal::parse(json).unwrap();
        assert!(
            matches!(signal, WorkflowSignal::ApprovalDecision { tool_call_id, approved, approve_all, .. }
            if tool_call_id == "call_123" && approved && !approve_all)
        );

        let json = r#"{"type":"stop"}"#;
        let signal = WorkflowSignal::parse(json).unwrap();
        assert!(matches!(signal, WorkflowSignal::Stop));

        let json = r#"{"type":"remove_queued_user_message","queued_user_message_id":"queue_1"}"#;
        let signal = WorkflowSignal::parse(json).unwrap();
        assert!(matches!(
            signal,
            WorkflowSignal::RemoveQueuedUserMessage {
                queued_user_message_id
            } if queued_user_message_id == "queue_1"
        ));
    }

    #[test]
    fn test_workflow_signal_validation() {
        // Stop is valid in all waiting states
        let stop = WorkflowSignal::Stop;
        assert!(stop.is_valid_for(None));
        assert!(stop.is_valid_for(Some(&WaitReason::UserInput)));
        assert!(stop.is_valid_for(Some(&WaitReason::Approval)));
        assert!(stop.is_valid_for(Some(&WaitReason::Confirmation)));
        assert!(stop.is_valid_for(Some(&WaitReason::SubAgent)));

        // UserMessage is only valid for UserInput waiting
        let user_msg = WorkflowSignal::UserMessage {
            content: "test".to_string(),
            attached_context: None,
            metadata: None,
            queued_user_message_id: None,
        };
        assert!(user_msg.is_valid_for(Some(&WaitReason::UserInput)));
        assert!(!user_msg.is_valid_for(Some(&WaitReason::Confirmation)));
        assert!(!user_msg.is_valid_for(Some(&WaitReason::Approval)));

        // ApprovalDecision is only valid for Approval waiting
        let approval = WorkflowSignal::ApprovalDecision {
            tool_call_id: "call_1".to_string(),
            approved: true,
            approve_all: false,
            rejection_message: None,
        };
        assert!(approval.is_valid_for(Some(&WaitReason::Approval)));
        assert!(!approval.is_valid_for(Some(&WaitReason::UserInput)));
        assert!(!approval.is_valid_for(Some(&WaitReason::Confirmation)));

        // Continue is only valid for Confirmation waiting
        let cont = WorkflowSignal::Continue;
        assert!(cont.is_valid_for(Some(&WaitReason::Confirmation)));
        assert!(!cont.is_valid_for(Some(&WaitReason::UserInput)));
        assert!(!cont.is_valid_for(Some(&WaitReason::Approval)));

        let child_complete = WorkflowSignal::SubAgentComplete {
            sub_agent_id: "subagent_1".to_string(),
            result: serde_json::json!({"status": "completed"}),
        };
        assert!(child_complete.is_valid_for(Some(&WaitReason::SubAgent)));
        assert!(!child_complete.is_valid_for(Some(&WaitReason::Approval)));

        let update_paths = WorkflowSignal::UpdateAllowedPaths {
            paths: vec!["/tmp/project".to_string()],
        };
        assert!(update_paths.is_valid_for(Some(&WaitReason::UserInput)));
        assert!(update_paths.is_valid_for(Some(&WaitReason::Approval)));

        let update_models = WorkflowSignal::UpdateModelConfig {
            configs: serde_json::json!({"act": "model-a"}),
        };
        assert!(update_models.is_valid_for(Some(&WaitReason::Confirmation)));
        assert!(update_models.is_valid_for(None));

        let remove_queued = WorkflowSignal::RemoveQueuedUserMessage {
            queued_user_message_id: "queue_1".to_string(),
        };
        assert!(remove_queued.is_valid_for(Some(&WaitReason::Approval)));
        assert!(remove_queued.is_valid_for(None));
    }
}
