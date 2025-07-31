use async_trait::async_trait;
use reqwest::Response;
use rust_i18n::t;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

use crate::ai::error::AiError;
use crate::ai::interaction::chat_completion::ChatProtocol;
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
use crate::ccproxy::{StreamFormat, StreamProcessor};
use crate::impl_stoppable;

const GEMINI_DEFAULT_API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta";

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct GeminiModel {
    name: String,         // e.g., "models/gemini-1.5-pro-latest"
    display_name: String, // e.g., "Gemini 1.5 Pro"
    description: Option<String>,
    #[serde(default)] // Make optional as not all models might have it explicitly
    input_token_limit: Option<u32>,
    #[serde(default)]
    output_token_limit: Option<u32>,
    supported_generation_methods: Vec<String>,
    #[serde(default)]
    version: Option<String>,
    // temperature, topP, topK could also be here if needed
}

#[derive(Deserialize, Debug)]
struct GeminiListModelsResponse {
    models: Vec<GeminiModel>,
}

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
        let mut gemini_contents = Vec::new();
        let mut system_instruction_val = None;

        for message_val in messages {
            // Use get for safer access
            let role = message_val
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let content_value = message_val.get("content"); // Option<&Value>
            let tool_calls_value = message_val.get("tool_calls"); // Option<&Value>

            let mut processed_parts_for_this_message: Vec<Value> = Vec::new();

            if let Some(cv) = content_value {
                if let Some(text_content) = cv.as_str() {
                    if !text_content.is_empty() {
                        processed_parts_for_this_message.push(json!({"text": text_content}));
                    }
                } else if let Some(array_content) = cv.as_array() {
                    for part_val in array_content {
                        if let Some(part_obj) = part_val.as_object() {
                            if let Some(part_type) = part_obj.get("type").and_then(Value::as_str) {
                                match part_type {
                                    "text" => {
                                        let text = part_obj
                                            .get("text")
                                            .and_then(Value::as_str)
                                            .unwrap_or("");
                                        if !text.is_empty() {
                                            processed_parts_for_this_message
                                                .push(json!({"text": text}));
                                        }
                                    }
                                    "image_url" => {
                                        // OpenAI specific, convert to Gemini inlineData
                                        if let Some(image_url_details) =
                                            part_obj.get("image_url").and_then(Value::as_object)
                                        {
                                            if let Some(url_str) =
                                                image_url_details.get("url").and_then(Value::as_str)
                                            {
                                                if url_str.starts_with("data:") {
                                                    // Expected format: "data:<mime_type>;base64,<data>"
                                                    let mut url_parts_iter = url_str.splitn(2, ',');
                                                    if let (Some(header_part), Some(data_part)) = (
                                                        url_parts_iter.next(),
                                                        url_parts_iter.next(),
                                                    ) {
                                                        // header_part = "data:image/jpeg;base64"
                                                        if let Some(media_type_and_encoding) =
                                                            header_part.strip_prefix("data:")
                                                        {
                                                            // media_type_and_encoding = "image/jpeg;base64"
                                                            let mut media_encoding_iter =
                                                                media_type_and_encoding
                                                                    .splitn(2, ';');
                                                            if let (
                                                                Some(mime_type),
                                                                Some(encoding),
                                                            ) = (
                                                                media_encoding_iter.next(),
                                                                media_encoding_iter.next(),
                                                            ) {
                                                                if encoding.to_lowercase()
                                                                    == "base64"
                                                                {
                                                                    processed_parts_for_this_message.push(json!({
                                                                        "inlineData": {
                                                                            "mimeType": mime_type,
                                                                            "data": data_part
                                                                        }
                                                                    }));
                                                                } else {
                                                                    log::warn!("Unsupported encoding in data URL for Gemini: '{}'. Expected 'base64'. Skipping image part. URL: {}", encoding, url_str);
                                                                }
                                                            } else {
                                                                log::warn!("Malformed data URL header for Gemini: could not split mime_type and encoding from '{}'. Skipping image part. URL: {}", media_type_and_encoding, url_str);
                                                            }
                                                        } else {
                                                            log::warn!("Malformed data URL for Gemini: 'data:' prefix found, but strip_prefix failed for '{}'. Skipping image part. URL: {}", header_part, url_str);
                                                        }
                                                    } else {
                                                        log::warn!("Malformed data URL for Gemini: could not split into header and data parts. Skipping image part. URL: {}", url_str);
                                                    }
                                                } else {
                                                    log::warn!("Image URL is not a data URL. Gemini requires base64 encoded images for inlineData. URL: {}. Skipping image part.", url_str);
                                                }
                                            } else {
                                                log::warn!("'image_url' object missing 'url' string. Skipping image part: {:?}", part_obj);
                                            }
                                        } else {
                                            log::warn!("'image_url' type part missing 'image_url' object details. Skipping part: {:?}", part_obj);
                                        }
                                    }
                                    "inlineData" | "fileData" => {
                                        // Already Gemini format
                                        processed_parts_for_this_message.push(part_val.clone());
                                    }
                                    _ => {
                                        log::warn!(
                                            "Unknown structured content part type '{}' for Gemini. Skipping: {:?}",
                                            part_type,
                                            part_val
                                        );
                                    }
                                }
                            } else if let Some(text_str) = part_val.as_str() {
                                // Simple string in parts array
                                if !text_str.is_empty() {
                                    processed_parts_for_this_message
                                        .push(json!({"text": text_str}));
                                }
                            } else {
                                log::warn!(
                                    "Unsupported content part in array for Gemini: {:?}. Skipping.",
                                    part_val
                                );
                            }
                        } else if let Some(text_str) = part_val.as_str() {
                            // Content is an array containing a simple string.
                            if !text_str.is_empty() {
                                processed_parts_for_this_message.push(json!({"text": text_str}));
                            }
                        } else {
                            log::warn!(
                                "Content part in array is not an object or string: {:?}. Skipping.",
                                part_val
                            );
                        }
                    }
                } else if cv.is_object() {
                    // Content is an object but not an array or string. This is unusual for OpenAI messages.
                    // Log a warning, as this might indicate an unexpected format.
                    log::warn!("Message content is an object but not an array of parts or a simple string. Attempting to treat as text: {:?}", cv);
                    // Try to serialize it to a string and use as text, or default to empty.
                    let text_representation = serde_json::to_string(cv).unwrap_or_default();
                    if !text_representation.is_empty()
                        && text_representation != "{}"
                        && text_representation != "null"
                    {
                        processed_parts_for_this_message.push(json!({"text": text_representation}));
                    }
                }
                // If content_value was null or some other unhandled type, processed_parts_for_this_message remains empty.
            }

            // Gemini API requires the "parts" field in a Content object to be non-empty.
            // This applies to user, model, and systemInstruction roles.
            // However, for an assistant message that will contain tool_calls,
            // it's okay for processed_parts_for_this_message (derived from original 'content' field)
            // to be empty at this stage. The functionCall parts will be added later.
            // If, after adding functionCall parts, the final list for an assistant message is still empty,
            // a fallback will be applied in that specific block.
            if processed_parts_for_this_message.is_empty() {
                let is_assistant_expecting_tools =
                    role == "assistant" && tool_calls_value.is_some();
                if !is_assistant_expecting_tools {
                    log::warn!(
                        "Content for role '{}' (message content: {:?}) resulted in empty parts for Gemini. Defaulting to a single empty text part.",
                        role, content_value
                    );
                    processed_parts_for_this_message.push(json!({"text": ""}));
                }
            }

            if role == "system" {
                system_instruction_val = Some(json!({
                    "role": "user", // Gemini system prompts are passed as user role in systemInstruction
                    "parts": processed_parts_for_this_message
                }));
            } else if role == "assistant" && tool_calls_value.is_some() {
                let mut final_assistant_parts = processed_parts_for_this_message.clone(); // Start with text/image parts

                if let Some(tool_calls_array) = tool_calls_value.and_then(Value::as_array) {
                    for tool_call_obj in tool_calls_array {
                        if let Some(name) = tool_call_obj
                            .get("function")
                            .and_then(|f| f.get("name"))
                            .and_then(Value::as_str)
                        {
                            let arguments_value = tool_call_obj
                                .get("function")
                                .and_then(|f| f.get("arguments")); // Option<&Value>

                            let args_obj: Value = arguments_value.map_or_else(
                                || {
                                    log::warn!("Tool call '{}' missing 'arguments'. Using empty object.", name);
                                    json!({})
                                },
                                |val| {
                                    if val.is_string() {
                                        serde_json::from_str(val.as_str().unwrap_or("{}"))
                                            .unwrap_or_else(|e| {
                                            log::warn!(
                                                "Failed to parse JSON string for tool arguments for '{}': {}. Value: '{}'. Using empty object.",
                                                name, e, val.as_str().unwrap_or("")
                                            );
                                            json!({})
                                        })
                                    } else if val.is_object() {
                                        val.clone()
                                    } else {
                                        log::warn!(
                                            "Tool arguments for '{}' are neither a string nor an object: {:?}. Using empty object.",
                                            name, val
                                        );
                                        json!({})
                                    }
                                },
                            );
                            final_assistant_parts.push(json!({
                                "functionCall": {
                                    "name": name,
                                    "args": args_obj
                                }
                            }));
                        } else {
                            log::warn!(
                                "Skipping malformed tool_call object in assistant message: {:?}",
                                tool_call_obj
                            );
                        }
                    }
                }
                if final_assistant_parts.is_empty() {
                    log::warn!("Assistant message with tool_calls resulted in empty parts for Gemini. Original message: {:?}", message_val);
                    final_assistant_parts.push(json!({"text": ""})); // Fallback
                }
                gemini_contents.push(json!({
                    "role": "model",
                    "parts": final_assistant_parts
                }));
            } else if role == "tool" {
                if let (Some(name_val_str), Some(content_val)) = (
                    message_val.get("name").and_then(Value::as_str),
                    message_val.get("content"), // Option<&Value>
                ) {
                    let tool_result_content: Value = if content_val.is_string() {
                        // Attempt to parse the string as JSON. If it fails, use the string directly.
                        serde_json::from_str(content_val.as_str().unwrap_or(""))
                            .unwrap_or_else(|e| {
                                log::debug!("Tool result content for '{}' is a string but not valid JSON. Passing as raw string. Error: {}, Value: '{}'", name_val_str, e, content_val.as_str().unwrap_or(""));
                                content_val.clone() // Use the original string Value
                            })
                    } else {
                        content_val.clone() // Already an object or other JSON type
                    };

                    gemini_contents.push(json!({
                        "role": "function",
                        "parts": [{
                            "functionResponse": {
                                "name": name_val_str,
                                "response": {
                                    "content": tool_result_content // This should be a JSON value
                                }
                            }
                        }]
                    }));
                } else {
                    log::warn!("Skipping malformed tool message: {:?}", message_val);
                }
            } else {
                // For "user" role, or "assistant" without tool_calls
                gemini_contents.push(json!({
                    "role": if role == "assistant" { "model" } else { "user" },
                    "parts": processed_parts_for_this_message
                }));
            }
        }

        let response_format_type = params
            .get("response_format")
            .and_then(|rf| rf.get("type"))
            .and_then(|t| t.as_str())
            .unwrap_or("text"); // Default to "text"

        let response_mime_type = if response_format_type == "json_object" {
            "application/json"
        } else {
            "text/plain" // Default for "text" or any other type
        };

        let mut payload = json!({
            "contents": gemini_contents,
            "generationConfig": {
                "responseMimeType": response_mime_type
            }
        });

        if let Some(obj) = payload.as_object_mut() {
            if let Some(instruction) = system_instruction_val {
                if instruction
                    .get("parts")
                    .and_then(Value::as_array)
                    .map_or(false, |p| !p.is_empty())
                {
                    obj.insert("systemInstruction".to_string(), instruction);
                } else {
                    log::warn!("System instruction parts were empty, not adding to payload.");
                }
            }

            if let Some(generation_config_value) = obj.get_mut("generationConfig") {
                if let Some(generation_config_map) = generation_config_value.as_object_mut() {
                    if let Some(temperature_val) =
                        params.get("temperature").and_then(|v| v.as_f64())
                    {
                        if temperature_val >= 0.0 && temperature_val <= 2.0 {
                            // Gemini's typical range
                            generation_config_map
                                .insert("temperature".to_string(), json!(temperature_val));
                        }
                    }

                    if let Some(max_tokens_val) = params.get("max_tokens").and_then(|v| v.as_u64())
                    {
                        if max_tokens_val > 0 {
                            generation_config_map
                                .insert("maxOutputTokens".to_string(), json!(max_tokens_val));
                        }
                    }

                    if let Some(top_k) = params.get("top_k").and_then(|v| v.as_i64()) {
                        if top_k > 0 {
                            // Gemini's topK must be positive
                            generation_config_map.insert("topK".to_string(), json!(top_k));
                        }
                    }
                    if let Some(top_p) = params.get("top_p").and_then(|v| v.as_f64()) {
                        if top_p > 0.0 && top_p <= 1.0 {
                            // Gemini's topP range
                            generation_config_map.insert("topP".to_string(), json!(top_p));
                        }
                    }

                    if let Some(stop_sequences) =
                        params.get("stop_sequences").and_then(|v| v.as_array())
                    {
                        if !stop_sequences.is_empty() {
                            generation_config_map
                                .insert("stopSequences".to_string(), json!(stop_sequences));
                        }
                    }

                    if let Some(candidate_count) =
                        params.get("candidate_count").and_then(|v| v.as_u64())
                    {
                        if candidate_count > 0 {
                            // Typically 1 for chat, but API might support more
                            generation_config_map
                                .insert("candidateCount".to_string(), json!(candidate_count));
                        }
                    }
                }
            }

            if params.get("tool_choice").and_then(|tc| tc.as_str()) != Some("none") {
                if let Some(tools_vec) = tools {
                    let gemini_tools = tools_vec
                        .into_iter()
                        .map(|tool| tool.to_gemini()) // Assuming MCPToolDeclaration has to_gemini()
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
        let mut reasoning_content = String::new();
        let mut full_response = String::new();
        let mut token_usage = TokenUsage::default();
        let start_time = Instant::now();

        let processor = StreamProcessor::new();
        let mut event_receiver = processor
            .process_stream(response, &StreamFormat::Gemini)
            .await;

        let mut accumulated_tool_calls: HashMap<u32, ToolCallDeclaration> = HashMap::new();
        let mut finish_reason = FinishReason::Complete;

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
                                Some(FinishReason::Error),
                            ));
                            err
                        })?;

                    for chunk in stream_chunks {
                        // Iterate over processed chunks
                        if let Some(content) = chunk.content.clone() {
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

                        // Gemini has not reasoning content, but we might get it in the future
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

                        if let Some(tool_call_parts) = chunk.tool_calls {
                            for part in tool_call_parts {
                                let acc_call = accumulated_tool_calls
                                    .entry(part.index)
                                    .or_insert_with(|| ToolCallDeclaration {
                                        index: part.index, // part.index should be unique for each tool call instance
                                        // If Gemini's streaming part doesn't have a persistent ID for the call, generate one.
                                        // This ID needs to be consistent for this specific tool call across AssistantAction and ToolCall messages.
                                        // For now, we assume part.id might be empty initially and populated if a full ID is found,
                                        // or we might need to generate it based on index if Gemini doesn't provide one per tool_call instance.
                                        // A robust solution might be to generate a UUID here if part.id is empty.
                                        id: if part.id.is_empty() {
                                            // Use a consistent way to generate ID if not provided by Gemini stream part
                                            // Using chat_id and index makes it unique per chat and tool call part.
                                            format!(
                                                "gemtool_{}_{}",
                                                chat_id.chars().take(8).collect::<String>(),
                                                part.index
                                            )
                                        } else {
                                            part.id.clone()
                                        }, // Ensure id is always populated
                                        name: part.name.clone(),
                                        arguments: Some(String::new()),
                                        results: None,
                                    });
                                // If a more complete ID or name comes later in the stream for the same index, update it.
                                if !part.id.is_empty() && acc_call.id.is_empty() {
                                    acc_call.id = part.id.clone();
                                } // This condition might be redundant if id is always populated above
                                if !part.name.is_empty() && acc_call.name.is_empty() {
                                    acc_call.name = part.name.clone();
                                } // This condition might be redundant if name is always populated above

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

                        // Check Gemini's finish reason
                        let current_chunk_finish_reason = chunk.finish_reason.as_deref();
                        if let Some(reason_str) = current_chunk_finish_reason {
                            // If tools have been accumulated and we get any terminal finish reason,
                            // it means the model has finished its turn and expects tool calls.
                            if !accumulated_tool_calls.is_empty()
                                && (reason_str == "STOP"
                                    || reason_str == "MAX_TOKENS"
                                    || reason_str == "TOOL_CODE"
                                    || reason_str == "FUNCTION_CALL")
                            {
                                finish_reason = FinishReason::ToolCalls;
                            } else {
                                // If no tools accumulated, map the finish reason normally
                                match reason_str {
                                    "STOP" => finish_reason = FinishReason::Complete,
                                    "MAX_TOKENS" => finish_reason = FinishReason::Complete, // As per your previous decision
                                    "TOOL_CODE" | "FUNCTION_CALL" => {
                                        // This case should ideally be caught by the `!accumulated_tool_calls.is_empty()` above
                                        // but if it somehow reaches here and tools *are* empty, it's an anomaly.
                                        log::warn!("Gemini finish_reason '{}' received but no tools were accumulated.", reason_str);
                                        finish_reason = FinishReason::Complete; // Or Error, depending on desired strictness
                                    }
                                    "SAFETY" | "RECITATION" | "OTHER" => {
                                        finish_reason = FinishReason::Error
                                    }
                                    unknown => {
                                        log::warn!("Unknown Gemini finish reason: {}", unknown);
                                        finish_reason = FinishReason::Complete;
                                    }
                                }
                            }
                        }

                        // If the determined finish_reason is ToolCalls (from this chunk or a previous one that set it)
                        // AND we have accumulated tools, then send AssistantAction and ToolCalls
                        if finish_reason == FinishReason::ToolCalls
                            && !accumulated_tool_calls.is_empty()
                        {
                            // This block should only execute once when all tool call parts are received and finish_reason is ToolCalls

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
                                        "id": tcd.id,
                                        "type": "function",
                                        "function": {
                                            "name": tcd.name,
                                            "arguments": arguments_str
                                        }
                                    })
                                })
                                .collect();

                            let assistant_action_message = json!({
                                "role": "assistant",
                                "content": if full_response.is_empty() { Value::Null } else { Value::String(full_response.clone()) },
                                "tool_calls": assistant_tool_requests
                            });

                            match serde_json::to_string(&assistant_action_message) {
                                Ok(serialized_assistant_action) => {
                                    callback(ChatResponse::new_with_arc(
                                        chat_id.clone(),
                                        serialized_assistant_action,
                                        MessageType::AssistantAction,
                                        metadata_option.clone(),
                                        None, // Individual AssistantAction doesn't need a finish reason here, the final "Finished" message will carry it.
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
                                            None,
                                        ));
                                    }
                                }
                            }
                            accumulated_tool_calls.clear(); // Clear after sending
                                                            // The loop will continue, but subsequent chunks should not re-trigger this block
                                                            // because accumulated_tool_calls is now empty.
                                                            // The final "Finished" message will use the `finish_reason` (ToolCalls).
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
                        None,
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
            Some(finish_reason),
        ));

        Ok(format!(
            "<think>{}</think>{}",
            reasoning_content, full_response
        ))
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

        let default_url = format!("{GEMINI_DEFAULT_API_BASE}");
        let base_url = api_url.unwrap_or(&default_url);
        // Determine if streaming is requested from params, default to true if not specified
        let is_streaming = params
            .get("stream")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        let with_models = if model.starts_with("models/") {
            ""
        } else {
            "models/"
        };
        let query_url = if is_streaming {
            format!(
                "{}/{}{}:streamGenerateContent?alt=sse&key={}",
                base_url,
                with_models,
                model,
                api_key.unwrap_or("")
            )
        } else {
            format!(
                "{}/{}{}:generateContent?key={}",
                base_url,
                with_models,
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
                    Some(FinishReason::Error),
                ));
                err
            })?;

        if response.is_error {
            let err = AiError::ApiRequestFailed {
                provider: "Gemini".to_string(),
                details: response.content.clone(),
            };
            // log::error!("Gemini API returned an error: {}", err);
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
            self.process_response(chat_id.clone(), raw_response, callback, metadata_option)
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

    async fn list_models(
        &self,
        api_url: Option<&str>,
        api_key: Option<&str>, // This is the Gemini API Key
        extra_args: Option<Value>,
    ) -> Result<Vec<ModelDetails>, AiError> {
        let base_url = api_url.unwrap_or(GEMINI_DEFAULT_API_BASE);

        if api_key.is_none() || api_key.as_deref().unwrap_or("").is_empty() {
            return Err(AiError::InvalidInput(
                t!("api_key_is_require_for_list_models", provider = "Gemini").to_string(),
            ));
        }
        let endpoint_with_key = format!("/models?key={}", api_key.unwrap_or_default());

        // For Gemini, API key is in query param, so ApiConfig.api_key should be None
        // to prevent DefaultApiClient from adding a "Bearer" token.
        let config = ApiConfig::new(
            Some(base_url.to_string()),
            None,
            get_proxy_type(extra_args.clone()),
            None, // No special headers needed if key is in URL for this request
        );

        let response = self
            .client
            .get_request(&config, &endpoint_with_key, None)
            .await
            .map_err(|e| AiError::ApiRequestFailed {
                provider: "Gemini".to_string(),
                details: e.to_string(), // Changed from e to e.to_string()
            })?;

        if response.is_error || response.content.is_empty() {
            return Err(AiError::ApiRequestFailed {
                provider: "Gemini".to_string(),
                details: response.content,
            });
        }

        #[cfg(debug_assertions)]
        log::debug!("Gemini list_models response: {}", &response.content);

        let models_response: GeminiListModelsResponse = serde_json::from_str(&response.content)
            .map_err(|e| AiError::ResponseParseFailed {
                provider: "Gemini".to_string(),
                details: e.to_string(),
            })?;

        let model_details: Vec<ModelDetails> = models_response
            .models
            .into_iter()
            // Filter for models that support 'generateContent', common for chat
            .filter(|m| {
                m.supported_generation_methods
                    .contains(&"generateContent".to_string())
            })
            .map(|model| {
                let id_for_utils = model.name.to_lowercase();

                ModelDetails {
                    id: model
                        .name
                        .clone()
                        .strip_prefix("models/")
                        .map(|s| s.to_string())
                        .unwrap_or_default(), // e.g. "models/gemini-1.5-pro-latest"
                    name: model.display_name.clone(),
                    protocol: ChatProtocol::Gemini,
                    max_input_tokens: model.input_token_limit,
                    max_output_tokens: model.output_token_limit,
                    description: model.description,
                    last_updated: model.version.clone(), // Use version as a proxy for update info
                    family: get_family_from_model_id(&id_for_utils),
                    function_call: Some(is_function_call_supported(&id_for_utils)),
                    reasoning: Some(is_reasoning_supported(&id_for_utils)),
                    image_input: Some(is_image_input_supported(&id_for_utils)),
                    metadata: Some(json!({
                        "version": model.version,
                        "supported_generation_methods": model.supported_generation_methods,
                    })),
                }
            })
            .collect();

        Ok(model_details)
    }
}
