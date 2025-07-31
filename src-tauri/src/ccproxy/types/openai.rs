use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Unified structure for chat messages, used in requests, non-stream responses, and stream deltas.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct UnifiedChatMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>, // "system", "user", "assistant", "tool"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<OpenAIMessageContent>, // Changed from Option<String>
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<UnifiedToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>, // Used by "tool" role messages in requests
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>, // Added for custom reasoning content
                                           // index is specific to streaming tool calls, but can be part of a unified tool call struct if needed for parsing
                                           // However, OpenAI's request/non-stream response tool_calls don't have an index.
                                           // So, `UnifiedToolCall` will handle the `index` for streaming.
}

/// Represents the content of an OpenAI message, which can be simple text or a list of parts for multimodal.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)] // Allows for either a simple string or an array of parts
pub enum OpenAIMessageContent {
    Text(String),
    Parts(Vec<OpenAIMessageContentPart>),
}

/// Represents a part of an OpenAI message content (e.g., text or image).
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum OpenAIMessageContentPart {
    Text { text: String },
    ImageUrl { image_url: OpenAIImageUrl },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OpenAIImageUrl {
    pub url: String, // Can be a URL or a data URI (e.g., "data:image/jpeg;base64,...")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>, // "auto", "low", "high"
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)] // Added Default
pub struct OpenAIChatCompletionRequest {
    pub model: String, // This will be our proxy alias
    pub messages: Vec<UnifiedChatMessage>,
    #[serde(default)]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<i32>,
    // Generation control parameters
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>, // Range: 0.0 to 2.0, default: 1.0
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>, // Range: 0.0 to 1.0, default: 1.0
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f32>, // Range: -2.0 to 2.0, default: 0.0
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f32>, // Range: -2.0 to 2.0, default: 0.0
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<OpenAIResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>, // Max 4 sequences
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<i32>, // For deterministic sampling
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>, // End-user identifier
    // Tools and function calling
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<OpenAITool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<OpenAIToolChoice>,
    // Advanced features
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<bool>, // Whether to return log probabilities
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_logprobs: Option<i32>, // Number of most likely tokens to return at each position
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<OpenAIStreamOptions>,
    // Advanced parameters
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logit_bias: Option<HashMap<String, f32>>, // Token ID to bias value mapping (-100 to 100)
}

#[derive(Serialize, Deserialize, Debug, Clone)] // Added Deserialize
pub struct OpenAIChatCompletionResponse {
    pub id: String,     // chat completion id
    pub object: String, // "chat.completion"
    pub created: u64,   // unix timestamp
    pub model: String,  // model name used
    pub choices: Vec<OpenAIChatCompletionChoice>,
    pub usage: Option<OpenAIUsage>,
}

#[derive(Serialize, Deserialize, Debug, Clone)] // Added Deserialize
pub struct OpenAIChatCompletionChoice {
    pub index: u32,
    pub message: UnifiedChatMessage, // For non-streaming, role: "assistant"
    pub finish_reason: Option<String>, // e.g., "stop", "length", "tool_calls", "content_filter"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<OpenAILogprobs>, // Log probabilities for the choice
}

#[derive(Serialize, Deserialize, Debug, Clone)] // Added Deserialize
pub struct OpenAIUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

// For streaming
#[derive(Serialize, Deserialize, Debug, Clone)] // Added Deserialize for consistency, though not directly causing the error
pub struct OpenAIChatCompletionStreamResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<String>, // "chat.completion.chunk"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub choices: Vec<OpenAIStreamChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<OpenAIUsage>,
}

#[derive(Serialize, Deserialize, Debug, Clone)] // Added Deserialize for consistency
pub struct OpenAIStreamChoice {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
    pub delta: UnifiedChatMessage, // Using the unified message structure for delta
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logprobs: Option<OpenAILogprobs>, // Log probabilities for streaming
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct OpenAIFunctionCall {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>, // JSON string
}

/// Unified structure for tool calls, used in requests, non-stream responses, and stream deltas.
#[derive(Serialize, Deserialize, Debug, Clone)] // Deserialize for AssistantAction parsing
pub struct UnifiedToolCall {
    // For streaming, OpenAI sends tool calls with an index.
    // For non-streaming response, `id` is present. For request, `id` is not part of `tools[].function`.
    // `id` here refers to the tool_call_id generated by the assistant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>, // e.g., "function"
    pub function: OpenAIFunctionCall, // Re-using OpenAIFunctionCall
    #[serde(skip_serializing_if = "Option::is_none")] // Only relevant for streaming delta
    pub index: Option<u32>,
}
/// Represents a tool provided in the OpenAI chat completion request.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OpenAITool {
    /// The type of the tool. Currently, only "function" is supported by OpenAI.
    pub r#type: String,
    /// The function definition.
    pub function: OpenAIFunctionDefinition,
}

