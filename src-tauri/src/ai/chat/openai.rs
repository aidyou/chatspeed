use async_trait::async_trait;
use chrono::{DateTime, Utc};
use reqwest::Response;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
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
use crate::ai::util::{
    get_family_from_model_id, get_proxy_type, init_extra_params, is_function_call_supported,
    is_image_input_supported, is_reasoning_supported, update_or_create_metadata,
};
use crate::ccproxy::{ChatProtocol, StreamFormat, StreamProcessor};
use crate::{ai::error::AiError, impl_stoppable};

const OPENAI_DEFAULT_API_BASE: &str = "https://api.openai.com/v1";

#[derive(Deserialize, Debug)]
struct OpenAIModel {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    object: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    created: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    owned_by: Option<String>,
    // We can add more fields if needed from the OpenAI response, like 'permission'
}

#[derive(Deserialize, Debug)]
struct OpenAIListModelsResponse {
    data: Vec<OpenAIModel>,
}

/// OpenAI chat implementation
#[derive(Clone)]
pub struct OpenAIChat {
    stop_flag: Arc<Mutex<bool>>,
    client: DefaultApiClient,
}

impl OpenAIChat {
    /// Creates a new instance of OpenAIChat
    pub fn new() -> Self {
        Self {
            stop_flag: Arc::new(Mutex::new(false)),
            client: DefaultApiClient::new(ErrorFormat::OpenAI),
        }
    }

