use reqwest::Response;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

/// Represents different types of proxy configurations
#[derive(Debug, Clone)]
pub enum ProxyType {
    None,
    System,
    Http(String, Option<String>, Option<String>), // Http(server, username, password)
}

impl From<ProxyType> for String {
    fn from(proxy_type: ProxyType) -> Self {
        match proxy_type {
            ProxyType::None => "none".to_string(),
            ProxyType::System => "system".to_string(),
            ProxyType::Http(server, username, password) => {
                if username.is_some() && password.is_some() {
                    format!(
                        "http://{}:{}@{}",
                        username.unwrap_or_default(),
                        password.unwrap_or_default(),
                        server
                    )
                } else {
                    format!("http://{}", server)
                }
            }
        }
    }
}
// 不能实现From<ProxyType> for &str，因为&str是引用类型，需要指定生命周期

/// Configuration for API requests
#[derive(Debug, Clone)]
pub struct ApiConfig {
    pub api_url: Option<String>,
    pub api_key: Option<String>,
    pub proxy_type: ProxyType,
    pub headers: Option<Value>,
}

impl ApiConfig {
    /// Creates a new ApiConfig with minimal required parameters
    ///
    /// # Example
    /// ```no_run
    /// let config = ApiConfig::new(
    ///     Some("https://api.example.com"),
    ///     Some("your-api-key"),
    ///     ProxyType::None
    ///     Some(json!({ "Authorization": "Bearer your-token" }))
    /// );
    /// ```
    pub fn new(
        api_url: Option<String>,
        api_key: Option<String>,
        proxy_type: ProxyType,
        headers: Option<Value>,
    ) -> Self {
        Self {
            api_url,
            api_key,
            proxy_type,
            headers,
        }
    }
}

/// Response wrapper for API calls
#[derive(Debug)]
pub struct ApiResponse {
    /// The response content
    pub content: String,
    /// Indicates if this is an error message
    pub is_error: bool,
    /// Raw response for stream processing
    pub raw_response: Option<Response>,
}

impl ApiResponse {
    /// Creates a new successful response
    pub fn success(content: String) -> Self {
        Self {
            content,
            is_error: false,
            raw_response: None,
        }
    }

    /// Creates a new successful stream response
    pub fn success_stream(response: Response) -> Self {
        Self {
            content: String::new(),
            is_error: false,
            raw_response: Some(response),
        }
    }

    /// Creates a new error response
    pub fn error(message: String) -> Self {
        Self {
            content: message,
            is_error: true,
            raw_response: None,
        }
    }
}

/// Represents different error response formats
pub enum ErrorFormat {
    /// OpenAI format
    OpenAI,
    /// Claude format
    Claude,
    /// Google format
    Google,
    /// Custom format with user-provided parser
    Custom(Box<dyn Fn(&str) -> Option<(String, String)> + Send + Sync>),
}

impl Clone for ErrorFormat {
    fn clone(&self) -> Self {
        match self {
            Self::OpenAI => Self::OpenAI,
            Self::Claude => Self::Claude,
            Self::Google => Self::Google,
            Self::Custom(_) => Self::Custom(Box::new(|_s| None)), // 提供一个默认的空解析器
        }
    }
}

impl fmt::Debug for ErrorFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OpenAI => write!(f, "ErrorFormat::OpenAI"),
            Self::Claude => write!(f, "ErrorFormat::Claude"),
            Self::Google => write!(f, "ErrorFormat::Google"),
            Self::Custom(_) => write!(f, "ErrorFormat::Custom(<function>)"),
        }
    }
}

impl Default for ErrorFormat {
    fn default() -> Self {
        Self::OpenAI
    }
}

