use serde_json::{json, Value};
use std::collections::HashMap;
use std::str::FromStr as _;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::{
    mpsc::{self, Receiver, Sender},
    Mutex,
};

use crate::ccproxy::ChatProtocol;
use crate::search::SearchResult;
use crate::tools::ToolManager;
use crate::{
    ai::{
        chat::openai::OpenAIChat,
        error::AiError,
        interaction::constants::{TOKENS, TOKENS_COMPLETION, TOKENS_PROMPT, TOKENS_TOTAL},
        traits::chat::{
            ChatCompletionResult, MCPToolDeclaration, MessageType, ModelDetails,
            ToolCallDeclaration, Usage,
        },
        traits::{
            chat::FinishReason,
            chat::{AiChatTrait, ChatResponse},
            stoppable::Stoppable,
        },
    },
    db::MainStore,
    libs::window_channels::WindowChannels,
};

macro_rules! init_chats {
    () => {{
        let mut chats = HashMap::new();
        chats.insert(ChatProtocol::OpenAI, HashMap::new());
        chats
    }};
}

#[macro_export]
macro_rules! create_chat {
    ($main_store:expr) => {{
        AiChatEnum::OpenAI(OpenAIChat::new($main_store.clone()))
    }};
}

macro_rules! get_or_create_chat {
    ($chats:expr, $chat_id:expr, $main_store:expr) => {{
        $chats
            .entry(ChatProtocol::OpenAI)
            .or_insert_with(HashMap::new)
            .entry($chat_id.clone())
            .or_insert_with(|| create_chat!($main_store))
            .clone()
    }};
}

macro_rules! impl_chat_method {
    ($self:expr, $method:ident, $($arg:expr),*) => {
        match $self {
            AiChatEnum::OpenAI(chat) => chat.$method($($arg),*).await,
        }
    };
}

#[derive(Clone)]
pub enum AiChatEnum {
    OpenAI(OpenAIChat),
}

pub struct ChatState {
    pub chats: Arc<Mutex<HashMap<ChatProtocol, HashMap<String, AiChatEnum>>>>,
    pub channels: Arc<WindowChannels>,
    pub tool_manager: Arc<ToolManager>,
    pub messages_history: Arc<Mutex<HashMap<String, Vec<Value>>>>,
    pub dispatcher_input_tx: Sender<Arc<ChatResponse>>,
    pub main_store: Arc<std::sync::RwLock<MainStore>>,
}

