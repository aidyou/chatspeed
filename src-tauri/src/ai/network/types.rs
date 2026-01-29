use reqwest::Response;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
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

#[derive(Serialize)]
pub struct ResponseError {
    status_code: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_type: Option<String>,
    message: String,
}

impl ResponseError {
    pub fn new(status_code: Option<u16>, error_type: Option<String>, message: String) -> Self {
        Self {
            status_code,
            error_type,
            message,
        }
    }
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
    pub fn error(error: ResponseError) -> Self {
        Self {
            content: json!(error).to_string(),
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
            Self::Custom(_) => Self::Custom(Box::new(|_s| None)), // Provide a default empty parser
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
    #[serde(default)]
    pub reference: Option<String>,
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
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub prompt_tokens: u64,
    #[serde(default)]
    pub completion_tokens: u64,
}
