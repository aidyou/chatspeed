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
                "temperature": params.get("temperature").unwrap_or(&json!(1.0)),
                "topK": params.get("top_k").unwrap_or(&json!(40)),
                "topP": params.get("top_p").unwrap_or(&json!(1.0)),
                "maxOutputTokens": params.get("max_tokens").unwrap_or(&json!(4096)),
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
                .process_stream_chunk(chunk, &StreamFormat::Gemini)
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
                if let Some(con) = chunk.content.clone() {
                    if !con.is_empty() {
                        full_response.push_str(&con);

                        callback(ChatResponse::new_with_arc(
                            chat_id.clone(),
                            con.clone(),
                            MessageType::Text,
                            metadata_option.clone(),
                        ));
                    }
                };

                if let Some(usage) = chunk.usage {
                    if usage.total_tokens > 0 {
                        token_usage = usage;

                        // Gemini does not provide tokens per second, so calculate it here
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
            }
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
        ));

        Ok(full_response)
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
        let (params, metadata_option) = init_extra_params(extra_params.clone());

        let base_url = api_url.unwrap_or("https://generativelanguage.googleapis.com/v1beta/models");
        let query_url = if params["stream"].as_bool().unwrap_or(false) {
            format!(
                "{}/{}:streamGenerateContent?alt=sse&key={}",
                base_url,
                model,
                api_key.unwrap_or("")
            )
        } else {
            format!(
                "{}/{}:generateContent?key={}",
                base_url,
                model,
                api_key.unwrap_or("")
            )
        };

        let response = self
            .client
            .post_request(
                &ApiConfig::new(Some(query_url), None, get_proxy_type(extra_params), None),
                "",
                self.build_request_body(messages, &params),
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
            self.process_response(chat_id.clone(), raw_response, callback, metadata_option)
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
