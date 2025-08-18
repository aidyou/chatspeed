use rmcp::model::IntoContents as _;
use rust_i18n::t;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fmt;

use crate::workflow::error::WorkflowError;

/// Model name, it's used to identify the model type.
/// The reasoning model is used for planning and analysis,
/// and the general model is used for text processing or general task.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ModelName {
    Reasoning,
    General,
}

impl fmt::Display for ModelName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelName::Reasoning => write!(f, "reasoning"),
            ModelName::General => write!(f, "general"),
        }
    }
}

impl AsRef<str> for ModelName {
    fn as_ref(&self) -> &str {
        match self {
            ModelName::Reasoning => "reasoning",
            ModelName::General => "general",
        }
    }
}

impl TryFrom<&str> for ModelName {
    type Error = WorkflowError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "reasoning" => Ok(ModelName::Reasoning),
            "general" => Ok(ModelName::General),
            _ => Err(WorkflowError::Initialization(
                t!("tools.invalid_model_name", model_name = value).to_string(),
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

impl ToolCallResult {
    pub fn success(content: Option<String>, structured_content: Option<Value>) -> Self {
        Self {
            content,
            structured_content,
            is_error: Some(false),
        }
    }
    pub fn error(error: String) -> Self {
        Self {
            content: Some(format!("Error: {}", error).to_string()),
            structured_content: Some(json!({"error": error})),
            is_error: Some(true),
        }
    }
}

impl From<ToolCallResult> for rmcp::model::CallToolResult {
    fn from(value: ToolCallResult) -> Self {
        Self {
            content: value.content.map(|c| c.into_contents()),
            structured_content: value.structured_content,
            is_error: value.is_error,
        }
    }
}

impl From<ToolCallResult> for Value {
    fn from(value: ToolCallResult) -> Self {
        serde_json::to_value(value).unwrap_or_default()
    }
}
