use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// Claude API native request format
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
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
    pub temperature: Option<f32>, // Range: 0.0 to 1.0, default: 1.0
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>, // Range: 0 to 1, for advanced use cases
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<i32>, // x >= 0, for advanced use cases
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>, // Custom text sequences that cause model to stop
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ClaudeNativeTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ClaudeToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ClaudeMetadata>, // Request metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ClaudeThinking>, // Extended thinking configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<ClaudeCacheControl>, // Cache control breakpoint
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClaudeNativeMessage {
    pub role: String, // "user" or "assistant"
    #[serde(deserialize_with = "deserialize_content")]
    pub content: Vec<ClaudeNativeContentBlock>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
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
    #[serde(default)]
    pub input_schema: Value,
}

/// Metadata for Claude requests
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClaudeMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>, // External identifier for the user (max length 256)
}

/// Extended thinking configuration for Claude
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClaudeThinking {
    #[serde(rename = "type")]
    pub thinking_type: String, // "enabled"
    pub budget_tokens: i32, // x >= 1024 and < max_tokens
}

/// Cache control configuration for Claude
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClaudeCacheControl {
    #[serde(rename = "type")]
    pub cache_type: String, // "ephemeral"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl: Option<String>, // "5m" or "1h", default: "5m"
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
    pub response_type: String, // "message"
    pub role: Option<String>, // "assistant"
    pub content: Vec<ClaudeNativeContentBlock>,
    pub model: Option<String>,       // Model that processed the request
    pub stop_reason: Option<String>, // "end_turn", "max_tokens", "stop_sequence", "tool_use", etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequence: Option<String>, // Which custom stop sequence was generated
    pub usage: Option<ClaudeNativeUsage>,
    pub error: Option<ClaudeNativeError>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ClaudeNativeUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u64>, // Input tokens used for cache creation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<u64>, // Input tokens read from cache
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation: Option<ClaudeCacheCreation>, // Cache token breakdown by TTL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_tool_use: Option<ClaudeServerToolUse>, // Server tool request counts
}

/// Cache creation token breakdown by TTL
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClaudeCacheCreation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ephemeral_1h_input_tokens: Option<u64>, // 1-hour cache input tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ephemeral_5m_input_tokens: Option<u64>, // 5-minute cache input tokens
}

/// Server tool usage statistics
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClaudeServerToolUse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub web_search_requests: Option<u64>, // Number of web search tool requests
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClaudeNativeError {
    #[serde(rename = "type")]
    pub error_type: String,
    pub message: String,
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

/// Simplified Claude stream event for internal processing
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
    // pub message_type: String,
    // pub role: String,
    pub model: String,
    pub usage: ClaudeStreamUsage,
}

// Custom deserialization function to handle both string and array formats for content
fn deserialize_content<'de, D>(deserializer: D) -> Result<Vec<ClaudeNativeContentBlock>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;

    match value {
        Value::String(text) => Ok(vec![ClaudeNativeContentBlock::Text { text }]),
        Value::Array(arr) => {
            let mut blocks = Vec::new();
            for item in arr {
                // Remove unknown fields first, then deserialize
                if let Value::Object(mut obj) = item {
                    // Remove unknown fields like cache_control
                    obj.remove("cache_control");

                    if let Ok(block) =
                        serde_json::from_value::<ClaudeNativeContentBlock>(Value::Object(obj))
                    {
                        blocks.push(block);
                    }
                } else if let Ok(block) = serde_json::from_value::<ClaudeNativeContentBlock>(item) {
                    blocks.push(block);
                }
            }
            Ok(blocks)
        }
        _ => Err(serde::de::Error::custom("Expected string or array")),
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
        Some(SystemInput::Array(blocks)) => Ok(Some(
            blocks
                .into_iter()
                .map(|x| x.text.clone())
                .collect::<Vec<String>>()
                .join("\n\n"),
        )),
        None => Ok(None),
    }
}

impl ClaudeNativeRequest {
    /// Validate request parameters according to Claude API constraints
    pub fn validate(&self) -> Result<(), String> {
        // Validate temperature range (0.0 to 1.0)
        if let Some(temp) = self.temperature {
            if temp < 0.0 || temp > 1.0 {
                return Err("Temperature must be between 0.0 and 1.0".to_string());
            }
        }

        // Validate top_p range (0.0 to 1.0)
        if let Some(top_p) = self.top_p {
            if top_p < 0.0 || top_p > 1.0 {
                return Err("top_p must be between 0.0 and 1.0".to_string());
            }
        }

        // Validate top_k is non-negative
        if let Some(top_k) = self.top_k {
            if top_k < 0 {
                return Err("top_k must be non-negative".to_string());
            }
        }

        // Validate max_tokens is positive
        if self.max_tokens <= 0 {
            return Err("max_tokens must be positive".to_string());
        }

        // Validate thinking budget_tokens if present
        if let Some(ref thinking) = self.thinking {
            if thinking.budget_tokens < 1024 {
                return Err("thinking budget_tokens must be at least 1024".to_string());
            }
            if thinking.budget_tokens >= self.max_tokens {
                return Err("thinking budget_tokens must be less than max_tokens".to_string());
            }
        }

        // Validate model name length (1-256 characters)
        if self.model.is_empty() || self.model.len() > 256 {
            return Err("Model name must be between 1 and 256 characters".to_string());
        }

        // Validate messages limit (max 100,000 messages)
        if self.messages.len() > 100_000 {
            return Err("Maximum 100,000 messages allowed".to_string());
        }

        // Validate user_id length if present
        if let Some(ref metadata) = self.metadata {
            if let Some(ref user_id) = metadata.user_id {
                if user_id.len() > 256 {
                    return Err("user_id must be at most 256 characters".to_string());
                }
            }
        }

        Ok(())
    }
}
