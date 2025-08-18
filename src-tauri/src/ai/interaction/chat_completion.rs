use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::str::FromStr;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::{collections::HashMap, fmt::Display};
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex; // Removed broadcast as it's not used in this file directly

use crate::ai::error::AiError;
use crate::ai::interaction::constants::{TOKENS, TOKENS_COMPLETION, TOKENS_PROMPT, TOKENS_TOTAL};
use crate::ai::traits::chat::{
    ChatCompletionResult, MCPToolDeclaration, MessageType, ModelDetails, ToolCallDeclaration, Usage,
};
use crate::search::SearchResult;
use crate::tools::ToolManager;
use crate::{
    ai::{
        chat::{claude::ClaudeChat, gemini::GeminiChat, openai::OpenAIChat},
        traits::{
            chat::FinishReason,
            chat::{AiChatTrait, ChatResponse},
            stoppable::Stoppable,
        },
    },
    libs::window_channels::WindowChannels,
};
use tokio::sync::mpsc::{self, Receiver, Sender};

use super::constants::{API_KEY_ROTATOR, BASE_URL};

/// Macro for initializing a nested HashMap structure for chat interfaces
///
/// Creates a two-level HashMap where:
/// - First level keys are ChatProtocol variants
/// - Second level maps chat IDs to their respective instances
///
/// # Parameters
/// - `$key`: One or more ChatProtocol variants to initialize
///
/// # Returns
/// Returns a HashMap<ChatProtocol, HashMap<String, AiChatEnum>> structure
///
/// # Example
/// ```no_run
/// let chats = init_chats! {
///     ChatProtocol::OpenAI,
///     ChatProtocol::Claude,
///     ChatProtocol::Gemini
/// };
///
/// assert!(chats.contains_key(&ChatProtocol::OpenAI));
/// assert!(chats.contains_key(&ChatProtocol::Claude));
/// assert!(chats.contains_key(&ChatProtocol::Gemini));
/// ```
macro_rules! init_chats {
    ($($key:expr),+ $(,)?) => {{
        let mut chats = HashMap::new();
        $(
            chats.insert($key, HashMap::new());
        )+
        chats
    }};
}

/// Macro for creating a new chat interface instance based on protocol.
///
/// # Parameters
/// - `$protocol`: Chat protocol type (Type: `ChatProtocol`)
///
/// # Returns
/// A new instance of the chat interface (Type: `AiChatEnum`)
#[macro_export]
macro_rules! create_chat {
    ($protocol:expr) => {
        match $protocol {
            ChatProtocol::OpenAI | ChatProtocol::Ollama | ChatProtocol::HuggingFace => {
                AiChatEnum::OpenAI(OpenAIChat::new())
            }
            ChatProtocol::Claude => AiChatEnum::Claude(ClaudeChat::new()),
            ChatProtocol::Gemini => AiChatEnum::Gemini(GeminiChat::new()),
        }
    };
}

/// Macro for getting or creating a chat interface instance
///
/// This macro provides thread-safe lazy initialization of chat interface instances.
/// It follows the pattern: Protocol → ChatID → Instance
///
/// # Parameters
/// - `$chats`: HashMap containing chat instances (Type: `HashMap<ChatProtocol, HashMap<String, AiChatEnum>>`)
/// - `$protocol`: Chat protocol type (Type: `ChatProtocol`)
/// - `$chat_id`: Unique session identifier (Type: `String`)
///
/// # Returns
/// Cloned instance of the chat interface (Type: `AiChatEnum`)
///
/// # Behavior
/// 1. Gets or creates protocol-level HashMap
/// 2. Gets or creates chat instance using the provided ID
/// 3. Returns a cloned reference to the instance
///
/// # Example
/// ```no_run
/// let mut chats = HashMap::new();
/// let protocol = ChatProtocol::OpenAI;
/// let chat_id = "session_123".to_string();
///
/// // First call creates new instance
/// let chat1 = get_or_create_chat!(chats, protocol, chat_id);
///
/// // Subsequent calls return same instance
/// let chat2 = get_or_create_chat!(chats, protocol, chat_id);
///
/// assert!(std::ptr::eq(&chat1, &chat2));
/// ```
macro_rules! get_or_create_chat {
    ($chats:expr, $protocol:expr, $chat_id:expr) => {{
        $chats
            .entry($protocol.clone())
            .or_insert_with(HashMap::new)
            .entry($chat_id.clone())
            .or_insert_with(|| create_chat!($protocol))
            .clone()
    }};
}

/// Macro to implement a method for different chat interfaces.
/// This macro matches the current chat interface and calls the specified method
/// with the provided arguments, returning the result.
macro_rules! impl_chat_method {
    ($self:expr, $method:ident, $($arg:expr),*) => {
        match $self {
            AiChatEnum::Claude(chat) => chat.$method($($arg),*).await,
            AiChatEnum::Gemini(chat) => chat.$method($($arg),*).await,
            AiChatEnum::OpenAI(chat) => chat.$method($($arg),*).await,
        }
    };
}

/// Enum representing different types of AI chat interfaces.
#[derive(Clone)]
pub enum AiChatEnum {
    Claude(ClaudeChat),
    Gemini(GeminiChat),
    OpenAI(OpenAIChat),
}

