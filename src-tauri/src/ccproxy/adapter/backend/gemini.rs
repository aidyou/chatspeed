use crate::ccproxy::adapter::unified::SseStatus;
use crate::ccproxy::get_tool_id;
use crate::ccproxy::types::{
    TOOL_PARSE_ERROR_REMINDER, TOOL_RESULT_SUFFIX_REMINDER, TOOL_TAG_END, TOOL_TAG_START,
};
use crate::ccproxy::{adapter::backend::update_message_block, types::ChatProtocol};
use async_trait::async_trait;
use reqwest::{Client, RequestBuilder};
use serde_json::json;
use std::sync::{Arc, RwLock};

use super::{BackendAdapter, BackendResponse};
use crate::ccproxy::adapter::{
    range_adapter::adapt_temperature,
    unified::{
        UnifiedContentBlock, UnifiedRequest, UnifiedResponse, UnifiedRole, UnifiedStreamChunk,
        UnifiedToolChoice, UnifiedUsage,
    },
};
use crate::ccproxy::gemini::{
    GeminiContent, GeminiFunctionCall, GeminiFunctionCallingConfig, GeminiFunctionDeclaration,
    GeminiFunctionResponse, GeminiGenerationConfig, GeminiInlineData, GeminiPart, GeminiRequest,
    GeminiResponse as GeminiNetworkResponse, GeminiTool as GeminiApiTool, GeminiToolConfig,
};
use crate::ccproxy::utils::token_estimator::estimate_tokens;

pub struct GeminiBackendAdapter;

impl GeminiBackendAdapter {
    /// Extract only Gemini-supported JSON Schema fields
    ///
    /// @link https://ai.google.dev/api/caching#Schema
    /// @link https://ai.google.dev/api/caching?hl=zh-cn#Schema
    fn extract_gemini_schema(schema: &serde_json::Value) -> serde_json::Value {
        match schema {
            serde_json::Value::Object(obj) => {
                let mut gemini_schema = serde_json::Map::new();

                for (key, value) in obj {
                    match key.as_str() {
                        "properties" => {
                            if let serde_json::Value::Object(props) = value {
                                let mut cleaned_props = serde_json::Map::new();
                                for (prop_name, prop_schema) in props {
                                    cleaned_props.insert(
                                        prop_name.clone(),
                                        Self::extract_gemini_schema(prop_schema),
                                    );
                                }
                                gemini_schema
                                    .insert(key.clone(), serde_json::Value::Object(cleaned_props));
                            }
                        }
                        "items" => {
                            gemini_schema.insert(key.clone(), Self::extract_gemini_schema(value));
                        }
                        "type" => {
                            let type_val = if let Some(arr) = value.as_array() {
                                arr.iter()
                                    .find(|v| !v.is_null())
                                    .cloned()
                                    .unwrap_or_else(|| value.clone())
                            } else {
                                value.clone()
                            };

                            if let Some(type_str) = type_val.as_str() {
                                let new_type = match type_str {
                                    "string" | "boolean" | "array" | "object" => type_val.clone(),
                                    "number" | "float" | "double" => json!("number"),
                                    "integer" | "int" | "int32" | "int64" | "uint" | "uint32"
                                    | "uint64" => json!("integer"),
                                    _ => {
                                        log::warn!(
                                            "Unknown type '{}' in schema, defaulting to string",
                                            type_str
                                        );
                                        json!("string")
                                    }
                                };
                                gemini_schema.insert(key.clone(), new_type);
                            } else {
                                gemini_schema.insert(key.clone(), type_val);
                            }
                        }
                        "format" => {
                            if let serde_json::Value::String(format_str) = value {
                                match format_str.as_str() {
                                    // Pass through supported formats
                                    "float" | "double" | "int32" | "int64" => {
                                        gemini_schema.insert(key.clone(), value.clone());
                                    }
                                    // Unsigned to signed
                                    "uint" | "uint32" | "uint64" | "int" | "integer" => {
                                        gemini_schema.insert(key.clone(), json!("int64"));
                                    }
                                    _ => {
                                        // For unsupported formats like "uri", "email", etc.,
                                        // we just ignore the format keyword but keep the parameter
                                        // by not adding the format field to the cleaned schema.
                                        log::debug!(
                                            "Ignoring unsupported format '{}' for Gemini API, treating as plain string.",
                                            format_str
                                        );
                                    }
                                }
                            }
                        }
                        // Include other supported fields directly
                        "description" | "enum" | "required" | "minimum" | "maximum"
                        | "minLength" | "maxLength" | "pattern" | "minItems" | "maxItems" => {
                            gemini_schema.insert(key.clone(), value.clone());
                        }
                        // Ignore unsupported fields like "$schema", "additionalProperties", etc.
                        _ => {}
                    }
                }

                serde_json::Value::Object(gemini_schema)
            }
            _ => schema.clone(),
        }
    }

