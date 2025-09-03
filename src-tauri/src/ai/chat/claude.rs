use async_trait::async_trait;
use reqwest::Response;
use serde_json::{from_str, json, Value};
use std::{collections::HashMap, sync::Arc, time::Instant};
use tokio::sync::Mutex;

use crate::ai::interaction::constants::{
    TOKENS, TOKENS_COMPLETION, TOKENS_PER_SECOND, TOKENS_PROMPT, TOKENS_TOTAL,
};
use crate::ai::network::{ApiClient, ApiConfig, DefaultApiClient, ErrorFormat, TokenUsage};
use crate::ai::traits::chat::{
    ChatResponse, FinishReason, MCPToolDeclaration, MessageType, ModelDetails, ToolCallDeclaration,
};
use crate::ai::traits::{chat::AiChatTrait, stoppable::Stoppable};
use crate::ai::util::{
    get_family_from_model_id, get_proxy_type, init_extra_params, is_function_call_supported,
    is_image_input_supported, is_reasoning_supported, update_or_create_metadata,
};
use crate::ccproxy::{ChatProtocol, StreamFormat, StreamProcessor};
use crate::{ai::error::AiError, impl_stoppable};

/// Represents the Claude chat implementation
#[derive(Clone)]
pub struct ClaudeChat {
    stop_flag: Arc<Mutex<bool>>,
    client: DefaultApiClient,
}

#[derive(serde::Deserialize, Debug)]
struct ClaudeApiModel {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_token_limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_token_limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    supports_tools: Option<bool>,
}

#[derive(serde::Deserialize, Debug)]
struct ClaudeListModelsResponse {
    data: Vec<ClaudeApiModel>,
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
        let mut system_prompt_str = None;

        if let Some(first_message) = messages.first() {
            if first_message.get("role").and_then(Value::as_str) == Some("system") {
                system_prompt_str = first_message
                    .get("content")
                    .and_then(Value::as_str)
                    .map(String::from);
                // Remove the system prompt from processed_messages as it's handled separately by Claude
                if system_prompt_str.is_some() {
                    processed_messages = messages.iter().skip(1).cloned().collect();
                }
            }
        }