impl ErrorFormat {
    /// Parse error message from response
    ///
    /// Returns (error_type, error_message) if parsing succeeds
    pub fn parse_error(&self, error_text: &str) -> Option<(String, String)> {
        match self {
            Self::OpenAI | Self::Claude => {
                if let Ok(json) = serde_json::from_str::<Value>(error_text) {
                    if let Some(error) = json.get("error") {
                        return Some((
                            error["type"].as_str().unwrap_or("").to_string(),
                            error["message"].as_str().unwrap_or(error_text).to_string(),
                        ));
                    }
                    if let Some(errors) = json.get("errors") {
                        return Some((
                            errors["type"].as_str().unwrap_or("").to_string(),
                            errors["message"].as_str().unwrap_or(error_text).to_string(),
                        ));
                    }
                    None
                } else {
                    None
                }
            }
            Self::Google => {
                if let Ok(json) = serde_json::from_str::<Value>(error_text) {
                    json.get("error").and_then(|error| {
                        Some((
                            error["status"].as_str().unwrap_or("").to_string(),
                            error["message"].as_str().unwrap_or(error_text).to_string(),
                        ))
                    })
                } else {
                    None
                }
            }
            Self::Custom(parser) => parser(error_text),
        }
    }
}

// =================================================
// ================== Response struct ==============
// =================================================

// =================================================
// Gemini response struct
// =================================================
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeminiResponse {
    pub candidates: Option<Vec<Candidate>>,
    pub usage_metadata: Option<UsageMetadata>,
    #[serde(default, rename = "promptFeedback")]
    pub prompt_feedback: Option<PromptFeedback>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageMetadata {
    pub prompt_token_count: u64,
    pub total_token_count: u64,
    #[serde(default)] // In case this field is not always present in all Gemini responses
    pub candidates_token_count: Option<u64>,
}

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PromptFeedback {
    #[serde(default)]
    pub block_reason: Option<String>,
    pub safety_ratings: Vec<SafetyRating>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SafetyRating {
    pub category: String,
    pub probability: String, // e.g., "NEGLIGIBLE", "LOW", "MEDIUM", "HIGH"
}

#[derive(Debug, Deserialize)]
pub struct Candidate {
    pub content: Content,
    #[serde(default, rename = "finishReason")]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Content {
    pub parts: Option<Vec<Part>>,
    #[serde(default)] // Add default in case role is not always present
    pub role: Option<String>, // Added field for role
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Part {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default, rename = "functionCall")]
    pub function_call: Option<GeminiFunctionCall>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GeminiFunctionCall {
    pub name: String,
    pub args: Value, // Gemini args are typically a JSON object
}

// =================================================
// OpenAI compatible response format
// =================================================
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAIStreamResponse {
    #[serde(default)]
    pub choices: Vec<OpenAIStreamChoice>,
    pub usage: Option<OpenAIUsage>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIStreamChoice {
    pub delta: OpenAIStreamDelta,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIStreamDelta {
    pub content: Option<String>,
    pub role: Option<String>,
    pub reasoning_content: Option<String>,
    #[serde(rename = "type")]
    pub msg_type: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCall>>,
}

#[derive(Debug, Deserialize)]
pub struct ToolCall {
    pub index: u32,
    pub id: Option<String>,
    pub function: ToolFunction,
}

#[derive(Debug, Deserialize)]
pub struct ToolFunction {
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIUsage {
    pub total_tokens: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

// =================================================
// Claude stream event
// =================================================

/// Claude stream event types
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeEventType {
    MessageStart,
    ContentBlockStart,
    ContentBlockDelta,
    ContentBlockStop,
    MessageDelta,
    MessageStop,
    Ping,
    Error,
}

/// Claude stream event
#[derive(Debug, Deserialize)]
pub struct ClaudeStreamEvent {
    #[serde(rename = "type")]
    pub event_type: ClaudeEventType,
    /// Content block object (only available in content_block_start events)
    #[serde(default)]
    pub content_block: Option<ClaudeContentBlock>,
    #[serde(default)]
    pub index: Option<u32>,
    #[serde(default)]
    pub delta: Option<Value>,
    #[serde(default)]
    pub error: Option<Value>,
    #[serde(default)]
    pub usage: Option<Value>,
    /// Full message object (only available in message_start/message_stop events)
    #[serde(default)]
    pub message: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ClaudeContentBlock {
    #[serde(rename = "type")]
    pub block_type: String, // "text" or "tool_use"
    #[serde(default)]
    pub text: Option<String>, // For text blocks
    #[serde(default)]
    pub id: Option<String>, // For tool_use blocks
    #[serde(default)]
    pub name: Option<String>, // For tool_use blocks
    #[serde(default)]
    pub input: Option<Value>, // For tool_use blocks (initial empty object)
}
