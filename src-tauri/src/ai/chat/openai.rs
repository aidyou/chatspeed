use async_trait::async_trait;
use reqwest::Response;
use rust_i18n::t;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::str::FromStr;
use std::{sync::Arc, time::Instant};
use tokio::sync::Mutex;

use crate::ai::interaction::constants::{
    TOKENS, TOKENS_COMPLETION, TOKENS_PER_SECOND, TOKENS_PROMPT, TOKENS_TOTAL,
};
use crate::ai::network::{ApiClient, ApiConfig, DefaultApiClient, ErrorFormat, TokenUsage};
use crate::ai::traits::chat::{
    ChatResponse, FinishReason, MCPToolDeclaration, MessageType, ModelDetails, ToolCallDeclaration,
};
use crate::ai::traits::{chat::AiChatTrait, stoppable::Stoppable};
use crate::ai::util::{init_extra_params, process_custom_headers, update_or_create_metadata};
use crate::ccproxy::{ChatProtocol, StreamFormat, StreamProcessor};
use crate::db::ModelConfig;
use crate::{
    ai::error::AiError, constants::INTERNAL_CCPROXY_API_KEY, db::MainStore, impl_stoppable,
};

/// A standardized error structure for streaming to the frontend.
#[derive(Serialize)]
struct JsonErrorPayload<'a> {
    status: u16,
    message: &'a str,
}

/// OpenAI chat implementation
#[derive(Clone)]
pub struct OpenAIChat {
    stop_flag: Arc<Mutex<bool>>,
    client: DefaultApiClient,
    main_store: Arc<std::sync::RwLock<MainStore>>,
}

impl OpenAIChat {
    /// Creates a new instance of OpenAIChat
    pub fn new(main_store: Arc<std::sync::RwLock<MainStore>>) -> Self {
        Self {
            stop_flag: Arc::new(Mutex::new(false)),
            client: DefaultApiClient::new(ErrorFormat::OpenAI),
            main_store,
        }
    }