        // Convert messages to Claude format, especially handling tool calls and results
        let claude_formatted_messages: Vec<Value> = processed_messages
            .into_iter()
            .map(|message| {
                let role = message
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let content_value = message.get("content");
                let tool_calls_value = message.get("tool_calls"); // OpenAI specific field

                if role == "assistant" && tool_calls_value.is_some() {
                    // This is an assistant message that previously requested tool calls (OpenAI format)
                    // Convert it to Claude's tool_use format within the assistant's content
                    let mut claude_content_parts = Vec::new();
                    if let Some(text_content) = content_value.and_then(Value::as_str) {
                        if !text_content.is_empty() {
                            claude_content_parts
                                .push(json!({"type": "text", "text": text_content}));
                        }
                    }
                    if let Some(tool_calls_array) = tool_calls_value.and_then(Value::as_array) {
                        for tool_call_obj in tool_calls_array {
                            if let (Some(id), Some(name), Some(args_str_val)) = (
                                tool_call_obj.get("id").and_then(Value::as_str),
                                tool_call_obj
                                    .get("function")
                                    .and_then(|f| f.get("name"))
                                    .and_then(Value::as_str),
                                tool_call_obj
                                    .get("function")
                                    .and_then(|f| f.get("arguments")), // arguments can be string or object
                            ) {
                                // Claude's "input" for tool_use is a JSON object.
                                let input_obj: Value = if args_str_val.is_string() {
                                    serde_json::from_str(args_str_val.as_str().unwrap_or("{}"))
                                        .unwrap_or(json!({}))
                                } else {
                                    args_str_val.clone() // Assume it's already a JSON object
                                };
                                claude_content_parts.push(json!({
                                    "type": "tool_use",
                                    "id": id,
                                    "name": name,
                                    "input": input_obj
                                }));
                            }
                        }
                    }
                    return json!({"role": "assistant", "content": claude_content_parts });
                } else if role == "tool" {
                    // This is a tool execution result (OpenAI format)
                    // Convert it to Claude's tool_result format within a "user" role message
                    if let (Some(tool_use_id), Some(_name), Some(content_result)) = (
                        message.get("tool_call_id").and_then(Value::as_str),
                        message.get("name").and_then(Value::as_str), // Name is part of the tool_result block for Claude
                        message.get("content"),
                    ) {
                        // Claude's tool_result content can be a string or a JSON object.
                        // If content_result is a string, it's fine. If it's an object, wrap it.
                        let claude_tool_content = if content_result.is_string() {
                            content_result.clone()
                        } else {
                            // If it's already a JSON object, or needs to be structured as {"type": "json", "json": ...}
                            // For simplicity, let's assume if it's not a string, it's a JSON value Claude can accept.
                            // A more robust way might be to check if it's an object and wrap if needed.
                            // For now, just passing it as is. If it's a simple string, Claude accepts it.
                            // If it's a JSON object, Claude also accepts it.
                            // If it's an error, "is_error": true should be added.
                            content_result.clone()
                        };
                        return json!({
                            "role": "user", // Tool results are sent as "user" role to Claude
                            "content": [{
                                "type": "tool_result",
                                "tool_use_id": tool_use_id,
                                "content": claude_tool_content
                                // "is_error": false, // Add if applicable
                            }]
                        });
                    }
                }
                // For user/assistant messages (without tool calls), or other roles, format content.
                // This needs to handle OpenAI's `image_url` and convert to Claude's `image` format.
                let mut claude_parts: Vec<Value> = Vec::new();

                if let Some(text_content) = content_value.and_then(Value::as_str) {
                    claude_parts.push(json!({"type": "text", "text": text_content}));
                } else if let Some(array_content) = content_value.and_then(Value::as_array) {
                    for part_val in array_content {
                        if let Some(part_obj) = part_val.as_object() {
                            if let Some(part_type) = part_obj.get("type").and_then(Value::as_str) {
                                match part_type {
                                    "text" => {
                                        // Ensure text content is not null, use empty string if it is.
                                        let text = part_obj.get("text").and_then(Value::as_str).unwrap_or("");
                                        if !text.is_empty(){
                                            claude_parts.push(json!({"type": "text", "text": text}));
                                        }
                                    }
                                    "image_url" => { // OpenAI specific, needs conversion
                                        if let Some(image_url_details) = part_obj.get("image_url").and_then(Value::as_object) {
                                            if let Some(url_str) = image_url_details.get("url").and_then(Value::as_str) {
                                                if url_str.starts_with("data:") {
                                                    // Expected format: "data:<media_type>;base64,<data>"
                                                    let mut url_parts_iter = url_str.splitn(2, ',');
                                                    let header_and_data_option = url_parts_iter.next().zip(url_parts_iter.next());

                                                    if let Some((header_part, data_part)) = header_and_data_option {
                                                        // header_part = "data:image/jpeg;base64"
                                                        if let Some(media_type_and_encoding) = header_part.strip_prefix("data:") {
                                                            // media_type_and_encoding = "image/jpeg;base64"
                                                            let mut media_encoding_iter = media_type_and_encoding.splitn(2, ';');
                                                            let media_and_enc_option = media_encoding_iter.next().zip(media_encoding_iter.next());

                                                            if let Some((media_type, encoding)) = media_and_enc_option {
                                                                if encoding.to_lowercase() == "base64" {
                                                                    claude_parts.push(json!({
                                                                        "type": "image",
                                                                        "source": {
                                                                            "type": "base64",
                                                                            "media_type": media_type,
                                                                            "data": data_part
                                                                        }
                                                                    }));
                                                                    continue; // Successfully converted and added
                                                                } else {
                                                                    log::warn!("Unsupported encoding in data URL: '{}'. Expected 'base64'. Skipping image part. URL: {}", encoding, url_str);
                                                                }
                                                            } else {
                                                                 log::warn!("Malformed data URL header: could not split media_type and encoding from '{}'. Skipping image part. URL: {}", media_type_and_encoding, url_str);
                                                            }
                                                        } else {
                                                            log::warn!("Malformed data URL: 'data:' prefix found, but strip_prefix failed for '{}'. Skipping image part. URL: {}", header_part, url_str);
                                                        }
                                                    } else {
                                                        log::warn!("Malformed data URL: could not split into header and data parts. Skipping image part. URL: {}", url_str);
                                                    }
                                                } else {
                                                    log::warn!("Image URL is not a data URL. Claude requires base64 encoded images. URL: {}. Skipping image part.", url_str);
                                                }
                                            } else {
                                                log::warn!("'image_url' object missing 'url' string. Skipping image part: {:?}", part_obj);
                                            }
                                        } else {
                                             log::warn!("'image_url' type part missing 'image_url' object details. Skipping part: {:?}", part_obj);
                                        }
                                    }
                                    "image" => { // Already in Claude's image format (or close enough)
                                        claude_parts.push(part_val.clone());
                                    }
                                    _ => { // Unknown part type
                                        log::warn!("Unknown content part type '{}' found. Passing through: {:?}", part_type, part_val);
                                        claude_parts.push(part_val.clone());
                                    }
                                }
                            } else { // Part in array is not an object or has no "type" field
                                log::warn!("Content part in array is malformed (not an object or missing 'type'): {:?}. Passing through.", part_val);
                                claude_parts.push(part_val.clone());
                            }
                        } else if let Some(s) = part_val.as_str() { // Part is a simple string in the array
                             claude_parts.push(json!({"type": "text", "text": s}));
                        } else { // Part in array is some other JSON type (number, boolean, null)
                            log::warn!("Unsupported content part in array: {:?}. Skipping.", part_val);
                        }
                    }
                } else {
                    // Content is not a string and not an array (e.g. null, or an object not representing parts).
                    // Default to an empty text part, as Claude requires content for user/assistant messages.
                    claude_parts.push(json!({"type": "text", "text": ""}));
                }

                // Claude API: "User and assistant messages must have a non-empty content array."
                if claude_parts.is_empty() && (role == "user" || role == "assistant") {
                    log::warn!("Content for role '{}' is empty after processing. Claude requires non-empty content. Defaulting to a single empty text part.", role);
                    claude_parts.push(json!({"type": "text", "text": ""}));
                }

                json!({"role": role, "content": claude_parts })
            })
            .collect();

