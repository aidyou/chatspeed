use serde::{Deserialize, Serialize};
use strum::{Display, EnumString};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Display, EnumString)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum WorkflowState {
    Pending,
    Thinking,
    Executing,
    Paused,
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
    /// Incremental chunk of text (for streaming thoughts or content)
    Chunk {
        content: String,
    },
    /// Full message update
    Message {
        role: String,
        content: String,
        step_type: Option<StepType>,
        step_index: i32,
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
    Error {
        message: String,
    },
}
