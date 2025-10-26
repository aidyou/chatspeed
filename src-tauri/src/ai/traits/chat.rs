use super::stoppable::Stoppable;
use crate::{ai::error::AiError, ccproxy::ChatProtocol};

use async_trait::async_trait;
use log::warn;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
use std::{default::Default, fmt::Display, sync::Arc};

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum MessageType {
    Error,
    Finished,
    Reasoning,
    Reference,
    Text,
    Think,
    ToolCalls, // Assistant tool selection
    ToolResults,
    Step,
}

impl Default for MessageType {
    fn default() -> Self {
        MessageType::Text
    }
}

impl<'de> Deserialize<'de> for MessageType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(MessageType::from_str(&s).unwrap_or_else(|| {
            warn!("Invalid message type: '{}', defaulting to Text", s);
            MessageType::Text
        }))
    }
}

impl Display for MessageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            MessageType::Error => "error",
            MessageType::Finished => "finished",
            MessageType::Reasoning | MessageType::Think => "reasoning",
            MessageType::Reference => "reference",
            MessageType::Text => "text",
            MessageType::ToolCalls => "tool_calls",
            MessageType::ToolResults => "tool_results",
            MessageType::Step => "step",
        };
        write!(f, "{}", s)
    }
}

impl MessageType {
    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "error" => Some(MessageType::Error),
            "finished" => Some(MessageType::Finished),
            "reasoning" | "think" | "thinking" => Some(MessageType::Reasoning),
            "reference" => Some(MessageType::Reference),
            "text" => Some(MessageType::Text),
            "tool_calls" => Some(MessageType::ToolCalls),
            "tool_results" => Some(MessageType::ToolResults),
            "step" => Some(MessageType::Step),
            _ => {
                warn!(
                    "Unrecognized message type: '{}', will be handled by deserializer default.",
                    value
                );
                None
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum FinishReason {
    Stop,          // Corresponds to OpenAI "stop"
    Length,        // Corresponds to OpenAI "length"
    ToolCalls,     // Corresponds to OpenAI "tool_calls"
    ContentFilter, // Corresponds to OpenAI "content_filter"
    Complete,      // Generic completion, can be used if specific reason is not critical
    Error,         // Internal error state (not directly related to OpenAI)
}

impl Display for FinishReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FinishReason::Length => write!(f, "length"),
            FinishReason::ToolCalls => write!(f, "tool_calls"),
            FinishReason::ContentFilter => write!(f, "content_filter"),
            FinishReason::Stop | FinishReason::Complete | FinishReason::Error => write!(f, "stop"),
        }
    }
}

/// ChatResponse represents a response for fontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatResponse {
    pub chat_id: String,
    pub chunk: String,
    pub r#type: MessageType,
    pub metadata: Option<Value>,
    pub finish_reason: Option<FinishReason>,
}

