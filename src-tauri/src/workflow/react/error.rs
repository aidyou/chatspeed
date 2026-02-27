use serde::Serialize;
use thiserror::Error;
use crate::db::error::StoreError;
use crate::tools::ToolError;
use crate::ai::error::AiError;

#[derive(Error, Debug, Serialize)]
#[serde(tag = "type", content = "details")]
pub enum WorkflowEngineError {
    #[error("Database error: {0}")]
    Db(#[from] StoreError),

    #[error("Tool error: {0}")]
    Tool(#[from] ToolError),

    #[error("AI Model error: {0}")]
    Ai(#[from] AiError),

    #[error("Context overflow: current tokens {current} exceeds max {max}")]
    ContextOverflow { current: usize, max: usize },

    #[error("Invalid state transition from {from} via {event}")]
    InvalidState { from: String, event: String },

    #[error("Security violation: {0}")]
    Security(String),

    #[error("Gateway error: {0}")]
    Gateway(String),

    #[error("Execution cancelled")]
    Cancelled,

    #[error("General error: {0}")]
    General(String),
}
