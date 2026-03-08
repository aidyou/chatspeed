use crate::ai::error::AiError;
use crate::db::error::StoreError;
use crate::tools::ToolError;

use serde::Serialize;
use thiserror::Error;

#[derive(Error, Debug, Serialize)]
#[serde(tag = "type", content = "details")]
pub enum WorkflowEngineError {
    #[error("Database error: {0}")]
    Db(#[from] StoreError),

    #[error("Tool error: {0}")]
    Tool(#[from] ToolError),

    #[error("AI Model error: {0}")]
    Ai(#[from] AiError),

    #[error("Security violation: {0}")]
    Security(String),

    #[error("Gateway error: {0}")]
    Gateway(String),

    #[error("General error: {0}")]
    General(String),
}
