use async_trait::async_trait;
use reqwest::Response;
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc, time::Instant};
use tokio::sync::Mutex;

use crate::ai::interaction::constants::{
    TOKENS, TOKENS_COMPLETION, TOKENS_PER_SECOND, TOKENS_PROMPT, TOKENS_TOTAL,
};
use crate::ai::network::{
    ApiClient, ApiConfig, DefaultApiClient, ErrorFormat, StreamFormat, StreamProcessor, TokenUsage,
};
use crate::ai::traits::chat::{ChatResponse, MCPToolDeclaration, MessageType, ToolCallDeclaration};
use crate::ai::traits::{chat::AiChatTrait, stoppable::Stoppable};
use crate::ai::util::{get_proxy_type, init_extra_params, update_or_create_metadata};
use crate::{ai::error::AiError, impl_stoppable};

/// Represents the Claude chat implementation
#[derive(Clone)]
pub struct ClaudeChat {
    stop_flag: Arc<Mutex<bool>>,
    client: DefaultApiClient,
}

impl ClaudeChat {
    /// Creates a new instance of ClaudeChat
    pub fn new() -> Self {
        Self {
            stop_flag: Arc::new(Mutex::new(false)),
            client: DefaultApiClient::new(ErrorFormat::Claude),
        }
    }

    /// Builds the request payload for Claude API
    ///
    /// # Arguments
    /// * `model` - The model to use
    /// * `messages` - The chat messages to process
    /// * `params` - Generation parameters like temperature, top_k, etc.
    ///
    /// # Returns
    /// A JSON payload formatted according to Claude API requirements
    fn build_request_body(
        &self,
        model: &str,
        messages: Vec<Value>,
        tools: Option<Vec<MCPToolDeclaration>>,
        params: &Value,
    ) -> Value {
        let mut processed_messages = messages.clone();
        let mut system_prompt = None;

        if let Some(first_message) = messages.first() {
            if first_message.get("role").and_then(Value::as_str) == Some("system") {
                system_prompt = first_message
                    .get("content")
                    .and_then(Value::as_str)
                    .map(String::from);
                processed_messages = messages.iter().skip(1).cloned().collect();
            }
        }

        let mut payload = json!({
            "model": model,
            "messages": processed_messages,
            "max_tokens": params.get("max_tokens").unwrap_or(&json!(4096)),
            "temperature": params.get("temperature").unwrap_or(&json!(1.0)),
            "stream": params.get("stream").unwrap_or(&json!(true)),
            "top_p": params.get("top_p").unwrap_or(&json!(1.0)),
            "top_k": params.get("top_k").unwrap_or(&json!(40)),
        });

        if let Some(obj) = payload.as_object_mut() {
            if let Some(prompt) = system_prompt {
                if !prompt.is_empty() {
                    obj.insert("system".to_string(), json!(prompt));
                }
            }

            if let Some(stop_sequences) = params.get("stop_sequences").and_then(|v| v.as_array()) {
                if !stop_sequences.is_empty() {
                    obj.insert("stop_sequences".to_string(), json!(stop_sequences));
                }
            }

            if let Some(user_id) = params.get("user_id").and_then(|v| v.as_str()) {
                if !user_id.is_empty() {
                    obj.insert("metadata".to_string(), json!({ "user_id": user_id }));
                }
            }

            if params.get("tool_choice").and_then(|tc| tc.as_str()) != Some("none") {
                if let Some(tools_vec) = tools {
                    let claude_tools = tools_vec
                        .into_iter()
                        .map(|tool| tool.to_claude())
                        .collect::<Vec<Value>>();
                    if !claude_tools.is_empty() {
                        obj.insert("tools".to_string(), json!(claude_tools));
                    }
                }
            }
        }

        #[cfg(debug_assertions)]
        {
            log::debug!(
                "claude payload: {}",
                serde_json::to_string_pretty(&payload).unwrap_or_default()
            )
        }

        payload
    }

