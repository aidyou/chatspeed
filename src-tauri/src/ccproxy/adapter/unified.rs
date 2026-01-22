use std::collections::HashMap;

use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;

use crate::ccproxy::{adapter::backend::generate_tool_prompt, gemini::SafetySetting};

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
    pub temperature: Option<f32>, // Range varies by protocol: OpenAI (0-2), Claude/Gemini (0-1)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>, // Range: 0-1 for all protocols
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<i32>, // OpenAI doesn't support directly, Claude/Gemini do
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,

    // OpenAI-specific parameters
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f32>, // Range: -2.0 to 2.0
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f32>, // Range: -2.0 to 2.0
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>, // Candidate count
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<i32>, // For deterministic sampling
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>, // End-user identifier
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<bool>, // Whether to return log probabilities
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_logprobs: Option<i32>, // Number of most likely tokens to return

    // Claude-specific parameters
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<UnifiedMetadata>, // Request metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<UnifiedThinking>, // Extended thinking configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<UnifiedCacheControl>, // Cache control

    // Gemini-specific parameters
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety_settings: Option<Vec<SafetySetting>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_schema: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_content: Option<String>, // Context cache content name

    // Ollama
    pub keep_alive: Option<String>,

    // For tool compatibility mode
    pub tool_compat_mode: bool,
    pub prompt_injection: Option<String>,
    pub prompt_enhance_text: Option<String>,
    pub combined_prompt: Option<String>,
    pub prompt_injection_position: Option<String>,
    pub custom_params: Option<Value>,
}

impl UnifiedRequest {
    pub fn enhance_prompt(&mut self) {
        // We only inject prompts when tools are present.
        if self.tools.as_deref().unwrap_or_default().is_empty() {
            return;
        }

        // Generate tool prompt if in compatibility mode.
        let tool_prompt = if self.tool_compat_mode {
            if let Some(tools) = &self.tools {
                let prompt = generate_tool_prompt(tools);
                self.tools = None; // Clear tools as they are now in the prompt.
                prompt
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        let mut prompt_parts: Vec<&str> = Vec::new();
        let enhance_text_str = self.prompt_enhance_text.as_deref().unwrap_or_default();

        // The 'replace' mode for prompt_injection is now conceptually handled by
        // the backend adapter, which will decide whether to use the original system_prompt or not.
        // Here, we just combine the tool and enhancement prompts.
        prompt_parts.push(&tool_prompt);
        prompt_parts.push(enhance_text_str);

        let final_combined_prompt = prompt_parts
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<&str>>()
            .join("\n\n");

        if !final_combined_prompt.is_empty() {
            self.combined_prompt = Some(final_combined_prompt);
            self.prompt_enhance_text = None; // clear enhance_text
        }
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
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

/// Unified metadata for requests (primarily for Claude)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>, // External identifier for the user
}

/// Unified thinking configuration (for Claude and Gemini)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UnifiedThinking {
    pub budget_tokens: Option<i32>, // Token budget for internal reasoning
    pub include_thoughts: Option<bool>,
}

/// Unified cache control configuration (primarily for Claude)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnifiedCacheControl {
    pub cache_type: String, // "ephemeral"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl: Option<String>, // "5m" or "1h"
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

    // Internal extension field for transmitting search reference information
    // Do not delete!
    Reference {
        delta: String,
    },
}

/// Token usage statistics for the completion.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UnifiedUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    // Claude-specific detailed usage
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<u64>,
    // Gemini-specific detailed usage
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_use_prompt_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thoughts_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_content_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cached_tokens: Option<u64>,

    // ollama
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_duration: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load_duration: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_eval_duration: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eval_duration: Option<u64>,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct UnifiedFunctionCallPart {
    pub name: String,
    pub args: String,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct StreamLogRecorder {
    pub chat_id: String,
    pub model: String,
    pub content: String,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_option_map_tools"
    )]
    pub tool_calls: Option<HashMap<String, UnifiedFunctionCallPart>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
}

impl StreamLogRecorder {
    pub fn new(chat_id: String, model: String) -> Self {
        Self {
            chat_id,
            model,
            ..Default::default()
        }
    }
}

fn serialize_option_map_tools<S>(
    tools: &Option<HashMap<String, UnifiedFunctionCallPart>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if let Some(tools) = tools {
        let formatted_values: Vec<_> = tools
            .values()
            .map(|v| serde_json::to_string(v).unwrap_or_default())
            .collect();
        formatted_values.serialize(serializer)
    } else {
        serializer.serialize_none()
    }
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
    // For token estimation
    pub estimated_input_tokens: f64,
    pub estimated_output_tokens: f64,
    // For tool compatibility mode
    pub tool_compat_mode: bool,
    pub tool_compat_buffer: String,
    pub in_tool_call_block: bool,
    pub tool_compat_fragment_buffer: String,
    pub tool_compat_fragment_count: u32,
    pub tool_compat_last_flush_time: std::time::Instant,
    // For gemini tools: tool_id -> tool define
    pub gemini_tools: HashMap<String, UnifiedFunctionCallPart>,
    // For tracking tool_id to index mapping
    pub tool_id_to_index: HashMap<String, u32>,
    // For ollama tools
    pub tool_name: Option<String>,
    pub tool_arguments: Option<String>,
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
            estimated_input_tokens: 0.0,
            estimated_output_tokens: 0.0,
            tool_compat_mode: false,
            tool_compat_buffer: String::new(),
            in_tool_call_block: false,
            tool_compat_fragment_buffer: String::new(),
            tool_compat_fragment_count: 0,
            tool_compat_last_flush_time: std::time::Instant::now(),
            gemini_tools: HashMap::new(),
            tool_id_to_index: HashMap::new(),
            tool_name: None,
            tool_arguments: None,
        }
    }
}

impl SseStatus {
    pub fn new(
        message_id: String,
        model_id: String,
        tool_compat_mode: bool,
        estimated_input_tokens: f64,
    ) -> Self {
        Self {
            message_id,
            model_id,
            tool_compat_mode,
            estimated_input_tokens,
            tool_compat_last_flush_time: std::time::Instant::now(),
            ..Default::default()
        }
    }
}
