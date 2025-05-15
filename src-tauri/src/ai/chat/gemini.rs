use async_trait::async_trait;
use reqwest::Response;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Instant;
use std::{error::Error, sync::Arc};
use tokio::sync::Mutex; // Import Mutex

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
    fn build_request_body(
        &self,
        messages: Vec<Value>,
        tools: Option<Vec<MCPToolDeclaration>>,
        params: &Value,
    ) -> Value {
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

        let response_format_type = params
            .get("response_format")
            .and_then(|rf| rf.get("type"))
            .and_then(|t| t.as_str())
            .unwrap_or("text");

        let response_mime_type = if response_format_type == "json_object" {
            "application/json"
        } else {
            "text/plain"
        };

        let mut payload = json!({
            "contents": contents,
            "generationConfig": {
                "temperature": params.get("temperature").unwrap_or(&json!(1.0)),
                "topK": params.get("top_k").unwrap_or(&json!(40)),
                "topP": params.get("top_p").unwrap_or(&json!(1.0)),
                "maxOutputTokens": params.get("max_tokens").unwrap_or(&json!(4096)),
                "responseMimeType": response_mime_type
            }
        });

        if let Some(obj) = payload.as_object_mut() {
            if let Some(instruction) = system_instruction {
                obj.insert("systemInstruction".to_string(), instruction);
            }

            // Modify the existing generationConfig object instead of replacing it
            if let Some(generation_config_value) = obj.get_mut("generationConfig") {
                if let Some(generation_config_map) = generation_config_value.as_object_mut() {
                    if let Some(stop_sequences) =
                        params.get("stop_sequences").and_then(|v| v.as_array())
                    {
                        if !stop_sequences.is_empty() {
                            generation_config_map
                                .insert("stopSequences".to_string(), json!(stop_sequences.clone()));
                        }
                    }

                    if let Some(candidate_count) =
                        params.get("candidate_count").and_then(|v| v.as_u64())
                    {
                        if candidate_count > 0 {
                            generation_config_map
                                .insert("candidateCount".to_string(), json!(candidate_count));
                        }
                    }
                }
            }

            if let Some(tools) = tools {
                if params.get("tool_choice").and_then(|tc| tc.as_str()) != Some("none") {
                    let gemini_tools = tools
                        .into_iter()
                        .map(|tool| tool.to_gemini())
                        .collect::<Vec<Value>>();
                    if !gemini_tools.is_empty() {
                        obj.insert(
                            "tools".to_string(),
                            json!([{ "functionDeclarations": gemini_tools }]),
                        );
                    }
                }
            }
        }

        #[cfg(debug_assertions)]
        {
            log::debug!(
                "Gemini payload: {}",
                serde_json::to_string_pretty(&payload).unwrap_or_default()
            );
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
        response: Response,
        callback: impl Fn(Arc<ChatResponse>) + Send + 'static,
        metadata_option: Option<Value>,
    ) -> Result<String, Box<dyn Error + Send + Sync>> {
        let mut full_response = String::new();
        let mut token_usage = TokenUsage::default();
        let start_time = Instant::now();

        let processor = StreamProcessor::new();
        let mut event_receiver = processor
            .process_stream(response, &StreamFormat::Gemini)
            .await;

        // Used to accumulate tool call information, with the key being the index of the tool call (tool_call.index)
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
                        // Handle text content
                        if let Some(content) = chunk.content.clone() {
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

                        // Handle tool calls
                        if let Some(tool_call_parts) = chunk.tool_calls {
                            for part in tool_call_parts {
                                // part is a ToolCallDeclaration
                                let acc_call = accumulated_tool_calls
                                    .entry(part.index) // Use part.index as HashMap key
                                    .or_insert_with(|| {
                                        // First time encountering this index, initialize with name and empty arguments
                                        ToolCallDeclaration {
                                            index: part.index,
                                            id: part.id.clone(), // Will be empty for Gemini stream delta
                                            name: part.name.clone(),
                                            arguments: Some(String::new()), // Initialize with empty string
                                            results: None,
                                        }
                                    });

                                // Append current part's arguments
                                if let Some(args_chunk) = part.arguments {
                                    if !args_chunk.is_empty() {
                                        // acc_call.arguments was initialized as Some(String::new()) in or_insert_with
                                        if let Some(existing_args) = acc_call.arguments.as_mut() {
                                            existing_args.push_str(&args_chunk);
                                        }
                                    }
                                }
                            }
                        }

                        // Check finish reason to see if tool calls are complete
                        // Gemini uses "STOP" or "TOOL_CODE" for tool call completion
                        if chunk.finish_reason == Some("STOP".to_string())
                            || chunk.finish_reason == Some("TOOL_CODE".to_string())
                        {
                            for tcd in accumulated_tool_calls.values() {
                                // Send the complete tool call (arguments should be full JSON string now)
                                callback(ChatResponse::new_with_arc(
                                    chat_id.clone(),
                                    serde_json::to_string(tcd).unwrap_or_default(),
                                    MessageType::ToolCall,
                                    metadata_option.clone(),
                                ));
                                #[cfg(debug_assertions)]
                                {
                                    log::debug!(
                                        "Tool call: {}",
                                        serde_json::to_string_pretty(tcd).unwrap_or_default()
                                    );
                                }
                            }
                            accumulated_tool_calls.clear(); // Clear for next potential tool call sequence
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
        tools: Option<Vec<MCPToolDeclaration>>,
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
                self.build_request_body(messages, tools, &params),
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