        let mut payload = json!({
            "model": model,
            "messages": claude_formatted_messages,
            "stream": params.get("stream").unwrap_or(&json!(true)),
        });

        if let Some(obj) = payload.as_object_mut() {
            if let Some(prompt) = system_prompt_str {
                if !prompt.is_empty() {
                    obj.insert("system".to_string(), json!(prompt));
                }
            }
            if let Some(max_tokens_val) = params.get("max_tokens").and_then(|v| v.as_u64()) {
                if max_tokens_val > 0 {
                    // Ensure max_tokens is positive
                    obj.insert("max_tokens".to_string(), json!(max_tokens_val));
                }
            }

            if let Some(temperature_val) = params.get("temperature").and_then(|v| v.as_f64()) {
                if temperature_val >= 0.0 && temperature_val <= 2.0 {
                    // Claude's typical range for temperature
                    obj.insert("temperature".to_string(), json!(temperature_val));
                }
            }

            if let Some(top_k) = params.get("top_k").and_then(|v| v.as_i64()) {
                if top_k > 0 {
                    obj.insert("top_k".to_string(), json!(top_k));
                }
            }

            if let Some(top_p) = params.get("top_p").and_then(|v| v.as_f64()) {
                if top_p > 0.0 && top_p <= 1.0 {
                    obj.insert("top_p".to_string(), json!(top_p));
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
        let mut reasoning_content = String::new();
        let mut full_response = String::new();
        let mut token_usage = TokenUsage::default();
        let start_time = Instant::now();

        // Accumulates tool call parts. Key is tool_call.index
        let mut accumulated_tool_calls: HashMap<u32, ToolCallDeclaration> = HashMap::new();
        let mut finish_reason = FinishReason::Complete;

        let mut tool_calls_messages_sent = false; // Flag to track if tool call messages were successfully sent
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
                                Some(FinishReason::Error),
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
                                reasoning_content.push_str(&content);

                                callback(ChatResponse::new_with_arc(
                                    chat_id.clone(),
                                    content,
                                    MessageType::Reasoning,
                                    metadata_option.clone(),
                                    None,
                                ));
                            }
                        }

                        // Process content only if it's not part of a tool call argument accumulation
                        // (tool call arguments are now exclusively in chunk.tool_calls)
                        if chunk.tool_calls.is_none()
                            || chunk
                                .tool_calls
                                .as_ref()
                                .map_or(true, |tc| tc.iter().all(|p| p.arguments.is_none()))
                        {
                            if let Some(content) = chunk.content {
                                if !content.is_empty() {
                                    full_response.push_str(&content);
                                    let msg_type = chunk.msg_type.unwrap_or(MessageType::Text);
                                    callback(ChatResponse::new_with_arc(
                                        chat_id.clone(),
                                        content,
                                        msg_type,
                                        metadata_option.clone(),
                                        None,
                                    ));
                                }
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
                                        arguments: if part.arguments.is_some() {
                                            Some(String::new())
                                        } else {
                                            None
                                        }, // Initialize only if args are expected
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
                                if let Some(args_chunk) = &part.arguments {
                                    // part.arguments now comes from stream.rs for deltas
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
                        // Also, ensure that we are not already in a state where tool calls were signaled.
                        // The `finish_reason` from the chunk can be "tool_use" or "tool_use_via_stop_sequence".
                        match chunk.finish_reason.as_deref() {
                            Some("tool_use") | Some("tool_use_via_stop_sequence") => {
                                if !send_tool_calls_signal {
                                    send_tool_calls_signal = true;
                                    // Set the overall finish_reason for the session if this is the first tool call signal
                                    finish_reason = FinishReason::ToolCalls;
                                }
                            }
                            _ => {} // Other finish reasons are handled by the final Finished message
                        };

                        // If a "tool_use" signal was received, send accumulated tool calls
                        // This block should execute once when the tool_use signal is received
                        if send_tool_calls_signal && !accumulated_tool_calls.is_empty() {
                            finish_reason = FinishReason::ToolCalls;

                            // First, send the AssistantAction message with all requested tool calls
                            let assistant_tool_requests: Vec<Value> = accumulated_tool_calls
                                .iter()
                                .map(|(idx, tcd)| {
                                    // Convert ToolCallDeclaration to the format expected in the assistant's message
                                    // OpenAI's assistant message tool_calls usually look like:
                                    // { "id": "...", "type": "function", "function": { "name": "...", "arguments": "..." } }
                                    // Our ToolCallDeclaration is already very close to this.
                                    let arguments_str =
                                        tcd.arguments.as_deref().unwrap_or_default();
                                    json!({
                                        "index": idx,
                                        "id": tcd.id, // Ensure this ID is meaningful and unique for matching later
                                        "type": "function", // Assuming all tools are functions for now
                                        "function": {
                                            "name": tcd.name,
                                            "arguments": arguments_str
                                        }
                                    })
                                })
                                .collect();

                            let assistant_action_message = json!({
                                "role": "assistant",
                                "content": if full_response.is_empty() { Value::Null } else { Value::String(full_response.clone()) }, // Use accumulated full_response
                                "tool_calls": assistant_tool_requests
                            });

                            match serde_json::to_string(&assistant_action_message) {
                                Ok(serialized_assistant_action) => {
                                    callback(ChatResponse::new_with_arc(
                                        chat_id.clone(),
                                        serialized_assistant_action,
                                        MessageType::AssistantAction,
                                        metadata_option.clone(),
                                        Some(FinishReason::ToolCalls), // Indicate this message is for tool calls
                                    ));
                                }
                                Err(e) => {
                                    log::error!(
                                        "Failed to serialize AssistantAction message: {}",
                                        e
                                    );
                                    // Optionally send an error message via callback here
                                }
                            }

                            for tcd in accumulated_tool_calls.values() {
                                match serde_json::to_string(tcd) {
                                    Ok(serialized_tcd) => {
                                        callback(ChatResponse::new_with_arc(
                                            chat_id.clone(),
                                            serialized_tcd,
                                            MessageType::ToolCall,
                                            metadata_option.clone(),
                                            None,
                                        ));
                                        #[cfg(debug_assertions)]
                                        {
                                            log::debug!(
                                                "Claude Tool call: {}",
                                                serde_json::to_string_pretty(tcd)
                                                    .unwrap_or_default()
                                            );
                                        }
                                    }
                                    Err(e) => {
                                        let err = AiError::ToolCallSerializationFailed {
                                            details: e.to_string(),
                                        };
                                        log::error!("Claude tool call serialization error for tool {:?}: {}", tcd.name, err);
                                        callback(ChatResponse::new_with_arc(
                                            chat_id.clone(),
                                            err.to_string(),
                                            MessageType::Error,
                                            metadata_option.clone(),
                                            None,
                                        ));
                                    }
                                }
                            }

                            accumulated_tool_calls.clear(); // Clear after sending
                                                            // After sending AssistantAction and ToolCalls, the AI's turn is effectively paused.
                                                            // The `finish_reason` should remain `FinishReason::ToolCalls` so that the
                                                            // final `MessageType::Finished` message carries this correct reason.
                        }
                    }
                    tool_calls_messages_sent = true; // Mark that tool call messages were sent
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
                        None,
                    ));
                }
            }
        }

        // After the stream processing loop finishes:
        // Check the final state based on the determined finish_reason and whether tool call messages were sent.
        if finish_reason != FinishReason::ToolCalls {
            // This is a normal finish (Complete, Error, etc., but not ToolCalls)
            // Send the final Finished message.
            callback(ChatResponse::new_with_arc(
                chat_id.clone(),
                String::new(), // No new content for this final "Finished" marker
                MessageType::Finished,
                Some(update_or_create_metadata(
                    metadata_option.clone(), // Clone metadata for this specific message
                    TOKENS,
                    json!({
                        TOKENS_TOTAL: token_usage.total_tokens,
                        TOKENS_PROMPT: token_usage.prompt_tokens,
                        TOKENS_COMPLETION: token_usage.completion_tokens,
                        TOKENS_PER_SECOND: token_usage.tokens_per_second
                    }), // Ensure it's an Option<Map<String, Value>> if needed by update_or_create_metadata
                )),
                Some(finish_reason.clone()), // Pass the determined finish reason
            ));
        } else if !tool_calls_messages_sent {
            // The finish_reason was ToolCalls, but the tool call messages (AssistantAction, ToolCall)
            // were never successfully sent during the stream processing.
            // This indicates an issue where the AI signaled tool use but the data was incomplete or missing.
            log::warn!("Chat {}: AI signaled tool use but no tool call data was provided before stream ended. Marking as error.", chat_id);
            callback(ChatResponse::new_with_arc(
                chat_id.clone(),
                "AI signaled tool use but did not provide tool call details.".to_string(),
                MessageType::Error, // Send as an error to the global processor
                metadata_option.clone(), // Pass original metadata
                Some(FinishReason::Error), // Explicitly mark as error
            ));
        }

        // The Ok result should represent the complete textual response from the assistant.
        Ok(format!(
            "<think>{}</think>{}",
            reasoning_content, full_response
        ))
    }
}