    /// Processes streaming response
    ///
    /// # Arguments
    /// * `response` - Raw streaming response from Claude API
    /// * `callback` - Function for sending updates to the client
    /// * `metadata_option` - Optional metadata to include in callbacks
    async fn handle_stream_response(
        &self,
        chat_id: String,
        response: Response,
        callback: impl Fn(Arc<ChatResponse>) + Send + 'static,
        metadata_option: Option<Value>,
    ) -> Result<String, AiError> {
        let mut full_response = String::new();
        let mut token_usage = TokenUsage::default();
        let start_time = Instant::now();

        // Accumulates tool call parts. Key is tool_call.index
        let mut accumulated_tool_calls: HashMap<u32, ToolCallDeclaration> = HashMap::new();

        let processor = StreamProcessor::new();
        let mut event_receiver = processor
            .process_stream(response, &StreamFormat::Claude)
            .await;

        while let Some(event) = event_receiver.recv().await {
            if self.should_stop().await {
                processor.stop();
                break;
            }

            match event {
                Ok(raw_bytes_from_sse_processor) => {
                    let chunks = self
                        .client
                        .process_stream_chunk(raw_bytes_from_sse_processor, &StreamFormat::Claude)
                        .await
                        .map_err(|e| {
                            let err = AiError::StreamProcessingFailed {
                                provider: "Claude".to_string(),
                                details: e.to_string(),
                            };
                            log::error!("Claude stream processing error: {}", err);
                            callback(ChatResponse::new_with_arc(
                                chat_id.clone(),
                                err.to_string(),
                                MessageType::Error,
                                metadata_option.clone(),
                            ));
                            err
                        })?;

                    let mut send_tool_calls_signal = false;

                    for chunk in chunks {
                        if let Some(new_usage) = chunk.usage {
                            // Prompt tokens usually come once, typically with MessageStop.
                            if new_usage.prompt_tokens > 0 {
                                token_usage.prompt_tokens = new_usage.prompt_tokens;
                            }
                            // Completion tokens accumulate from MessageDelta and MessageStop.
                            token_usage.completion_tokens += new_usage.completion_tokens;
                            token_usage.total_tokens =
                                token_usage.prompt_tokens + token_usage.completion_tokens;

                            // Calculate tokens_per_second based on accumulated completion_tokens
                            let current_completion_tokens = token_usage.completion_tokens as f64;
                            let duration = start_time.elapsed();
                            token_usage.tokens_per_second = if duration.as_secs_f64() > 0.0
                                && current_completion_tokens > 0.0
                            {
                                current_completion_tokens / duration.as_secs_f64()
                            } else {
                                0.0
                            };
                        }
                        if let Some(content) = chunk.reasoning_content {
                            if !content.is_empty() {
                                full_response.push_str(&format!("<think>{}</think>", content));

                                callback(ChatResponse::new_with_arc(
                                    chat_id.clone(),
                                    content,
                                    MessageType::Reasoning,
                                    metadata_option.clone(),
                                ));
                            }
                        }

                        if let Some(content) = chunk.content {
                            if !content.is_empty() {
                                full_response.push_str(&content);

                                callback(ChatResponse::new_with_arc(
                                    chat_id.clone(),
                                    content,
                                    MessageType::Text,
                                    metadata_option.clone(),
                                ));
                            }
                        }

                        // Accumulate tool call parts
                        if let Some(tool_call_parts) = chunk.tool_calls {
                            for part in tool_call_parts {
                                let entry = accumulated_tool_calls
                                    .entry(part.index)
                                    .or_insert_with(|| ToolCallDeclaration {
                                        index: part.index,
                                        id: part.id.clone(),
                                        name: part.name.clone(),
                                        arguments: Some(String::new()), // Initialize for accumulation
                                        results: None,
                                    });

                                // Update id/name if they were empty and this part provides them
                                // (e.g. first part is content_block_start)
                                if !part.id.is_empty() && entry.id.is_empty() {
                                    entry.id = part.id.clone();
                                }
                                if !part.name.is_empty() && entry.name.is_empty() {
                                    entry.name = part.name.clone();
                                }

                                // Append arguments if this part has them (e.g. from input_json_delta)
                                if let Some(args_chunk) = part.arguments {
                                    if !args_chunk.is_empty() {
                                        entry
                                            .arguments
                                            .get_or_insert_with(String::new)
                                            .push_str(&args_chunk);
                                    }
                                }
                            }
                        }

                        // Check for finish reason that signals tool call completion
                        if chunk.finish_reason.as_deref() == Some("tool_use") {
                            send_tool_calls_signal = true;
                        }
                    }

                    // If a "tool_use" signal was received, send accumulated tool calls
                    if send_tool_calls_signal {
                        if !accumulated_tool_calls.is_empty() {
                            for tcd in accumulated_tool_calls.values() {
                                match serde_json::to_string(tcd) {
                                    Ok(serialized_tcd) => {
                                        callback(ChatResponse::new_with_arc(
                                            chat_id.clone(),
                                            serialized_tcd, // Send the raw data for tool call
                                            MessageType::ToolCall,
                                            metadata_option.clone(),
                                        ));
                                    }
                                    Err(e) => {
                                        let err = AiError::ToolCallSerializationFailed {
                                            details: e.to_string(),
                                        };
                                        log::error!(
                                            "Claude tool call serialization error for tool {:?}: {}",
                                            tcd.name, err
                                        );
                                        callback(ChatResponse::new_with_arc(
                                            chat_id.clone(),
                                            err.to_string(),
                                            MessageType::Error,
                                            metadata_option.clone(),
                                        ));
                                        // Decide if this should be a fatal error for the stream.
                                        // For now, we log, send an error message, and continue processing other tool calls or stream parts.
                                        // If it should be fatal, uncomment the next line:
                                        // return Err(err);
                                    }
                                }
                            }
                            accumulated_tool_calls.clear(); // Clear after sending
                        }
                    }
                }
                Err(e) => {
                    let err = AiError::StreamProcessingFailed {
                        provider: "Claude".to_string(),
                        details: e.to_string(),
                    };
                    log::error!("Claude stream event error: {}", err);
                    callback(ChatResponse::new_with_arc(
                        chat_id.clone(),
                        err.to_string(),
                        MessageType::Error,
                        metadata_option.clone(),
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
        ));
        Ok(full_response)
    }
}

impl_stoppable!(ClaudeChat);

#[async_trait]
impl AiChatTrait for ClaudeChat {
    /// Implements chat functionality for Claude API
    ///
    /// # Arguments
    /// * `api_url` - Optional API endpoint URL
    /// * `model` - The model to use
    /// * `api_key` - Optional API key
    /// * `messages` - The chat messages
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

        let headers = json!({
            "x-api-key": api_key.unwrap_or(""),
            "anthropic-version": "2023-06-01",
            "content-type": "application/json",
        });

        let response = self
            .client
            .post_request(
                &ApiConfig::new(
                    Some(
                        api_url
                            .unwrap_or("https://api.anthropic.com/v1")
                            .to_string(),
                    ),
                    None,
                    get_proxy_type(extra_params),
                    Some(headers),
                ),
                "messages",
                self.build_request_body(model, messages, tools, &params),
                true,
            )
            .await
            .map_err(|network_err| {
                let err = AiError::ApiRequestFailed {
                    provider: "Claude".to_string(),
                    details: network_err.to_string(),
                };
                log::error!("Claude API request failed: {}", err);
                callback(ChatResponse::new_with_arc(
                    chat_id.clone(),
                    err.to_string(),
                    MessageType::Error,
                    metadata_option.clone(),
                ));
                err
            })?;

        if response.is_error {
            let err = AiError::ApiRequestFailed {
                provider: "Claude".to_string(),
                details: response.content.clone(),
            };
            log::error!("Claude API returned an error: {}", err);
            callback(ChatResponse::new_with_arc(
                chat_id.clone(),
                err.to_string(),
                MessageType::Error,
                metadata_option.clone(),
            ));
            return Err(err);
        }

        if let Some(raw_response) = response.raw_response {
            self.handle_stream_response(chat_id, raw_response, callback, metadata_option)
                .await
        } else {
            callback(ChatResponse::new_with_arc(
                chat_id.clone(),
                response.content.clone(),
                MessageType::Finished,
                metadata_option,
            ));
            Ok(response.content)
        }
    }
}
