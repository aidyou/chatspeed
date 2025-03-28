use async_trait::async_trait;
use reqwest::Response;
use rust_i18n::t;
use serde_json::{json, Value};
use std::time::Instant;
use std::{error::Error, sync::Arc};
use tokio::sync::Mutex;

use crate::ai::interaction::constants::{
    TOKENS, TOKENS_COMPLETION, TOKENS_PER_SECOND, TOKENS_PROMPT, TOKENS_TOTAL,
};
use crate::ai::network::{
    ApiClient, ApiConfig, DefaultApiClient, ErrorFormat, StreamFormat, TokenUsage,
};
use crate::ai::traits::chat::{ChatResponse, MessageType};
use crate::ai::traits::{chat::AiChatTrait, stoppable::Stoppable};
use crate::ai::util::{get_proxy_type, init_extra_params, update_or_create_metadata};
use crate::impl_stoppable;

/// Represents the Anthropic chat implementation
#[derive(Clone)]
pub struct AnthropicChat {
    stop_flag: Arc<Mutex<bool>>,
    client: DefaultApiClient,
}

impl AnthropicChat {
    /// Creates a new instance of AnthropicChat
    pub fn new() -> Self {
        Self {
            stop_flag: Arc::new(Mutex::new(false)),
            client: DefaultApiClient::new(ErrorFormat::Anthropic),
        }
    }

    /// Builds the request payload for Anthropic API
    ///
    /// # Arguments
    /// * `model` - The model to use
    /// * `messages` - The chat messages to process
    /// * `params` - Generation parameters like temperature, top_k, etc.
    ///
    /// # Returns
    /// A JSON payload formatted according to Anthropic API requirements
    fn build_request_body(&self, model: &str, messages: Vec<Value>, params: &Value) -> Value {
        json!({
            "model": model,
            "messages": messages,
            "max_tokens": params.get("max_tokens").unwrap_or(&json!(4096)),
            "temperature": params.get("temperature").unwrap_or(&json!(1.0)),
            "stream": params.get("stream").unwrap_or(&json!(true)),
            "top_p": params.get("top_p").unwrap_or(&json!(1.0)),
            "top_k": params.get("top_k").unwrap_or(&json!(40)),
        })
    }

    /// Processes streaming response
    ///
    /// # Arguments
    /// * `response` - Raw streaming response from Anthropic API
    /// * `callback` - Function for sending updates to the client
    /// * `metadata_option` - Optional metadata to include in callbacks
    async fn handle_stream_response(
        &self,
        chat_id: String,
        mut response: Response,
        callback: impl Fn(Arc<ChatResponse>) + Send + 'static,
        metadata_option: Option<Value>,
    ) -> Result<String, Box<dyn Error + Send + Sync>> {
        let mut full_response = String::new();
        let mut token_usage = TokenUsage::default();
        let start_time = Instant::now();

        while let Some(chunk) = response.chunk().await.map_err(|e| {
            let error = t!("chat.stream_read_error", error = e.to_string()).to_string();
            callback(ChatResponse::new_with_arc(
                chat_id.clone(),
                error.clone(),
                MessageType::Error,
                metadata_option.clone(),
            ));
            Box::new(std::io::Error::new(std::io::ErrorKind::Other, error))
        })? {
            if self.should_stop().await {
                break;
            }

            let chunks = self
                .client
                .process_stream_chunk(chunk, &StreamFormat::Anthropic)
                .await
                .map_err(|e| {
                    callback(ChatResponse::new_with_arc(
                        chat_id.clone(),
                        e.to_string(),
                        MessageType::Error,
                        metadata_option.clone(),
                    ));
                    Box::new(std::io::Error::new(std::io::ErrorKind::Other, e))
                })?;

            for chunk in chunks {
                if let Some(new_usage) = chunk.usage {
                    token_usage = new_usage;

                    // Anthropic does not provide tokens per second, so calculate it here
                    let completion_tokens = token_usage.completion_tokens as f64;
                    let duration = start_time.elapsed();
                    token_usage.tokens_per_second = if duration.as_secs() > 0 {
                        completion_tokens / duration.as_secs_f64()
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

impl_stoppable!(AnthropicChat);

#[async_trait]
impl AiChatTrait for AnthropicChat {
    /// Implements chat functionality for Anthropic API
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
        extra_params: Option<Value>,
        callback: impl Fn(Arc<ChatResponse>) + Send + 'static,
    ) -> Result<String, Box<dyn Error + Send + Sync>> {
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
                self.build_request_body(model, messages, &params),
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
                metadata_option.clone(),
            ));
            return Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                response.content,
            )));
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