impl_stoppable!(ClaudeChat);

const CALUDE_BASE_URL: &str = "https://api.anthropic.com/v1";

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
                    Some(api_url.unwrap_or(CALUDE_BASE_URL).to_string()),
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
                    Some(FinishReason::Error),
                ));
                err
            })?;

        if response.is_error {
            let err = AiError::ApiRequestFailed {
                provider: "Claude".to_string(),
                details: response.content.clone(),
            };
            // log::error!("Claude API returned an error: {}", err);
            callback(ChatResponse::new_with_arc(
                chat_id.clone(),
                err.to_string(),
                MessageType::Error,
                metadata_option.clone(),
                Some(FinishReason::Error),
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
                Some(FinishReason::Error),
            ));
            Ok(response.content)
        }
    }

    /// Lists available models of claude
    ///
    /// # Arguments
    /// * `api_url` - Optional API endpoint URL
    /// * `api_key` - Optional API key
    /// * `extra_params` - Additional parameters including proxy settings
    ///
    /// # Returns
    /// * `Vec<ModelDetails>` - List of available models
    /// * `AiError` - Error if the request fails
    async fn list_models(
        &self,
        api_url: Option<&str>,
        api_key: Option<&str>,
        extra_params: Option<Value>,
    ) -> Result<Vec<ModelDetails>, AiError> {
        let headers = json!({
            "x-api-key": api_key.unwrap_or(""),
            "anthropic-version": "2023-06-01", // Consistent with chat endpoint
        });

        let query_params = json!({
            "limit": 500 // Fetch up to 100 models, adjust if needed
        });

        let response = self
            .client
            .get_request(
                &ApiConfig::new(
                    Some(api_url.unwrap_or(CALUDE_BASE_URL).to_string()),
                    None,
                    get_proxy_type(extra_params),
                    Some(headers),
                ),
                "models", // Endpoint path
                Some(query_params),
            )
            .await
            .map_err(|network_err| {
                let err = AiError::ApiRequestFailed {
                    provider: "Claude".to_string(),
                    details: network_err.to_string(),
                };
                log::error!("Claude list_models API request failed: {}", err);
                err
            })?;

        if response.is_error || response.content.is_empty() {
            let err = AiError::ApiRequestFailed {
                provider: "Claude".to_string(),
                details: response.content,
            };
            log::error!("{}", err);
            return Err(err);
        }

        #[cfg(debug_assertions)]
        log::debug!("Claude list_models response: {}", &response.content);

        let api_response: ClaudeListModelsResponse = from_str(&response.content).map_err(|e| {
            let err = AiError::ResponseParseFailed {
                provider: "Claude".to_string(),
                details: e.to_string(),
            };
            log::error!("Claude list_models response parsing failed: {}", err);
            err
        })?;

        let models = api_response
            .data
            .into_iter()
            .fold(std::collections::HashMap::new(), |mut acc, model| {
                acc.insert(model.id.to_lowercase(), model);
                acc
            })
            .into_values()
            .map(|api_model| {
                let model_id = api_model.id.to_lowercase();
                ModelDetails {
                    id: api_model.id.clone(),
                    name: api_model.display_name.unwrap_or(model_id.clone()),
                    protocol: ChatProtocol::Claude,
                    max_input_tokens: api_model.input_token_limit,
                    max_output_tokens: api_model.output_token_limit,
                    description: api_model.description,
                    last_updated: api_model.created_at, // Use created_at from API
                    family: get_family_from_model_id(&model_id),
                    // Prioritize API's supports_tools, fallback to helper function if not present
                    function_call: api_model
                        .supports_tools
                        .or_else(|| Some(is_function_call_supported(&model_id))),
                    reasoning: Some(is_reasoning_supported(&model_id)),
                    image_input: Some(is_image_input_supported(&model_id)),
                    metadata: None,
                }
            })
            .collect();

        Ok(models)
    }
}