impl ChatResponse {
    pub fn new_with_arc(
        chat_id: String,
        chunk: String,
        r#type: MessageType,
        metadata: Option<Value>,
        finish_reason: Option<FinishReason>,
    ) -> Arc<Self> {
        Arc::new(Self {
            chat_id,
            chunk,
            r#type,
            metadata,
            finish_reason,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(unused)]
pub struct Usage {
    pub total_tokens: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

// #[derive(Debug, Clone, Serialize, Deserialize, Default)]
// #[serde(rename_all = "camelCase")]
// pub struct ChatCompletionResult {
//     #[serde(skip_serializing_if = "Option::is_none")]
//     #[allow(unused)]
//     pub chat_id: Option<String>,

//     /// content
//     pub content: String,

//     /// reasoning
//     #[serde(skip_serializing_if = "Option::is_none")]
//     #[allow(unused)]
//     pub reasoning: Option<String>,

//     /// token usage
//     #[serde(skip_serializing_if = "Option::is_none")]
//     #[allow(unused)]
//     pub usage: Option<Usage>,

//     /// reference
//     #[serde(skip_serializing_if = "Option::is_none")]
//     #[allow(unused)]
//     pub reference: Option<Vec<SearchResult>>,

//     #[serde(skip_serializing_if = "Option::is_none")]
//     #[allow(unused)]
//     pub tools: Option<Vec<ToolCallDeclaration>>,

//     /// finish_reason
//     #[serde(skip_serializing_if = "Option::is_none")]
//     pub finish_reason: Option<FinishReason>,
// }

// =================================================
// Tool definition start
// =================================================

/// ToolCallDeclaration represents a tool call declaration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCallDeclaration {
    #[serde(skip_serializing, default)]
    pub index: u32,
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub results: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MCPToolDeclaration {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    #[serde(default, skip_serializing)]
    pub disabled: bool,
}

impl MCPToolDeclaration {
    /// Converts the tool declaration to standard JSON format
    ///
    /// # Returns
    /// Returns a `serde_json::Value` containing:
    /// - "name": Tool name as string
    /// - "description": Tool description as string
    /// - "parameters": Input schema as JSON value
    pub fn to_standard(&self) -> Value {
        json!({
            "name": &self.name,
            "description": &self.description,
            "parameters": &self.input_schema,
        })
    }

    /// Converts the tool declaration to OpenAI compatible format
    ///
    /// # Returns
    /// Returns a `serde_json::Value` containing:
    /// - "tool_type": Always "function"
    /// - "function": Standard tool declaration
    pub fn to_openai(&self) -> Value {
        json!({
            "type": "function",
            "function": self.to_standard(),
        })
    }

    /// Converts the tool declaration to Gemini compatible format
    ///
    /// # Returns
    /// Same as standard format (Gemini uses standard format)
    pub fn to_gemini(&self) -> Value {
        fn filter_gemini_schema(schema: &mut Value) {
            if let Some(obj) = schema.as_object_mut() {
                let mut filtered_obj = serde_json::Map::new();
                let allowed_keys = [
                    "type",
                    "description",
                    "properties",
                    "required",
                    "items",
                    "enum",
                ];

                for (key, value) in obj.iter() {
                    if allowed_keys.contains(&key.as_str()) {
                        let mut cloned_value = value.clone();
                        if key == "properties" {
                            if let Some(props_map) = cloned_value.as_object_mut() {
                                for (_, prop_schema) in props_map.iter_mut() {
                                    filter_gemini_schema(prop_schema);
                                }
                            }
                        } else if key == "items" {
                            // 'items' can be a schema object or an array of schemas (though less common for function params)
                            // For simplicity, handling the common case where 'items' is a single schema object.
                            filter_gemini_schema(&mut cloned_value);
                        }
                        filtered_obj.insert(key.clone(), cloned_value);
                    }
                }
                *obj = filtered_obj;
            }
        }

        let mut gemini_tool_declaration = self.to_standard();

        if let Some(parameters_val) = gemini_tool_declaration.get_mut("parameters") {
            // Create a mutable copy to pass to the filter function
            let mut mutable_parameters = parameters_val.clone();
            filter_gemini_schema(&mut mutable_parameters);
            // Replace the original parameters with the filtered ones
            if let Some(obj) = gemini_tool_declaration.as_object_mut() {
                obj.insert("parameters".to_string(), mutable_parameters);
            }
        }
        // Gemini API expects functionDeclarations to not have 'type: object' at the root of parameters if it's an object.
        // It directly expects properties. However, standard JSON schema for an object *does* have 'type: object'.
        // For now, let's assume the `filter_gemini_schema` handles the inner parts correctly.
        // If Gemini is very strict about the top-level `parameters` not having `type: "object"`,
        // further adjustment might be needed here based on exact API requirements.
        gemini_tool_declaration
    }

    pub fn get_input_schema(&self) -> &serde_json::Value {
        &self.input_schema
    }

    /// Converts the tool declaration to Claude compatible format
    ///
    /// # Returns
    /// Returns the raw serialized tool declaration
    pub fn to_claude(&self) -> Value {
        json!(self)
    }
}

// =================================================
// End of Tool definition
// =================================================

// =================================================
// Start of Chat trait
// =================================================

/// Generic struct containing model details
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelDetails {
    /// Unique identifier used for API calls
    /// Examples: "gpt-4-turbo", "gemini-1.5-pro-latest", "claude-3-opus-20240229"
    pub id: String,

    /// User-friendly display name
    /// Examples: "GPT-4 Turbo", "Gemini 1.5 Pro", "Claude 3 Opus"
    pub name: String,

    /// Model provider
    pub protocol: ChatProtocol,

    /// Maximum number of input tokens the model can handle
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<u32>,

    /// Maximum number of output tokens the model can generate
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,

    /// Brief description of the model or its capabilities
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Timestamp or date string of model creation/last update (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>, // Alternatively use chrono::DateTime<chrono::Utc>

    /// Model family or series (optional, e.g. "GPT-4", "Claude-3")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,

    /// reasoning support, like deepseek-r1
    pub reasoning: Option<bool>,

    /// function calling support
    pub function_call: Option<bool>,

    /// image support, like claude series
    pub image_input: Option<bool>,

    /// Additional metadata for provider-specific information
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[async_trait]
pub trait AiChatTrait: Send + Sync + Stoppable {
    /// Sends a chat request to the AI API and processes the response.
    ///
    /// # Arguments
    /// - `provider_id`: The ID of the AI API endpoint.
    /// - `model`: The model to be used for the chat.
    /// - `messages`: A vector of messages in the expected format for the AI API.
    ///   Each message is represented as a JSON object containing:
    ///   - `role`: A string indicating the role of the message sender.
    ///     Supported roles include:
    ///     - `user`: Represents the input message from the user.
    ///     - `assistant`: Represents the response message from the AI assistant.
    ///     - `system`: Used for system-level instructions or context setting.
    ///     - `tool`: Used for tool calls.
    ///   - `content`: A string containing the actual message content.
    /// - `tools`: An optional vector of tools to be used in the chat.
    ///     Each tool is defined as a `MCPToolDeclaration` object containing
    /// - `extra_params`: An optional extra parameters for the request.
    ///     - `max_tokens`: An optional maximum number of tokens to generate in the response.
    ///     - `stream`: A boolean indicating whether to stream the response. This parameter depends on whether the AI interface supports streaming responses.
    ///     - `temperature`: A float number indicating the temperature of the response. This parameter depends on the API support.
    ///     - `top_p`: A float number indicating the top_p of the response. This parameter depends on the API support.
    ///     - `top_k`: An integer indicating the top_k of the response. This parameter depends on the API support.
    ///     - `proxy_type`: A string indicating the proxy type. valid values are `system`, `http`, `none`.
    ///     - `proxy_server`: A string indicating the proxy server. like `http://127.0.0.1:7890`.
    ///     - Additional parameters that may be included in the callback function, like label.
    /// - `callback`: A callback function to handle streaming responses.
    ///     - `content`: `String`: The content of the message.
    ///     - `is_error`: `bool`: A boolean indicating whether the message is a error message.
    ///     - `is_finished`: `bool`: A boolean indicating whether the message is finished.
    ///     - `is_reasoning`: `bool`: A boolean indicating whether the message is a reasoning message.
    ///     - `r#type`: `String`: The type of the message.
    ///     - `Value`: Additional parameters that may be included in the callback function, like label.
    ///
    /// # Returns
    /// - A `Result` containing the full response as a `String` or an error if the request fails.
    async fn chat(
        &self,
        provider_id: i64,
        model: &str,
        chat_id: String,
        messages: Vec<Value>,
        tools: Option<Vec<MCPToolDeclaration>>,
        extra_params: Option<Value>,
        callback: impl Fn(Arc<ChatResponse>) + Send + 'static,
    ) -> Result<String, AiError>;

    /// Lists available models from the AI API provider.
    ///
    /// # Arguments
    /// - `api_protocol`: Protocol of the AI API, e.g. "OpenAi", "Gemini".
    /// - `api_url`: Optional URL of the AI API endpoint. If None, uses default endpoint.
    /// - `api_key`: Optional API key for authentication. Required if API needs authentication.
    /// - `extra_args`: Optional additional arguments for the request.
    ///   May include provider-specific parameters like:
    ///   - `organization`: Organization ID for some providers (e.g. OpenAI)
    ///   - `project`: Project ID for some providers (e.g. Google AI)
    ///   - Other provider-specific arguments
    ///
    /// # Returns
    /// - `Result<Vec<String>, AiError>` containing:
    ///   - On success: Vector of model IDs (e.g. ["gpt-4-turbo", "claude-3-opus"])
    ///   - On failure: `AiError` with details about what went wrong
    async fn list_models(
        &self,
        api_protocol: String,
        api_url: Option<&str>,
        api_key: Option<&str>,
        extra_args: Option<Value>,
    ) -> Result<Vec<ModelDetails>, AiError>;
}
