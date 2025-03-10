use async_trait::async_trait;
use reqwest::Response;
use rust_i18n::t;
use serde_json::{json, Value};
use std::{error::Error, sync::Arc};
use tokio::sync::Mutex;

use crate::ai::network::{
    ApiClient, ApiConfig, DefaultApiClient, ErrorFormat, StreamChunk, StreamFormat, TokenUsage,
};
use crate::ai::traits::chat::ChatResponse;
use crate::ai::traits::{chat::AiChatTrait, stoppable::Stoppable};
use crate::impl_stoppable;
use crate::libs::ai_util;

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
            "max_tokens": params.get("max_tokens").unwrap(),
            "temperature": params.get("temperature").unwrap(),
            "stream": params.get("stream").unwrap(),
            "top_p": params.get("top_p").unwrap(),
            "top_k": params.get("top_k").unwrap(),
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

        while let Some(chunk) = response.chunk().await.map_err(|e| {
            let error = t!("chat.stream_read_error", error = e.to_string()).to_string();
            callback(ChatResponse::new_with_arc(
                chat_id.clone(),
                error.clone(),
                true,
                true,
                false,
                None,
                metadata_option.clone(),
            ));
            Box::new(std::io::Error::new(std::io::ErrorKind::Other, error))
        })? {
            if self.should_stop().await {
                break;
            }

            let StreamChunk {
                reasoning_content,
                content,
                usage,
                msg_type,
            } = self
                .client
                .process_stream_chunk(chunk, &StreamFormat::Anthropic)
                .await
                .map_err(|e| {
                    callback(ChatResponse::new_with_arc(
                        chat_id.clone(),
                        e.to_string(),
                        true,
                        true,
                        false,
                        None,
                        metadata_option.clone(),
                    ));
                    Box::new(std::io::Error::new(std::io::ErrorKind::Other, e))
                })?;

            if let Some(new_usage) = usage {
                token_usage = new_usage;
            }

            if let Some(content) = reasoning_content {
                if !content.is_empty() {
                    full_response.push_str(&content);
                    callback(ChatResponse::new_with_arc(
                        chat_id.clone(),
                        content,
                        false,
                        false,
                        false,
                        msg_type.clone(),
                        metadata_option.clone(),
                    ));
                }
            }
            if let Some(content) = content {
                if !content.is_empty() {
                    full_response.push_str(&content);
                    callback(ChatResponse::new_with_arc(
                        chat_id.clone(),
                        content,
                        false,
                        false,
                        false,
                        msg_type.clone(),
                        metadata_option.clone(),
                    ));
                }
            }
        }

        // Send final response with token usage
        callback(ChatResponse::new_with_arc(
            chat_id.clone(),
            String::new(),
            false,
            true,
            false,
            None,
            Some(ai_util::update_or_create_metadata(
                metadata_option,
                "tokens",
                json!({
                    "total": token_usage.total_tokens,
                    "prompt": token_usage.prompt_tokens,
                    "completion": token_usage.completion_tokens
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
        let (params, metadata_option) = ai_util::init_extra_params(extra_params.clone());

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
                    ai_util::get_proxy_type(extra_params),
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
                    true,
                    true,
                    false,
                    None,
                    metadata_option.clone(),
                ));
                Box::new(std::io::Error::new(std::io::ErrorKind::Other, e))
            })?;

        if response.is_error {
            callback(ChatResponse::new_with_arc(
                chat_id.clone(),
                response.content.clone(),
                true,
                true,
                false,
                None,
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
                false,
                true,
                false,
                None,
                metadata_option,
            ));
            Ok(response.content)
        }
    }
}
