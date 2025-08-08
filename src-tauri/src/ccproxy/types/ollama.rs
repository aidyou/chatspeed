use serde::{Deserialize, Serialize};
use serde_json::Value;

// Represents the request for a chat completion.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct OllamaChatCompletionRequest {
    pub model: String,
    pub messages: Vec<OllamaMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>, // "json"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<OllamaOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_alive: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<OllamaTool>>,
}

// Represents a message in the chat completion request.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct OllamaMessage {
    pub role: String, // "system", "user", "assistant", or "tool"
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<String>>, // List of base64-encoded images
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OllamaToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

// Represents a tool call from the model.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OllamaToolCall {
    pub function: OllamaFunctionCall,
}

// Represents a function call within a tool call.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct OllamaFunctionCall {
    pub name: String,
    pub arguments: Value, // JSON object
}

// Represents the non-streaming response for a chat completion.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OllamaChatCompletionResponse {
    pub model: String,
    pub created_at: String,
    pub message: OllamaMessage,
    pub done: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_duration: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load_duration: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_eval_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_eval_duration: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eval_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eval_duration: Option<u64>,
}

// Represents a single chunk in a streaming response.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct OllamaStreamResponse {
    pub model: String,
    pub created_at: String,
    pub message: OllamaMessage,
    pub done: bool,
    // The final stream object contains these additional fields.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_duration: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load_duration: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_eval_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_eval_duration: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eval_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eval_duration: Option<u64>,
}

// Represents additional model parameters.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_keep: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_predict: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub typical_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat_last_n: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeat_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub penalize_newline: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub numa: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_ctx: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_batch: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_gpu: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_gpu: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_mmap: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_thread: Option<i32>,
}

// Represents a tool that can be used by the model.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OllamaTool {
    pub r#type: String, // "function"
    pub function: OllamaFunctionDefinition,
}

// Represents the definition of a function tool.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OllamaFunctionDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value, // JSON schema
}
