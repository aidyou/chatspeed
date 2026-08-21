use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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
        message_id: Option<String>,
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
        tool_name: String,
        arguments: serde_json::Value,
        details: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        display_type: Option<String>,
    },
    ApprovalResolved {
        tool_call_id: String,
        tool_name: String,
        approved: bool,
        approve_all: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        approval_status: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        execution_status: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        rejection_message: Option<String>,
    },
    QueuedUserMessageRemoved {
        queued_user_message_id: String,
    },
    ToolStarted {
        tool_call_id: String,
        tool_name: String,
        arguments: serde_json::Value,
    },
    ToolCompleted {
        tool_call_id: String,
        tool_name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<serde_json::Value>,
    },
    ToolFailed {
        tool_call_id: String,
        tool_name: String,
        error: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        error_type: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error_details: Option<serde_json::Value>,
    },
    /// A top-level workflow task passed all completion checks and any configured final review.
    TaskCompleted {
        tool_call_id: String,
        segment_id: i32,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage_summary: Option<crate::workflow::react::usage::WorkflowUsageSummary>,
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
        current_context_tokens: usize,
        max_context_tokens: usize,
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
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<serde_json::Value>,
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
    /// Run pressure compression immediately using the current context projection.
    ManualCompress,
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
    /// Update the resolved sandbox configuration for Shell execution.
    UpdateSandboxConfig {
        #[serde(default, alias = "sandboxConfig")]
        sandbox_config: Option<crate::tools::AgentSandboxConfig>,
        #[serde(alias = "executionMode")]
        execution_mode: crate::tools::ShellExecutionMode,
        #[serde(default, alias = "sandboxSchemeId")]
        sandbox_scheme_id: Option<String>,
    },
    /// Update the primary agent's execution and communication style at runtime.
    UpdatePersonality {
        #[serde(default)]
        personality: Option<String>,
    },
    /// Replace the user-selected visible tool set at runtime.
    UpdateAvailableTools {
        #[serde(alias = "tools", default)]
        available_tools: Vec<String>,
    },
    /// Replace auto-approve tool list at runtime
    UpdateAutoApprovedTools {
        #[serde(alias = "tools", default)]
        auto_approve: Vec<String>,
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
            (WorkflowSignal::ManualCompress, _) => true,
            (WorkflowSignal::UpdateApprovalLevel { .. }, _) => true,
            (WorkflowSignal::UpdatePhase { .. }, _) => true,
            (WorkflowSignal::UpdateAllowedPaths { .. }, _) => true,
            (WorkflowSignal::UpdateModelConfig { .. }, _) => true,
            (WorkflowSignal::UpdateSkillsConfig { .. }, _) => true,
            (WorkflowSignal::UpdateSandboxConfig { .. }, _) => true,
            (WorkflowSignal::UpdatePersonality { .. }, _) => true,
            (WorkflowSignal::UpdateAvailableTools { .. }, _) => true,
            (WorkflowSignal::UpdateAutoApprovedTools { .. }, _) => true,
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
            WorkflowSignal::ManualCompress => "manual_compress",
            WorkflowSignal::UpdateApprovalLevel { .. } => "update_approval_level",
            WorkflowSignal::UpdatePhase { .. } => "update_phase",
            WorkflowSignal::UpdateAllowedPaths { .. } => "update_allowed_paths",
            WorkflowSignal::UpdateModelConfig { .. } => "update_model_config",
            WorkflowSignal::UpdateSkillsConfig { .. } => "update_skills_config",
            WorkflowSignal::UpdateSandboxConfig { .. } => "update_sandbox_config",
            WorkflowSignal::UpdatePersonality { .. } => "update_personality",
            WorkflowSignal::UpdateAvailableTools { .. } => "update_available_tools",
            WorkflowSignal::UpdateAutoApprovedTools { .. } => "update_auto_approved_tools",
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
    #[serde(default)]
    pub usage_summary: Option<crate::workflow::react::usage::WorkflowUsageSummary>,
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
        if let Some(usage_summary) = &self.usage_summary {
            result["usage_summary"] = serde_json::json!(usage_summary);
        }
        result
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PendingFinalReview {
    pub sub_agent_id: String,
    pub completion_summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingCompletionReport {
    pub source_message_id: Option<i64>,
    pub content: String,
    pub content_hash: String,
    pub segment_id: i32,
    pub created_at_step: usize,
}

impl PendingCompletionReport {
    pub fn new(
        content: &str,
        source_message_id: Option<i64>,
        segment_id: i32,
        created_at_step: usize,
    ) -> Self {
        let content = content.trim().to_string();
        let content_hash = hex::encode(Sha256::digest(content.as_bytes()));
        Self {
            source_message_id,
            content,
            content_hash,
            segment_id,
            created_at_step,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueuedUserMessage {
    pub queued_user_message_id: String,
    pub content: String,
    #[serde(default)]
    pub attached_context: Option<String>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// The deterministic precedence used when several user directives belong to one task.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EffectiveTaskDirectivePrecedence {
    LaterDirectivesOverrideConflicts,
}

impl Default for EffectiveTaskDirectivePrecedence {
    fn default() -> Self {
        Self::LaterDirectivesOverrideConflicts
    }
}

/// One durable user instruction contributing to the current task objective.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EffectiveTaskDirective {
    pub source_message_id: i64,
    pub content: String,
}

/// Runtime-owned lifecycle state for the compact current-goal handoff.
///
/// The model may phrase `current_goal`, but the source IDs and completion
/// evidence are assigned by the runtime from durable workflow messages.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskGoalStatus {
    Active,
    Complete,
    None,
}

pub(crate) const MAX_USER_EXECUTION_REQUIREMENTS: usize = 16;
pub(crate) const MAX_USER_EXECUTION_REQUIREMENT_CHARS: usize = 1_000;
pub(crate) const MAX_USER_EXECUTION_REQUIREMENTS_CHARS: usize = 8_000;

/// User-provided execution prerequisites that may be required by later work.
/// The strings intentionally remain open-ended and close to the user's wording.
pub(crate) type UserExecutionRequirements = Vec<String>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskGoalState {
    #[serde(default = "default_task_goal_state_version")]
    pub version: u8,
    pub status: TaskGoalStatus,
    #[serde(default)]
    pub current_goal: Option<String>,
    #[serde(default)]
    pub source_message_ids: Vec<i64>,
    /// Runtime-projected bounded text of the latest effective user directive.
    /// It is a guard against a model reinterpreting an older directive as the
    /// current execution mode after a compression boundary.
    #[serde(default)]
    pub latest_directive: Option<TaskGoalSourcePreview>,
    #[serde(default)]
    pub completion_evidence_message_id: Option<i64>,
    /// User-provided execution prerequisites that later work may need verbatim.
    /// These remain plain strings because their shape is intentionally open-ended.
    #[serde(default)]
    pub user_execution_requirements: UserExecutionRequirements,
}

fn default_task_goal_state_version() -> u8 {
    1
}

/// Bounded source material supplied to a compression model and persisted only
/// as a hidden projection aid. Durable workflow messages remain authoritative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskGoalLedger {
    pub source_message_ids: Vec<i64>,
    pub source_previews: Vec<TaskGoalSourcePreview>,
    pub previous_state: Option<TaskGoalState>,
    pub user_execution_requirements: UserExecutionRequirements,
    pub requires_active_goal: bool,
    pub completion_evidence_message_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskGoalSourcePreview {
    pub source_message_id: i64,
    pub content: String,
}

/// Structured task authority persisted with runtime recovery state.
///
/// The transcript remains the durable record of the conversation. This structure records the
/// already-classified user directives that define the active task so compression and recovery do
/// not need to infer scope from a generated summary or a later LLM request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EffectiveTaskObjective {
    #[serde(default = "default_effective_task_objective_version")]
    pub version: u8,
    pub initial_source_message_id: i64,
    pub latest_source_message_id: i64,
    #[serde(default)]
    pub directive_precedence: EffectiveTaskDirectivePrecedence,
    #[serde(default)]
    pub directives: Vec<EffectiveTaskDirective>,
}

fn default_effective_task_objective_version() -> u8 {
    1
}

impl EffectiveTaskObjective {
    pub fn new(source_message_id: i64, content: String) -> Self {
        Self {
            version: default_effective_task_objective_version(),
            initial_source_message_id: source_message_id,
            latest_source_message_id: source_message_id,
            directive_precedence: EffectiveTaskDirectivePrecedence::default(),
            directives: vec![EffectiveTaskDirective {
                source_message_id,
                content,
            }],
        }
    }

    pub fn append_directive(&mut self, source_message_id: i64, content: String) {
        if let Some(existing) = self
            .directives
            .iter_mut()
            .find(|directive| directive.source_message_id == source_message_id)
        {
            existing.content = content;
        } else {
            self.directives.push(EffectiveTaskDirective {
                source_message_id,
                content,
            });
            self.directives
                .sort_by_key(|directive| directive.source_message_id);
        }
        self.latest_source_message_id = source_message_id;
    }

    #[cfg(test)]
    pub fn source_message_ids(&self) -> Vec<i64> {
        self.directives
            .iter()
            .map(|directive| directive.source_message_id)
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutionContext {
    pub session_id: String,
    pub state: RuntimeState,
    pub wait_reason: Option<WaitReason>,
    #[serde(default)]
    pub queued_user_messages: Vec<QueuedUserMessage>,
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
    /// Canonical `ask_user` call currently awaiting a user response.
    #[serde(default)]
    pub awaiting_user_tool_call_id: Option<String>,
    /// Structured current-task authority used to rebuild compression handoffs.
    #[serde(default)]
    pub effective_task_objective: Option<EffectiveTaskObjective>,
    #[serde(default)]
    pub sub_agent_sessions: Vec<String>,
    #[serde(default)]
    pub pending_sub_agent_completions: Vec<SubAgentCompletion>,
    #[serde(default)]
    pub pending_final_review: Option<PendingFinalReview>,
    #[serde(default)]
    pub pending_completion_reports: Vec<PendingCompletionReport>,
    #[serde(default)]
    pub removed_queued_user_message_ids: Vec<String>,
}

fn default_execution_context_segment_id() -> i32 {
    1
}

impl ExecutionContext {
    pub const CURRENT_VERSION: &'static str = "1.4.0";

    /// Resets task-local runtime state while preserving the durable session and
    /// the current manual-clear segment boundary.
    pub fn reset_for_new_context(&mut self) {
        self.state = RuntimeState::Pending;
        self.wait_reason = None;
        self.queued_user_messages.clear();
        self.current_step = 0;
        self.max_steps = 0;
        self.pending_tools.clear();
        self.last_action_summary = None;
        self.waiting_on_sub_agent_id = None;
        self.awaiting_user_tool_call_id = None;
        self.effective_task_objective = None;
        self.sub_agent_sessions.clear();
        self.pending_sub_agent_completions.clear();
        self.pending_final_review = None;
        self.pending_completion_reports.clear();
        self.removed_queued_user_message_ids.clear();
        self.version = Self::CURRENT_VERSION.to_string();
    }

    #[cfg(test)]
    pub fn new(session_id: String) -> Self {
        Self {
            session_id,
            state: RuntimeState::Pending,
            wait_reason: None,
            queued_user_messages: Vec::new(),
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
            awaiting_user_tool_call_id: None,
            effective_task_objective: None,
            sub_agent_sessions: Vec::new(),
            pending_sub_agent_completions: Vec::new(),
            pending_final_review: None,
            pending_completion_reports: Vec::new(),
            removed_queued_user_message_ids: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_for_new_context_clears_all_task_local_state() {
        let mut context = ExecutionContext::new("session-1".to_string());
        context.state = RuntimeState::Completed;
        context.wait_reason = Some(WaitReason::SubAgent);
        context.queued_user_messages.push(QueuedUserMessage {
            queued_user_message_id: "queued-1".to_string(),
            content: "queued".to_string(),
            attached_context: None,
            metadata: None,
        });
        context.current_segment_id = 7;
        context.current_step = 9;
        context.max_steps = 50;
        context.pending_tools.push(PendingTool {
            tool_call_id: "tool-1".to_string(),
            tool_name: "read_file".to_string(),
            arguments: serde_json::json!({}),
            details: None,
            display_type: None,
        });
        context.last_action_summary = Some("old task".to_string());
        context.current_context_tokens = Some(42);
        context.max_context_tokens = Some(8192);
        context.last_event_id = Some(99);
        context.waiting_on_sub_agent_id = Some("subagent-1".to_string());
        context.sub_agent_sessions.push("subagent-1".to_string());
        context.pending_final_review = Some(PendingFinalReview {
            sub_agent_id: "reviewer-1".to_string(),
            completion_summary: "review".to_string(),
        });
        context
            .pending_completion_reports
            .push(PendingCompletionReport::new(
                "Completed: old. Verified: old. Remaining: none.",
                None,
                7,
                9,
            ));
        context
            .removed_queued_user_message_ids
            .push("queued-removed".to_string());

        context.reset_for_new_context();

        assert_eq!(context.session_id, "session-1");
        assert_eq!(context.current_segment_id, 7);
        assert_eq!(context.current_context_tokens, Some(42));
        assert_eq!(context.max_context_tokens, Some(8192));
        assert_eq!(context.last_event_id, Some(99));
        assert_eq!(context.state, RuntimeState::Pending);
        assert_eq!(context.wait_reason, None);
        assert!(context.queued_user_messages.is_empty());
        assert_eq!(context.current_step, 0);
        assert_eq!(context.max_steps, 0);
        assert!(context.pending_tools.is_empty());
        assert_eq!(context.last_action_summary, None);
        assert_eq!(context.waiting_on_sub_agent_id, None);
        assert!(context.sub_agent_sessions.is_empty());
        assert!(context.pending_sub_agent_completions.is_empty());
        assert_eq!(context.pending_final_review, None);
        assert!(context.pending_completion_reports.is_empty());
        assert!(context.removed_queued_user_message_ids.is_empty());
    }

    #[test]
    fn test_task_completed_gateway_payload_serialization() {
        let payload = GatewayPayload::TaskCompleted {
            tool_call_id: "call_complete_123".to_string(),
            segment_id: 7,
            usage_summary: None,
        };
        let serialized = serde_json::to_value(payload).unwrap();
        assert_eq!(serialized["type"], "task_completed");
        assert_eq!(serialized["tool_call_id"], "call_complete_123");
        assert_eq!(serialized["segment_id"], 7);
    }

    #[test]
    fn test_compression_applied_gateway_payload_serialization() {
        let payload = GatewayPayload::CompressionApplied {
            compressed_until_message_id: 42,
            current_context_tokens: 2048,
            max_context_tokens: 8192,
        };
        let serialized = serde_json::to_value(payload).unwrap();
        assert_eq!(serialized["type"], "compression_applied");
        assert_eq!(serialized["compressed_until_message_id"], 42);
        assert_eq!(serialized["current_context_tokens"], 2048);
        assert_eq!(serialized["max_context_tokens"], 8192);
    }

    #[test]
    fn test_confirm_gateway_payload_serialization() {
        let payload = GatewayPayload::Confirm {
            id: "call_confirm_123".to_string(),
            action: "bash".to_string(),
            tool_name: "bash".to_string(),
            arguments: serde_json::json!({"command": "pwd"}),
            details: serde_json::json!({"command": "pwd"}),
            display_type: Some("text".to_string()),
        };
        let serialized = serde_json::to_value(payload).unwrap();
        assert_eq!(serialized["type"], "confirm");
        assert_eq!(serialized["id"], "call_confirm_123");
        assert_eq!(serialized["tool_name"], "bash");
        assert_eq!(serialized["arguments"]["command"], "pwd");
        assert_eq!(serialized["display_type"], "text");
    }

    #[test]
    fn test_approval_resolved_gateway_payload_preserves_tool_identity() {
        let payload = GatewayPayload::ApprovalResolved {
            tool_call_id: "call_plan_123".to_string(),
            tool_name: "submit_plan".to_string(),
            approved: true,
            approve_all: false,
            approval_status: Some("approved".to_string()),
            execution_status: Some("approval_submitted".to_string()),
            rejection_message: None,
        };
        let serialized = serde_json::to_value(payload).unwrap();
        assert_eq!(serialized["type"], "approval_resolved");
        assert_eq!(serialized["tool_call_id"], "call_plan_123");
        assert_eq!(serialized["tool_name"], "submit_plan");
        assert_eq!(serialized["approval_status"], "approved");
        assert_eq!(serialized["execution_status"], "approval_submitted");
        assert!(serialized.get("rejection_message").is_none());
    }

    #[test]
    fn test_execution_context_new() {
        let ctx = ExecutionContext::new("test-session".to_string());
        assert_eq!(ctx.session_id, "test-session");
        assert_eq!(ctx.state, RuntimeState::Pending);
        assert!(ctx.wait_reason.is_none());
        assert_eq!(ctx.current_segment_id, 1);
        assert!(ctx.pending_tools.is_empty());
        assert!(ctx.last_event_id.is_none());
        assert_eq!(ctx.version, "1.4.0");
        assert!(ctx.waiting_on_sub_agent_id.is_none());
        assert!(ctx.awaiting_user_tool_call_id.is_none());
        assert!(ctx.effective_task_objective.is_none());
        assert!(ctx.sub_agent_sessions.is_empty());
        assert!(ctx.pending_sub_agent_completions.is_empty());
        assert!(ctx.pending_final_review.is_none());
        assert!(ctx.pending_completion_reports.is_empty());
    }

    #[test]
    fn test_execution_context_serialization_roundtrip() {
        let mut ctx = ExecutionContext::new("test-session".to_string());
        ctx.state = RuntimeState::Waiting;
        ctx.wait_reason = Some(WaitReason::Approval);
        ctx.current_segment_id = 3;
        ctx.current_step = 5;
        ctx.max_steps = 100;
        ctx.removed_queued_user_message_ids = vec!["queue-removed".to_string()];
        ctx.pending_tools.push(PendingTool {
            tool_call_id: "call_123".to_string(),
            tool_name: "bash".to_string(),
            arguments: serde_json::json!({"command": "ls"}),
            details: Some(serde_json::json!("List files")),
            display_type: Some("text".to_string()),
        });
        ctx.pending_completion_reports
            .push(PendingCompletionReport::new(
                "Implemented the requested change.\nVerified the focused tests passed.",
                Some(77),
                3,
                5,
            ));
        let mut objective = EffectiveTaskObjective::new(101, "Audit the workflow".to_string());
        objective.append_directive(102, "Implement the confirmed fixes".to_string());
        ctx.effective_task_objective = Some(objective);

        let json = serde_json::to_string(&ctx).unwrap();
        let deserialized: ExecutionContext = serde_json::from_str(&json).unwrap();

        assert_eq!(ctx, deserialized);
    }

    #[test]
    fn execution_context_deserializes_legacy_snapshot_without_completion_drafts() {
        let legacy = serde_json::json!({
            "session_id": "legacy-session",
            "state": "pending",
            "wait_reason": null,
            "current_segment_id": 2,
            "current_step": 3,
            "max_steps": 100,
            "pending_tools": [],
            "last_action_summary": null,
            "version": "1.4.0"
        });

        let context: ExecutionContext = serde_json::from_value(legacy).unwrap();

        assert!(context.pending_completion_reports.is_empty());
        assert!(context.effective_task_objective.is_none());
    }

    #[test]
    fn effective_task_objective_preserves_directive_identity_and_precedence() {
        let mut objective = EffectiveTaskObjective::new(11, "Audit only".to_string());
        objective.append_directive(12, "Implement the confirmed fixes".to_string());
        objective.append_directive(13, "Keep the scope focused".to_string());

        assert_eq!(objective.initial_source_message_id, 11);
        assert_eq!(objective.latest_source_message_id, 13);
        assert_eq!(objective.source_message_ids(), vec![11, 12, 13]);
        assert_eq!(
            objective.directive_precedence,
            EffectiveTaskDirectivePrecedence::LaterDirectivesOverrideConflicts
        );
    }

    #[test]
    fn test_gateway_message_serializes_persisted_id() {
        let payload = GatewayPayload::Message {
            message_id: Some("9007199254740993".to_string()),
            role: "assistant".to_string(),
            content: "done".to_string(),
            reasoning: None,
            step_type: Some(StepType::Think),
            step_index: 2,
            is_error: false,
            error_type: None,
            metadata: None,
        };

        let value = serde_json::to_value(payload).unwrap();
        assert_eq!(value["type"], "message");
        assert_eq!(value["message_id"], "9007199254740993");
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

        let json = r#"{"type":"manual_compress"}"#;
        let signal = WorkflowSignal::parse(json).unwrap();
        assert!(matches!(signal, WorkflowSignal::ManualCompress));

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

        let manual_compress = WorkflowSignal::ManualCompress;
        assert!(manual_compress.is_valid_for(None));
        assert!(manual_compress.is_valid_for(Some(&WaitReason::Approval)));

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