impl ChatState {
    pub fn new(
        channels: Arc<WindowChannels>,
        app_handle_option: Option<AppHandle>,
        main_store: Arc<std::sync::RwLock<MainStore>>,
    ) -> Arc<Self> {
        let (dispatcher_input_tx, dispatcher_input_rx) = mpsc::channel(256);

        let chats = init_chats!();

        let tool_manager = Arc::new(ToolManager::new());

        let state = Arc::new(Self {
            chats: Arc::new(Mutex::new(chats)),
            channels,
            tool_manager: tool_manager.clone(),
            messages_history: Arc::new(Mutex::new(HashMap::new())),
            dispatcher_input_tx,
            main_store,
        });

        let processor_state_clone = Arc::clone(&state);
        tokio::spawn(global_message_processor_loop(
            processor_state_clone,
            dispatcher_input_rx,
        ));

        if let Some(app_handle) = app_handle_option {
            let mut mcp_status_receiver = tool_manager.subscribe_mcp_status_events();
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
    pub async fn chat(
        &self,
        provider_id: i64,
        model: &str,
        chat_id: String,
        messages: Vec<Value>,
        tools: Option<Vec<MCPToolDeclaration>>,
        extra_params: Option<Value>,
        callback: impl Fn(Arc<ChatResponse>) + Send + 'static,
    ) -> Result<String, AiError> {
        impl_chat_method!(
            self,
            chat,
            provider_id,
            model,
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
        api_protocol: String,
        api_url: Option<&str>,
        api_key: Option<&str>,
        metadata: Option<Value>,
    ) -> Result<Vec<ModelDetails>, AiError> {
        impl_chat_method!(self, list_models, api_protocol, api_url, api_key, metadata).map_err(
            |e| {
                log::error!("Error listing models: {}", e);
                e
            },
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
) -> (Option<String>, Option<String>) {
    let mut api_url_clone = if api_url == Some("") || api_url.is_none() {
        if chat_protocol == ChatProtocol::OpenAI {
            super::constants::BASE_URL
                .get(&chat_protocol.to_string())
                .and_then(|s| Some(s.to_string()))
        } else {
            None
        }
    } else {
        api_url.map(|s| s.to_string())
    };

    if chat_protocol == ChatProtocol::HuggingFace {
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
            api_key_clone = keys.first().map(|s| s.to_string());
        }
    }

    (api_url_clone, api_key_clone)
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
    main_store: Arc<std::sync::RwLock<MainStore>>,
    api_protocol: String,
    api_url: Option<&str>,
    api_key: Option<&str>,
    metadata: Option<Value>,
) -> Result<Vec<ModelDetails>, String> {
    let chat_protocol = ChatProtocol::from_str(&api_protocol)?;
    let (api_url_clone, api_key_clone) = prepare_chat_parameters(
        chat_protocol.clone(),
        api_url.as_deref(),
        "".to_string(),
        api_key.as_deref(),
    )
    .await;

    let chat_instance = create_chat!(main_store);
    chat_instance
        .list_models(
            api_protocol,
            api_url_clone.as_deref(),
            api_key_clone.as_deref(),
            metadata,
        )
        .await
        .map_err(|e| e.to_string())
}

pub async fn start_new_chat_interaction(
    chat_state_arc: Arc<ChatState>,
    provider_id: i64,
    model: String,
    chat_id: String,
    messages: Vec<Value>,
    tools: Option<Vec<MCPToolDeclaration>>,
    mut metadata: Option<Value>,
    callback: Option<Box<dyn Fn(Arc<ChatResponse>) + Send + 'static>>,
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

    let internal_cb: Box<dyn Fn(Arc<ChatResponse>) + Send + 'static> = if let Some(cb) = callback {
        cb
    } else {
        let dispatcher_tx_clone = chat_state_arc.dispatcher_input_tx.clone();
        Box::new(move |chunk: Arc<ChatResponse>| {
            if let Err(e) = dispatcher_tx_clone.try_send(chunk) {
                log::error!("Failed to send AI chunk to global dispatcher: {}", e);
            }
        })
    };

    // Augment the metadata with chat-specific parameters.
    // This information (`chat_param`) is crucial for subsequent operations,
    // especially if the conversation involves tool calls, as it allows the system
    // to reconstruct the context for the next AI turn.
    let chat_param_value = json!({
        "protocol": ChatProtocol::OpenAI.to_string(),
        "provider_id": provider_id,
        "model": model.clone(),
        "org_metadata": metadata.clone().unwrap_or_default(),
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

    let chat_interface = {
        let mut chats_guard = chat_state_arc.chats.lock().await;
        get_or_create_chat!(
            chats_guard,
            chat_id.clone(),
            chat_state_arc.main_store.clone()
        )
    };

    let _ = tokio::spawn(async move {
        chat_interface.set_stop_flag(false).await;
        let result = chat_interface
            .chat(
                provider_id,
                &model,
                chat_id.clone(),
                messages,
                tools,
                metadata,
                internal_cb,
            )
            .await;

        if let Err(e) = result {
            log::error!("Chat execution failed for chat_id {}: {}", chat_id, e);
        }
    });

    Ok(())
}

#[derive(Debug)]
struct PendingToolCalls {
    requested_count: usize,
    results: HashMap<String, Value>,
    assistant_message_with_tool_requests: Option<Value>,
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
            MessageType::ToolCalls => {
                log::debug!(
                    "Global processor received ToolCalls for chat_id {}: {:?}",
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
            MessageType::ToolResults => {
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

                                // All tool call information is merged into the following format,
                                // which is convenient for storage and frontend display.
                                // [
                                //   {
                                //     "id": "call_123",
                                //     "type": "function",
                                //     "function": {
                                //       "name": "search_weather",
                                //       "arguments": "{\"city\": \"beijing\"}"
                                //     },
                                //     "result": "{\"temperature\": \"31°\", \"condition\": \"晴朗\"}"
                                //   }
                                // ]
                                // Consolidate tool calls and results into a single message for the frontend.
                                let tool_calls_value = pending
                                    .assistant_message_with_tool_requests
                                    .as_ref()
                                    .and_then(|v| v.get("tool_calls"))
                                    .cloned()
                                    .unwrap_or_else(|| json!([]));

                                let tool_calls_array =
                                    tool_calls_value.as_array().unwrap_or(&vec![]).to_vec();

                                let merged_tool_info: Vec<Value> = tool_calls_array
                                    .iter()
                                    .map(|call| {
                                        let mut call_obj =
                                            call.as_object().cloned().unwrap_or_default();
                                        if let Some(id_val) = call_obj.get("id") {
                                            if let Some(id_str) = id_val.as_str() {
                                                if let Some(result) = pending.results.get(id_str) {
                                                    // The result from tool execution is often a JSON string.
                                                    // To avoid double-escaped JSON, we attempt to parse it.
                                                    // If it's not a valid JSON string, we keep it as a string.
                                                    let result_value = serde_json::from_str(
                                                        result.as_str().unwrap_or(""),
                                                    )
                                                    .unwrap_or_else(|_| result.clone());
                                                    call_obj
                                                        .insert("result".to_string(), result_value);
                                                }
                                            }
                                        }
                                        json!(call_obj)
                                    })
                                    .collect();

                                // Send tool call information to the frontend
                                let consolidated_chunk = ChatResponse::new_with_arc(
                                    chat_id.clone(),
                                    serde_json::to_string(&merged_tool_info).unwrap_or_default(),
                                    MessageType::ToolResults,
                                    response_chunk.metadata.clone(),
                                    None,
                                );

                                // Send the single consolidated message to the frontend
                                if let Some(tx) =
                                    chat_state_arc.channels.get_sender(&window_label).await
                                {
                                    if let Err(e_send) = tx.try_send(consolidated_chunk) {
                                        log::error!("Failed to send consolidated ToolCall to window '{}' (chat_id {}) channel: {}", window_label, chat_id, e_send);
                                    }
                                }

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
                            let chat_param =
                                cc_metadata_clone.as_ref().and_then(|m| m.get("chat_param"));

                            let provider_id = chat_param
                                .and_then(|p| p.get("provider_id")?.as_i64())
                                .unwrap_or_default();

                            // Use `match` to safely get the model, preventing potential panics.
                            let model = match chat_param.and_then(|p| p.get("model")?.as_str()) {
                                Some(m) => m.to_string(),
                                None => {
                                    log::error!("Could not find model in chat_param for tool call continuation. Metadata: {:?}", cc_metadata_clone);
                                    return; // Exit the task if model is not found
                                }
                            };

                            let org_metadata = chat_param
                                .and_then(|p| p.get("org_metadata"))
                                .map(Value::clone);

                            if let Err(e) = start_new_chat_interaction(
                                cc_chat_state_clone.clone(),
                                provider_id,
                                model,
                                cc_chat_id_clone.clone(),
                                messages_for_next_ai_turn,
                                None,
                                org_metadata,
                                Some(Box::new(internal_cb_for_next_turn)),
                            )
                            .await
                            {
                                log::error!("Failed to start subsequent chat interaction after tool call: {}", e);
                            }
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
                                MessageType::ToolResults, // 复用 ToolCall 类型
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
                    None => pending_tool_calls_map
                        .get(&chat_id)
                        .map_or(true, |pending_state| {
                            pending_state.is_complete() || pending_state.requested_count == 0
                        }),
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

/// Asynchronously sends a chat request to the selected AI chat interface.
/// It's a blocking version of `complete_chat_async`. The chat response will
/// be returned after the chat is finished.
///
/// # Arguments
/// * `state` - The chat state containing chat interfaces
/// * `provider_id` - The provider ID to use
/// * `model` - The model name to use
/// * `chat_id` - Unique identifier for this chat session
/// * `messages` - Vector of chat messages
/// * `tools` - Optional vector of tools to use
/// * `metadata` - Optional additional parameters for the chat
///
/// # Returns
/// * `Result<String, String>` - Ok with the chat response, Err with error message
pub async fn complete_chat_blocking(
    chat_state: Arc<ChatState>,
    provider_id: i64,
    model: String,
    chat_id: String,
    messages: Vec<Value>,
    tools: Option<Vec<MCPToolDeclaration>>,
    metadata: Option<Value>,
) -> Result<ChatCompletionResult, String> {
    let (tx, mut rx) = mpsc::channel(100);

    let callback = move |chunk: Arc<ChatResponse>| {
        if let Err(e) = tx.blocking_send(chunk) {
            log::error!("Failed to send chat response through channel: {}", e);
        }
    };

    start_new_chat_interaction(
        chat_state,
        provider_id,
        model,
        chat_id.clone(),
        messages,
        tools,
        metadata.clone(),
        Some(Box::new(callback)),
    )
    .await?;

    let mut content = String::new();
    let mut reasoning = String::new();
    let mut reference = Vec::new();
    let mut usage = None;
    let mut final_metadata_from_stream = metadata;
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
                        }
                        .to_string());
                    }
                }
                MessageType::Error => {
                    return Err(AiError::UpstreamChatError {
                        message: chunk.chunk.clone(),
                    }
                    .to_string())
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
                MessageType::ToolResults => {
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
