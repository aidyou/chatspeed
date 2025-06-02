use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Represents a target backend model for a proxy alias.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BackendModelTarget {
    /// The ID of the provider (corresponds to `modelStore.provider.id`).
    pub id: i64,
    /// The ID of the model within the provider (corresponds to `modelStore.provider.models[n].id`).
    pub model: String,
}

/// Configuration for chat completion proxy.
/// Maps a proxy alias (String key) to a list of backend model targets.
pub type ChatCompletionProxyConfig = HashMap<String, Vec<BackendModelTarget>>;

/// Represents an access key for the chat completion proxy.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProxyAccessKey {
    /// A descriptive name for the access key.
    pub name: String,
    /// The actual token string (e.g., "cs-xxxx").
    pub token: String,
}

/// Configuration for chat completion proxy access keys.
pub type ChatCompletionProxyKeysConfig = Vec<ProxyAccessKey>;

/// Represents a model in the OpenAI-compatible list models response.
#[derive(Serialize, Debug)]
pub struct OpenAIModel {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub owned_by: String,
}

/// Represents the OpenAI-compatible list models response structure.
#[derive(Serialize, Debug)]
pub struct OpenAIListModelsResponse {
    pub object: String,
    pub data: Vec<OpenAIModel>,
}

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

#[derive(Serialize, Deserialize, Debug, Clone)] // Added Serialize
pub struct OpenAIChatCompletionRequest {
    pub model: String, // This will be our proxy alias
    pub messages: Vec<UnifiedChatMessage>,
    #[serde(default)]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<i32>,
    // Add other common OpenAI parameters if needed, e.g., temperature, max_tokens, etc.
    // These will be passed through to the actual AI call if not overridden by proxy config.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<u32>, // Number of chat completion choices to generate for each input message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    // tools, tool_choice, etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<OpenAITool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<Value>, // Can be "none", "auto", or {"type": "function", "function": {"name": "my_function"}}
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
    pub finish_reason: Option<String>, // e.g., "stop", "length", "tool_calls"
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
    pub id: String,
    pub object: String, // "chat.completion.chunk"
    pub created: u64,
    pub model: String,
    pub choices: Vec<OpenAIStreamChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<OpenAIUsage>,
}

#[derive(Serialize, Deserialize, Debug, Clone)] // Added Deserialize for consistency
pub struct OpenAIStreamChoice {
    pub index: u32,
    pub delta: UnifiedChatMessage, // Using the unified message structure for delta
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
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

/// Represents a parsed Server-Sent Event with its distinct fields.
/// Adapters will produce this, and handlers.rs will consume it to build warp::sse::Event.
#[derive(Serialize, Debug, Clone, Default)]
pub struct SseEvent {
    /// The event's 'id' field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The event's 'event' field (type of event).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>, // Renamed from 'event' to avoid conflict with warp::sse::Event builder methods
    /// The event's 'data' field content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    /// The event's 'retry' field value (as a string, to be parsed later if needed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<String>,
}
