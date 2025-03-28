use rust_i18n::t;
use serde_json::{json, Value};
use std::str::FromStr;
use std::sync::Arc;
use std::{collections::HashMap, fmt::Display};
use tokio::sync::Mutex;

use crate::ai::interaction::constants::{TOKENS, TOKENS_COMPLETION, TOKENS_PROMPT, TOKENS_TOTAL};
use crate::ai::traits::chat::{ChatCompletionResult, MessageType, Usage};
use crate::http::chp::SearchResult;
use crate::{
    ai::{
        chat::{anthropic::AnthropicChat, gemini::GeminiChat, openai::OpenAIChat},
        traits::{
            chat::{AiChatTrait, ChatResponse},
            stoppable::Stoppable,
        },
    },
    libs::window_channels::WindowChannels,
};

use super::constants::{API_KEY_ROTATOR, BASE_URL};

/// Macro to initialize a HashMap of chat interfaces.
/// This macro takes key-value pairs and constructs a HashMap where keys are strings
/// and values are chat interface instances.
macro_rules! init_chats {
    ($($key:expr => $value:expr),+ $(,)?) => {{
        let mut chats = HashMap::new();
        $(
            chats.insert($key, $value);
        )+
        chats
    }};
}

/// Macro to implement a method for different chat interfaces.
/// This macro matches the current chat interface and calls the specified method
/// with the provided arguments, returning the result.
macro_rules! impl_chat_method {
    ($self:expr, $method:ident, $($arg:expr),*) => {
        match $self {
            AiChatEnum::Anthropic(chat) => chat.$method($($arg),*).await,
            AiChatEnum::Gemini(chat) => chat.$method($($arg),*).await,
            // AiChatEnum::HuggingFace(chat) => chat.$method($($arg),*).await,
            AiChatEnum::OpenAI(chat) => chat.$method($($arg),*).await,
        }
    };
}

/// Enum representing different types of AI chat interfaces.
#[derive(Clone)]
pub enum AiChatEnum {
    Anthropic(AnthropicChat),
    Gemini(GeminiChat),
    // HuggingFace(HuggingFaceChat),
    OpenAI(OpenAIChat),
}

/// Struct representing the state of the chat system, holding a collection of chat interfaces.
pub struct ChatState {
    pub chats: Arc<Mutex<HashMap<ChatProtocol, AiChatEnum>>>,
    pub channels: Arc<WindowChannels>,
}

impl ChatState {
    /// Creates a new instance of ChatState, initializing the available chat interfaces.
    ///
    /// # Returns
    /// A new instance of `ChatState` containing initialized chat interfaces.
    pub fn new(channels: Arc<WindowChannels>) -> Self {
        let chats = init_chats! {
            ChatProtocol::OpenAI => AiChatEnum::OpenAI(OpenAIChat::new()),
            ChatProtocol::Ollama => AiChatEnum::OpenAI(OpenAIChat::new()),
            ChatProtocol::HuggingFace => AiChatEnum::OpenAI(OpenAIChat::new()),
            ChatProtocol::Anthropic => AiChatEnum::Anthropic(AnthropicChat::new()),
            ChatProtocol::Gemini => AiChatEnum::Gemini(GeminiChat::new()),
        };

        Self {
            chats: Arc::new(Mutex::new(chats)),
            channels,
        }
    }
}

