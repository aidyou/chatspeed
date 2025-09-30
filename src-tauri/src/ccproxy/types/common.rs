use indexmap::IndexMap;
use std::{
    collections::HashMap,
    fmt::{self, Display},
    str::FromStr,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
pub type ChatCompletionProxyConfig = HashMap<String, IndexMap<String, Vec<BackendModelTarget>>>;

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

/// Represents different types of stream response formats
pub enum StreamFormat {
    /// OpenAI compatible format
    /// data: {"choices":[{"delta":{"content":"Hello"},"index":0}]}
    OpenAI,

    /// Google AI (Gemini) format
    /// {"candidates":[{"content":{"parts":[{"text":"Hello"}],"role":"model"},"index":0}]}
    Gemini,

    /// Claude format
    /// {"completion":"Hello","usage":{"input_tokens":10,"output_tokens":10}}
    Claude,
}

impl fmt::Debug for StreamFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OpenAI => write!(f, "StreamFormat::OpenAI"),
            Self::Gemini => write!(f, "StreamFormat::Gemini"),
            Self::Claude => write!(f, "StreamFormat::Claude"),
        }
    }
}

pub struct ProxyModel {
    pub provider: String,
    pub chat_protocol: ChatProtocol,
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub metadata: Option<Value>,
    pub prompt_injection: String,
    pub prompt_injection_position: Option<String>,
    pub prompt_text: String,
    pub tool_filter: HashMap<String, i8>,
    // ratio of the temperature
    pub temperature: f32,
    // pub max_context: usize,
}

//======================================================
// Chat Protocol
//======================================================

#[derive(Debug, Clone, Deserialize, Serialize, Hash, PartialEq, Eq)]
pub enum ChatProtocol {
    OpenAI,
    Claude,
    Gemini,
    Ollama,
    HuggingFace,
}

impl Default for ChatProtocol {
    fn default() -> Self {
        ChatProtocol::OpenAI
    }
}

impl Display for ChatProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                ChatProtocol::OpenAI => "openai",
                ChatProtocol::Claude => "claude",
                ChatProtocol::Gemini => "gemini",
                ChatProtocol::Ollama => "ollama",
                ChatProtocol::HuggingFace => "huggingface",
            }
        )
    }
}

impl FromStr for ChatProtocol {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "openai" => Ok(ChatProtocol::OpenAI),
            "claude" => Ok(ChatProtocol::Claude),
            "gemini" => Ok(ChatProtocol::Gemini),
            "ollama" => Ok(ChatProtocol::Ollama),
            "huggingface" => Ok(ChatProtocol::HuggingFace),
            _ => Err(format!("Invalid AiProtocol: {}", s)),
        }
    }
}

impl TryFrom<String> for ChatProtocol {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}
