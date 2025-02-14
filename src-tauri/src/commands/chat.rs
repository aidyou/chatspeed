//! # AI Commands
//!
//! This module provides Tauri commands for interacting with AI chat interfaces.
//! It allows sending messages to different AI providers, receiving responses,
//! and stopping ongoing chats.
//!
//! ## Usage
//! To use the AI commands, you need to call the `chat_with_ai` and `stop_chat` functions.
//!
//! ### Example
//!
//! #### Chat with AI
//! ```js
//! import { invoke } from '@tauri-apps/api/core'
//!
//! const result = await invoke('chat_with_ai', {
//!     apiProtocol: 'openai',
//!     apiUrl: 'https://api.openai.com/v1/chat/completions',
//!     model: 'gpt-3.5-turbo',
//!     apiKey: 'your-api-key',
//!     messages: [
//!         { role: 'system', content: 'your prompt here' },
//!         { role: 'user', content: 'Hello, how are you?' }
//!     ],
//!     maxTokens: 100
//! })
//! window.addEventListener('ai_chunk', (event) => {
//!   console.log('ai_chunk', event)
//! })
//! ```
//!
//! #### Stop Chat
//! ```js
//! import { invoke } from '@tauri-apps/api/core'
//!
//! const result = await invoke('stop_chat', { apiProtocol: 'openai' })
//! ```
//!
//! ## How to add new AI providers
//!
//! - To add a new AI provider, you need to implement the `AiChatTrait` and `Stoppable` traits for the provider.
//!   You can use the existing providers as examples.
//! - Add new methods to the `AiChatEnum` enum.
//! - Then add new states to the `ChatState`::new method.
//! - Finally, add the new AI provider to the `init_chats!` macro.

use crate::ai::chat::{anthropic::AnthropicChat, gemini::GeminiChat, openai::OpenAIChat};
use crate::ai::traits::{chat::AiChatTrait, stoppable::Stoppable};
use crate::db::MainStore;
use crate::libs::lang::{get_available_lang, lang_to_iso_639_1};
use crate::libs::window_channels::WindowChannels;
use rust_i18n::t;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::State;
use tokio::sync::Mutex;
use whatlang::detect;

