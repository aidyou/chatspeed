use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Display, EnumString)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "snake_case")]
pub enum WorkflowState {
    Pending,
    Thinking,
    Executing,
    Auditing,
    Paused,
    AwaitingApproval,
    AwaitingAutoApproval,
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
        state: WorkflowState,
    },
    Confirm {
        id: String,
        action: String,
        details: String,
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
