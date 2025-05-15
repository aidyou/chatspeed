use async_trait::async_trait;
use reqwest::Response;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Instant;
use std::{error::Error, sync::Arc};
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
use crate::impl_stoppable;

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
    ) -> Result<String, Box<dyn Error + Send + Sync>> {
        let mut full_response = String::new();
        let mut token_usage = TokenUsage::default();
        let start_time = Instant::now();

        let processor = StreamProcessor::new();
        let mut event_receiver = processor
            .process_stream(response, &StreamFormat::OpenAI)
            .await;

        // 用于累积工具调用信息，键为工具调用的索引 (tool_call.index)
        let mut accumulated_tool_calls: HashMap<u32, ToolCallDeclaration> = HashMap::new();

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
                            callback(ChatResponse::new_with_arc(
                                chat_id.clone(),
                                e.clone(),
                                MessageType::Error,
                                metadata_option.clone(),
                            ));
                            Box::new(std::io::Error::new(std::io::ErrorKind::Other, e))
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
                                let msg_type = chunk.msg_type.unwrap_or(MessageType::Text);
                                callback(ChatResponse::new_with_arc(
                                    chat_id.clone(),
                                    content,
                                    msg_type,
                                    metadata_option.clone(),
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
                            for tcd in accumulated_tool_calls.values() {
                                let serialized_tcd =
                                    // Trim arguments to remove leading and trailing whitespace
                                    serde_json::to_string(&{
                                        let mut trimmed_tcd = tcd.clone();
                                        if let Some(args) = trimmed_tcd.arguments.as_mut() {
                                            *args = args.trim().to_string();
                                        }
                                        trimmed_tcd
                                    })
                                    .unwrap_or_else(|e| {
                                        log::error!("Failed to serialize ToolCallDeclaration: {}", e);
                                        "{}".to_string() // return empty string if serialization fails
                                    });
                                callback(ChatResponse::new_with_arc(
                                    chat_id.clone(),
                                    serialized_tcd,
                                    MessageType::ToolCall,
                                    metadata_option.clone(),
                                ));

                                #[cfg(debug_assertions)]
                                {
                                    log::debug!(
                                        "tool_call: {}",
                                        serde_json::to_string_pretty(tcd).unwrap_or_default()
                                    );
                                }
                            }
                            accumulated_tool_calls.clear(); // Clear for next batch (though uncommon)
                        }
                    }
                }
                Err(e) => {
                    callback(ChatResponse::new_with_arc(
                        chat_id.clone(),
                        e,
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
    ) -> Result<String, Box<dyn Error + Send + Sync>> {
        let (params, metadata_option) = init_extra_params(extra_params.clone());

        let mut payload = json!({
            "model": model,
            "messages": messages,
            "stream": params.get("stream").unwrap_or(&json!(true)),
            "max_tokens": params.get("max_tokens").unwrap_or(&json!(4096)),
            "temperature": params.get("temperature").unwrap_or(&json!(1.0)),
            "top_p": params.get("top_p").unwrap_or(&json!(1.0)),
            "frequency_penalty": params.get("frequency_penalty").unwrap_or(&json!(0.0)),
            "presence_penalty": params.get("presence_penalty").unwrap_or(&json!(0.0)),
            "response_format": params.get("response_format").unwrap_or(&json!("text")),
        });

        // Add optional parameters if they exist and are not null
        if let Some(obj) = payload.as_object_mut() {
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
                        obj.insert(
                            "tool_choice".to_string(),
                            params.get("tool_choice").cloned().unwrap_or(json!("auto")),
                        );
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
                    Some(api_url.unwrap_or("https://api.openai.com/v1").to_string()),
                    api_key.map(String::from),
                    get_proxy_type(extra_params),
                    None,
                ),
                "chat/completions",
                payload,
                true,
            )
            .await
            .map_err(|e| {
                callback(ChatResponse::new_with_arc(
                    chat_id.clone(),
                    e.clone(),
                    MessageType::Error,
                    metadata_option.clone(),
                ));
                Box::new(std::io::Error::new(std::io::ErrorKind::Other, e))
            })?;

        if response.is_error {
            callback(ChatResponse::new_with_arc(
                chat_id.clone(),
                response.content.clone(),
                MessageType::Error,
                metadata_option,
            ));
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                response.content,
            )));
        }

        if let Some(raw_response) = response.raw_response {
            self.handle_stream_response(chat_id.clone(), raw_response, callback, metadata_option)
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
