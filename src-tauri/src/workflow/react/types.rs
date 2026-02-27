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
pub struct GatewayEvent {
    pub session_id: String,
    pub payload: GatewayPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GatewayPayload {
    Text { content: String },
    State { state: WorkflowState },
    Confirm { id: String, action: String, details: String },
    Error { message: String },
}
