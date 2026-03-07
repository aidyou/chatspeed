use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Display, EnumString)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "snake_case")]
pub enum WorkflowState {
    Pending,
    Thinking,
    Executing,
    Paused,
    AwaitingApproval,
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
    Error {
        message: String,
    },
}