/// Macro to initialize a HashMap of chat interfaces.
/// This macro takes key-value pairs and constructs a HashMap where keys are strings
/// and values are chat interface instances.
macro_rules! init_chats {
    ($($key:expr => $value:expr),+ $(,)?) => {{
        let mut chats = HashMap::new();
        $(
            chats.insert($key.to_string(), $value);
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
enum AiChatEnum {
    Anthropic(AnthropicChat),
    Gemini(GeminiChat),
    // HuggingFace(HuggingFaceChat),
    OpenAI(OpenAIChat),
}

/// Struct representing the state of the chat system, holding a collection of chat interfaces.
pub struct ChatState {
    chats: Arc<Mutex<HashMap<String, AiChatEnum>>>,
    channels: Arc<WindowChannels>,
}

impl ChatState {
    /// Creates a new instance of ChatState, initializing the available chat interfaces.
    ///
    /// # Returns
    /// A new instance of `ChatState` containing initialized chat interfaces.
    pub fn new(channels: Arc<WindowChannels>) -> Self {
        let chats = init_chats! {
            "openai" => AiChatEnum::OpenAI(OpenAIChat::new()),
            "ollama" => AiChatEnum::OpenAI(OpenAIChat::new()),
            "anthropic" => AiChatEnum::Anthropic(AnthropicChat::new()),
            "gemini" => AiChatEnum::Gemini(GeminiChat::new()),
            // "huggingface" => AiChatEnum::HuggingFace(HuggingFaceChat::new()),
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
    /// # Parameters
    /// - `api_url`: Optional API URL for the chat request.
    /// - `model`: The model to be used for the chat.
    /// - `api_key`: Optional API key for authentication.
    /// - `messages`: A vector of messages to be sent in the chat.
    /// - `max_tokens`: Optional maximum number of tokens for the response.
    /// - `extra_params`: Optional extra parameters for the chat.
    /// - `callback`: A callback function to handle response chunks.
    ///
    /// # Returns
    /// A `Result` containing the response string or an error.
    async fn chat(
        &self,
        api_url: Option<&str>,
        model: &str,
        api_key: Option<&str>,
        messages: Vec<Value>,
        extra_params: Option<Value>,
        callback: impl Fn(String, bool, bool, bool, Option<Value>) + Send + 'static,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        impl_chat_method!(
            self,
            chat,
            api_url,
            model,
            api_key,
            messages,
            extra_params,
            callback
        )
    }

    /// Asynchronously sets a stop flag for the selected chat interface.
    ///
    /// # Parameters
    /// - `value`: A boolean indicating whether to stop the chat.
    async fn set_stop_flag(&self, value: bool) {
        impl_chat_method!(self, set_stop_flag, value)
    }
}

/// Tauri command to interact with the AI chat system.
/// This command handles sending messages to the AI and receiving responses.
///
/// # Parameters
/// - `window` - The Tauri window instance, automatically injected by Tauri
/// - `state` - The state of the chat system, automatically injected by Tauri
/// - `api_protocol` - The API provider to use for the chat.
/// - `api_url` - The API URL to use for the chat.
/// - `model` - The model to use for the chat.
/// - `api_key` - The API key to use for the chat.
/// - `messages` - The messages to send to the chat.
/// - `extra_params` - Optional extra parameters for the chat.
///
/// # Returns
/// A `Result` containing the response string or an error message.
///
/// # Example
/// ```js
/// import { invoke } from '@tauri-apps/api/core'
///
/// const result = await invoke('chat_with_ai', {
///     apiProtocol: 'openai',
///     apiUrl: 'https://api.openai.com/v1/chat/completions',
///     model: 'gpt-3.5-turbo',
///     apiKey: 'your-api-key',
///     messages: [
///         {role: 'system', content: 'your prompt here'},
///         { role: 'user', content: 'Hello, how are you?' }
///     ],
///     maxTokens: 100
/// })
/// window.addEventListener('ai_chunk', (event) => {
///   console.log('ai_chunk', event)
/// })
/// ```
#[tauri::command]
pub async fn chat_with_ai(
    window: tauri::Window, // The Tauri window instance, automatically injected by Tauri
    state: State<'_, ChatState>, // The state of the chat system, automatically injected by Tauri
    main_state: State<'_, Arc<std::sync::Mutex<MainStore>>>,
    api_protocol: String,
    api_url: Option<&str>,
    model: String,
    api_key: Option<&str>,
    messages: Vec<Value>,
    mut metadata: Option<Value>,
) -> Result<String, String> {
    let tx = state.channels.get_or_create_channel(window.clone()).await?;

    let callback = move |chunk: String,
                         is_error: bool,
                         is_done: bool,
                         is_reasoning: bool,
                         cb_metadata: Option<Value>| {
        let _ = tx.try_send((chunk, is_error, is_done, is_reasoning, cb_metadata));
    };

    // If the proxy type is http, get the proxy server and username/password from the config
    if let Some(md) = metadata.as_mut() {
        if let Some("http") = md.get("proxyType").and_then(Value::as_str) {
            let config_store = main_state.lock().map_err(|e| e.to_string())?;
            let proxy_server = config_store.get_config("proxy_server", String::new());
            if !proxy_server.is_empty() {
                md["proxyServer"] = json!(proxy_server);
                md["proxyUsername"] =
                    json!(config_store.get_config("proxy_username", String::new()));
                md["proxyPassword"] =
                    json!(config_store.get_config("proxy_password", String::new()));
            }
        }
    }

    let chats = state.chats.lock().await;
    match chats.get(&api_protocol).cloned() {
        Some(chat_interface) => {
            // Extract data needed for the async task
            let api_url_clone = if api_url == Some("") || api_url.is_none() {
                // Set the Api url base for ollama
                if api_protocol == "ollama" {
                    Some("http://localhost:11434/v1/chat/completions".to_string())
                } else {
                    None
                }
            } else {
                api_url.map(|s| s.to_string())
            };
            let api_key_clone = api_key.map(|s| s.to_string());

            tokio::spawn(async move {
                // reset stop flag
                chat_interface.set_stop_flag(false).await;
                chat_interface
                    .chat(
                        api_url_clone.as_deref(),
                        &model,
                        api_key_clone.as_deref(),
                        messages,
                        metadata,
                        callback,
                    )
                    .await
                    .map_err(|e| e.to_string())
            });
        }
        None => {
            return Err(t!("chat.invalid_api_provider", provider = api_protocol).to_string());
        }
    }

    Ok("".to_string())
}

/// Tauri command to stop the ongoing chat for a specific API provider.
/// This command sets the stop flag for the selected chat interface.
///
/// # Parameters
/// - `state` - The state of the chat system, automatically injected by Tauri
/// - `api_protocol` - The API provider for which to stop the chat.
///
/// # Returns
/// A `Result` indicating success or an error message.
///
/// # Example
/// ```js
/// import { invoke } from '@tauri-apps/api/core'
///
/// const result = await invoke('stop_chat', { apiProtocol: 'openai' })
/// ```
#[tauri::command]
pub async fn stop_chat(state: State<'_, ChatState>, api_protocol: String) -> Result<(), String> {
    // Lock the chat state to access the chat interfaces.
    let chats = state.chats.lock().await;
    let chat_interface = chats.get(&api_protocol);

    match chat_interface {
        Some(chat_enum) => {
            chat_enum.set_stop_flag(true).await; // Set the stop flag to true.
            Ok(()) // Return success.
        }
        None => Err(t!("chat.invalid_api_provider", provider = api_protocol).to_string()), // Handle invalid API provider case.
    }
}

/// Detects the language of a given text and returns the corresponding language code.
///
/// # Parameters
/// - `text` - The text to detect the language of.
///
/// # Returns
/// A `Result` containing the language code or an error message.
#[tauri::command]
pub fn detect_language(text: &str) -> Result<Value, String> {
    let detected_lang = detect(text);

    if let Some(info) = detected_lang {
        let languages = get_available_lang().map_err(|e| e.to_string())?;
        let lang_code = lang_to_iso_639_1(&info.lang().code()).map_err(|e| e.to_string())?;

        if let Some(lang_name) = languages.get(lang_code) {
            Ok(json!({ "lang": lang_name.to_string(), "code": lang_code }))
        } else {
            Ok(json!({ "lang": info.lang().name().to_string(), "code": lang_code }))
        }
    } else {
        Err(t!("chat.failed_to_detect_language").to_string())
    }
}