    /// Processes streaming response
    async fn handle_stream_response(
        &self,
        chat_id: String,
        response: Response,
        callback: impl Fn(Arc<ChatResponse>) + Send + 'static,
        metadata_option: Option<Value>,
        provider_name: String, // Added to correctly attribute errors
    ) -> Result<String, AiError> {
        let mut full_response = String::new();
        let mut reasoning_content = String::new();
        let mut token_usage = TokenUsage::default();
        let start_time = Instant::now();

        let processor = StreamProcessor::new();
        let mut event_receiver = processor
            .process_stream(response, &StreamFormat::OpenAI)
            .await;

        // 用于累积工具调用信息，键为工具调用的索引 (tool_call.index)
        let mut accumulated_tool_calls: HashMap<u32, ToolCallDeclaration> = HashMap::new();
        let mut finish_reason = FinishReason::Complete;

        while let Some(event) = event_receiver.recv().await {
            if self.should_stop().await {
                processor.stop();
                break;
            }

            match event {
                Ok(chunk) => {
                    let chunks = self.client.process_stream_chunk(chunk).await.map_err(|e| {
                        let err = AiError::StreamProcessingFailed {
                            provider: provider_name.clone(),
                            details: e.to_string(),
                        };
                        log::error!("{} stream processing error: {}", provider_name, err);

                        let error_payload = JsonErrorPayload {
                            status: 500, // Internal processing error
                            message: &err.to_string(),
                        };
                        let chunk = serde_json::to_string(&error_payload)
                            .unwrap_or_else(|_| err.to_string());

                        callback(ChatResponse::new_with_arc(
                            chat_id.clone(),
                            chunk,
                            MessageType::Error,
                            metadata_option.clone(),
                            Some(FinishReason::Error),
                        ));
                        err
                    })?;

                    for chunk in chunks {
                        if let Some(new_usage) = chunk.usage {
                            if new_usage.total_tokens > 0 {
                                token_usage = new_usage;

                                // OpenAI does not provide tokens per second, so calculate it here
                                if token_usage.tokens_per_second == 0.0 {
                                    let completion_tokens = token_usage.completion_tokens as f64;
                                    let duration = start_time.elapsed();
                                    token_usage.tokens_per_second = if duration.as_secs() > 0 {
                                        completion_tokens / duration.as_secs_f64()
                                    } else {
                                        0.0
                                    };
                                }
                            }
                        }

                        if let Some(content) = chunk.reasoning_content {
                            if !content.is_empty() {
                                reasoning_content.push_str(&content);

                                callback(ChatResponse::new_with_arc(
                                    chat_id.clone(),
                                    content,
                                    MessageType::Reasoning,
                                    metadata_option.clone(),
                                    None,
                                ));
                            }
                        }

                        if let Some(content) = chunk.content {
                            if !content.is_empty() {
                                full_response.push_str(&content);
                                let msg_type = chunk.msg_type.unwrap_or(MessageType::Text);
                                callback(ChatResponse::new_with_arc(
                                    chat_id.clone(),
                                    content,
                                    msg_type,
                                    metadata_option.clone(),
                                    None,
                                ));
                            }
                        }

                        // Tool call handling
                        if let Some(tool_call_parts) = chunk.tool_calls {
                            for part in tool_call_parts {
                                // part is a ToolCallDeclaration
                                let acc_call = accumulated_tool_calls
                                    .entry(part.index)
                                    .or_insert_with(|| ToolCallDeclaration {
                                        index: part.index,
                                        id: part.id.clone(),
                                        name: part.name.clone(),
                                        arguments: Some(String::new()),
                                        results: None,
                                    });

                                // Append current part's arguments
                                if let Some(args_chunk) = part.arguments {
                                    if !args_chunk.is_empty() {
                                        // acc_call.arguments was initialized as Some(String::new()) in or_insert_with
                                        // So it's safe to unwrap here
                                        if let Some(existing_args) = acc_call.arguments.as_mut() {
                                            existing_args.push_str(&args_chunk);
                                        }
                                    }
                                }
                            }
                        }

                        // If finish_reason is tool_calls, send accumulated tool calls
                        if chunk.finish_reason == Some("tool_calls".to_string()) {
                            if !accumulated_tool_calls.is_empty() {
                                finish_reason = FinishReason::ToolCalls;
                                self.process_and_send_tool_calls(
                                    &mut accumulated_tool_calls,
                                    &full_response,
                                    &chat_id,
                                    &metadata_option,
                                    &provider_name,
                                    &callback,
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    let err = AiError::StreamProcessingFailed {
                        provider: provider_name.clone(),
                        details: e.to_string(),
                    };
                    log::error!("{} stream event error: {}", provider_name, err);

                    let error_payload = JsonErrorPayload {
                        status: 500, // Internal processing error
                        message: &err.to_string(),
                    };
                    let chunk =
                        serde_json::to_string(&error_payload).unwrap_or_else(|_| err.to_string());

                    callback(ChatResponse::new_with_arc(
                        chat_id.clone(),
                        chunk,
                        MessageType::Error,
                        metadata_option.clone(),
                        None,
                    ));
                    return Err(err);
                }
            }
        }

        if !accumulated_tool_calls.is_empty() {
            log::debug!(
                "Manually triggering tool call processing after stream end for chat_id: {}",
                chat_id
            );
            finish_reason = FinishReason::ToolCalls;
            self.process_and_send_tool_calls(
                &mut accumulated_tool_calls,
                &full_response,
                &chat_id,
                &metadata_option,
                &provider_name,
                &callback,
            );
        }

        callback(ChatResponse::new_with_arc(
            chat_id.clone(),
            String::new(),
            MessageType::Finished,
            Some(update_or_create_metadata(
                metadata_option,
                TOKENS,
                json!({
                    TOKENS_TOTAL: token_usage.total_tokens,
                    TOKENS_PROMPT: token_usage.prompt_tokens,
                    TOKENS_COMPLETION: token_usage.completion_tokens,
                    TOKENS_PER_SECOND: token_usage.tokens_per_second
                }),
            )),
            Some(finish_reason),
        ));

        Ok(json!({
            "reasoning": reasoning_content,
            "content": full_response
        })
        .to_string())
    }

    /// Processes and sends accumulated tool calls.
    fn process_and_send_tool_calls<F: Fn(Arc<ChatResponse>) + Send + 'static>(
        &self,
        accumulated_tool_calls: &mut HashMap<u32, ToolCallDeclaration>,
        full_response: &str,
        chat_id: &str,
        metadata_option: &Option<Value>,
        provider_name: &str,
        callback: &F,
    ) {
        let assistant_tool_requests: Vec<Value> = accumulated_tool_calls
            .iter()
            .map(|(idx, tcd)| {
                let arguments_str = tcd.arguments.as_deref().unwrap_or_default();
                json!({
                    "index": idx,
                    "id": tcd.id,
                    "type": "function",
                    "function": { "name": tcd.name, "arguments": arguments_str }
                })
            })
            .collect();

        let assistant_action_message = json!({
            "role": "assistant",
            "content": full_response,
            "tool_calls": assistant_tool_requests
        });

        match serde_json::to_string(&assistant_action_message) {
            Ok(serialized_assistant_action) => {
                callback(ChatResponse::new_with_arc(
                    chat_id.to_string(),
                    serialized_assistant_action,
                    MessageType::ToolCalls,
                    metadata_option.clone(),
                    None,
                ));
            }
            Err(e) => {
                let err = AiError::ToolCallSerializationFailed {
                    details: e.to_string(),
                };
                log::error!("Failed to serialize AssistantAction message: {}", err);

                let error_payload = JsonErrorPayload {
                    status: 500,
                    message: &err.to_string(),
                };
                let chunk =
                    serde_json::to_string(&error_payload).unwrap_or_else(|_| err.to_string());

                callback(ChatResponse::new_with_arc(
                    chat_id.to_string(),
                    chunk,
                    MessageType::Error,
                    metadata_option.clone(),
                    Some(FinishReason::Error),
                ));
                return;
            }
        }

        for tcd in accumulated_tool_calls.values() {
            let mut trimmed_tcd = tcd.clone();
            if let Some(args) = trimmed_tcd.arguments.as_mut() {
                *args = args.trim().to_string();
            }
            match serde_json::to_string(&trimmed_tcd) {
                Ok(serialized_tcd) => {
                    callback(ChatResponse::new_with_arc(
                        chat_id.to_string(),
                        serialized_tcd,
                        MessageType::ToolResults,
                        metadata_option.clone(),
                        None,
                    ));
                    #[cfg(debug_assertions)]
                    {
                        log::debug!(
                            "tool_call: {}",
                            serde_json::to_string_pretty(tcd).unwrap_or_default()
                        );
                    }
                }
                Err(e) => {
                    let err = AiError::ToolCallSerializationFailed {
                        details: e.to_string(),
                    };
                    log::error!(
                        "{} tool call serialization error for tool {:?}: {}",
                        provider_name,
                        tcd.name,
                        err
                    );
                    let error_payload = JsonErrorPayload {
                        status: 500,
                        message: &err.to_string(),
                    };
                    let chunk =
                        serde_json::to_string(&error_payload).unwrap_or_else(|_| err.to_string());
                    callback(ChatResponse::new_with_arc(
                        chat_id.to_string(),
                        chunk,
                        MessageType::Error,
                        metadata_option.clone(),
                        Some(FinishReason::Error),
                    ));
                }
            }
        }
        accumulated_tool_calls.clear();
    }

    /// Determines the appropriate API endpoint based on tools and function call settings
    fn get_endpoint(
        &self,
        messages: &[Value],
        tools: &Option<Vec<MCPToolDeclaration>>,
        model_detail: &[ModelConfig],
        model: &str,
    ) -> &'static str {
        let has_tool_calls = messages.iter().any(|msg| {
            msg.get("role")
                .and_then(|r| r.as_str())
                .map(|role| {
                    role == "tool"
                        || (role == "assistant"
                            && msg
                                .get("tool_calls")
                                .and_then(|tc| tc.as_array())
                                .map(|arr| !arr.is_empty())
                                .unwrap_or(false))
                })
                .unwrap_or(false)
        });

        let has_tools = tools.as_ref().map(|t| !t.is_empty()).unwrap_or(false);
        let function_call = model_detail
            .iter()
            .find(|m| m.id == model)
            .and_then(|m| m.function_call)
            .unwrap_or(false);

        if has_tool_calls || has_tools {
            if function_call {
                "/v1/chat/completions"
            } else {
                "/compat_mode/v1/chat/completions"
            }
        } else {
            "/v1/chat/completions"
        }
    }
}

impl_stoppable!(OpenAIChat);

#[async_trait]
impl AiChatTrait for OpenAIChat {
    /// Implements chat functionality for OpenAI API
    ///
    /// # Arguments
    /// * `provider_id` - provider id
    /// * `model` - The model to use
    /// * `chat_id` - Unique identifier for the chat session
    /// * `messages` - The chat messages
    /// * `tools` - Optional tools to use
    /// * `extra_params` - Additional parameters including proxy settings
    /// * `callback` - Function for sending updates to the client
    async fn chat(
        &self,
        provider_id: i64,
        model: &str,
        chat_id: String,
        messages: Vec<Value>,
        tools: Option<Vec<MCPToolDeclaration>>,
        extra_params: Option<Value>,
        callback: impl Fn(Arc<ChatResponse>) + Send + 'static,
    ) -> Result<String, AiError> {
        let model_detail = self
            .main_store
            .read()
            .map_err(|e| {
                let err = AiError::InitFailed(
                    t!("db.failed_to_lock_main_store", error = e.to_string()).to_string(),
                );
                let error_payload = JsonErrorPayload {
                    status: 500,
                    message: &err.to_string(),
                };
                let chunk =
                    serde_json::to_string(&error_payload).unwrap_or_else(|_| err.to_string());
                callback(ChatResponse::new_with_arc(
                    chat_id.clone(),
                    chunk,
                    MessageType::Error,
                    extra_params.clone(),
                    Some(FinishReason::Error),
                ));
                err
            })?
            .config
            .get_ai_model_by_id(provider_id)
            .map_err(|e| {
                let err = AiError::InitFailed(e.to_string());
                let error_payload = JsonErrorPayload {
                    status: 500,
                    message: &err.to_string(),
                };
                let chunk =
                    serde_json::to_string(&error_payload).unwrap_or_else(|_| err.to_string());
                callback(ChatResponse::new_with_arc(
                    chat_id.clone(),
                    chunk,
                    MessageType::Error,
                    extra_params.clone(),
                    Some(FinishReason::Error),
                ));
                err
            })?;

        let params = init_extra_params(model_detail.metadata.clone());

        let url = crate::constants::get_static_var(&crate::constants::CHAT_COMPLETION_PROXY);

        // Initialize the payload with essential fields.
        // We delegate most parameter injection (max_tokens, temperature, etc.) to the CCProxy layer.
        let mut payload = json!({
            "model": model,
            "messages": messages,
            "stream": params.get("stream").unwrap_or(&json!(true)),
        });

        // Add optional parameters if they exist and are not null
        if let Some(obj) = payload.as_object_mut() {
            // Handle max_tokens if provided
            if model_detail.max_tokens > 0 {
                obj.insert("max_tokens".to_string(), json!(model_detail.max_tokens));
            }

            // Handle temperature if provided
            // OpenAI temperature range is typically 0 to 2.
            // You might want to add validation here, e.g., if temperature_val >= 0.0 && temperature_val <= 2.0
            if model_detail.temperature >= 0.0 && model_detail.temperature <= 2.0 {
                obj.insert("temperature".to_string(), json!(model_detail.temperature));
            }

            // Zero -> unset
            if model_detail.top_p > 0.0 && model_detail.top_p <= 1.0 {
                obj.insert("top_p".to_string(), json!(model_detail.top_p));
            }

            // frequency_penalty: OpenAI default is 0.0. Range: -2.0 to 2.0.
            if let Some(v) = params
                .get("frequency_penalty")
                .as_ref()
                .and_then(|v| v.as_f64())
            {
                if v != 0.0 {
                    obj.insert("frequency_penalty".to_string(), json!(v));
                }
            }

            // presence_penalty: OpenAI default is 0.0. Range: -2.0 to 2.0.
            if let Some(v) = params
                .get("presence_penalty")
                .as_ref()
                .and_then(|v| v.as_f64())
            {
                if v != 0.0 {
                    obj.insert("presence_penalty".to_string(), json!(v));
                }
            }

            // Handle response_format if provided
            // OpenAI expects an object like {"type": "text"} or {"type": "json_object"}
            if let Some(rf_val) = params.get("response_format") {
                if let Some(rf_str) = rf_val.as_str() {
                    // If user provides a string like "text" or "json_object"
                    obj.insert("response_format".to_string(), json!({ "type": rf_str }));
                } else if rf_val.is_object() {
                    // If user provides the full object e.g. {"type": "json_object"}
                    obj.insert("response_format".to_string(), rf_val.clone());
                }
                // If not a known string or an object, it's omitted, letting OpenAI use its default.
                // Consider logging a warning for unsupported formats if necessary.
            }

            if let Some(stop_val) = params.get("stop_sequences").cloned() {
                if !stop_val.is_null() {
                    obj.insert("stop".to_string(), stop_val);
                }
            }
            if let Some(n_val) = params.get("candidate_count").cloned() {
                if let Some(n) = n_val.as_u64() {
                    // Check if it's a number and > 0
                    if n > 0 {
                        obj.insert("n".to_string(), json!(n));
                    }
                }
            }
            if let Some(user_val) = params.get("user_id").cloned() {
                if !user_val.is_null() {
                    obj.insert("user".to_string(), user_val);
                }
            }

            match tools.as_ref() {
                Some(tools) => {
                    let openai_tools = tools
                        .into_iter()
                        .map(|tool| tool.to_openai())
                        .collect::<Vec<Value>>();
                    if !openai_tools.is_empty() {
                        obj.insert("tools".to_string(), json!(openai_tools));
                        // Only add tool_choice if it's explicitly provided in the params.
                        // Avoids sending a default "auto" which may not be supported by all models.
                        if let Some(tool_choice_val) = params.get("tool_choice").cloned() {
                            if tool_choice_val.as_str().map_or(true, |s| !s.is_empty()) {
                                obj.insert("tool_choice".to_string(), tool_choice_val);
                            }
                        }
                    }
                }
                None => {}
            }
        }

        let internal_api_key = INTERNAL_CCPROXY_API_KEY.read().clone();
        let mut headers_json = json!({
            "X-Provider-Id": provider_id.to_string(),
            "X-Model-Id": model,
            "X-Internal-Request": "true",
        });

        // Add custom headers from model metadata
        let custom_headers = process_custom_headers(&model_detail.metadata, &chat_id);
        if let Some(headers_obj) = headers_json.as_object_mut() {
            for (k, v) in custom_headers {
                headers_obj.insert(k, json!(v));
            }
        }

        let config = ApiConfig::new(
            Some(url),
            Some(internal_api_key),
            crate::ai::network::ProxyType::None, // No proxy, as we are calling localhost
            Some(headers_json),
        );

        let endpoint = self.get_endpoint(&messages, &tools, &model_detail.models, model);

        let response = self
            .client
            .post_request(&config, endpoint, payload, true)
            .await
            .map_err(|e| {
                let err = AiError::ApiRequestFailed {
                    status_code: 0, // N/A for network errors before HTTP response
                    provider: model_detail.api_protocol.clone(),
                    details: e.to_string(),
                };

                let error_payload = JsonErrorPayload {
                    status: 503, // Service Unavailable or network error
                    message: &err.to_string(),
                };
                let chunk =
                    serde_json::to_string(&error_payload).unwrap_or_else(|_| err.to_string());

                callback(ChatResponse::new_with_arc(
                    chat_id.clone(),
                    chunk,
                    MessageType::Error,
                    extra_params.clone(),
                    Some(FinishReason::Error),
                ));
                err
            })?;

        if response.is_error {
            let status_code = response
                .raw_response
                .as_ref()
                .map(|r| r.status().as_u16())
                .unwrap_or(500); // Default to 500 if no status

            let err = AiError::ApiRequestFailed {
                status_code,
                provider: model_detail.name.clone(),
                details: response.content.clone(),
            };

            let error_payload = JsonErrorPayload {
                status: status_code,
                message: &err.to_string(),
            };
            let chunk = serde_json::to_string(&error_payload).unwrap_or_else(|_| err.to_string());

            callback(ChatResponse::new_with_arc(
                chat_id.clone(),
                chunk,
                MessageType::Error,
                extra_params,
                Some(FinishReason::Error),
            ));
            return Err(err);
        }

        if let Some(raw_response) = response.raw_response {
            self.handle_stream_response(
                chat_id.clone(),
                raw_response,
                callback,
                extra_params,
                model_detail.name.clone(),
            )
            .await
        } else {
            // This case occurs when the request fails in a way that a response object isn't available,
            // but it wasn't caught by the initial network error mapping. The content field contains the error details.
            let err = AiError::ApiRequestFailed {
                status_code: 0, // N/A for network errors before HTTP response
                provider: model_detail.name.clone(),
                details: response.content.clone(),
            };

            let error_payload = JsonErrorPayload {
                status: 500,
                message: &err.to_string(),
            };
            let chunk = serde_json::to_string(&error_payload).unwrap_or_else(|_| err.to_string());

            callback(ChatResponse::new_with_arc(
                chat_id.clone(),
                chunk,
                MessageType::Error,
                extra_params,
                Some(FinishReason::Error),
            ));
            return Err(err);
        }
    }

    async fn list_models(
        &self,
        api_protocol: String,
        api_url: Option<&str>,
        api_key: Option<&str>,
        extra_args: Option<Value>,
    ) -> Result<Vec<ModelDetails>, AiError> {
        let model_details = match ChatProtocol::from_str(&api_protocol) {
            Ok(ChatProtocol::Claude) => {
                super::list_models::claude_list_models(api_url, api_key, extra_args).await
            }
            Ok(ChatProtocol::Gemini) => {
                super::list_models::gemini_list_models(api_url, api_key, extra_args).await
            }
            _ => super::list_models::openai_list_models(api_url, api_key, extra_args).await,
        }?;

        Ok(model_details)
    }
}
