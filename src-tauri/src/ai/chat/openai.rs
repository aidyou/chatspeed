use async_trait::async_trait;
use reqwest::Response;
use rust_i18n::t;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::str::FromStr;
use std::{sync::Arc, time::Instant};
use tokio::sync::Mutex;

use crate::ai::network::{ApiClient, ApiConfig, DefaultApiClient, ErrorFormat, TokenUsage};
use crate::ai::traits::chat::{
    ChatMetadata, ChatResponse, FinishReason, MCPToolDeclaration, MessageType, ModelDetails,
    ToolCallDeclaration,
};
use crate::ai::traits::{chat::AiChatTrait, stoppable::Stoppable};
use crate::ai::util::{
    init_request_params, init_request_params_value, merge_custom_params, merge_custom_params_value,
    process_custom_headers,
};
use crate::ccproxy::adapter::input::helper::thinking_adapter::build_openai_compat_thinking_fields;
use crate::ccproxy::{ChatProtocol, StreamFormat, StreamProcessor};
use crate::db::{AiModel, ModelConfig};
use crate::{
    ai::error::AiError, constants::INTERNAL_CCPROXY_API_KEY, db::MainStore, impl_stoppable,
};

/// A standardized error structure for streaming to the frontend.
#[derive(Serialize)]
struct JsonErrorPayload<'a> {
    status: u16,
    message: &'a str,
}

const MAX_TOOL_CALLS_PER_RESPONSE: usize = 15;