/// Defines the structure of a function tool.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OpenAIFunctionDefinition {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parameters: Value, // JSON schema for the function's parameters
}

/// Response format specification for OpenAI requests
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OpenAIResponseFormat {
    #[serde(rename = "type")]
    pub format_type: String, // "text" or "json_object"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json_schema: Option<Value>, // JSON schema for structured output
}

/// Tool choice specification for OpenAI requests
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum OpenAIToolChoice {
    String(String), // "none", "auto", "required"
    Object(OpenAIToolChoiceObject),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OpenAIToolChoiceObject {
    #[serde(rename = "type")]
    pub choice_type: String, // "function"
    pub function: OpenAIToolChoiceFunction,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OpenAIToolChoiceFunction {
    pub name: String,
}

/// Stream options for OpenAI requests
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OpenAIStreamOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_usage: Option<bool>, // Whether to include usage in final chunk
}

/// Log probabilities result for OpenAI responses
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OpenAILogprobs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<OpenAILogprobsContent>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OpenAILogprobsContent {
    pub token: String,
    pub logprob: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<Vec<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_logprobs: Option<Vec<OpenAITopLogprob>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OpenAITopLogprob {
    pub token: String,
    pub logprob: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<Vec<u8>>,
}

/// Server-Sent Events structure for OpenAI streaming responses
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SseEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<OpenAIUsage>, // Added usage field
}

impl OpenAIChatCompletionRequest {
    /// Validate request parameters according to OpenAI API constraints
    pub fn validate(&self) -> Result<(), String> {
        // Validate temperature range (0.0 to 2.0)
        if let Some(temp) = self.temperature {
            if temp < 0.0 || temp > 2.0 {
                return Err("Temperature must be between 0.0 and 2.0".to_string());
            }
        }

        // Validate top_p range (0.0 to 1.0)
        if let Some(top_p) = self.top_p {
            if top_p < 0.0 || top_p > 1.0 {
                return Err("top_p must be between 0.0 and 1.0".to_string());
            }
        }

        // Validate presence_penalty range (-2.0 to 2.0)
        if let Some(penalty) = self.presence_penalty {
            if penalty < -2.0 || penalty > 2.0 {
                return Err("presence_penalty must be between -2.0 and 2.0".to_string());
            }
        }

        // Validate frequency_penalty range (-2.0 to 2.0)
        if let Some(penalty) = self.frequency_penalty {
            if penalty < -2.0 || penalty > 2.0 {
                return Err("frequency_penalty must be between -2.0 and 2.0".to_string());
            }
        }

        // Validate stop sequences (max 4)
        // if let Some(ref stop) = self.stop {
        //     if stop.len() > 4 {
        //         return Err("Maximum 4 stop sequences allowed".to_string());
        //     }
        // }

        // Validate max_tokens is positive
        // if let Some(max_tokens) = self.max_tokens {
        //     if max_tokens <= 0 {
        //         return Err("max_tokens must be positive".to_string());
        //     }
        // }

        // Validate top_logprobs range (0 to 20)
        if let Some(top_logprobs) = self.top_logprobs {
            if top_logprobs < 0 || top_logprobs > 20 {
                return Err("top_logprobs must be between 0 and 20".to_string());
            }
        }

        // Validate logit_bias values range (-100 to 100)
        if let Some(ref logit_bias) = self.logit_bias {
            for (_, &bias) in logit_bias.iter() {
                if bias < -100.0 || bias > 100.0 {
                    return Err("logit_bias values must be between -100.0 and 100.0".to_string());
                }
            }
        }

        Ok(())
    }
}