impl AiChatEnum {
    /// Asynchronously sends a chat request to the selected AI chat interface.
    ///
    /// # Arguments
    /// - `api_url`: Optional API URL for the chat request.
    /// - `model`: The model to be used for the chat.
    /// - `api_key`: Optional API key for authentication.
    /// - `chat_id`: Unique identifier for this chat session
    /// - `messages`: A vector of messages to be sent in the chat.
    /// - `extra_params`: Optional extra parameters for the chat.
    /// - `callback`: A callback function to handle response chunks.
    ///
    /// # Returns
    /// A `Result` containing the response string or an error.
    pub async fn chat(
        &self,
        api_url: Option<&str>,
        model: &str,
        api_key: Option<&str>,
        chat_id: String,
        messages: Vec<Value>,
        extra_params: Option<Value>,
        callback: impl Fn(Arc<ChatResponse>) + Send + 'static,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        impl_chat_method!(
            self,
            chat,
            api_url,
            model,
            api_key,
            chat_id,
            messages,
            extra_params,
            callback
        )
    }

    /// Asynchronously sets a stop flag for the selected chat interface.
    ///
    /// # Arguments
    /// - `value`: A boolean indicating whether to stop the chat.
    pub async fn set_stop_flag(&self, value: bool) {
        impl_chat_method!(self, set_stop_flag, value)
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum ChatProtocol {
    OpenAI,
    Anthropic,
    Gemini,
    Ollama,
    HuggingFace,
}

impl Display for ChatProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                ChatProtocol::OpenAI => "openai",
                ChatProtocol::Anthropic => "anthropic",
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
            "anthropic" => Ok(ChatProtocol::Anthropic),
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

/// Executes an AI chat request with the given parameters.
///
/// This function encapsulates the core logic for interacting with AI chat interfaces.
/// It can be used by both the Tauri command and workflow tools.
///
/// # Arguments
/// * `state` - The chat state containing chat interfaces
/// * `main_state` - The main application state
/// * `api_protocol` - The API provider to use (e.g., "openai", "anthropic")
/// * `api_url` - Optional API URL override
/// * `model` - The model name to use, e.g., "deepseek-r1"
/// * `api_key` - Optional API key override
/// * `chat_id` - Unique identifier for this chat session
/// * `messages` - Vector of chat messages
/// * `metadata` - Optional additional parameters for the chat
/// * `callback` - Callback function to handle chat responses
///
/// # Returns
/// * `Result<(), String>` - Ok if the chat was started successfully, Err with error message otherwise
pub async fn complete_chat_async(
    chat_state: &ChatState,
    chat_protocol: ChatProtocol,
    api_url: Option<&str>,
    model: String,
    api_key: Option<&str>,
    chat_id: String,
    messages: Vec<Value>,
    mut metadata: Option<Value>,
    callback: impl Fn(Arc<ChatResponse>) + Send + 'static,
) -> Result<(), String> {
    let mut api_url_clone = if api_url == Some("") || api_url.is_none() {
        if chat_protocol == ChatProtocol::OpenAI {
            if let Some(base_url) = BASE_URL.get(&chat_protocol.to_string()) {
                Some(base_url.to_string())
            } else {
                None
            }
        } else {
            None
        }
    } else {
        api_url.map(|s| s.to_string())
    };

    if chat_protocol == ChatProtocol::HuggingFace {
        // huggingface requires `top_p` must be > 0.0 and < 1.0
        if let Some(md) = metadata.as_mut() {
            if let Some(topp) = md["top_p"].as_f64() {
                if topp == 1.0 {
                    md["top_p"] = json!(0.99);
                }
            }
        }
        api_url_clone = get_hf_base_url(api_url_clone.as_deref(), model.as_str());
    }

    // if api_key has \n, then use ApiKeyRotator
    let mut api_key_clone = api_key.map(|s| s.to_string());
    if let Some(key) = api_key_clone.as_mut() {
        if key.contains('\n') {
            let keys = key
                .split('\n')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<String>>();
            api_key_clone = API_KEY_ROTATOR
                .get_next_key(&api_url_clone.clone().unwrap_or_default(), keys)
                .await;
        }
    }

    let chats = chat_state.chats.lock().await;
    match chats.get(&chat_protocol).cloned() {
        Some(chat_interface) => {
            let _ = tokio::spawn(async move {
                // reset stop flag
                chat_interface.set_stop_flag(false).await;
                chat_interface
                    .chat(
                        api_url_clone.as_deref(),
                        &model,
                        api_key_clone.as_deref(),
                        chat_id,
                        messages,
                        metadata,
                        callback,
                    )
                    .await
                    .map_err(|e| e.to_string())
            });

            Ok(())
        }
        None => Err(t!("chat.invalid_api_provider", provider = chat_protocol).to_string()),
    }
}

/// Get the base URL for HuggingFace
///
/// # Arguments
/// * `input_url` - The input URL
///
/// # Returns
/// * `Option<String>` - The base URL for HuggingFace
fn get_hf_base_url(input_url: Option<&str>, model: &str) -> Option<String> {
    let base_url = input_url
        .and_then(|url| url.split_once("/hf-inference/models"))
        .map(|(base, _)| format!("{}/hf-inference/models/{model}/v1", base))
        .unwrap_or_else(|| format!("https://router.huggingface.co/hf-inference/models/{model}/v1"));
    Some(base_url)
}

/// Asynchronously sends a chat request to the selected AI chat interface.
/// It's a blocking version of `complete_chat_async`. The chat response will
/// be returned after the chat is finished.
///
/// # Arguments
/// * `state` - The chat state containing chat interfaces
/// * `chat_protocol` - The chat protocol to use
/// * `api_url` - Optional API URL override
/// * `model` - The model name to use
/// * `api_key` - Optional API key override
/// * `chat_id` - Unique identifier for this chat session
/// * `messages` - Vector of chat messages
/// * `metadata` - Optional additional parameters for the chat
///
/// # Returns
/// * `Result<String, String>` - Ok with the chat response, Err with error message
pub async fn complete_chat_blocking(
    state: &ChatState,
    chat_protocol: ChatProtocol,
    api_url: Option<&str>,
    model: String,
    api_key: Option<&str>,
    chat_id: String,
    messages: Vec<Value>,
    metadata: Option<Value>,
) -> Result<ChatCompletionResult, String> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Arc<ChatResponse>>(100);

    let callback = move |chunk: Arc<ChatResponse>| {
        let tx = tx.clone();
        if let Err(e) = tx.try_send(chunk) {
            log::error!("Failed to send chat response through channel: {}", e);
        }
    };

    // Execute the chat
    complete_chat_async(
        state,
        chat_protocol,
        api_url,
        model,
        api_key,
        chat_id.clone(),
        messages,
        metadata,
        callback,
    )
    .await?;

    // Receive and process results
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut reference = Vec::new();
    let mut usage = None;
    while let Some(chunk) = rx.recv().await {
        if chunk.chat_id == chat_id {
            match chunk.r#type {
                MessageType::Text => content.push_str(&chunk.chunk),
                // If necessary, you can keep the reasoning content and references
                MessageType::Reasoning | MessageType::Think => reasoning.push_str(&chunk.chunk),
                MessageType::Reference => {
                    if let Ok(parsed_chunk) =
                        serde_json::from_str::<Vec<SearchResult>>(&chunk.chunk)
                            .map_err(|e| e.to_string())
                    {
                        reference.extend(parsed_chunk);
                    }
                }
                MessageType::Error => return Err(chunk.chunk.clone()),
                MessageType::Finished => {
                    if let Some(metadata) = chunk.metadata.as_ref() {
                        if let Some(tokens) = metadata.get(TOKENS) {
                            if let Some(tokens) = tokens.as_object() {
                                usage = Some(Usage {
                                    prompt_tokens: tokens
                                        .get(TOKENS_PROMPT)
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or_default(),
                                    completion_tokens: tokens
                                        .get(TOKENS_COMPLETION)
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or_default(),
                                    total_tokens: tokens
                                        .get(TOKENS_TOTAL)
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or_default(),
                                });
                            }
                        }
                    }
                    break;
                }
                _ => {}
            }
        }
    }
    content = content.trim().to_string();
    if content.starts_with("<think>") && content.contains("</think>") {
        if let Some((r, c)) = content.split_once("</think>") {
            if reasoning.is_empty() {
                reasoning = r.trim_start_matches("<think>").trim().to_string();
            }
            content = c.trim().to_string();
        }
    }

    Ok(ChatCompletionResult {
        chat_id: Some(chat_id),
        content,
        reasoning: Some(reasoning),
        reference: Some(reference),
        usage,
    })
}
