use std::fmt::Display;

use rmcp::model::IntoContents as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolCategory {
    Web,
}

impl Display for ToolCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolCategory::Web => write!(f, "Web"),
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
            content: value.content.map(|c| c.into_contents()).unwrap_or_default(),
            structured_content: value.structured_content,
            is_error: value.is_error,
            meta: None,
        }
    }
}

impl From<ToolCallResult> for Value {
    fn from(value: ToolCallResult) -> Self {
        serde_json::to_value(value).unwrap_or_default()
    }
}
