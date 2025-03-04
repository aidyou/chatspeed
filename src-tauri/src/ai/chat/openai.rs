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
        mut response: Response,
        callback: impl Fn(Arc<ChatResponse>) + Send + 'static,
        metadata_option: Option<Value>,
    ) -> Result<String, Box<dyn Error + Send + Sync>> {
        let mut full_response = String::new();
        let mut token_usage = TokenUsage::default();

        while let Some(chunk) = response.chunk().await.map_err(|e| {
            let error = t!("chat.stream_read_error", error = e.to_string());
            callback(ChatResponse::new_with_arc(
                chat_id.clone(),
                error.clone().to_string(),
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
                .process_stream_chunk(chunk, &StreamFormat::OpenAI)
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
                        true,
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

impl_stoppable!(OpenAIChat);

#[async_trait]
impl AiChatTrait for OpenAIChat {
    /// Implements chat functionality for OpenAI API
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

        let response = self
            .client
            .post_request(
                &ApiConfig::new(
                    Some(api_url.unwrap_or("https://api.openai.com/v1").to_string()),
                    api_key.map(String::from),
                    ai_util::get_proxy_type(extra_params),
                    None,
                ),
                "chat/completions",
                json!({
                    "model": model,
                    "messages": messages,
                    "stream": params.get("stream").unwrap(),
                    "max_tokens": params.get("max_tokens").unwrap(),
                    "temperature": params.get("temperature").unwrap(),
                    "top_p": params.get("top_p").unwrap(),
                }),
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
