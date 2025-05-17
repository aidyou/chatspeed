use async_trait::async_trait;
use reqwest::Response;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

use crate::ai::error::AiError;
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
            let content_value = &message["content"]; // Keep as Value

            // Handle different content types (string or array of parts)
            let parts = if content_value.is_string() {
                vec![json!({"text": content_value.as_str().unwrap_or_default()})]
            } else if content_value.is_array() {
                // Assuming the array structure is compatible or needs specific mapping
                // For now, let's try to pass it as is if it's an array of parts.
                // This might need adjustment based on actual content structure for Gemini.
                content_value
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|part| {
                        // A simple heuristic: if part is a string, wrap it. If object, assume it's a valid part.
                        if part.is_string() {
                            json!({"text": part.as_str().unwrap_or_default()})
                        } else {
                            part
                        }
                    })
                    .collect()
            } else {
                // Default or error handling for unexpected content type
                vec![json!({"text": ""})]
            };

            if role == "system" {
                system_instruction = Some(json!({
                    "role": "user", // Gemini system prompts are passed as user role in systemInstruction
                    "parts": parts
                }));
            } else if role == "tool" {
                // Handle tool messages
                contents.push(json!({
                    "role": "function", // Gemini uses "function" for tool responses
                    "parts": parts // Assuming parts here are structured for function response
                }));
            } else {
                contents.push(json!({
                    "role": if role == "assistant" { "model" } else { "user" },
                    "parts": parts
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
                "topP": params.get("top_p").unwrap_or(&json!(0.95)),
                "maxOutputTokens": params.get("max_tokens").unwrap_or(&json!(8192)),
                "responseMimeType": response_mime_type
            }
        });

        if let Some(obj) = payload.as_object_mut() {
            if let Some(instruction) = system_instruction {
                obj.insert("systemInstruction".to_string(), instruction);
            }

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

            if let Some(tools_vec) = tools {
                if params.get("tool_choice").and_then(|tc| tc.as_str()) != Some("none") {
                    let gemini_tools = tools_vec // Use the renamed variable
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

    /// Processes the response from Gemini API (streaming)
    async fn process_response(
        &self,
        chat_id: String,
        response: Response,
        callback: impl Fn(Arc<ChatResponse>) + Send + 'static,
        metadata_option: Option<Value>,
    ) -> Result<String, AiError> {
        let mut full_response = String::new();
        let mut token_usage = TokenUsage::default();
        let start_time = Instant::now();

        let processor = StreamProcessor::new();
        let mut event_receiver = processor
            .process_stream(response, &StreamFormat::Gemini)
            .await;

        let mut accumulated_tool_calls: HashMap<u32, ToolCallDeclaration> = HashMap::new();

        while let Some(event) = event_receiver.recv().await {
            if self.should_stop().await {
                processor.stop();
                break;
            }

            match event {
                Ok(chunk_bytes) => {
                    // Renamed from chunk to chunk_bytes for clarity
                    let stream_chunks = self
                        .client
                        .process_stream_chunk(chunk_bytes, &StreamFormat::Gemini)
                        .await
                        .map_err(|e| {
                            let err = AiError::StreamProcessingFailed {
                                provider: "Gemini".to_string(),
                                details: e.to_string(),
                            };
                            log::error!("Gemini stream processing error: {}", err);
                            callback(ChatResponse::new_with_arc(
                                chat_id.clone(),
                                err.to_string(),
                                MessageType::Error,
                                metadata_option.clone(),
                            ));
                            err
                        })?;

                    for chunk in stream_chunks {
                        // Iterate over processed chunks
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
                                // Gemini might send usage at the end or with specific events
                                token_usage = usage;
                                let completion_tokens = token_usage.completion_tokens as f64;
                                let duration = start_time.elapsed();
                                token_usage.tokens_per_second =
                                    if duration.as_secs_f64() > 0.0 && completion_tokens > 0.0 {
                                        completion_tokens / duration.as_secs_f64()
                                    } else {
                                        0.0
                                    };
                            }
                        }

                        if let Some(reasoning_content) = chunk.reasoning_content {
                            if !reasoning_content.is_empty() {
                                full_response
                                    .push_str(&format!("<think>{}</think>", reasoning_content));
                                callback(ChatResponse::new_with_arc(
                                    chat_id.clone(),
                                    reasoning_content,
                                    MessageType::Reasoning,
                                    metadata_option.clone(),
                                ));
                            }
                        }

                        if let Some(tool_call_parts) = chunk.tool_calls {
                            for part in tool_call_parts {
                                let acc_call = accumulated_tool_calls
                                    .entry(part.index)
                                    .or_insert_with(|| ToolCallDeclaration {
                                        index: part.index,
                                        id: part.id.clone(), // Gemini might not provide ID in delta
                                        name: part.name.clone(),
                                        arguments: Some(String::new()),
                                        results: None,
                                    });

                                if !part.id.is_empty() && acc_call.id.is_empty() {
                                    acc_call.id = part.id.clone();
                                }
                                if !part.name.is_empty() && acc_call.name.is_empty() {
                                    acc_call.name = part.name.clone();
                                }

                                if let Some(args_chunk) = part.arguments {
                                    if !args_chunk.is_empty() {
                                        acc_call
                                            .arguments
                                            .get_or_insert_with(String::new)
                                            .push_str(&args_chunk);
                                    }
                                }
                            }
                        }

                        if chunk.finish_reason == Some("STOP".to_string()) // General stop
                            || chunk.finish_reason == Some("TOOL_CODE".to_string()) // Tool use finished
                            || chunk.finish_reason == Some("FUNCTION_CALL".to_string())
                        // Alternative for tool/function call finish
                        {
                            if !accumulated_tool_calls.is_empty() {
                                for tcd in accumulated_tool_calls.values() {
                                    match serde_json::to_string(tcd) {
                                        Ok(serialized_tcd) => {
                                            callback(ChatResponse::new_with_arc(
                                                chat_id.clone(),
                                                serialized_tcd,
                                                MessageType::ToolCall,
                                                metadata_option.clone(),
                                            ));
                                            #[cfg(debug_assertions)]
                                            {
                                                log::debug!(
                                                    "Gemini Tool call: {}",
                                                    serde_json::to_string_pretty(tcd)
                                                        .unwrap_or_default()
                                                );
                                            }
                                        }
                                        Err(e) => {
                                            let err = AiError::ToolCallSerializationFailed {
                                                details: e.to_string(),
                                            };
                                            log::error!("Gemini tool call serialization error for tool {:?}: {}", tcd.name, err);
                                            callback(ChatResponse::new_with_arc(
                                                chat_id.clone(),
                                                err.to_string(),
                                                MessageType::Error,
                                                metadata_option.clone(),
                                            ));
                                        }
                                    }
                                }
                                accumulated_tool_calls.clear();
                            }
                        }
                    }
                }
                Err(e) => {
                    // Error from SSE processor
                    let err = AiError::StreamProcessingFailed {
                        provider: "Gemini".to_string(),
                        details: e.to_string(), // e is likely String here from StreamProcessor
                    };
                    log::error!("Gemini stream event error: {}", err);
                    callback(ChatResponse::new_with_arc(
                        chat_id.clone(),
                        err.to_string(),
                        MessageType::Error,
                        metadata_option.clone(),
                    ));
                }
            }
        }

        callback(ChatResponse::new_with_arc(
            chat_id.clone(),
            String::new(), // Empty content for Finished message
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
        // Changed return type
        let (params, metadata_option) = init_extra_params(extra_params.clone());

        let base_url = api_url.unwrap_or("https://generativelanguage.googleapis.com/v1beta/models");
        // Determine if streaming is requested from params, default to true if not specified
        let is_streaming = params
            .get("stream")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        let query_url = if is_streaming {
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
                "", // Gemini endpoint path is part of query_url
                self.build_request_body(messages, tools, &params),
                true,
            )
            .await
            .map_err(|network_err| {
                let err = AiError::ApiRequestFailed {
                    provider: "Gemini".to_string(),
                    details: network_err.to_string(),
                };
                log::error!("Gemini API request failed: {}", err);
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
                provider: "Gemini".to_string(),
                details: response.content.clone(),
            };
            log::error!("Gemini API returned an error: {}", err);
            callback(ChatResponse::new_with_arc(
                chat_id.clone(),
                err.to_string(),
                MessageType::Error,
                metadata_option.clone(),
            ));
            return Err(err);
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