/// Struct representing the state of the chat system, holding a collection of chat interfaces.
/// 协议类型 -> 聊天ID -> 聊天实例
/// Protocol -> Chat ID -> Chat instance
pub struct ChatState {
    pub chats: Arc<Mutex<HashMap<ChatProtocol, HashMap<String, AiChatEnum>>>>,
    // 新增深度搜索状态跟踪
    // New depth search state tracking
    pub active_searches: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    // channels for sending and receiving events
    pub channels: Arc<WindowChannels>,
    // tool manager
    pub tool_manager: Arc<ToolManager>,
    pub messages_history: Arc<Mutex<HashMap<String, Vec<Value>>>>,
    pub dispatcher_input_tx: Sender<Arc<ChatResponse>>,
}

impl ChatState {
    pub fn new(channels: Arc<WindowChannels>) -> Arc<Self> {
        Self::new_with_apphandle(channels, None)
    }

    /// Creates a new instance of ChatState, initializing the available chat interfaces.
    /// Optionally takes an AppHandle for emitting events.
    ///
    /// # Arguments
    /// - `channels`: Arc to WindowChannels.
    /// - `app_handle_option`: Optional AppHandle for emitting events.
    /// - `main_store`: Arc to MainStore.
    ///
    /// # Returns
    /// A new instance of `ChatState` containing initialized chat interfaces.
    pub fn new_with_apphandle(
        channels: Arc<WindowChannels>,
        app_handle_option: Option<AppHandle>,
    ) -> Arc<Self> {
        let (dispatcher_input_tx, dispatcher_input_rx) = mpsc::channel(256);

        let chats = init_chats! {
            ChatProtocol::OpenAI,
            ChatProtocol::Ollama,
            ChatProtocol::HuggingFace,
            ChatProtocol::Claude,
            ChatProtocol::Gemini
        };

        let tool_manager = Arc::new(ToolManager::new());

        let state = Arc::new(Self {
            chats: Arc::new(Mutex::new(chats)),
            active_searches: Arc::new(Mutex::new(HashMap::new())),
            channels,
            tool_manager: tool_manager.clone(), // Clone for ChatState
            messages_history: Arc::new(Mutex::new(HashMap::new())),
            dispatcher_input_tx,
        });

        let processor_state_clone = Arc::clone(&state);
        tokio::spawn(global_message_processor_loop(
            processor_state_clone,
            dispatcher_input_rx,
        ));

        if let Some(app_handle) = app_handle_option {
            let mut mcp_status_receiver = tool_manager.subscribe_mcp_status_events(); // Assuming ToolManager provides this
            let app_handle_clone_for_spawn = app_handle.clone();
            tokio::spawn(async move {
                loop {
                    match mcp_status_receiver.recv().await {
                        Ok((server_name, status)) => {
                            log::info!(
                                "MCP Server '{}' status changed to: {:?}",
                                server_name,
                                status
                            );
                            if let Err(e) = app_handle_clone_for_spawn.emit(
                                "sync_state",
                                json!({
                                    "type": "mcp_status_changed",
                                    "name": server_name,
                                    "status": status,
                                }),
                            ) {
                                log::error!("Failed to emit mcp_server_status_update event: {}", e);
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            // Be specific with RecvError type
                            log::warn!("MCP status event listener lagged by {} messages.", n);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            log::info!("MCP status event channel closed.");
                            break;
                        }
                    }
                }
            });
        }
        state
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
        tools: Option<Vec<MCPToolDeclaration>>,
        extra_params: Option<Value>,
        callback: impl Fn(Arc<ChatResponse>) + Send + 'static,
    ) -> Result<String, AiError> {
        impl_chat_method!(
            self,
            chat,
            api_url,
            model,
            api_key,
            chat_id,
            messages,
            tools,
            extra_params,
            callback
        )
    }

    /// list models from the selected AI chat interface.
    ///
    /// # Arguments
    /// - `api_url`: Optional API URL for the chat request.
    /// - `api_key`: Optional API key for authentication.
    /// - `extra_params`: Optional extra parameters for the chat.
    ///
    /// # Returns
    /// A `Result` containing the list of models or an error.
    pub async fn list_models(
        &self,
        api_url: Option<&str>,
        api_key: Option<&str>,
        extra_params: Option<Value>,
    ) -> Result<Vec<ModelDetails>, AiError> {
        impl_chat_method!(self, list_models, api_url, api_key, extra_params).map_err(|e| {
            log::error!("Error listing models: {}", e);
            e
        })
    }

    /// Asynchronously sets a stop flag for the selected chat interface.
    ///
    /// # Arguments
    /// - `value`: A boolean indicating whether to stop the chat.
    pub async fn set_stop_flag(&self, value: bool) {
        impl_chat_method!(self, set_stop_flag, value)
    }
}

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

/// Prepares the parameters required for the AiChatTrait::chat method.
/// This function handles API URL overrides, HuggingFace-specific settings,
/// and API key rotation.
///
/// # Arguments
/// * `chat_protocol` - The chat protocol to use
/// * `api_url` - Optional API URL override
/// * `model` - The model name to use
/// * `api_key` - Optional API key override
/// * `metadata` - Optional additional parameters for the chat
///
/// # Returns
/// A tuple containing:
/// * `api_url`: The prepared API URL
/// * `api_key`: The prepared API key
/// * `metadata`: The prepared metadata
async fn prepare_chat_parameters(
    chat_protocol: ChatProtocol,
    api_url: Option<&str>,
    model: String,
    api_key: Option<&str>,
    mut metadata: Option<Value>,
) -> (Option<String>, Option<String>, Option<Value>) {
    let mut api_url_clone = if api_url == Some("") || api_url.is_none() {
        if chat_protocol == ChatProtocol::OpenAI {
            BASE_URL
                .get(&chat_protocol.to_string())
                .and_then(|s| Some(s.to_string()))
        } else {
            None
        }
    } else {
        api_url.map(|s| s.to_string())
    };

    if chat_protocol == ChatProtocol::HuggingFace {
        // huggingface requires `top_p` must be > 0.0 and < 1.0
        if let Some(md) = metadata.as_mut() {
            if let Some(topp) = md.get("top_p").and_then(|v| v.as_f64()) {
                if topp >= 1.0 {
                    if let Some(obj) = md.as_object_mut() {
                        obj.insert("top_p".to_string(), json!(0.99));
                    }
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

    (api_url_clone, api_key_clone, metadata)
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

/// list models from the selected AI chat interface.
///
/// # Arguments
/// * `api_protocol` - The chat protocol to use
/// * `api_url` - Optional API URL override
/// * `api_key` - Optional API key override
/// * `metadata` - Optional additional parameters for the chat
///
/// # Returns
/// * `Result<Vec<ModelDetails>, AiError>` - Ok with the list of models, Err with error message
pub async fn list_models_async(
    api_protocol: String,
    api_url: Option<String>,
    api_key: Option<String>,
    metadata: Option<Value>,
) -> Result<Vec<ModelDetails>, String> {
    let chat_protocol = ChatProtocol::from_str(&api_protocol)?;
    let (api_url_clone, api_key_clone, metadata_clone) = prepare_chat_parameters(
        chat_protocol.clone(),
        api_url.as_deref(),
        "".to_string(),
        api_key.as_deref(),
        metadata,
    )
    .await;

    let chat_instance = create_chat!(chat_protocol);
    chat_instance
        .list_models(
            api_url_clone.as_deref(),
            api_key_clone.as_deref(),
            metadata_clone,
        )
        .await
        .map_err(|e| e.to_string())
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
    chat_protocol: ChatProtocol,
    api_url: Option<&str>,
    model: String,
    api_key: Option<&str>,
    chat_id: String,
    messages: Vec<Value>,
    tools: Option<Vec<MCPToolDeclaration>>,
    metadata: Option<Value>,
) -> Result<ChatCompletionResult, AiError> {
    // Prepare the chat parameters
    let (api_url_clone, api_key_clone, metadata_clone) = prepare_chat_parameters(
        chat_protocol.clone(),
        api_url,
        model.clone(),
        api_key,
        metadata,
    )
    .await;

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Arc<ChatResponse>>(100);

    let tx_clone = tx.clone();
    let callback = move |chunk: Arc<ChatResponse>| {
        if let Err(e) = tx_clone.try_send(chunk) {
            log::error!("Failed to send chat response through channel: {}", e);
        }
    };

    let chat_instance = create_chat!(chat_protocol);
    let _ = chat_instance
        .chat(
            api_url_clone.as_deref(),
            &model,
            api_key_clone.as_deref(),
            chat_id.clone(),
            messages,
            tools,
            metadata_clone.clone(),
            callback,
        )
        .await?;

    let mut content = String::new();
    let mut reasoning = String::new();
    let mut reference = Vec::new();
    let mut usage = None;
    let mut final_metadata_from_stream = metadata_clone;
    let mut tools = Vec::new();
    let mut finish_reason = Option::<FinishReason>::None;

    while let Some(chunk) = rx.recv().await {
        finish_reason = chunk.finish_reason.clone();
        if chunk.chat_id == chat_id {
            if let Some(md) = &chunk.metadata {
                final_metadata_from_stream = Some(md.clone());
            }
            match chunk.r#type {
                MessageType::Text => content.push_str(&chunk.chunk),
                MessageType::Reasoning | MessageType::Think => reasoning.push_str(&chunk.chunk),
                MessageType::Reference => {
                    if let Ok(parsed_chunk) =
                        serde_json::from_str::<Vec<SearchResult>>(&chunk.chunk)
                    {
                        reference.extend(parsed_chunk);
                    } else {
                        let err_details = format!(
                            "Failed to deserialize SearchResult from chunk: {}",
                            chunk.chunk
                        );
                        log::error!("{}", err_details);
                        return Err(AiError::DeserializationFailed {
                            context: "SearchResult".to_string(),
                            details: err_details,
                        });
                    }
                }
                MessageType::Error => {
                    return Err(AiError::UpstreamChatError {
                        message: chunk.chunk.clone(),
                    })
                }
                MessageType::Finished => {
                    let md_to_use = chunk
                        .metadata
                        .as_ref()
                        .or(final_metadata_from_stream.as_ref());
                    if let Some(current_metadata) = md_to_use {
                        if let Some(tokens_val) = current_metadata.get(TOKENS) {
                            if let Some(tokens_obj) = tokens_val.as_object() {
                                usage = Some(Usage {
                                    prompt_tokens: tokens_obj
                                        .get(TOKENS_PROMPT)
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or_default(),
                                    completion_tokens: tokens_obj
                                        .get(TOKENS_COMPLETION)
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or_default(),
                                    total_tokens: tokens_obj
                                        .get(TOKENS_TOTAL)
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or_default(),
                                });
                            }
                        }
                    }
                    break;
                }
                MessageType::ToolCall => {
                    if let Ok(tool_decl_to_execute) =
                        serde_json::from_str::<ToolCallDeclaration>(&chunk.chunk)
                    {
                        tools.push(tool_decl_to_execute);
                    }
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
        tools: Some(tools),
        finish_reason,
    })
}

/// Executes an AI chat request with the given parameters.
///
/// This function encapsulates the core logic for interacting with AI chat interfaces.
/// It can be used by both the Tauri command and workflow tools.
///
/// # Arguments
/// * `state` - The chat state containing chat interfaces
/// * `main_state` - The main application state
/// * `api_protocol` - The API provider to use (e.g., "openai", "claude")
/// * `api_url` - Optional API URL override
/// * `model` - The model name to use, e.g., "deepseek-r1"
/// * `api_key` - Optional API key override
/// * `chat_id` - Unique identifier for this chat session
/// * `messages` - Vector of chat messages
/// * `metadata` - Optional additional parameters for the chat
/// * `tools` - Optional tools to use for the chat
/// * `callback` - Callback function to handle chat responses
///
/// ## Behavior for `chat_state_arc`
/// - If `Some(Arc<ChatState>)` is provided, the function uses the shared chat state,
///   allowing access to chat history, tool management, and persistence of the chat interface instance.
/// - If `None` is provided, a new, isolated `AiChatEnum` instance is created for this call.
///   This is typically used for scenarios like proxying stream requests where a persistent chat state or tool calls are not required.
///
/// # Returns
/// * `Result<(), String>` - Ok if the chat was started successfully, Err with error message otherwise
pub async fn complete_chat_async(
    chat_state_arc: Option<Arc<ChatState>>,
    chat_protocol: ChatProtocol,
    api_url: Option<&str>,
    model: String,
    api_key: Option<&str>,
    chat_id: String,
    messages: Vec<Value>,
    tools: Option<Vec<MCPToolDeclaration>>,
    metadata: Option<Value>,
    callback: impl Fn(Arc<ChatResponse>) + Send + 'static,
) -> Result<(), AiError> {
    // Prepare the chat parameters
    let (api_url_clone, api_key_clone, metadata) = prepare_chat_parameters(
        chat_protocol.clone(),
        api_url,
        model.clone(),
        api_key,
        metadata,
    )
    .await;

    let chat_interface = if let Some(chat_state) = chat_state_arc {
        let mut chats_guard = chat_state.chats.lock().await;
        let obj = get_or_create_chat!(chats_guard, chat_protocol, chat_id.clone());
        drop(chats_guard);

        obj
    } else {
        create_chat!(chat_protocol)
    };

    let _ = tokio::spawn(async move {
        // reset stop flag
        chat_interface.set_stop_flag(false).await;
        let result = chat_interface
            .chat(
                api_url_clone.as_deref(),
                &model,
                api_key_clone.as_deref(),
                chat_id.clone(),
                messages,
                tools,
                metadata,
                callback,
            )
            .await;

        if let Err(e) = result {
            log::error!("Chat execution failed for chat_id {}: {}", chat_id, e);
        }
    });

    Ok(())
}

/// Initiates a new chat interaction by setting up the necessary state and dispatching
/// the request to the underlying AI chat completion service.
///
/// This function serves as a primary entry point for starting or continuing a chat conversation.
/// It performs several key setup tasks before handing off to the asynchronous chat processing:
/// 1. Updates the shared message history for the given `chat_id` with the latest set of messages.
///    It's crucial that the `messages` argument contains the complete history up to the current turn.
/// 2. Prepares an internal callback (`internal_cb`) that routes AI response chunks to a global
///    message dispatcher. This dispatcher can then handle tasks like tool invocation or forwarding
///    responses to the UI.
/// 3. Augments the provided `metadata` with essential chat parameters (protocol, model, API details, etc.)
///    under a "chat_param" key. This enriched metadata can be used by downstream processes,
///    particularly for handling subsequent turns in a multi-turn conversation (e.g., after tool use).
/// 4. Calls `complete_chat_async` to execute the actual chat request asynchronously.
///
/// # Arguments
/// * `chat_state_arc` - An `Arc<ChatState>` providing access to the shared application state,
///   including message history, tool manager, and communication channels.
/// * `chat_protocol` - The `ChatProtocol` (e.g., OpenAI, Claude) to be used for this interaction.
/// * `api_url` - An optional override for the API endpoint URL. If `None` or empty,
///   a default URL for the protocol might be used.
/// * `model` - A `String` specifying the exact AI model to use (e.g., "gpt-4", "claude-3-opus").
/// * `api_key` - An optional API key for authentication with the AI service.
/// * `chat_id` - A `String` that uniquely identifies the current chat session. This is used
///   for managing history and state associated with this conversation.
/// * `messages` - A `Vec<Value>` representing the full sequence of messages in the chat up to this
///   point, including the latest user message. This will replace the existing history for the `chat_id`.
/// * `tools` - An optional `Vec<MCPToolDeclaration>` defining any tools that the AI model can
///   request to use during its response generation.
/// * `mut metadata` - An `Option<Value>` for passing additional parameters or context to the AI model.
///   This function will modify it to include "chat_param".
///
/// # Returns
/// * `Result<(), String>` - Returns `Ok(())` if the chat interaction was successfully initiated.
///   Returns `Err(String)` if there was an error during the setup or when calling `complete_chat_async`.
pub async fn start_new_chat_interaction(
    chat_state_arc: Arc<ChatState>,
    chat_protocol: ChatProtocol,
    api_url: Option<&str>,
    model: String,
    api_key: Option<&str>,
    chat_id: String,
    messages: Vec<Value>,
    tools: Option<Vec<MCPToolDeclaration>>,
    mut metadata: Option<Value>,
) -> Result<(), String> {
    // Update the shared message history with the complete list of messages for this turn.
    // This ensures that the history reflects the state *before* the AI responds to the current messages.
    {
        let mut history_guard = chat_state_arc.messages_history.lock().await;
        let session_messages = history_guard
            .entry(chat_id.clone())
            .or_insert_with(Vec::new);
        // Replace the history with the current full list.
        // This assumes `messages` IS the complete history up to this point, including the latest user message.
        *session_messages = messages.clone();
    }

    // Prepare a callback to dispatch AI response chunks to the global message processor.
    // The global processor handles further logic like tool calls or UI updates.
    let dispatcher_tx_clone = chat_state_arc.dispatcher_input_tx.clone();
    let internal_cb = move |chunk: Arc<ChatResponse>| {
        if let Err(e) = dispatcher_tx_clone.try_send(chunk) {
            log::error!("Failed to send AI chunk to global dispatcher: {}", e);
        }
    };

    // Augment the metadata with chat-specific parameters.
    // This information (`chat_param`) is crucial for subsequent operations,
    // especially if the conversation involves tool calls, as it allows the system
    // to reconstruct the context for the next AI turn.
    let chat_param_value = json!({
        "protocol": chat_protocol.to_string(),
        "model": model.clone(),
        "api_url": api_url.unwrap_or("").to_string(),
        "api_key": api_key.unwrap_or("").to_string(),
        "org_metadata": metadata.clone().unwrap_or_default(), // Store original metadata
    });

    if let Some(md_val) = metadata.as_mut() {
        if let Some(md_obj) = md_val.as_object_mut() {
            md_obj.insert("chat_param".to_string(), chat_param_value);
        } else {
            // If metadata was Some but not an object, overwrite it.
            // This case might be unlikely if metadata is typically an object or None.
            metadata = Some(json!({ "chat_param": chat_param_value }));
        }
    } else {
        // If metadata was None, create it with chat_param.
        metadata = Some(json!({ "chat_param": chat_param_value }));
    }

    // Asynchronously execute the chat completion.
    // The `internal_cb` will handle streaming responses to the global dispatcher.
    complete_chat_async(
        Some(chat_state_arc),
        chat_protocol,
        api_url,
        model,
        api_key,
        chat_id,
        messages, // Pass the full list of messages for the AI to process.
        tools,
        metadata, // Pass the augmented metadata.
        internal_cb,
    )
    .await
    .map_err(|e| e.to_string()) // Convert AiError to String for the return type.
}

/// Holds data about pending tool calls for a given chat session.
#[derive(Debug)] // Added Debug for logging
struct PendingToolCalls {
    requested_count: usize,
    results: HashMap<String, Value>, // tool_call_id -> result Value
    assistant_message_with_tool_requests: Option<Value>, // The full assistant message
}

impl PendingToolCalls {
    fn new() -> Self {
        PendingToolCalls {
            requested_count: 0,
            results: HashMap::new(),
            assistant_message_with_tool_requests: None,
        }
    }

    fn add_result(&mut self, tool_call_id: String, result: Value) {
        self.results.insert(tool_call_id, result);
    }

    fn is_complete(&self) -> bool {
        self.requested_count > 0 && self.results.len() >= self.requested_count
    }
}

/// Global message processor loop that handles incoming chat responses and manages tool calls
///
/// This async function runs in a continuous loop to:
/// 1. Process incoming chat response chunks from the dispatcher
/// 2. Handle tool call requests from AI assistants
/// 3. Track pending tool call states
/// 4. Coordinate tool execution and result processing
/// 5. Manage chat history and metadata
///
/// # Arguments
/// * `chat_state_arc` - Shared state containing chat interfaces, tool manager and message history
/// * `dispatcher_input_rx` - Receiver channel for incoming chat response chunks
async fn global_message_processor_loop(
    chat_state_arc: Arc<ChatState>,
    mut dispatcher_input_rx: Receiver<Arc<ChatResponse>>,
) {
    log::info!("Global message processor loop started.");
    let mut pending_tool_calls_map: HashMap<String, PendingToolCalls> = HashMap::new();

    while let Some(response_chunk) = dispatcher_input_rx.recv().await {
        let chat_id = response_chunk.chat_id.clone();
        let window_label_from_metadata = response_chunk
            .metadata
            .as_ref()
            .and_then(|m| {
                m.get("windowLabel")
                    .or_else(|| m.get("label"))
                    .or_else(|| m.get("window_label"))
                    .and_then(|v| v.as_str())
            })
            .map(String::from);

        let window_label = window_label_from_metadata.unwrap_or_else(|| {
            log::warn!("window_label/label not found in ChatResponse metadata for chat_id: {}. Defaulting to 'main'. UI updates may fail.", chat_id);
            "main".to_string()
        });

        match response_chunk.r#type {
            MessageType::AssistantAction => {
                log::debug!(
                    "Global processor received AssistantAction for chat_id {}: {:?}",
                    chat_id,
                    response_chunk.chunk
                );
                if let Ok(assistant_msg_value) =
                    serde_json::from_str::<Value>(&response_chunk.chunk)
                {
                    if assistant_msg_value.get("role").and_then(Value::as_str) == Some("assistant")
                    {
                        if let Some(tool_calls_array) = assistant_msg_value
                            .get("tool_calls")
                            .and_then(|tc| tc.as_array())
                        {
                            if !tool_calls_array.is_empty() {
                                let pending = pending_tool_calls_map
                                    .entry(chat_id.clone())
                                    .or_insert_with(PendingToolCalls::new);
                                pending.requested_count = tool_calls_array.len();
                                pending.assistant_message_with_tool_requests =
                                    Some(assistant_msg_value.clone());
                                log::info!(
                                    "Chat {}: Assistant requested {} tools. Message: {:?}",
                                    chat_id,
                                    pending.requested_count,
                                    assistant_msg_value
                                );

                                let mut history_guard =
                                    chat_state_arc.messages_history.lock().await;
                                let session_messages = history_guard
                                    .entry(chat_id.clone())
                                    .or_insert_with(Vec::new);
                                session_messages.push(assistant_msg_value.clone());
                                drop(history_guard);
                            } else {
                                log::warn!("Chat {}: AssistantAction received, but 'tool_calls' array is empty.", chat_id);
                            }
                        } else {
                            log::warn!("Chat {}: AssistantAction received, but no 'tool_calls' array found in chunk or it's not an array. Chunk: {:?}", chat_id, response_chunk.chunk);
                        }
                    } else {
                        log::warn!("Chat {}: AssistantAction chunk is not an assistant role message or is malformed: {:?}", chat_id, assistant_msg_value);
                    }
                } else {
                    log::error!(
                        "Chat {}: Failed to parse AssistantAction chunk: {}",
                        chat_id,
                        response_chunk.chunk
                    );
                }
                if let Some(tx) = chat_state_arc.channels.get_sender(&window_label).await {
                    if let Err(e_send) = tx.try_send(response_chunk.clone()) {
                        log::error!("Failed to send AssistantAction to window '{}' (chat_id {}) channel: {}", window_label, chat_id, e_send);
                    }
                }
            }
            MessageType::ToolCall => {
                // Check if this is a processed tool call result
                let is_processed_result = response_chunk
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get("is_internal_tool_result").and_then(Value::as_bool))
                    .unwrap_or(false);

                if is_processed_result {
                    // It is a processed tool call result.
                    log::debug!(
                        "Global processor received Processed ToolCall Result for chat_id {}: {:?}",
                        chat_id,
                        response_chunk.chunk
                    );

                    let tool_result_msg_for_history: Value =
                        serde_json::from_str(&response_chunk.chunk)
                            .map_err(|e| {
                                log::error!(
                            "Failed to parse tool_result_msg_for_history from chunk: {}. Error: {}",
                            response_chunk.chunk,
                            e
                        );
                            })
                            .unwrap_or_else(
                                |_| json!({"error": "failed to parse tool result chunk"}),
                            );

                    // 从 tool_result_msg_for_history 中提取 tool_call_id
                    let tool_call_id = tool_result_msg_for_history
                        .get("tool_call_id")
                        .and_then(Value::as_str)
                        .map(String::from)
                        .unwrap_or_else(|| {
                            log::error!(
                                "tool_call_id not found in processed tool result chunk: {:?}",
                                tool_result_msg_for_history
                            );
                            "unknown_tool_call_id".to_string()
                        });

                    let mut all_tools_done = false;
                    let mut messages_for_next_ai_turn = Vec::new();
                    {
                        let mut history_guard = chat_state_arc.messages_history.lock().await;
                        let session_messages = history_guard
                            .entry(chat_id.clone())
                            .or_insert_with(Vec::new);
                        session_messages.push(tool_result_msg_for_history.clone());

                        let mut should_remove_pending_state = false;
                        if let Some(pending) = pending_tool_calls_map.get_mut(&chat_id) {
                            let result_content_str = tool_result_msg_for_history
                                .get("content")
                                .and_then(Value::as_str)
                                .unwrap_or("");
                            let result_value_for_pending: Value =
                                serde_json::from_str(result_content_str).unwrap_or_else(|_| {
                                    Value::String(result_content_str.to_string())
                                });
                            pending.add_result(tool_call_id.clone(), result_value_for_pending);

                            if pending.is_complete() {
                                all_tools_done = true;
                                messages_for_next_ai_turn = session_messages.clone();
                                should_remove_pending_state = true;
                                log::info!(
                                    "Chat {}: All {} tools executed (processed result).",
                                    chat_id,
                                    pending.requested_count
                                );
                            } else {
                                log::info!(
                                    "Chat {}: Tool {}/{} executed (processed result). Waiting for more.",
                                    chat_id,
                                    pending.results.len(),
                                    pending.requested_count
                                );
                            }
                        } else {
                            log::warn!("Chat {}: Received processed tool result for tool_id {}, but no pending tool call state found. Assuming single tool call.", chat_id, tool_call_id);
                            all_tools_done = true;
                            messages_for_next_ai_turn = session_messages.clone();
                        }
                        if should_remove_pending_state {
                            pending_tool_calls_map.remove(&chat_id);
                            log::info!(
                                "Chat {}: Pending tool call state cleared (processed result).",
                                chat_id
                            );
                        }
                    }

                    if all_tools_done {
                        log::info!("Global processor: All tools for chat_id {} executed. Sending results back to AI (from processed result).", chat_id);
                        let dispatcher_tx_clone = chat_state_arc.dispatcher_input_tx.clone();
                        let internal_cb_for_next_turn = move |chunk: Arc<ChatResponse>| {
                            if let Err(e) = dispatcher_tx_clone.try_send(chunk) {
                                log::error!(
                                    "Failed to send next AI chunk to global dispatcher: {}",
                                    e
                                );
                            }
                        };
                        let cc_chat_state_clone = chat_state_arc.clone();
                        let cc_chat_id_clone = chat_id.clone();
                        let cc_metadata_clone = response_chunk.metadata.clone();
                        #[cfg(debug_assertions)]
                        {
                            log::debug!(
                                "metadata for next round: {}",
                                serde_json::to_string_pretty(&cc_metadata_clone)
                                    .unwrap_or_default()
                            );
                        }

                        tokio::spawn(async move {
                            let protocol = cc_metadata_clone
                                .as_ref()
                                .and_then(|m| m.get("chat_param")?.get("protocol")?.as_str())
                                .and_then(|s| ChatProtocol::from_str(s).ok())
                                .unwrap_or_default_log(
                                    "using default protocol for tool result turn",
                                );
                            let model = cc_metadata_clone
                                .as_ref()
                                .and_then(|m| {
                                    m.get("chat_param")?
                                        .get("model")?
                                        .as_str()
                                        .map(String::from)
                                })
                                .unwrap_or_default_log("using default model for tool result turn");
                            let api_url_from_meta = cc_metadata_clone
                                .as_ref()
                                .and_then(|m| m.get("chat_param")?.get("api_url")?.as_str())
                                .filter(|s| !s.is_empty()) // Treat empty string as None
                                .map(String::from);
                            let api_key_from_meta = cc_metadata_clone
                                .as_ref()
                                .and_then(|m| m.get("chat_param")?.get("api_key")?.as_str())
                                .filter(|s| !s.is_empty()) // Treat empty string as None
                                .map(String::from);
                            // metadata from previous turn
                            let org_metadata = cc_metadata_clone
                                .as_ref()
                                .and_then(|m| m.get("chat_param")?.get("org_metadata"))
                                .map(Value::clone);

                            let _ = complete_chat_async(
                                Some(cc_chat_state_clone.clone()),
                                protocol,
                                api_url_from_meta.as_deref(),
                                model,
                                api_key_from_meta.as_deref(),
                                cc_chat_id_clone.clone(),
                                messages_for_next_ai_turn,
                                None, // AI的回应轮次不需要工具
                                org_metadata,
                                internal_cb_for_next_turn,
                            )
                            .await;
                        });
                    }
                } else {
                    // 这是AI初次请求执行工具
                    if let Ok(tool_decl_to_execute) =
                        serde_json::from_str::<ToolCallDeclaration>(&response_chunk.chunk)
                    {
                        let tool_call_id = tool_decl_to_execute.id.clone();
                        let tool_name = tool_decl_to_execute.name.clone();
                        let arguments_str_opt = tool_decl_to_execute.arguments.clone();

                        // 为派生任务克隆必要的数据
                        let cs_arc_clone = chat_state_arc.clone();
                        let cid_clone = chat_id.clone();
                        let t_name_clone = tool_name.clone();
                        let tc_id_clone = tool_call_id.clone();
                        let metadata_clone = response_chunk.metadata.clone(); // 此元数据包含 chat_param

                        tokio::spawn(async move {
                            let args_value = arguments_str_opt
                                .as_ref()
                                .and_then(|s| {
                                    if s.is_empty() {
                                        Some(json!({}))
                                    } else {
                                        serde_json::from_str(s).ok()
                                    }
                                })
                                .unwrap_or_else(|| json!({}));

                            log::info!(
                                "Spawned task: Executing tool '{}' (ID: {}) for chat_id: {}",
                                t_name_clone,
                                tc_id_clone,
                                cid_clone
                            );

                            let tool_execution_actual_result = match cs_arc_clone
                                .tool_manager
                                .tool_call(&t_name_clone, args_value)
                                .await
                            {
                                Ok(result) => result,
                                Err(e) => {
                                    log::error!(
                                        "Tool execution failed for '{}' (ID: {}): {}",
                                        t_name_clone,
                                        tc_id_clone,
                                        e
                                    );
                                    json!({"error": format!("Tool execution failed: {}", e)})
                                }
                            };

                            let tool_result_msg_for_history = json!({
                                "role": "tool",
                                "tool_call_id": tc_id_clone.clone(),
                                "name": t_name_clone.clone(),
                                "content": tool_execution_actual_result.as_str().map(String::from).unwrap_or_else(||
                                    serde_json::to_string(&tool_execution_actual_result).unwrap_or_else(|_| {
                                        "Failed to serialize tool result".to_string()
                                    })
                                ),
                            });

                            let mut new_metadata = metadata_clone.unwrap_or_else(|| json!({}));
                            if let Some(obj) = new_metadata.as_object_mut() {
                                obj.insert(
                                    "is_internal_tool_result".to_string(),
                                    Value::Bool(true),
                                );
                            } else {
                                new_metadata = json!({"is_internal_tool_result": true});
                            }

                            let result_response_chunk = ChatResponse::new_with_arc(
                                cid_clone.clone(),
                                serde_json::to_string(&tool_result_msg_for_history)
                                    .unwrap_or_default(),
                                MessageType::ToolCall, // 复用 ToolCall 类型
                                Some(new_metadata),
                                None,
                            );

                            if let Err(e) = cs_arc_clone
                                .dispatcher_input_tx
                                .send(result_response_chunk)
                                .await
                            {
                                log::error!("Failed to send processed tool result back to dispatcher for chat_id {}: {}", cid_clone, e);
                            }
                        });
                    } else {
                        log::error!(
                            "Global processor: Failed to parse ToolCallDeclaration for chat_id {}: {}",
                            chat_id, response_chunk.chunk
                        );
                        // （可选）如果解析 ToolCallDeclaration 失败，则向UI发送错误
                        if let Some(tx) = chat_state_arc.channels.get_sender(&window_label).await {
                            let error_response = ChatResponse::new_with_arc(
                                chat_id.clone(),
                                format!(
                                    "Failed to parse tool call instruction: {}",
                                    response_chunk.chunk
                                ),
                                MessageType::Error,
                                response_chunk.metadata.clone(),
                                Some(FinishReason::Error),
                            );
                            if let Err(e_send) = tx.try_send(error_response) {
                                log::error!("Failed to send parse error (ToolCallDeclaration) to window '{}' (chat_id {}) channel: {}", window_label, chat_id, e_send);
                            }
                        }
                    }
                }
            }
            MessageType::Finished => {
                log::debug!(
                    "Global processor: Received Finished for chat_id: {}",
                    chat_id
                );

                // 重要提醒：工具调用的时候不要将当轮结束标志发送给前端，否则可能导致前端结束会话
                // IMPORTANT: Do not send Finished with ToolCalls reason to UI,
                // otherwise UI will end the session.
                if response_chunk.finish_reason != Some(FinishReason::ToolCalls) {
                    if let Some(tx) = chat_state_arc.channels.get_sender(&window_label).await {
                        if let Err(e_send) = tx.try_send(response_chunk.clone()) {
                            log::error!("Failed to send Finished response to window '{}' (chat_id {}) channel: {}", window_label, chat_id, e_send);
                        }
                    }
                }

                let session_ended_naturally = match response_chunk.finish_reason {
                    Some(FinishReason::ToolCalls) => false,
                    Some(_) => true,
                    None => {
                        // If no specific reason, check pending tools as a fallback
                        pending_tool_calls_map
                            .get(&chat_id)
                            .map_or(true, |pending_state| {
                                pending_state.is_complete() || pending_state.requested_count == 0
                            })
                    }
                };

                if session_ended_naturally {
                    // Extract chat protocol from metadata for cleanup
                    if let Some(protocol_str) = response_chunk
                        .metadata
                        .as_ref()
                        .and_then(|m| m.get("chat_param")?.get("protocol")?.as_str())
                    {
                        if let Ok(protocol) = ChatProtocol::from_str(protocol_str) {
                            let mut chats_guard = chat_state_arc.chats.lock().await;
                            if let Some(protocol_chats) = chats_guard.get_mut(&protocol) {
                                if protocol_chats.remove(&chat_id).is_some() {
                                    #[cfg(debug_assertions)]
                                    log::debug!(
                                        "Removed chat instance for chat_id: {} under protocol: {}",
                                        chat_id,
                                        protocol
                                    );
                                }
                            }
                        }
                    }

                    // Clean up only if the session ended naturally or due to an error, AND no tools are pending
                    // (The check for pending tools is implicitly handled if finish_reason is not ToolCalls)
                    log::debug!(
                        "Session for chat_id: {} is considered finished. Cleaning up resources.",
                        chat_id
                    );
                    let mut history_guard = chat_state_arc.messages_history.lock().await;
                    history_guard.remove(&chat_id);
                    drop(history_guard);
                    pending_tool_calls_map.remove(&chat_id);
                }
                // Do not remove AiChatEnum instance from ChatState.chats as it's shared per protocol.
            }
            _ => {
                if let Some(tx) = chat_state_arc.channels.get_sender(&window_label).await {
                    if let Err(e_send) = tx.try_send(response_chunk.clone()) {
                        log::error!("Failed to send message type {:?} to window '{}' (chat_id {}) channel: {}", response_chunk.r#type, window_label, chat_id, e_send);
                    }
                }
            }
        }
    }
    log::info!("Global message processor loop stopped.");
}

trait UnwrapOrDefaultLog<T> {
    fn unwrap_or_default_log(self, msg: &str) -> T
    where
        T: Default; // Removed Display constraint for broader compatibility
}

impl<T> UnwrapOrDefaultLog<T> for Option<T> {
    fn unwrap_or_default_log(self, msg: &str) -> T
    where
        T: Default, // Removed Display constraint
    {
        match self {
            Some(val) => val,
            None => {
                log::warn!("{} - Using default value", msg); // Log a generic message
                T::default()
            }
        }
    }
}