fn extract_reasoning_from_openai_message(message: &Value) -> String {
    if let Some(reasoning_content) = message
        .get("reasoning_content")
        .and_then(|value| value.as_str())
    {
        let trimmed = reasoning_content.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    if let Some(thinking) = message.get("thinking").and_then(|value| value.as_str()) {
        let trimmed = thinking.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    if let Some(details) = message
        .get("reasoning_details")
        .and_then(|value| value.as_array())
    {
        let combined = details
            .iter()
            .filter(|detail| {
                detail.get("type").and_then(|value| value.as_str()) == Some("reasoning.text")
            })
            .filter_map(|detail| detail.get("text").and_then(|value| value.as_str()))
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if !combined.is_empty() {
            return combined;
        }
    }

    String::new()
}

struct PreparedToolCalls {
    calls: Vec<(u32, ToolCallDeclaration)>,
    raw_count: usize,
    duplicate_count: usize,
    limit_dropped_count: usize,
}

/// OpenAI chat implementation
#[derive(Clone)]
pub struct OpenAIChat {
    stop_flag: Arc<Mutex<bool>>,
    client: DefaultApiClient,
    main_store: Arc<std::sync::RwLock<MainStore>>,
}

impl OpenAIChat {
    /// Creates a new instance of OpenAIChat
    pub fn new(main_store: Arc<std::sync::RwLock<MainStore>>) -> Self {
        Self {
            stop_flag: Arc::new(Mutex::new(false)),
            client: DefaultApiClient::new(ErrorFormat::OpenAI),
            main_store,
        }
    }

    /// Processes streaming response
    async fn handle_stream_response(
        &self,
        chat_id: String,
        response: Response,
        callback: impl Fn(Arc<ChatResponse>) + Send + 'static,
        metadata_option: Option<ChatMetadata>,
        provider_name: String, // Added to correctly attribute errors
        is_stream: bool,       // Added to distinguish between stream and non-stream responses
    ) -> Result<String, AiError> {
        // If not streaming, handle as non-streaming response
        if !is_stream {
            let response_text = response.text().await.map_err(|e| {
                let err = AiError::ApiRequestFailed {
                    status_code: 500,
                    provider: provider_name.clone(),
                    details: format!("Failed to read response body: {}", e),
                };
                log::error!("{} response reading error: {}", provider_name, err);
                err
            })?;
            return self
                .handle_non_stream_response(
                    chat_id,
                    response_text,
                    callback,
                    metadata_option,
                    provider_name,
                )
                .await;
        }

        // Streaming response handling
        let mut full_response = String::new();
        let mut reasoning_content = String::new();
        let mut token_usage = TokenUsage::default();
        let start_time = Instant::now();

        let processor = StreamProcessor::new();
        let mut event_receiver = processor
            .process_stream(response, &StreamFormat::OpenAI)
            .await;

        // Used to accumulate tool call information, key is the tool call index (tool_call.index)
        let mut accumulated_tool_calls: HashMap<u32, ToolCallDeclaration> = HashMap::new();
        let mut finish_reason = FinishReason::Complete;

        while let Some(event) = event_receiver.recv().await {
            if self.should_stop().await {
                processor.stop();
                break;
            }

            match event {
                Ok(chunk) => {
                    let chunks = self.client.process_stream_chunk(chunk).await.map_err(|e| {
                        let err = AiError::StreamProcessingFailed {
                            provider: provider_name.clone(),
                            details: e.to_string(),
                        };
                        log::error!("{} stream processing error: {}", provider_name, err);

                        let error_payload = JsonErrorPayload {
                            status: 500, // Internal processing error
                            message: &err.to_string(),
                        };
                        let chunk = serde_json::to_string(&error_payload)
                            .unwrap_or_else(|_| err.to_string());

                        callback(ChatResponse::new_with_arc(
                            chat_id.clone(),
                            chunk,
                            MessageType::Error,
                            metadata_option.as_ref().and_then(|m| m.to_value()),
                            Some(FinishReason::Error),
                        ));
                        err
                    })?;

                    for chunk in chunks {
                        // CRITICAL: Always capture usage regardless of other fields
                        if let Some(new_usage) = chunk.usage {
                            if new_usage.total_tokens > 0 {
                                token_usage = new_usage;

                                // OpenAI does not provide tokens per second, so calculate it here
                                if token_usage.tokens_per_second == 0.0 {
                                    let completion_tokens = token_usage.completion_tokens as f64;
                                    let duration = start_time.elapsed();
                                    token_usage.tokens_per_second = if duration.as_secs_f64() > 0.1
                                    {
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
                                    metadata_option.as_ref().and_then(|m| m.to_value()),
                                    None,
                                ));
                            }
                        }

                        if let Some(content) = chunk.content {
                            if !content.is_empty() {
                                let msg_type = chunk.msg_type.clone().unwrap_or(MessageType::Text);
                                if msg_type == MessageType::Error {
                                    callback(ChatResponse::new_with_arc(
                                        chat_id.clone(),
                                        content.clone(),
                                        MessageType::Error,
                                        metadata_option.as_ref().and_then(|m| m.to_value()),
                                        Some(FinishReason::Error),
                                    ));
                                    return Err(AiError::StreamProcessingFailed {
                                        provider: provider_name.clone(),
                                        details: content,
                                    });
                                }

                                full_response.push_str(&content);
                                callback(ChatResponse::new_with_arc(
                                    chat_id.clone(),
                                    content,
                                    msg_type,
                                    metadata_option.as_ref().and_then(|m| m.to_value()),
                                    None,
                                ));
                            }
                        }

                        // Tool call handling
                        if let Some(tool_call_parts) = chunk.tool_calls {
                            for part in tool_call_parts {
                                // part is a ToolCallDeclaration
                                let acc_call = accumulated_tool_calls
                                    .entry(part.index)
                                    .or_insert_with(|| ToolCallDeclaration {
                                        index: part.index,
                                        id: part.id.clone(),
                                        name: part.name.clone(),
                                        arguments: Some(String::new()),
                                        results: None,
                                    });

                                // Append current part's arguments
                                if let Some(args_chunk) = part.arguments {
                                    if !args_chunk.is_empty() {
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
                                if self.should_stop().await {
                                    log::info!(
                                        "Skipping tool call processing for chat_id {} because stop_flag is set",
                                        chat_id
                                    );
                                    accumulated_tool_calls.clear();
                                    break;
                                }
                                finish_reason = FinishReason::ToolCalls;
                                self.process_and_send_tool_calls(
                                    &mut accumulated_tool_calls,
                                    &full_response,
                                    &chat_id,
                                    &metadata_option,
                                    &provider_name,
                                    &callback,
                                );
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

                    let error_payload = JsonErrorPayload {
                        status: 500, // Internal processing error
                        message: &err.to_string(),
                    };
                    let chunk =
                        serde_json::to_string(&error_payload).unwrap_or_else(|_| err.to_string());

                    callback(ChatResponse::new_with_arc(
                        chat_id.clone(),
                        chunk,
                        MessageType::Error,
                        metadata_option.as_ref().and_then(|m| m.to_value()),
                        None,
                    ));
                    return Err(err);
                }
            }
        }

        if !accumulated_tool_calls.is_empty() {
            if self.should_stop().await {
                log::info!(
                    "Skipping trailing tool call processing for chat_id {} because stop_flag is set",
                    chat_id
                );
                accumulated_tool_calls.clear();
            } else {
                log::debug!(
                    "Manually triggering tool call processing after stream end for chat_id: {}",
                    chat_id
                );
                finish_reason = FinishReason::ToolCalls;
                self.process_and_send_tool_calls(
                    &mut accumulated_tool_calls,
                    &full_response,
                    &chat_id,
                    &metadata_option,
                    &provider_name,
                    &callback,
                );
            }
        }

        #[cfg(debug_assertions)]
        {
            log::debug!(
                "total_tokens: {}, prompt_tokens: {}, completion_tokens: {}",
                token_usage.total_tokens,
                token_usage.prompt_tokens,
                token_usage.completion_tokens
            );
        }

        callback(ChatResponse::new_with_arc(
            chat_id.clone(),
            String::new(),
            MessageType::Finished,
            {
                let mut meta = metadata_option.unwrap_or_default();
                meta.update_usage(
                    token_usage.total_tokens,
                    token_usage.prompt_tokens,
                    token_usage.completion_tokens,
                    token_usage.tokens_per_second,
                );
                meta.to_value()
            },
            Some(finish_reason),
        ));

        Ok(json!({
            "reasoning": reasoning_content,
            "content": full_response
        })
        .to_string())
    }

    /// Handles non-streaming response
    async fn handle_non_stream_response(
        &self,
        chat_id: String,
        response_text: String,
        callback: impl Fn(Arc<ChatResponse>) + Send + 'static,
        metadata_option: Option<ChatMetadata>,
        provider_name: String,
    ) -> Result<String, AiError> {
        // Parse the JSON response
        let parsed: Value = serde_json::from_str(&response_text).map_err(|e| {
            let err = AiError::ApiRequestFailed {
                status_code: 500,
                provider: provider_name.clone(),
                details: format!("Failed to parse response: {}", e),
            };
            log::error!("{} response parsing error: {}", provider_name, err);
            err
        })?;

        // Extract content from the response
        let content = parsed["choices"]
            .as_array()
            .and_then(|choices| choices.first())
            .and_then(|choice| choice["message"]["content"].as_str())
            .unwrap_or("");

        let reasoning_content = parsed["choices"]
            .as_array()
            .and_then(|choices| choices.first())
            .map(|choice| extract_reasoning_from_openai_message(&choice["message"]))
            .unwrap_or_default();

        // Extract token usage if available
        let token_usage = if let Some(usage) = parsed.get("usage") {
            TokenUsage {
                total_tokens: usage["total_tokens"].as_u64().unwrap_or(0),
                prompt_tokens: usage["prompt_tokens"].as_u64().unwrap_or(0),
                completion_tokens: usage["completion_tokens"].as_u64().unwrap_or(0),
                tokens_per_second: 0.0,
            }
        } else {
            TokenUsage::default()
        };

        // Send the content to callback
        if !reasoning_content.is_empty() {
            callback(ChatResponse::new_with_arc(
                chat_id.clone(),
                reasoning_content.clone(),
                MessageType::Reasoning,
                metadata_option.as_ref().and_then(|m| m.to_value()),
                None,
            ));
        }

        if !content.is_empty() {
            callback(ChatResponse::new_with_arc(
                chat_id.clone(),
                content.to_string(),
                MessageType::Text,
                metadata_option.as_ref().and_then(|m| m.to_value()),
                None,
            ));
        }

        // Send finished message
        callback(ChatResponse::new_with_arc(
            chat_id.clone(),
            String::new(),
            MessageType::Finished,
            {
                let mut meta = metadata_option.unwrap_or_default();
                meta.update_usage(
                    token_usage.total_tokens,
                    token_usage.prompt_tokens,
                    token_usage.completion_tokens,
                    token_usage.tokens_per_second,
                );
                meta.to_value()
            },
            Some(FinishReason::Complete),
        ));

        Ok(json!({
            "reasoning": reasoning_content,
            "content": content
        })
        .to_string())
    }

    /// Processes and sends accumulated tool calls.
    fn process_and_send_tool_calls<F: Fn(Arc<ChatResponse>) + Send + 'static>(
        &self,
        accumulated_tool_calls: &mut HashMap<u32, ToolCallDeclaration>,
        full_response: &str,
        chat_id: &str,
        metadata_option: &Option<ChatMetadata>,
        provider_name: &str,
        callback: &F,
    ) {
        let mut ordered_tool_calls: Vec<(u32, ToolCallDeclaration)> = accumulated_tool_calls
            .iter()
            .map(|(idx, tcd)| (*idx, tcd.clone()))
            .collect();
        ordered_tool_calls.sort_by_key(|(idx, _)| *idx);

        let prepared_tool_calls = Self::prepare_tool_calls(ordered_tool_calls);
        if prepared_tool_calls.calls.is_empty() {
            log::warn!(
                "{} proposed {} raw tool calls, but none remained after dedupe/limit filtering",
                provider_name,
                prepared_tool_calls.raw_count
            );
            accumulated_tool_calls.clear();
            return;
        }

        if prepared_tool_calls.duplicate_count > 0 || prepared_tool_calls.limit_dropped_count > 0 {
            log::warn!(
                "{} proposed {} raw tool calls; dispatching {} after filtering (duplicates_dropped={}, limit_dropped={}, max_per_response={})",
                provider_name,
                prepared_tool_calls.raw_count,
                prepared_tool_calls.calls.len(),
                prepared_tool_calls.duplicate_count,
                prepared_tool_calls.limit_dropped_count,
                MAX_TOOL_CALLS_PER_RESPONSE
            );
        } else {
            log::info!(
                "{} proposed {} tool calls; dispatching all {}",
                provider_name,
                prepared_tool_calls.raw_count,
                prepared_tool_calls.calls.len()
            );
        }

        let assistant_tool_requests: Vec<Value> = prepared_tool_calls
            .calls
            .iter()
            .map(|(idx, tcd)| {
                let arguments_str = tcd
                    .arguments
                    .as_deref()
                    .map(Self::normalize_tool_arguments)
                    .unwrap_or_default();
                json!({
                    "index": idx,
                    "id": tcd.id,
                    "type": "function",
                    "function": { "name": tcd.name, "arguments": arguments_str }
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
                    chat_id.to_string(),
                    serialized_assistant_action,
                    MessageType::ToolCalls,
                    metadata_option.as_ref().and_then(|m| m.to_value()),
                    None,
                ));
            }
            Err(e) => {
                let err = AiError::ToolCallSerializationFailed {
                    details: e.to_string(),
                };
                log::error!("Failed to serialize AssistantAction message: {}", err);

                let error_payload = JsonErrorPayload {
                    status: 500,
                    message: &err.to_string(),
                };
                let chunk =
                    serde_json::to_string(&error_payload).unwrap_or_else(|_| err.to_string());

                callback(ChatResponse::new_with_arc(
                    chat_id.to_string(),
                    chunk,
                    MessageType::Error,
                    metadata_option.as_ref().and_then(|m| m.to_value()),
                    Some(FinishReason::Error),
                ));
                return;
            }
        }

        for (_, tcd) in &prepared_tool_calls.calls {
            let mut trimmed_tcd = tcd.clone();
            if let Some(args) = trimmed_tcd.arguments.as_mut() {
                *args = Self::normalize_tool_arguments(args);
            }
            match serde_json::to_string(&trimmed_tcd) {
                Ok(serialized_tcd) => {
                    callback(ChatResponse::new_with_arc(
                        chat_id.to_string(),
                        serialized_tcd,
                        MessageType::ToolResults,
                        metadata_option.as_ref().and_then(|m| m.to_value()),
                        None,
                    ));
                    log::debug!("dispatching proposed tool call: {}", trimmed_tcd.name);
                    #[cfg(debug_assertions)]
                    {
                        log::debug!(
                            "tool_call: {}",
                            serde_json::to_string_pretty(&trimmed_tcd).unwrap_or_default()
                        );
                    }
                }
                Err(e) => {
                    let err = AiError::ToolCallSerializationFailed {
                        details: e.to_string(),
                    };
                    log::error!(
                        "{} tool call serialization error for tool {:?}: {}",
                        provider_name,
                        trimmed_tcd.name,
                        err
                    );
                    let error_payload = JsonErrorPayload {
                        status: 500,
                        message: &err.to_string(),
                    };
                    let chunk =
                        serde_json::to_string(&error_payload).unwrap_or_else(|_| err.to_string());
                    callback(ChatResponse::new_with_arc(
                        chat_id.to_string(),
                        chunk,
                        MessageType::Error,
                        metadata_option.as_ref().and_then(|m| m.to_value()),
                        Some(FinishReason::Error),
                    ));
                }
            }
        }
        accumulated_tool_calls.clear();
    }

    fn prepare_tool_calls(
        ordered_tool_calls: Vec<(u32, ToolCallDeclaration)>,
    ) -> PreparedToolCalls {
        let raw_count = ordered_tool_calls.len();
        let mut seen = HashSet::new();
        let mut deduped_calls = Vec::new();
        let mut duplicate_count = 0;

        for (idx, mut tcd) in ordered_tool_calls {
            let normalized_args = tcd
                .arguments
                .as_deref()
                .map(Self::normalize_tool_arguments)
                .unwrap_or_default();
            let dedupe_key = format!("{}:{}", tcd.name, normalized_args);
            if !seen.insert(dedupe_key) {
                duplicate_count += 1;
                continue;
            }

            tcd.arguments = Some(normalized_args);
            deduped_calls.push((idx, tcd));
        }

        let limit_dropped_count = deduped_calls
            .len()
            .saturating_sub(MAX_TOOL_CALLS_PER_RESPONSE);
        deduped_calls.truncate(MAX_TOOL_CALLS_PER_RESPONSE);

        PreparedToolCalls {
            calls: deduped_calls,
            raw_count,
            duplicate_count,
            limit_dropped_count,
        }
    }

    fn normalize_tool_arguments(raw: &str) -> String {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return String::new();
        }

        let cleaned = crate::libs::util::format_json_str(trimmed);
        if let Ok(parsed) = serde_json::from_str::<Value>(&cleaned) {
            let canonical = Self::canonicalize_json_value(parsed);
            return serde_json::to_string(&canonical).unwrap_or(cleaned);
        }

        let start = cleaned
            .char_indices()
            .find(|(_, ch)| *ch == '{' || *ch == '[')
            .map(|(idx, _)| idx);

        let Some(start_idx) = start else {
            return trimmed.to_string();
        };

        let candidate = &cleaned[start_idx..];
        for (idx, ch) in candidate.char_indices().rev() {
            if ch != '}' && ch != ']' {
                continue;
            }

            let slice = &candidate[..=idx];
            if let Ok(parsed) = serde_json::from_str::<Value>(slice) {
                return serde_json::to_string(&parsed).unwrap_or_else(|_| slice.to_string());
            }
        }

        trimmed.to_string()
    }

    fn canonicalize_json_value(value: Value) -> Value {
        match value {
            Value::Object(map) => {
                let sorted = map
                    .into_iter()
                    .map(|(key, value)| (key, Self::canonicalize_json_value(value)))
                    .collect::<BTreeMap<_, _>>();
                let mut canonical = serde_json::Map::new();
                for (key, value) in sorted {
                    canonical.insert(key, value);
                }
                Value::Object(canonical)
            }
            Value::Array(items) => Value::Array(
                items
                    .into_iter()
                    .map(Self::canonicalize_json_value)
                    .collect(),
            ),
            other => other,
        }
    }

    /// Determines the appropriate API endpoint based on tools and function call settings
    fn get_endpoint(
        &self,
        messages: &[Value],
        tools: &Option<Vec<MCPToolDeclaration>>,
        model_detail: &[ModelConfig],
        model: &str,
    ) -> &'static str {
        let has_tool_calls = messages.iter().any(|msg| {
            msg.get("role")
                .and_then(|r| r.as_str())
                .map(|role| {
                    role == "tool"
                        || (role == "assistant"
                            && msg
                                .get("tool_calls")
                                .and_then(|tc| tc.as_array())
                                .map(|arr| !arr.is_empty())
                                .unwrap_or(false))
                })
                .unwrap_or(false)
        });

        let has_tools = tools.as_ref().map(|t| !t.is_empty()).unwrap_or(false);
        let function_call = model_detail
            .iter()
            .find(|m| m.id == model)
            .and_then(|m| m.function_call)
            .unwrap_or(false);

        if has_tool_calls || has_tools {
            if function_call {
                "/v1/chat/completions"
            } else {
                "/compat_mode/v1/chat/completions"
            }
        } else {
            "/v1/chat/completions"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::extract_reasoning_from_openai_message;
    use serde_json::json;

    #[test]
    fn extract_reasoning_prefers_reasoning_content() {
        let message = json!({
            "reasoning_content": "direct reasoning",
            "thinking": "fallback thinking",
            "reasoning_details": [
                { "type": "reasoning.text", "text": "detail" }
            ]
        });

        assert_eq!(
            extract_reasoning_from_openai_message(&message),
            "direct reasoning"
        );
    }

    #[test]
    fn extract_reasoning_falls_back_to_thinking() {
        let message = json!({
            "thinking": "thinking text"
        });

        assert_eq!(
            extract_reasoning_from_openai_message(&message),
            "thinking text"
        );
    }

    #[test]
    fn extract_reasoning_combines_reasoning_details() {
        let message = json!({
            "reasoning_details": [
                { "type": "reasoning.text", "text": " first " },
                { "type": "reasoning.summary", "text": "ignored" },
                { "type": "reasoning.text", "text": "second" }
            ]
        });

        assert_eq!(
            extract_reasoning_from_openai_message(&message),
            "first\nsecond"
        );
    }
}

impl_stoppable!(OpenAIChat);

#[async_trait]
impl AiChatTrait for OpenAIChat {
    /// Implements chat functionality for OpenAI API
    async fn chat(
        &self,
        provider_id: i64,
        model: &str,
        chat_id: String,
        messages: Vec<Value>,
        tools: Option<Vec<MCPToolDeclaration>>,
        metadata: Option<ChatMetadata>,
        callback: impl Fn(Arc<ChatResponse>) + Send + 'static,
    ) -> Result<String, AiError> {
        // Always reset stop flag at the beginning of each chat request.
        // Stop should cancel only the currently in-flight request, not future turns.
        self.set_stop_flag(false).await;

        let mut final_model = model.to_string();
        let mut proxy_group: Option<String> = None;

        let model_detail = if provider_id == 0 {
            // Proxy Mode: Parse group and alias from model string (format: "group@alias")
            if let Some((group, alias)) = model.split_once('@') {
                proxy_group = Some(group.to_string());
                final_model = alias.to_string();
            }
            // Return a default model config for proxy mode
            AiModel {
                name: "Internal Proxy".to_string(),
                api_protocol: "openai".to_string(),
                ..Default::default()
            }
        } else {
            self.main_store
                .read()
                .map_err(|e| {
                    let err = AiError::InitFailed(
                        t!("db.failed_to_lock_main_store", error = e.to_string()).to_string(),
                    );
                    let error_payload = JsonErrorPayload {
                        status: 500,
                        message: &err.to_string(),
                    };
                    let chunk =
                        serde_json::to_string(&error_payload).unwrap_or_else(|_| err.to_string());
                    callback(ChatResponse::new_with_arc(
                        chat_id.clone(),
                        chunk,
                        MessageType::Error,
                        metadata.as_ref().and_then(|m| m.to_value()),
                        Some(FinishReason::Error),
                    ));
                    err
                })?
                .config
                .get_ai_model_by_id(provider_id)
                .map_err(|e| {
                    let err = AiError::InitFailed(e.to_string());
                    let error_payload = JsonErrorPayload {
                        status: 500,
                        message: &err.to_string(),
                    };
                    let chunk =
                        serde_json::to_string(&error_payload).unwrap_or_else(|_| err.to_string());
                    callback(ChatResponse::new_with_arc(
                        chat_id.clone(),
                        chunk,
                        MessageType::Error,
                        metadata.as_ref().and_then(|m| m.to_value()),
                        Some(FinishReason::Error),
                    ));
                    err
                })?
        };

        let mut merged_metadata = metadata.unwrap_or_default();
        // Priority: User Metadata > Model Config
        merged_metadata.merge_with_model_config(&model_detail);

        let params = init_request_params(&Some(merged_metadata.clone()));

        let url = crate::constants::get_static_var(&crate::constants::CHAT_COMPLETION_PROXY);

        // Priority for stream: metadata > model_metadata > default (true)
        let stream_enabled = merged_metadata.stream.unwrap_or_else(|| {
            init_request_params_value(model_detail.metadata.clone())
                .get("stream")
                .and_then(|v| v.as_bool())
                .unwrap_or(true)
        });

        let mut payload = json!({
            "model": final_model,
            "messages": messages,
            "stream": stream_enabled,
        });

        if stream_enabled {
            if let Some(obj) = payload.as_object_mut() {
                obj.insert(
                    "stream_options".to_string(),
                    json!({ "include_usage": true }),
                );
            }
        }

        // Add custom body parameters from model metadata
        merge_custom_params_value(&mut payload, &model_detail.metadata);
        // Add custom body parameters from user metadata (ChatMetadata)
        merge_custom_params(&mut payload, &Some(merged_metadata.clone()));

        // Inject standard parameters with range validation
        if let Some(obj) = payload.as_object_mut() {
            // max_tokens: must be positive
            if let Some(v) = merged_metadata.max_tokens {
                if v > 0 {
                    obj.insert("max_tokens".to_string(), json!(v));
                }
            }

            if merged_metadata.thinking.is_some() {
                let compat_thinking = build_openai_compat_thinking_fields(
                    &final_model,
                    merged_metadata.thinking.as_ref(),
                );

                if !obj.contains_key("thinking") {
                    if let Some(thinking) = compat_thinking.thinking {
                        obj.insert("thinking".to_string(), json!(thinking));
                    }
                }
                if !obj.contains_key("reasoning_effort") {
                    if let Some(reasoning_effort) = compat_thinking.reasoning_effort {
                        obj.insert("reasoning_effort".to_string(), json!(reasoning_effort));
                    }
                }
                if !obj.contains_key("thinking_budget") {
                    if let Some(thinking_budget) = compat_thinking.thinking_budget {
                        obj.insert("thinking_budget".to_string(), json!(thinking_budget));
                    }
                }
            }

            // temperature: OpenAI range is typically 0.0 to 2.0
            if let Some(v) = merged_metadata.temperature {
                if v >= 0.0 && v <= 2.0 {
                    obj.insert("temperature".to_string(), json!(v));
                }
            }

            // top_p: range 0.0 to 1.0, zero means unset
            if let Some(v) = merged_metadata.top_p {
                if v > 0.0 && v <= 1.0 {
                    obj.insert("top_p".to_string(), json!(v));
                }
            }

            // Other OpenAI params from initialized request params
            if let Some(v) = params.get("frequency_penalty") {
                if v.as_f64() != Some(0.0) {
                    obj.insert("frequency_penalty".to_string(), v.clone());
                }
            }
            if let Some(v) = params.get("presence_penalty") {
                if v.as_f64() != Some(0.0) {
                    obj.insert("presence_penalty".to_string(), v.clone());
                }
            }
            if let Some(v) = params.get("response_format") {
                obj.insert("response_format".to_string(), v.clone());
            }
            if let Some(v) = params.get("stop") {
                if !v.is_null() {
                    obj.insert("stop".to_string(), v.clone());
                }
            }
            if let Some(v) = params.get("n") {
                if !v.is_null() {
                    obj.insert("n".to_string(), v.clone());
                }
            }
            if let Some(v) = params.get("user") {
                if !v.is_null() {
                    obj.insert("user".to_string(), v.clone());
                }
            }

            if let Some(tools_list) = tools.as_ref() {
                let openai_tools: Vec<Value> =
                    tools_list.iter().map(|tool| tool.to_openai()).collect();
                if !openai_tools.is_empty() {
                    obj.insert("tools".to_string(), json!(openai_tools));
                    if let Some(choice) = params.get("tool_choice") {
                        obj.insert("tool_choice".to_string(), choice.clone());
                    }
                }
            }
        }

        let internal_api_key = INTERNAL_CCPROXY_API_KEY.read().clone();
        let mut headers_json = json!({
            "x-cs-model-id": final_model,
            "x-cs-internal-request": "true",
        });

        if provider_id != 0 {
            if let Some(obj) = headers_json.as_object_mut() {
                obj.insert(
                    "x-cs-provider-id".to_string(),
                    json!(provider_id.to_string()),
                );
            }
        }

        let custom_headers = process_custom_headers(&Some(merged_metadata.clone()), &chat_id);
        if let Some(headers_obj) = headers_json.as_object_mut() {
            for (k, v) in custom_headers {
                headers_obj.insert(k, json!(v));
            }
        }

        let config = ApiConfig::new(
            Some(url),
            Some(internal_api_key),
            crate::ai::network::ProxyType::None, // No proxy, as we are calling localhost
            Some(headers_json),
        );

        let base_endpoint =
            self.get_endpoint(&messages, &tools, &model_detail.models, &final_model);
        let endpoint = if let Some(group) = proxy_group {
            format!("/{}{}", group, base_endpoint)
        } else {
            base_endpoint.to_string()
        };

        let response = self
            .client
            .post_request(&config, &endpoint, payload, stream_enabled)
            .await
            .map_err(|e| {
                let err = AiError::ApiRequestFailed {
                    status_code: 0,
                    provider: model_detail.api_protocol.clone(),
                    details: e.to_string(),
                };

                let error_payload = JsonErrorPayload {
                    status: 503,
                    message: &err.to_string(),
                };
                let chunk =
                    serde_json::to_string(&error_payload).unwrap_or_else(|_| err.to_string());

                callback(ChatResponse::new_with_arc(
                    chat_id.clone(),
                    chunk,
                    MessageType::Error,
                    merged_metadata.to_value(),
                    Some(FinishReason::Error),
                ));
                err
            })?;

        if response.is_error {
            let status_code = response.status_code;

            let details = if let Ok(error_payload) =
                serde_json::from_str::<crate::ai::network::ResponseError>(&response.content)
            {
                error_payload.message
            } else {
                response.content.clone()
            };

            let err = AiError::ApiRequestFailed {
                status_code,
                provider: model_detail.name.clone(),
                details,
            };

            let error_payload = JsonErrorPayload {
                status: status_code,
                message: &err.to_string(),
            };
            let chunk = serde_json::to_string(&error_payload).unwrap_or_else(|_| err.to_string());

            callback(ChatResponse::new_with_arc(
                chat_id.clone(),
                chunk,
                MessageType::Error,
                merged_metadata.to_value(),
                Some(FinishReason::Error),
            ));
            return Err(err);
        }

        if let Some(raw_response) = response.raw_response {
            self.handle_stream_response(
                chat_id.clone(),
                raw_response,
                callback,
                Some(merged_metadata),
                model_detail.name.clone(),
                stream_enabled,
            )
            .await
        } else {
            self.handle_non_stream_response(
                chat_id.clone(),
                response.content,
                callback,
                Some(merged_metadata),
                model_detail.name.clone(),
            )
            .await
        }
    }

    async fn list_models(
        &self,
        api_protocol: String,
        api_url: Option<&str>,
        api_key: Option<&str>,
        extra_args: Option<ChatMetadata>,
    ) -> Result<Vec<ModelDetails>, AiError> {
        let metadata_value = extra_args.and_then(|m| m.to_value());
        let model_details = match ChatProtocol::from_str(&api_protocol) {
            Ok(ChatProtocol::Claude) => {
                super::list_models::claude_list_models(api_url, api_key, metadata_value).await
            }
            Ok(ChatProtocol::Gemini) => {
                super::list_models::gemini_list_models(api_url, api_key, metadata_value).await
            }
            _ => super::list_models::openai_list_models(api_url, api_key, metadata_value).await,
        }?;

        Ok(model_details)
    }
}