    /// Process tool compatibility mode chunk
    async fn process_tool_compat_chunk(
        &self,
        chunk: bytes::Bytes,
        sse_status: Arc<RwLock<SseStatus>>,
    ) -> Result<Vec<UnifiedStreamChunk>, anyhow::Error> {
        let chunk_str = String::from_utf8_lossy(&chunk);
        let mut unified_chunks = Vec::new();

        for line in chunk_str.lines() {
            if line.starts_with("data:") {
                let data_str = line["data:".len()..].trim();
                let gemini_response: GeminiNetworkResponse = serde_json::from_str(data_str)
                    .map_err(|e| {
                        log::error!(
                            "Gemini delta deserialize failed, delta: {}, error:{}",
                            &data_str,
                            e.to_string()
                        );
                        e
                    })?;

                if let Ok(mut status) = sse_status.write() {
                    if !status.message_start {
                        status.message_start = true;
                        unified_chunks.push(UnifiedStreamChunk::MessageStart {
                            id: status.message_id.clone(),
                            model: status.model_id.clone(),
                            usage: UnifiedUsage {
                                input_tokens: 0,
                                output_tokens: 0,
                                ..Default::default()
                            },
                        });
                    }
                }

                if let Some(candidates) = gemini_response.candidates {
                    for candidate in candidates {
                        for part in candidate.content.parts {
                            if let Some(text) = part.text.clone() {
                                if !text.is_empty() {
                                    if let Ok(mut status) = sse_status.write() {
                                        // Add text to fragment buffer for tool compatibility processing
                                        status.tool_compat_fragment_buffer.push_str(&text);
                                        status.tool_compat_fragment_count += 1;

                                        let now = std::time::Instant::now();
                                        let time_since_flush = now
                                            .duration_since(status.tool_compat_last_flush_time)
                                            .as_millis();

                                        // Force flush if conditions are met
                                        let should_flush = status.tool_compat_fragment_count >= 25
                                            || time_since_flush >= 100
                                            || status.tool_compat_fragment_buffer.len() > 500
                                            || status
                                                .tool_compat_fragment_buffer
                                                .contains(TOOL_TAG_START)
                                            || status
                                                .tool_compat_fragment_buffer
                                                .contains(TOOL_TAG_END);

                                        if should_flush {
                                            self.flush_tool_compat_buffer(
                                                &mut status,
                                                &mut unified_chunks,
                                            );
                                        }
                                    }
                                }
                            }
                        }

                        if let Some(finish_reason) = candidate.finish_reason {
                            let stop_reason = finish_reason.to_string();
                            let usage = gemini_response
                                .usage_metadata
                                .clone()
                                .map(|u| UnifiedUsage {
                                    input_tokens: u.prompt_token_count,
                                    output_tokens: u.candidates_token_count.unwrap_or(0),
                                    cache_creation_input_tokens: None,
                                    cache_read_input_tokens: None,
                                    tool_use_prompt_tokens: u.tool_use_prompt_token_count,
                                    thoughts_tokens: u.thoughts_token_count,
                                    cached_content_tokens: u.cached_content_token_count,
                                    total_duration: None,
                                    load_duration: None,
                                    prompt_eval_duration: None,
                                    eval_duration: None,
                                    prompt_cached_tokens: None,
                                })
                                .unwrap_or_default();

                            // On stream finish, process any remaining buffers
                            if let Ok(mut status) = sse_status.write() {
                                // Use the common function to handle incomplete tags
                                crate::ccproxy::adapter::backend::common::auto_complete_and_process_tool_tag(&mut status, &mut unified_chunks);

                                // After attempting to complete and process tags, send any remaining text.
                                if !status.tool_compat_buffer.is_empty() {
                                    unified_chunks.push(UnifiedStreamChunk::Text {
                                        delta: status.tool_compat_buffer.clone(),
                                    });
                                    status.tool_compat_buffer.clear();
                                }
                            }

                            unified_chunks
                                .push(UnifiedStreamChunk::MessageStop { stop_reason, usage });
                        }
                    }
                }
            }
        }

        Ok(unified_chunks)
    }

