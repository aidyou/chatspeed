use async_trait::async_trait;
use rust_i18n::t;
use serde_json::{json, Value};
use std::{error::Error, sync::Arc};
use tokio::sync::Mutex;

use crate::ai::network::{
    ApiClient, ApiConfig, DefaultApiClient, ErrorFormat, StreamChunk, StreamFormat,
};
use crate::ai::traits::chat::ChatResponse;
use crate::ai::traits::{chat::AiChatTrait, stoppable::Stoppable};
use crate::impl_stoppable;
use crate::libs::ai_util;

/// Represents the Gemini chat implementation
#[derive(Clone)]
pub struct GeminiChat {
    stop_flag: Arc<Mutex<bool>>,
    client: DefaultApiClient,
}

impl GeminiChat {
    /// Creates a new instance of GeminiChat
    pub fn new() -> Self {
        Self {
            stop_flag: Arc::new(Mutex::new(false)),
            client: DefaultApiClient::new(ErrorFormat::Google),
        }
    }

    /// Builds the request payload for Gemini API
    ///
    /// # Arguments
    /// * `messages` - The chat messages to process
    /// * `params` - Generation parameters like temperature, top_k, etc.
    ///
    /// # Returns
    /// A JSON payload formatted according to Gemini API requirements
    fn build_request_body(&self, messages: Vec<Value>, params: &Value) -> Value {
        let mut contents = Vec::new();
        let mut system_instruction = None;

        for message in messages {
            let role = message["role"].as_str().unwrap_or_default();
            let content = message["content"].as_str().unwrap_or_default();

            if role == "system" {
                system_instruction = Some(json!({
                    "role": "user",
                    "parts": [{"text": content}]
                }));
            } else {
                contents.push(json!({
                    "role": if role == "assistant" { "model" } else { "user" },
                    "parts": [{"text": content}]
                }));
            }
        }

        let mut payload = json!({
            "contents": contents,
            "generationConfig": {
                "temperature": params.get("temperature").unwrap(),
                "topK": params.get("top_k").unwrap(),
                "topP": params.get("top_p").unwrap(),
                "maxOutputTokens": params.get("max_tokens").unwrap(),
                "responseMimeType": "text/plain"
            }
        });

        if let Some(instruction) = system_instruction {
            payload["systemInstruction"] = instruction;
        }

        payload
    }

    /// Processes the response from Gemini API
    ///
    /// # Arguments
    /// * `response` - The API response content
    /// * `callback` - Callback function for sending updates
    /// * `metadata_option` - Optional metadata to include in callbacks
    ///
    /// # Returns
    /// The generated text or an error
    async fn process_response(
        &self,
        chat_id: String,
        response: String,
        callback: impl Fn(Arc<ChatResponse>) + Send + 'static,
        metadata_option: Option<Value>,
    ) -> Result<String, Box<dyn Error + Send + Sync>> {
        let StreamChunk {
            reasoning_content,
            content,
            usage,
            msg_type,
        } = self
            .client
            .process_stream_chunk(response.as_bytes().to_vec().into(), &StreamFormat::Gemini)
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

        if self.should_stop().await {
            return Ok(String::new());
        }
        let reasoning = if let Some(content) = reasoning_content {
            content
        } else {
            String::new()
        };

        let content = content.ok_or_else(|| {
            let error = t!("chat.failed_to_extract_text").to_string();
            callback(ChatResponse::new_with_arc(
                chat_id.clone(),
                error.clone(),
                true,
                true,
                false,
                msg_type.clone(),
                metadata_option.clone(),
            ));
            error
        })?;

        callback(ChatResponse::new_with_arc(
            chat_id.clone(),
            format!("{}{}", reasoning, content.clone()),
            false,
            true,
            false,
            msg_type.clone(),
            Some(ai_util::update_or_create_metadata(
                metadata_option,
                "tokens",
                json!({
                    "total": usage.as_ref().map(|u| u.total_tokens).unwrap_or(0),
                    "prompt": usage.as_ref().map(|u| u.prompt_tokens).unwrap_or(0),
                    "completion": usage.as_ref().map(|u| u.completion_tokens).unwrap_or(0)
                }),
            )),
        ));

        Ok(content)
    }
}

impl_stoppable!(GeminiChat);

#[async_trait]
impl AiChatTrait for GeminiChat {
    /// Implements chat functionality for Gemini API
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
                    Some(format!(
                        "{}/{}:generateContent?key={}",
                        api_url
                            .unwrap_or("https://generativelanguage.googleapis.com/v1beta/models"),
                        model,
                        api_key.unwrap_or("")
                    )),
                    None,
                    ai_util::get_proxy_type(extra_params),
                    None,
                ),
                "",
                self.build_request_body(messages, &params),
                false,
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

        self.process_response(chat_id.clone(), response.content, callback, metadata_option)
            .await
    }
}
