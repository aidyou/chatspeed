use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ccproxy::gemini::SafetySetting;

// ===================================
// Unified Request Structures
// ===================================

/// The unified, protocol-agnostic representation of a chat completion request.
/// This struct is the central "canonical" model for all incoming requests.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct UnifiedRequest {
    pub model: String,
    pub messages: Vec<UnifiedMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<UnifiedTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<UnifiedToolChoice>,
    pub stream: bool,
    // Common generation parameters
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,

    // For openai only
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<Value>,

    // For gemini only
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety_settings: Option<Vec<SafetySetting>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_schema: Option<Value>,

    // For tool compatibility mode
    pub tool_compat_mode: bool,
}

/// A single message in the chat history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedMessage {
    pub role: UnifiedRole,
    pub content: Vec<UnifiedContentBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
}

/// The role of the message author.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum UnifiedRole {
    System,
    User,
    Assistant,
    Tool,
}

/// A block of content within a message, allowing for multimodal inputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UnifiedContentBlock {
    Text {
        text: String,
    },
    Image {
        media_type: String,
        data: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
    /// The thinking is a claude specific feature, and for openai reasoning field
    Thinking {
        thinking: String,
    },
}

/// A tool that the model can call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedTool {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Value,
}

/// Controls how the model uses tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnifiedToolChoice {
    None,
    Auto,
    Required,
    Tool { name: String },
}

// ===================================
// Unified Response Structures
// ===================================

/// The unified, protocol-agnostic representation of a full chat completion response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedResponse {
    pub id: String,
    pub model: String,
    pub content: Vec<UnifiedContentBlock>,
    pub stop_reason: Option<String>,
    pub usage: UnifiedUsage,
}

/// The unified, protocol-agnostic representation of a single chunk from a streaming response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UnifiedStreamChunk {
    /// The start of the message stream.
    MessageStart {
        id: String,
        model: String,
        usage: UnifiedUsage,
    },
    /// A delta of "thinking" or reasoning content, this is a special field for claude
    Thinking {
        delta: String,
    },
    /// A delta of regular text content.
    Text {
        delta: String,
    },
    /// The start of a tool call.
    ToolUseStart {
        tool_type: String,
        id: String,
        name: String,
    },
    /// A delta of the arguments for a tool call.
    ToolUseDelta {
        id: String,
        delta: String, // JSON string delta
    },
    /// The end of a tool call.
    ToolUseEnd {
        id: String,
    },
    /// The end of the message stream.
    MessageStop {
        stop_reason: String,
        usage: UnifiedUsage,
    },
    /// content_block like:
    /// {"type":"content_block_start","index":1,"content_block":{"type":"server_tool_use","id":"srvtoolu_014hJH82Qum7Td6UV8gDXThB","name":"web_search","input":{}}}
    /// or
    /// {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}
    ContentBlockStart {
        index: u32,
        block: Value,
    },
    ContentBlockStop {
        index: u32,
    },
    /// An error occurred during the stream.
    Error {
        message: String,
    },
}

/// Token usage statistics for the completion.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UnifiedUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

pub struct SseStatus {
    pub message_start: bool,
    pub thinking_delta_count: u32,
    pub text_delta_count: u32,
    pub tool_delta_count: u32,
    pub model_id: String,
    pub message_id: String,
    pub tool_id: String,
    pub message_index: u32,
    pub current_content_block: String,
    // For tool compatibility mode
    pub tool_compat_mode: bool,
    pub tool_compat_buffer: String,
    pub in_tool_call_block: bool,
    pub tool_compat_fragment_buffer: String,
    pub tool_compat_fragment_count: u32,
    pub tool_compat_last_flush_time: std::time::Instant,
}

impl Default for SseStatus {
    fn default() -> Self {
        Self {
            message_start: false,
            thinking_delta_count: 0,
            text_delta_count: 0,
            tool_delta_count: 0,
            model_id: String::new(),
            message_id: String::new(),
            tool_id: String::new(),
            message_index: 0,
            current_content_block: String::new(),
            tool_compat_mode: false,
            tool_compat_buffer: String::new(),
            in_tool_call_block: false,
            tool_compat_fragment_buffer: String::new(),
            tool_compat_fragment_count: 0,
            tool_compat_last_flush_time: std::time::Instant::now(),
        }
    }
}
impl SseStatus {
    pub fn new(message_id: String, model_id: String, tool_compat_mode: bool) -> Self {
        Self {
            message_id,
            model_id,
            tool_compat_mode,
            tool_compat_last_flush_time: std::time::Instant::now(),
            ..Default::default()
        }
    }
}
