use std::fmt::Display;

use rmcp::model::IntoContents as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ToolScope {
    /// 仅在普通聊天中可用
    Chat,
    /// 仅在 ReAct 工作流中可用
    Workflow,
    /// 两者均可用
    Both,
}

impl ToolScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Workflow => "workflow",
            Self::Both => "both",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ToolCategory {
    FileSystem,
    Interaction,
    Mcp,
    Search,
    Skill,
    System,
    Web,
}

impl Display for ToolCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolCategory::FileSystem => write!(f, "FileSystem"),
            ToolCategory::Interaction => write!(f, "Interaction"),
            ToolCategory::Mcp => write!(f, "MCP"),
            ToolCategory::System => write!(f, "System"),
            ToolCategory::Web => write!(f, "Web"),
            ToolCategory::Search => write!(f, "Search"),
            ToolCategory::Skill => write!(f, "Skill"),
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
            content: Some(json!({"error": error}).to_string()),
            structured_content: None,
            is_error: Some(true),
        }
    }
}

impl From<ToolCallResult> for rmcp::model::CallToolResult {
    fn from(value: ToolCallResult) -> Self {
        let content = value.content.map(|c| c.into_contents()).unwrap_or_default();
        match (value.structured_content, value.is_error.unwrap_or(false)) {
            (Some(structured_content), true) => {
                let mut result = rmcp::model::CallToolResult::structured_error(structured_content);
                if !content.is_empty() {
                    result.content = content;
                }
                result
            }
            (Some(structured_content), false) => {
                let mut result = rmcp::model::CallToolResult::structured(structured_content);
                if !content.is_empty() {
                    result.content = content;
                }
                result
            }
            (None, true) => rmcp::model::CallToolResult::error(content),
            (None, false) => rmcp::model::CallToolResult::success(content),
        }
    }
}

impl From<ToolCallResult> for Value {
    fn from(value: ToolCallResult) -> Self {
        serde_json::to_value(value).unwrap_or_default()
    }
}
