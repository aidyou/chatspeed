use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// Claude API native request format
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClaudeNativeRequest {
    pub model: String,
    pub messages: Vec<ClaudeNativeMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default, deserialize_with = "deserialize_system_field")]
    pub system: Option<String>,
    pub max_tokens: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ClaudeNativeTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ClaudeToolChoice>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClaudeNativeMessage {
    pub role: String, // "user" or "assistant"
    #[serde(deserialize_with = "deserialize_content")]
    pub content: Vec<ClaudeNativeContentBlock>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum ClaudeNativeContentBlock {
    Text {
        #[serde(deserialize_with = "deserialize_text_or_array_of_strings")]
        text: String,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        #[serde(deserialize_with = "deserialize_text_or_array_of_strings")]
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    Image {
        source: ClaudeImageSource,
    },
    Thinking {
        // Added Thinking variant
        thinking: String,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClaudeImageSource {
    #[serde(rename = "type")]
    pub source_type: String,
    pub media_type: String,
    pub data: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClaudeNativeTool {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Value,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "type")]
pub enum ClaudeToolChoice {
    Auto,
    Any,
    Tool { name: String },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SystemBlock {
    pub r#type: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<Value>,
}

/// Claude API native response format
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClaudeNativeResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub response_type: String,
    pub role: Option<String>,
    pub content: Vec<ClaudeNativeContentBlock>,
    pub model: Option<String>,
    pub stop_reason: Option<String>,
    pub usage: Option<ClaudeNativeUsage>,
    pub error: Option<ClaudeNativeError>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClaudeNativeUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClaudeNativeError {
    #[serde(rename = "type")]
    pub error_type: String,
    pub message: String,
}

/// Claude 流式响应事件
#[derive(Deserialize, Debug, Clone)]
pub struct ClaudeStreamEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub message: Option<ClaudeStreamMessageStart>,
    pub index: Option<u32>,
    pub content_block: Option<ClaudeStreamContentBlock>,
    pub delta: Option<ClaudeStreamDelta>,
    pub usage: Option<ClaudeStreamUsage>,
    pub error: Option<ClaudeNativeError>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ClaudeStreamMessageStart {
    pub id: String,
    #[serde(rename = "type")]
    pub _message_type: String,
    pub role: String,
    pub model: String,
    pub usage: ClaudeStreamUsage,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ClaudeStreamContentBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    pub text: Option<String>,
    pub id: Option<String>,
    pub name: Option<String>,
    pub input: Option<Value>,
    pub tool_use_id: Option<String>,
    pub content: Option<Value>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ClaudeStreamDelta {
    #[serde(rename = "type")]
    pub delta_type: Option<String>,
    pub text: Option<String>,
    pub stop_reason: Option<String>,
    pub partial_json: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ClaudeStreamUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

// Custom deserialization function to handle both string and array formats for content
fn deserialize_content<'de, D>(deserializer: D) -> Result<Vec<ClaudeNativeContentBlock>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum ContentInput {
        String(String),
        Array(Vec<ClaudeNativeContentBlock>),
    }

    let content_input = ContentInput::deserialize(deserializer)?;

    match content_input {
        ContentInput::String(text) => Ok(vec![ClaudeNativeContentBlock::Text { text }]),
        ContentInput::Array(blocks) => Ok(blocks),
    }
}

// Custom deserialization for a field that can be a string or an array of strings.
fn deserialize_text_or_array_of_strings<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrVec {
        String(String),
        Array(Vec<String>),
    }

    match StringOrVec::deserialize(deserializer)? {
        StringOrVec::String(s) => Ok(s),
        StringOrVec::Array(v) => Ok(v.join("\n")), // Join array of strings with newline
    }
}

fn deserialize_system_field<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum SystemInput {
        String(String),
        Array(Vec<SystemBlock>),
    }

    let system_input = Option::<SystemInput>::deserialize(deserializer)?;

    match system_input {
        Some(SystemInput::String(s)) => Ok(Some(s)),
        Some(SystemInput::Array(blocks)) => {
            if let Some(first_block) = blocks.first() {
                Ok(Some(first_block.text.clone()))
            } else {
                Ok(None)
            }
        }
        None => Ok(None),
    }
}

/// State for managing stream events
pub struct StreamState {
    pub thinking_started: bool,
    pub text_started: bool,
}

impl Default for StreamState {
    fn default() -> Self {
        Self {
            thinking_started: false,
            text_started: false,
        }
    }
}