    /// Processes streaming response
    ///
    /// # Arguments
    /// * `response` - Raw streaming response from OpenAI API
    /// * `callback` - Function for sending updates to the client
    /// * `metadata_option` - Optional metadata to include in callbacks
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
                    let chunks = self
                        .client
                        .process_stream_chunk(chunk, &StreamFormat::OpenAI)
                        .await
                        .map_err(|e| {
                            let err = AiError::StreamProcessingFailed {
                                provider: provider_name.clone(),
                                details: e.to_string(),
                            };
                            log::error!("{} stream processing error: {}", provider_name, err);
                            callback(ChatResponse::new_with_arc(
                                chat_id.clone(),
                                err.to_string(),
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
                                    .entry(part.index) // Use part.index as HashMap key
                                    .or_insert_with(|| {
                                        // First time encountering this index, this part should contain valid id and name
                                        // part.id and part.name are already String type (unwrapped_or_default in stream.rs)
                                        ToolCallDeclaration {
                                            index: part.index,
                                            id: part.id.clone(),
                                            name: part.name.clone(),
                                            arguments: Some(String::new()), // Initialize with empty string
                                            results: None,
                                        }
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

                                // First, send the AssistantAction message with all requested tool calls
                                let assistant_tool_requests: Vec<Value> = accumulated_tool_calls
                                    .iter()
                                    .map(|(idx, tcd)| {
                                        // Convert ToolCallDeclaration to the format expected in the assistant's message
                                        // OpenAI's assistant message tool_calls usually look like:
                                        // { "id": "...", "type": "function", "function": { "name": "...", "arguments": "..." } }
                                        // Our ToolCallDeclaration is already very close to this.
                                        // We might need to ensure the arguments are a string as expected by some models.
                                        let arguments_str =
                                            tcd.arguments.as_deref().unwrap_or_default();
                                        json!({
                                            "index": idx,
                                            "id": tcd.id,
                                            "type": "function", // Assuming all tools are functions for now
                                            "function": {
                                                "name": tcd.name,
                                                "arguments": arguments_str
                                            }
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
                                            chat_id.clone(),
                                            serialized_assistant_action,
                                            MessageType::AssistantAction,
                                            metadata_option.clone(),
                                            None,
                                        ));
                                    }
                                    Err(e) => {
                                        log::error!(
                                            "Failed to serialize AssistantAction message: {}",
                                            e
                                        );
                                        // Optionally send an error message via callback here
                                    }
                                }

                                // Then, send individual ToolCall messages for each tool
                                for tcd in accumulated_tool_calls.values() {
                                    let mut trimmed_tcd = tcd.clone();
                                    if let Some(args) = trimmed_tcd.arguments.as_mut() {
                                        *args = args.trim().to_string();
                                    }
                                    match serde_json::to_string(&trimmed_tcd) {
                                        Ok(serialized_tcd) => {
                                            callback(ChatResponse::new_with_arc(
                                                chat_id.clone(),
                                                serialized_tcd,
                                                MessageType::ToolCall,
                                                metadata_option.clone(),
                                                None,
                                            ));
                                            #[cfg(debug_assertions)]
                                            {
                                                log::debug!(
                                                    "tool_call: {}",
                                                    serde_json::to_string_pretty(tcd)
                                                        .unwrap_or_default()
                                                );
                                            }
                                        }
                                        Err(e) => {
                                            let err = AiError::ToolCallSerializationFailed {
                                                details: e.to_string(),
                                            };
                                            log::error!(
                                                "{} tool call serialization error for tool {:?}: {}",
                                                provider_name, tcd.name, err
                                            );
                                            callback(ChatResponse::new_with_arc(
                                                chat_id.clone(),
                                                err.to_string(),
                                                MessageType::Error,
                                                metadata_option.clone(),
                                                None,
                                            ));
                                        }
                                    }
                                }
                                accumulated_tool_calls.clear(); // Clear for next batch
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
                    callback(ChatResponse::new_with_arc(
                        chat_id.clone(),
                        err.to_string(),
                        MessageType::Error,
                        metadata_option.clone(),
                        None,
                    ));
                }
            }
        }

        // Send final response with token usage
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

        Ok(format!(
            "<think>{}</think>{}",
            reasoning_content, full_response
        ))
    }
}

impl_stoppable!(OpenAIChat);

#[async_trait]
impl AiChatTrait for OpenAIChat {
    /// Implements chat functionality for OpenAI API
    ///
    /// # Arguments
    /// * `api_url` - Optional API endpoint URL
    /// * `model` - The model to use
    /// * `api_key` - Optional API key
    /// * `chat_id` - Unique identifier for the chat session
    /// * `messages` - The chat messages
    /// * `tools` - Optional tools to use
    /// * `extra_params` - Additional parameters including proxy settings
    /// * `callback` - Function for sending updates to the client
    async fn chat(
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
        let (params, metadata_option) = init_extra_params(extra_params.clone());

        let provider_name_for_error =
            if api_url.is_some() && api_url != Some(OPENAI_DEFAULT_API_BASE) {
                "OpenAI Compatible".to_string() // User-facing name for compatible APIs
            } else {
                "OpenAI".to_string()
            };

        let mut payload = json!({
            "model": model,
            "messages": messages,
            "stream": params.get("stream").unwrap_or(&json!(true)),
        });

        // Add optional parameters if they exist and are not null
        if let Some(obj) = payload.as_object_mut() {
            // Handle max_tokens if provided
            if let Some(max_tokens_val) = params.get("max_tokens").and_then(|v| v.as_u64()) {
                if max_tokens_val > 0 {
                    obj.insert("max_tokens".to_string(), json!(max_tokens_val));
                }
            }

            // Handle temperature if provided
            // OpenAI temperature range is typically 0 to 2.
            if let Some(temperature_val) = params.get("temperature").and_then(|v| v.as_f64()) {
                // You might want to add validation here, e.g., if temperature_val >= 0.0 && temperature_val <= 2.0
                if temperature_val >= 0.0 && temperature_val <= 2.0 {
                    obj.insert("temperature".to_string(), json!(temperature_val));
                }
            }

            // Zero -> unset
            if let Some(top_p) = params.get("top_p").as_ref().and_then(|v| v.as_f64()) {
                if top_p > 0.0 && top_p <= 1.0 {
                    obj.insert("top_p".to_string(), json!(top_p));
                }
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

            match tools {
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

        #[cfg(debug_assertions)]
        log::debug!(
            "OpenAI Request Body (final): {}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );

        let response = self
            .client
            .post_request(
                &ApiConfig::new(
                    Some(api_url.unwrap_or(OPENAI_DEFAULT_API_BASE).to_string()),
                    api_key.map(String::from),
                    get_proxy_type(extra_params),
                    None,
                ),
                "chat/completions",
                payload,
                true,
            )
            .await
            .map_err(|network_err| {
                let err = AiError::ApiRequestFailed {
                    provider: provider_name_for_error.clone(),
                    details: network_err.to_string(),
                };
                log::error!("{} API request failed: {}", provider_name_for_error, err);
                callback(ChatResponse::new_with_arc(
                    chat_id.clone(),
                    err.to_string(),
                    MessageType::Error,
                    metadata_option.clone(),
                    Some(FinishReason::Error),
                ));
                err
            })?;

        if response.is_error {
            let err = AiError::ApiRequestFailed {
                provider: provider_name_for_error.clone(),
                details: response.content.clone(),
            };
            // log::error!("{} API returned an error: {}", provider_name_for_error, err);
            callback(ChatResponse::new_with_arc(
                chat_id.clone(),
                err.to_string(),
                MessageType::Error,
                metadata_option,
                Some(FinishReason::Error),
            ));
            return Err(err);
        }

        if let Some(raw_response) = response.raw_response {
            self.handle_stream_response(
                chat_id.clone(),
                raw_response,
                callback,
                metadata_option,
                provider_name_for_error,
            )
            .await
        } else {
            callback(ChatResponse::new_with_arc(
                chat_id.clone(),
                response.content.clone(),
                MessageType::Finished,
                metadata_option,
                Some(FinishReason::Error),
            ));
            Ok(response.content)
        }
    }

    async fn list_models(
        &self,
        api_url: Option<&str>,
        api_key: Option<&str>,
        extra_args: Option<Value>,
    ) -> Result<Vec<ModelDetails>, AiError> {
        let base_url = api_url.unwrap_or(OPENAI_DEFAULT_API_BASE);
        let mut effective_api_key = api_key.map(String::from);

        let mut custom_headers = json!({});
        if let Some(args) = &extra_args {
            if let Some(key_from_extra) = args.get("api_key").and_then(|v| v.as_str()) {
                if !key_from_extra.is_empty() {
                    effective_api_key = Some(key_from_extra.to_string());
                }
            }
            if let Some(org_id) = args.get("organization").and_then(|v| v.as_str()) {
                if !org_id.is_empty() {
                    custom_headers
                        .as_object_mut()
                        .unwrap()
                        .insert("OpenAI-Organization".to_string(), json!(org_id));
                }
            }
        }

        let config = ApiConfig::new(
            Some(base_url.to_string()),
            effective_api_key, // Handled by DefaultApiClient for Bearer token
            get_proxy_type(extra_args.clone()), // Pass extra_args for proxy settings
            if custom_headers.as_object().map_or(true, |m| m.is_empty()) {
                None
            } else {
                Some(custom_headers)
            },
        );

        let response = self
            .client
            .get_request(&config, "/models", None)
            .await
            .map_err(|e| AiError::ApiRequestFailed {
                provider: "OpenAI".to_string(),
                details: e,
            })?;

        if response.is_error || response.content.is_empty() {
            return Err(AiError::ApiRequestFailed {
                provider: "OpenAI".to_string(),
                details: response.content,
            });
        }

        #[cfg(debug_assertions)]
        log::debug!("OpenAI list_models response: {}", &response.content);

        let models_response: OpenAIListModelsResponse = serde_json::from_str(&response.content)
            .map_err(|e| {
                log::error!(
                    "Failed to parse OpenAI models response: {}, content:{}",
                    e,
                    &response.content
                );
                AiError::ResponseParseFailed {
                    provider: "OpenAI".to_string(),
                    details: e.to_string(),
                }
            })?;

        let model_details: Vec<ModelDetails> = models_response
            .data
            .into_iter()
            .fold(std::collections::HashMap::new(), |mut acc, model| {
                acc.insert(model.id.to_lowercase(), model);
                acc
            })
            .into_values()
            .map(|model| {
                let id = model.id.to_lowercase();

                ModelDetails {
                    id: model.id.clone(),
                    name: model.id, // OpenAI API doesn't provide a separate "friendly name"
                    protocol: ChatProtocol::OpenAI,
                    max_input_tokens: None,  // Not provided by /v1/models
                    max_output_tokens: None, // Not provided by /v1/models
                    description: Some(format!(
                        "Owned by: {}",
                        model.owned_by.as_deref().unwrap_or("unknown")
                    )),
                    last_updated: DateTime::from_timestamp(model.created.unwrap_or_default(), 0)
                        .map(|dt: DateTime<Utc>| dt.to_rfc3339()),
                    family: get_family_from_model_id(&id),
                    function_call: Some(is_function_call_supported(&id)),
                    reasoning: Some(is_reasoning_supported(&id)),
                    image_input: Some(is_image_input_supported(&id)),
                    metadata: Some(json!({
                        "object": model.object.unwrap_or_default(),
                        "owned_by": model.owned_by.unwrap_or_default(),
                    })),
                }
            })
            .collect();

        Ok(model_details)
    }
}