    /// Flush tool compatibility buffer and process tool calls
    fn flush_tool_compat_buffer(
        &self,
        status: &mut std::sync::RwLockWriteGuard<SseStatus>,
        unified_chunks: &mut Vec<UnifiedStreamChunk>,
    ) {
        let fragment = status.tool_compat_fragment_buffer.clone();
        status.tool_compat_buffer.push_str(&fragment);
        status.tool_compat_fragment_buffer.clear();
        status.tool_compat_fragment_count = 0;
        status.tool_compat_last_flush_time = std::time::Instant::now();

        // Process tool calls in the buffer using common utility
        crate::ccproxy::adapter::backend::common::process_tool_calls_in_buffer(
            status,
            unified_chunks,
        );
    }
}

#[async_trait]
impl BackendAdapter for GeminiBackendAdapter {
    async fn adapt_request(
        &self,
        client: &Client,
        unified_request: &mut UnifiedRequest,
        _api_key: &str,
        full_provider_url: &str,
        _model: &str,
        log_proxy_to_file: bool,
        headers: &mut reqwest::header::HeaderMap,
    ) -> Result<RequestBuilder, anyhow::Error> {
        crate::ccproxy::adapter::backend::common::preprocess_unified_request(unified_request);

        // --- Tool Compatibility Mode Handling ---
        // If tool_compat_mode is enabled, we inject a system prompt with tool definitions
        // into the system message. This is a specific adaptation for models that
        // don't support native tool calling APIs but can follow instructions.
        //
        // The call to `unified_request.enhance_prompt` should be placed as early as possible because
        // it handles tool progression in compatibility mode and removes the tool calling module.
        unified_request.enhance_prompt();

        let mut gemini_contents: Vec<GeminiContent> = Vec::new();

        // --- Message Processing ---
        if unified_request.tool_compat_mode {
            // Special handling for tool compatibility mode to mimic a more natural
            // conversation flow for models that don't natively support tool calls.
            let mut processed_messages: Vec<GeminiContent> = Vec::new();
            let mut tool_results_buffer: Vec<String> = Vec::new();

            for msg in &unified_request.messages {
                match msg.role {
                    UnifiedRole::Assistant => {
                        // If there are pending tool results, flush them as a single user message first.
                        if !tool_results_buffer.is_empty() {
                            processed_messages.push(GeminiContent {
                                role: Some("user".to_string()),
                                parts: vec![GeminiPart {
                                    text: Some(tool_results_buffer.join("\n")),
                                    ..Default::default()
                                }],
                            });
                            tool_results_buffer.clear();
                        }

                        // Process the assistant message, converting ToolUse blocks to XML.
                        let mut content_parts: Vec<String> = Vec::new();

                        for block in &msg.content {
                            match block {
                                UnifiedContentBlock::Text { text } => {
                                    // Trim whitespace from the text part to avoid accumulating newlines
                                    let trimmed_text = text.trim();
                                    if !trimmed_text.is_empty() {
                                        content_parts.push(trimmed_text.to_string());
                                    }
                                }
                                UnifiedContentBlock::ToolUse { id, name, input } => {
                                    // Format the tool use block as XML
                                    let tool_use_xml =
                                        crate::ccproxy::helper::tool_use_xml::format_tool_use_xml(
                                            id, name, input,
                                        );
                                    content_parts.push(tool_use_xml);
                                }
                                // Ignore other block types like Image, ToolResult, etc. in this context
                                _ => {}
                            }
                        }

                        // Join all parts with a consistent separator
                        let final_content = content_parts.join("\n");

                        if !final_content.is_empty() {
                            processed_messages.push(GeminiContent {
                                role: Some("model".to_string()),
                                parts: vec![GeminiPart {
                                    text: Some(final_content),
                                    ..Default::default()
                                }],
                            });
                        }
                    }
                    UnifiedRole::Tool => {
                        // Collect tool results into a buffer to be flushed later.
                        for block in &msg.content {
                            if let UnifiedContentBlock::ToolResult {
                                tool_use_id,
                                content,
                                ..
                            } = block
                            {
                                let result_xml = format!(
                                    "<cs:tool_result id=\"{}\">{}</cs:tool_result>",
                                    tool_use_id, content
                                );
                                tool_results_buffer.push(result_xml);
                            }
                        }
                    }
                    UnifiedRole::User => {
                        // Flush any pending tool results before processing the user message.
                        if !tool_results_buffer.is_empty() {
                            processed_messages.push(GeminiContent {
                                role: Some("user".to_string()),
                                parts: vec![GeminiPart {
                                    text: Some(tool_results_buffer.join("\n")),
                                    ..Default::default()
                                }],
                            });
                            tool_results_buffer.clear();
                        }

                        // Add the actual user message.
                        let content_text = msg
                            .content
                            .iter()
                            .filter_map(|block| {
                                if let UnifiedContentBlock::Text { text } = block {
                                    Some(text.as_str())
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n\n");

                        if !content_text.is_empty() {
                            processed_messages.push(GeminiContent {
                                role: Some("user".to_string()),
                                parts: vec![GeminiPart {
                                    text: Some(content_text),
                                    ..Default::default()
                                }],
                            });
                        }
                    }
                    _ => {} // Ignore System role in this context, it's handled separately.
                }
            }

            // Flush any remaining tool results at the end of the message list.
            if !tool_results_buffer.is_empty() {
                processed_messages.push(GeminiContent {
                    role: Some("user".to_string()),
                    parts: vec![GeminiPart {
                        text: Some(tool_results_buffer.join("\n")),
                        ..Default::default()
                    }],
                });
            }

            // Append reminder to the last tool result message to prevent duplicate replies
            if let Some(last_message) = processed_messages.last_mut() {
                if last_message.role.as_deref() == Some("user") {
                    if let Some(part) = last_message.parts.get_mut(0) {
                        if let Some(text) = &mut part.text {
                            // Check if this user message contains a tool result.
                            // This indicates it's an auto-generated message, not a real user query.
                            if text.contains("<cs:tool_result") {
                                text.push_str("\n");
                                text.push_str(TOOL_RESULT_SUFFIX_REMINDER);
                            }
                        }
                    }
                }
            }

            gemini_contents = processed_messages;
        } else {
            // Standard processing for non-tool-compatibility mode.
            for msg in &unified_request.messages {
                let gemini_role = match msg.role {
                    UnifiedRole::User => "user",
                    UnifiedRole::Assistant => "model",
                    UnifiedRole::Tool => "user", // Gemini treats tool responses as from the user
                    UnifiedRole::System => continue, // System prompt is handled at the top level
                };

                let mut parts: Vec<GeminiPart> = Vec::new();
                for block in &msg.content {
                    match block {
                        UnifiedContentBlock::Text { text } => {
                            parts.push(GeminiPart {
                                text: Some(text.clone()),
                                ..Default::default()
                            });
                        }
                        UnifiedContentBlock::Image { media_type, data } => {
                            parts.push(GeminiPart {
                                inline_data: Some(GeminiInlineData {
                                    mime_type: media_type.clone(),
                                    data: data.clone(),
                                }),
                                ..Default::default()
                            });
                        }
                        UnifiedContentBlock::ToolUse { id: _, name, input } => {
                            parts.push(GeminiPart {
                                function_call: Some(GeminiFunctionCall {
                                    name: name.clone(),
                                    args: input.clone(),
                                }),
                                ..Default::default()
                            });
                        }
                        UnifiedContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            is_error: _,
                        } => {
                            parts.push(GeminiPart {
                                function_response: Some(GeminiFunctionResponse {
                                    name: tool_use_id.clone(),
                                    response: json!({ "content": content.clone() }),
                                }),
                                ..Default::default()
                            });
                        }
                        _ => {} // Ignore other block types
                    }
                }

                if !parts.is_empty() {
                    gemini_contents.push(GeminiContent {
                        role: Some(gemini_role.to_string()),
                        parts,
                    });
                }
            }
        }

        // --- Prompt Injection Logic ---
        let injection_pos = unified_request
            .prompt_injection_position
            .as_deref()
            .unwrap_or("system");
        let combined_prompt_text = unified_request
            .combined_prompt
            .as_deref()
            .unwrap_or_default();
        let mut final_system_instruction: Option<GeminiContent> = None;

        if unified_request.tool_compat_mode {
            // In tool compatibility mode, always inject prompt as system instruction
            let injection_mode = unified_request
                .prompt_injection
                .as_deref()
                .unwrap_or("enhance");
            let original_system_prompt =
                unified_request.system_prompt.as_deref().unwrap_or_default();

            let prompt = if injection_mode == "replace" {
                combined_prompt_text.to_string()
            } else {
                // "enhance" or default
                [original_system_prompt, combined_prompt_text]
                    .iter()
                    .filter(|s| !s.is_empty())
                    .map(|s| *s)
                    .collect::<Vec<&str>>()
                    .join("\n\n")
            };

            if !prompt.trim().is_empty() {
                final_system_instruction = Some(GeminiContent {
                    role: None, // Conceptual role
                    parts: vec![GeminiPart {
                        text: Some(prompt),
                        ..Default::default()
                    }],
                });
            }
        } else if injection_pos == "user" && !combined_prompt_text.is_empty() {
            // Find the last user message and append the prompt.
            if let Some(last_user_msg) = gemini_contents
                .iter_mut()
                .rfind(|m| m.role.as_deref() == Some("user"))
            {
                last_user_msg.parts.insert(
                    0,
                    GeminiPart {
                        text: Some(combined_prompt_text.to_string()),
                        ..Default::default()
                    },
                );
            }

            // Use the original system prompt as is.
            if let Some(sys_prompt_str) = &unified_request.system_prompt {
                if !sys_prompt_str.trim().is_empty() {
                    final_system_instruction = Some(GeminiContent {
                        role: None, // Conceptual role
                        parts: vec![GeminiPart {
                            text: Some(sys_prompt_str.clone()),
                            ..Default::default()
                        }],
                    });
                }
            }
        } else {
            // injection_pos == "system" or default behavior
            let injection_mode = unified_request
                .prompt_injection
                .as_deref()
                .unwrap_or("enhance");
            let original_system_prompt =
                unified_request.system_prompt.as_deref().unwrap_or_default();

            let prompt = if injection_mode == "replace" {
                combined_prompt_text.to_string()
            } else {
                // "enhance" or default
                [original_system_prompt, combined_prompt_text]
                    .iter()
                    .filter(|s| !s.is_empty())
                    .map(|s| *s)
                    .collect::<Vec<&str>>()
                    .join("\n\n")
            };

            if !prompt.trim().is_empty() {
                final_system_instruction = Some(GeminiContent {
                    role: None, // Conceptual role
                    parts: vec![GeminiPart {
                        text: Some(prompt),
                        ..Default::default()
                    }],
                });
            }
        }

        let system_instruction = final_system_instruction;

        let gemini_tools = unified_request.tools.as_ref().map(|tools| {
            vec![GeminiApiTool {
                function_declarations: tools
                    .iter()
                    .map(|tool| GeminiFunctionDeclaration {
                        name: tool.name.clone(),
                        description: tool.description.clone(),
                        parameters: Some(Self::extract_gemini_schema(&tool.input_schema)),
                    })
                    .collect(),
            }]
        });

        let gemini_tool_config = unified_request.tool_choice.as_ref().map(|choice| {
            let mode = match choice {
                UnifiedToolChoice::None => "NONE".to_string(),
                UnifiedToolChoice::Auto => "AUTO".to_string(),
                UnifiedToolChoice::Required => "ANY".to_string(),
                UnifiedToolChoice::Tool { name: _ } => "ANY".to_string(),
            };
            GeminiToolConfig {
                function_calling_config: Some(GeminiFunctionCallingConfig { mode }),
            }
        });

        let gemini_request = GeminiRequest {
            contents: gemini_contents,
            generation_config: Some(GeminiGenerationConfig {
                temperature: unified_request
                    .temperature
                    .map(|t| adapt_temperature(t, ChatProtocol::Gemini)),
                top_p: unified_request.top_p,
                top_k: unified_request.top_k.map(|v| v as i32),
                max_output_tokens: unified_request.max_tokens.map(|v| v as i32),
                stop_sequences: unified_request.stop_sequences.clone(),
                response_mime_type: unified_request.response_mime_type.clone(),
                response_schema: unified_request.response_schema.clone(),
                thinking_config: unified_request.thinking.as_ref().map(|t| {
                    crate::ccproxy::types::gemini::GeminiThinkingConfig {
                        thinking_budget: t.budget_tokens,
                        include_thoughts: t.include_thoughts,
                    }
                }),
            }),
            tools: gemini_tools,
            tool_config: gemini_tool_config,
            system_instruction,
            safety_settings: unified_request.safety_settings.clone(),
            cached_content: unified_request.cached_content.clone(),
        };

        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );

        let mut request_json = serde_json::to_value(&gemini_request)?;

        // Merge custom params from model config
        crate::ai::util::merge_custom_params(&mut request_json, &unified_request.custom_params);

        if log_proxy_to_file {
            // Log the request to a file
            log::info!(target: "ccproxy_logger","Gemini Request Body: \n{}\n----------------\n", serde_json::to_string_pretty(&request_json).unwrap_or_default());
        }

        Ok(client.post(full_provider_url).json(&request_json))
    }

    async fn adapt_response(
        &self,
        backend_response: BackendResponse,
    ) -> Result<UnifiedResponse, anyhow::Error> {
        let gemini_response: GeminiNetworkResponse =
            serde_json::from_slice(&backend_response.body)?;

        let mut content_blocks = Vec::new();
        let mut stop_reason = None;
        let mut usage = UnifiedUsage::default();

        // Handle tool compatibility mode parsing
        if backend_response.tool_compat_mode {
            if let Some(candidates) = gemini_response.candidates {
                if let Some(candidate) = candidates.into_iter().next() {
                    if let Some(text) = candidate
                        .content
                        .parts
                        .into_iter()
                        .find_map(|part| part.text)
                    {
                        let mut processed_text = text.clone();
                        let open_tags = processed_text.matches(TOOL_TAG_START).count();
                        let close_tags = processed_text.matches(TOOL_TAG_END).count();
                        let end_tag_len = TOOL_TAG_END.len();
                        if open_tags > 0 && open_tags != close_tags {
                            log::warn!(
                                "Mismatched tool tags in response: {} open, {} close",
                                open_tags,
                                close_tags
                            );
                        }

                        while let Some(start_pos) = processed_text.find(TOOL_TAG_START) {
                            // Add text before the tool call as a separate text block
                            if start_pos > 0 {
                                content_blocks.push(UnifiedContentBlock::Text {
                                    text: processed_text[..start_pos].to_string(),
                                });
                            }

                            // Search for the end tag *after* the start tag to handle multiple tools correctly
                            // and prevent panics from invalid slicing.
                            if let Some(relative_end_pos) =
                                processed_text[start_pos..].find(TOOL_TAG_END)
                            {
                                let end_pos = start_pos + relative_end_pos;
                                let tool_xml = &processed_text[start_pos..end_pos + end_tag_len];
                                match crate::ccproxy::helper::tool_use_xml::ToolUse::try_from(
                                    tool_xml,
                                ) {
                                    Ok(parsed_tool) => {
                                        log::debug!(
                                            "tool_use parse result: name: {}, param: {:?}",
                                            &parsed_tool.name,
                                            &parsed_tool.args
                                        );
                                        content_blocks.push(parsed_tool.into());
                                    }
                                    Err(e) => {
                                        let tool_xml =
                                            &processed_text[start_pos..end_pos + end_tag_len];

                                        log::warn!(
                                            "parse tool xml failed, error: {}, xml: {}",
                                            e.to_string(),
                                            tool_xml
                                        );
                                        // If parsing fails, treat the text as a regular text block
                                        content_blocks.push(UnifiedContentBlock::Text {
                                            text: tool_xml.to_string(),
                                        });

                                        // Send the corrective reminder.
                                        content_blocks.push(UnifiedContentBlock::Text {
                                            text: TOOL_PARSE_ERROR_REMINDER.to_string(),
                                        });
                                    }
                                }
                                // Remove the parsed tool XML from the text
                                processed_text =
                                    processed_text[(end_pos + end_tag_len)..].to_string();
                            } else {
                                // Incomplete tool call, treat remaining as text
                                if !processed_text.is_empty() {
                                    content_blocks.push(UnifiedContentBlock::Text {
                                        text: processed_text.clone(),
                                    });
                                }
                                break;
                            }
                        }
                        // Add any remaining text after the last tool call
                        if !processed_text.is_empty() {
                            content_blocks.push(UnifiedContentBlock::Text {
                                text: processed_text,
                            });
                        }
                    }
                    stop_reason = candidate.finish_reason.map(|r| r.to_string());
                }
            }
        } else {
            // Original parsing logic for non-tool compatibility mode
            if let Some(candidates) = gemini_response.candidates {
                if let Some(candidate) = candidates.into_iter().next() {
                    for part in candidate.content.parts {
                        if matches!(part.thought, Some(true)) {
                            if let Some(text) = part.text {
                                content_blocks
                                    .push(UnifiedContentBlock::Thinking { thinking: text });
                            }
                        } else if let Some(text) = part.text {
                            content_blocks.push(UnifiedContentBlock::Text { text });
                        } else if let Some(function_call) = part.function_call {
                            content_blocks.push(UnifiedContentBlock::ToolUse {
                                id: get_tool_id(), // Gemini doesn't provide tool_use_id in non-streaming
                                name: function_call.name,
                                input: function_call.args,
                            });
                        }
                    }
                    stop_reason = candidate.finish_reason.map(|r| r.to_string());
                }
            }
        }

        if let Some(usage_meta) = gemini_response.usage_metadata {
            usage.input_tokens = usage_meta.prompt_token_count;
            usage.output_tokens = usage_meta.candidates_token_count.unwrap_or(0);
            usage.tool_use_prompt_tokens = usage_meta.tool_use_prompt_token_count;
            usage.thoughts_tokens = usage_meta.thoughts_token_count;
            usage.cached_content_tokens = usage_meta.cached_content_token_count;
        }

        Ok(UnifiedResponse {
            id: uuid::Uuid::new_v4().to_string(), // Generate a new ID as Gemini doesn't provide one
            model: "gemini".to_string(),          // Model name might need to be passed through
            content: content_blocks,
            stop_reason,
            usage,
        })
    }

    async fn adapt_stream_chunk(
        &self,
        chunk: bytes::Bytes,
        sse_status: Arc<RwLock<SseStatus>>,
    ) -> Result<Vec<UnifiedStreamChunk>, anyhow::Error> {
        let chunk_str = String::from_utf8_lossy(&chunk);
        let mut unified_chunks = Vec::new();

        // Check tool compatibility mode first
        let tool_compat_mode = {
            if let Ok(status) = sse_status.read() {
                status.tool_compat_mode
            } else {
                false
            }
        };

        if tool_compat_mode {
            // Process tool compatibility mode
            return self.process_tool_compat_chunk(chunk, sse_status).await;
        }

        // Original non-tool-compatibility mode processing
        for line in chunk_str.lines() {
            if line.starts_with("data:") {
                let data_str = line["data:".len()..].trim();
                let gemini_response: GeminiNetworkResponse = serde_json::from_str(data_str)
                    .map_err(|e| {
                        log::error!(
                            "Gemini delta deserialize failed, delta: {}, error:{}",
                            &data_str,
                            e.to_string()
                        );
                        e
                    })?;

                if let Ok(mut status) = sse_status.write() {
                    if !status.message_start {
                        status.message_start = true;
                        // Gemini does not provide a message ID in the stream, so we use the one from the initial status.
                        unified_chunks.push(UnifiedStreamChunk::MessageStart {
                            id: status.message_id.clone(),
                            model: status.model_id.clone(),
                            usage: UnifiedUsage {
                                input_tokens: 0, // Gemini stream doesn't provide input tokens in the first chunk
                                output_tokens: 0,
                                ..Default::default()
                            },
                        });
                    }
                }

                if let Some(candidates) = gemini_response.candidates {
                    for candidate in candidates {
                        for part in candidate.content.parts {
                            if matches!(part.thought, Some(true)) {
                                if let Some(text) = part.text.clone() {
                                    if !text.is_empty() {
                                        if let Ok(mut status) = sse_status.write() {
                                            status.estimated_output_tokens +=
                                                estimate_tokens(&text);
                                        }
                                        unified_chunks
                                            .push(UnifiedStreamChunk::Thinking { delta: text });
                                    }
                                }
                            } else if let Some(text) = part.text.clone() {
                                if !text.is_empty() {
                                    if let Ok(mut status) = sse_status.write() {
                                        if status.text_delta_count == 0 {
                                            // Server may output: message -> tool -> message
                                            // So when outputting a message, if there is tool content, it means the tool output has ended
                                            if status.tool_delta_count > 0 {
                                                unified_chunks.push(
                                                    UnifiedStreamChunk::ToolUseEnd {
                                                        id: status.tool_id.clone(),
                                                    },
                                                );
                                                // reset tool delta count
                                                status.tool_delta_count = 0;
                                            }

                                            // start the new content block
                                            unified_chunks.push(
                                                UnifiedStreamChunk::ContentBlockStart {
                                                    index: status.message_index,
                                                    block: json!({
                                                         "type": "text",
                                                         "text": ""
                                                    }),
                                                },
                                            );
                                        }

                                        status.text_delta_count += 1;
                                        status.estimated_output_tokens += estimate_tokens(&text);
                                        update_message_block(&mut status, "text".to_string());
                                    }
                                    unified_chunks.push(UnifiedStreamChunk::Text { delta: text });
                                }
                            } else if let Some(function_call) = part.function_call.clone() {
                                // Gemini's functionCall in a stream for parallel calls is a COMPLETE, ATOMIC event.
                                // It is NOT a delta of a larger call.
                                // Therefore, for each one, we must emit a full Start -> Delta -> End sequence.

                                // Generate a unique ID for this specific tool call.
                                let tool_id = get_tool_id();
                                let mut message_index = 0;
                                // We need a unique ID for each parallel call. Let's use an index from our status.
                                if let Ok(mut status) = sse_status.write() {
                                    // Increment the total count of tools we've seen in this turn.
                                    status.tool_delta_count += 1;

                                    if status.tool_id != "" {
                                        // send tool stop
                                        unified_chunks.push(UnifiedStreamChunk::ContentBlockStop {
                                            index: status.message_index,
                                        })
                                    }
                                    status.tool_id = tool_id.clone();
                                    update_message_block(
                                        &mut status,
                                        format!("{tool_id}").to_string(),
                                    );
                                    message_index = status.message_index;
                                    // Record tool_id to index mapping
                                    status
                                        .tool_id_to_index
                                        .insert(tool_id.clone(), message_index);
                                } else {
                                    // Handle error, maybe continue to next line
                                    log::warn!(
                                        "failed to get status write lock, block index: {}",
                                        message_index
                                    );
                                    continue;
                                }

                                let tool_name = function_call.name.clone();

                                // The `args` field contains the full JSON for the arguments.
                                // It might be a complex JSON object, not just a string.
                                let args_json_string = function_call.args.to_string();

                                // 1. Announce the start of this specific tool call.
                                // for claude only
                                unified_chunks.push(UnifiedStreamChunk::ContentBlockStart {
                                    index: message_index,
                                    block: json!({
                                        "type":"tool_use",
                                        "id": tool_id.clone(),
                                        "name": tool_name.clone(),
                                        "input":{}
                                    }),
                                });
                                // for openai and gemini
                                unified_chunks.push(UnifiedStreamChunk::ToolUseStart {
                                    tool_type: "tool_use".to_string(), // or whatever is appropriate
                                    id: tool_id.clone(),
                                    name: tool_name,
                                });

                                // 2. Send all its arguments in a single delta.
                                if let Ok(mut status) = sse_status.write() {
                                    status.estimated_output_tokens +=
                                        estimate_tokens(&args_json_string);
                                }
                                unified_chunks.push(UnifiedStreamChunk::ToolUseDelta {
                                    id: tool_id.clone(),
                                    delta: args_json_string,
                                });

                                // 3. Immediately announce the end of this specific tool call.
                                unified_chunks.push(UnifiedStreamChunk::ToolUseEnd { id: tool_id });
                            }
                        }

                        if let Some(finish_reason) = candidate.finish_reason {
                            let stop_reason = finish_reason.to_string();
                            let usage = gemini_response
                                .usage_metadata
                                .clone()
                                .map(|u| UnifiedUsage {
                                    input_tokens: u.prompt_token_count,
                                    output_tokens: u.candidates_token_count.unwrap_or(0),
                                    cache_creation_input_tokens: None,
                                    cache_read_input_tokens: None,
                                    tool_use_prompt_tokens: u.tool_use_prompt_token_count,
                                    thoughts_tokens: u.thoughts_token_count,
                                    cached_content_tokens: u.cached_content_token_count,
                                    total_duration: None,
                                    load_duration: None,
                                    prompt_eval_duration: None,
                                    eval_duration: None,
                                    prompt_cached_tokens: None,
                                })
                                .unwrap_or_default();
                            unified_chunks
                                .push(UnifiedStreamChunk::MessageStop { stop_reason, usage });
                        }
                    }
                }
            }
        }

        Ok(unified_chunks)
    }
}
