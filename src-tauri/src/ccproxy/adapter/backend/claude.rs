use async_trait::async_trait;
use reqwest::{Client, RequestBuilder};
use serde_json::Value;
use std::sync::{Arc, RwLock};

use super::{BackendAdapter, BackendResponse};
use crate::ccproxy::claude::{
    ClaudeNativeContentBlock, ClaudeNativeMessage, ClaudeNativeRequest, ClaudeNativeResponse,
    ClaudeNativeTool, ClaudeStreamEvent, ClaudeToolChoice,
};
use crate::ccproxy::get_tool_id;
use crate::ccproxy::types::{TOOL_PARSE_ERROR_REMINDER, TOOL_TAG_END, TOOL_TAG_START};
use crate::ccproxy::{
    adapter::{
        backend::update_message_block,
        range_adapter::adapt_temperature,
        unified::{
            SseStatus, UnifiedContentBlock, UnifiedRequest, UnifiedResponse, UnifiedRole,
            UnifiedStreamChunk, UnifiedToolChoice, UnifiedUsage,
        },
    },
    types::ChatProtocol,
};

pub struct ClaudeBackendAdapter;

impl ClaudeBackendAdapter {
    /// Validate that tool call sequences are properly formed for Claude API
    fn validate_tool_call_sequence(
        &self,
        messages: &[crate::ccproxy::adapter::unified::UnifiedMessage],
    ) -> Result<(), anyhow::Error> {
        let mut pending_tool_calls: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for (i, msg) in messages.iter().enumerate() {
            match msg.role {
                UnifiedRole::Assistant => {
                    // Check for tool calls in assistant messages
                    for block in &msg.content {
                        if let UnifiedContentBlock::ToolUse { id, name, .. } = block {
                            log::debug!(
                                "Found tool call in message[{}]: id={}, name={}",
                                i,
                                id,
                                name
                            );
                            pending_tool_calls.insert(id.clone());
                        }
                    }
                }
                UnifiedRole::User | UnifiedRole::Tool => {
                    // Check for tool results in user/tool messages
                    for block in &msg.content {
                        if let UnifiedContentBlock::ToolResult { tool_use_id, .. } = block {
                            log::debug!(
                                "Found tool result in message[{}]: tool_use_id={}",
                                i,
                                tool_use_id
                            );
                            pending_tool_calls.remove(tool_use_id.as_str());
                        }
                    }
                }
                _ => {}
            }
        }

        // Check if there are any unresolved tool calls
        if !pending_tool_calls.is_empty() {
            let missing_ids: Vec<String> = pending_tool_calls.into_iter().collect();
            log::error!(
                "Tool call validation failed. Missing tool results for IDs: {:?}",
                missing_ids
            );

            // Log the full message sequence for debugging
            for (i, msg) in messages.iter().enumerate() {
                log::error!(
                    "Message[{}] role={:?}, content_blocks={}",
                    i,
                    msg.role,
                    msg.content.len()
                );
                for (j, block) in msg.content.iter().enumerate() {
                    match block {
                        UnifiedContentBlock::Text { text } => {
                            log::error!("  Block[{}]: Text ({}chars)", j, text.len());
                        }
                        UnifiedContentBlock::ToolUse { id, name, .. } => {
                            log::error!("  Block[{}]: ToolUse id={}, name={}", j, id, name);
                        }
                        UnifiedContentBlock::ToolResult { tool_use_id, .. } => {
                            log::error!("  Block[{}]: ToolResult tool_use_id={}", j, tool_use_id);
                        }
                        _ => {
                            log::error!("  Block[{}]: {:?}", j, std::mem::discriminant(block));
                        }
                    }
                }
            }

            return Err(anyhow::anyhow!(
                "Tool call sequence validation failed. Assistant messages with tool_calls must be followed by corresponding tool result messages. Missing responses for tool_call_ids: {:?}",
                missing_ids
            ));
        }

        Ok(())
    }

    fn process_tool_compat_content(
        &self,
        text: &str,
        sse_status: &Arc<RwLock<SseStatus>>,
        unified_chunks: &mut Vec<UnifiedStreamChunk>,
    ) {
        if let Ok(mut status) = sse_status.write() {
            status.tool_compat_fragment_buffer.push_str(text);
            status.tool_compat_fragment_count += 1;

            let now = std::time::Instant::now();
            let time_since_flush = now
                .duration_since(status.tool_compat_last_flush_time)
                .as_millis();

            let should_flush = status.tool_compat_fragment_count >= 25
                || time_since_flush >= 100
                || status.tool_compat_fragment_buffer.len() > 500
                || status.tool_compat_fragment_buffer.contains(TOOL_TAG_START)
                || status.tool_compat_fragment_buffer.contains(TOOL_TAG_END);

            if should_flush {
                self.flush_tool_compat_buffer(&mut status, unified_chunks);
            }
        }
    }

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

        crate::ccproxy::adapter::backend::common::process_tool_calls_in_buffer(
            status,
            unified_chunks,
        );

        self.handle_remaining_buffer_content(status, unified_chunks);
    }

    fn handle_remaining_buffer_content(
        &self,
        status: &mut std::sync::RwLockWriteGuard<SseStatus>,
        unified_chunks: &mut Vec<UnifiedStreamChunk>,
    ) {
        if !status.in_tool_call_block && !status.tool_compat_buffer.is_empty() {
            let buffer = &status.tool_compat_buffer;
            let tool_start = TOOL_TAG_START;
            let tool_end = TOOL_TAG_END;

            let mut partial_tag_len = 0;

            for i in (1..=std::cmp::min(buffer.len(), tool_start.len())).rev() {
                if buffer.ends_with(&tool_start[..i]) {
                    partial_tag_len = i;
                    break;
                }
            }

            if partial_tag_len == 0 {
                for i in (1..=std::cmp::min(buffer.len(), tool_end.len())).rev() {
                    if buffer.ends_with(&tool_end[..i]) {
                        partial_tag_len = i;
                        break;
                    }
                }
            }

            let text_to_flush_len = buffer.len() - partial_tag_len;
            if text_to_flush_len > 0 {
                let text_to_flush = &buffer[..text_to_flush_len];
                unified_chunks.push(UnifiedStreamChunk::Text {
                    delta: text_to_flush.to_string(),
                });
                status.tool_compat_buffer = buffer[text_to_flush_len..].to_string();
            }
        }
    }
}

#[async_trait]
impl BackendAdapter for ClaudeBackendAdapter {
    async fn adapt_request(
        &self,
        client: &Client,
        unified_request: &mut UnifiedRequest,
        api_key: &str,
        full_provider_url: &str,
        model: &str,
        log_proxy_to_file: bool,
    ) -> Result<RequestBuilder, anyhow::Error> {
        // Validate tool call sequence before processing
        self.validate_tool_call_sequence(&unified_request.messages)?;

        // Typically, Claude models have excellent tool call support，
        // and do not require enabling tool compatibility mode.
        // For logical consistency and to extend the capabilities of models compatible with the Claude protocol,
        // tool compatibility mode is also enabled here.
        unified_request.enhance_prompt();

        let mut claude_messages = Vec::new();

        if unified_request.tool_compat_mode {
            let mut processed_messages: Vec<ClaudeNativeMessage> = Vec::new();
            let mut tool_results_buffer: Vec<String> = Vec::new();

            for msg in &unified_request.messages {
                match msg.role {
                    UnifiedRole::Assistant => {
                        if !tool_results_buffer.is_empty() {
                            processed_messages.push(ClaudeNativeMessage {
                                role: "user".to_string(),
                                content: vec![ClaudeNativeContentBlock::Text {
                                    text: tool_results_buffer.join("\n"),
                                }],
                            });
                            tool_results_buffer.clear();
                        }

                        let mut content_parts: Vec<String> = Vec::new();
                        for block in &msg.content {
                            match block {
                                UnifiedContentBlock::Text { text } => {
                                    let trimmed_text = text.trim();
                                    if !trimmed_text.is_empty() {
                                        content_parts.push(trimmed_text.to_string());
                                    }
                                }
                                UnifiedContentBlock::ToolUse { id, name, input } => {
                                    let tool_use_xml =
                                        crate::ccproxy::helper::tool_use_xml::format_tool_use_xml(
                                            id, name, input,
                                        );
                                    content_parts.push(tool_use_xml);
                                }
                                _ => {} // Other content blocks are ignored for assistant messages in this mode
                            }
                        }
                        let final_content = content_parts.join("\n");
                        if !final_content.is_empty() {
                            processed_messages.push(ClaudeNativeMessage {
                                role: "assistant".to_string(),
                                content: vec![ClaudeNativeContentBlock::Text {
                                    text: final_content,
                                }],
                            });
                        }
                    }
                    UnifiedRole::Tool => {
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
                        if !tool_results_buffer.is_empty() {
                            processed_messages.push(ClaudeNativeMessage {
                                role: "user".to_string(),
                                content: vec![ClaudeNativeContentBlock::Text {
                                    text: tool_results_buffer.join("\n"),
                                }],
                            });
                            tool_results_buffer.clear();
                        }

                        let content_text = msg
                            .content
                            .iter()
                            .filter_map(|b| {
                                if let UnifiedContentBlock::Text { text } = b {
                                    Some(text.as_str())
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n\n");
                        if !content_text.is_empty() {
                            processed_messages.push(ClaudeNativeMessage {
                                role: "user".to_string(),
                                content: vec![ClaudeNativeContentBlock::Text {
                                    text: content_text,
                                }],
                            });
                        }
                    }
                    _ => {} // Other roles are ignored in tool compat mode
                }
            }

            if !tool_results_buffer.is_empty() {
                processed_messages.push(ClaudeNativeMessage {
                    role: "user".to_string(),
                    content: vec![ClaudeNativeContentBlock::Text {
                        text: tool_results_buffer.join("\n"),
                    }],
                });
            }
            claude_messages = processed_messages;
        } else {
            for msg in &unified_request.messages {
                let role = match msg.role {
                    UnifiedRole::User => "user",
                    UnifiedRole::Assistant => "assistant",
                    UnifiedRole::Tool => "user", // Claude tool results are sent by the user
                    UnifiedRole::System => continue, // System prompt is top-level in Claude
                };

                let mut content_blocks = Vec::new();
                for block in &msg.content {
                    match block {
                        UnifiedContentBlock::Text { text } => {
                            content_blocks
                                .push(ClaudeNativeContentBlock::Text { text: text.clone() });
                        }
                        UnifiedContentBlock::Image { media_type, data } => {
                            content_blocks.push(ClaudeNativeContentBlock::Image {
                                source: crate::ccproxy::types::claude::ClaudeImageSource {
                                    source_type: "base64".to_string(),
                                    media_type: media_type.clone(),
                                    data: data.clone(),
                                },
                            });
                        }
                        UnifiedContentBlock::ToolUse { id, name, input } => {
                            content_blocks.push(ClaudeNativeContentBlock::ToolUse {
                                id: id.clone(),
                                name: name.clone(),
                                input: input.clone(),
                            });
                        }
                        UnifiedContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            is_error,
                        } => {
                            content_blocks.push(ClaudeNativeContentBlock::ToolResult {
                                tool_use_id: tool_use_id.clone(),
                                content: content.clone(),
                                is_error: Some(*is_error),
                            });
                        }
                        UnifiedContentBlock::Thinking { thinking } => {
                            content_blocks.push(ClaudeNativeContentBlock::Thinking {
                                thinking: thinking.to_string(),
                            });
                        }
                    }
                }
                claude_messages.push(ClaudeNativeMessage {
                    role: role.to_string(),
                    content: content_blocks,
                });
            }
        }

        let claude_tools = unified_request.tools.as_ref().map(|tools| {
            tools
                .iter()
                .map(|tool| ClaudeNativeTool {
                    name: tool.name.clone(),
                    description: tool.description.clone(),
                    input_schema: tool.input_schema.clone(),
                })
                .collect()
        });

        let claude_tool_choice = unified_request.tool_choice.as_ref().map(|choice| {
            match choice {
                UnifiedToolChoice::None => ClaudeToolChoice::Auto, // Claude doesn't have a direct 'none'
                UnifiedToolChoice::Auto => ClaudeToolChoice::Auto,
                UnifiedToolChoice::Required => ClaudeToolChoice::Any,
                UnifiedToolChoice::Tool { name } => ClaudeToolChoice::Tool { name: name.clone() },
            }
        });

        let injection_pos = unified_request
            .prompt_injection_position
            .as_deref()
            .unwrap_or("system");
        let combined_prompt_text = unified_request
            .combined_prompt
            .as_deref()
            .unwrap_or_default();
        let mut final_system_prompt: Option<String> = None;

        if injection_pos == "user" && !combined_prompt_text.is_empty() {
            if let Some(last_user_msg) = claude_messages.iter_mut().rfind(|m| m.role == "user") {
                last_user_msg.content.insert(
                    0,
                    ClaudeNativeContentBlock::Text {
                        text: combined_prompt_text.to_string(),
                    },
                );
            }
            final_system_prompt = unified_request.system_prompt.clone();
        } else {
            let injection_mode = unified_request
                .prompt_injection
                .as_deref()
                .unwrap_or("enhance");
            let original_system_prompt =
                unified_request.system_prompt.as_deref().unwrap_or_default();

            let prompt = if injection_mode == "replace" {
                combined_prompt_text.to_string()
            } else {
                [original_system_prompt, combined_prompt_text]
                    .iter()
                    .filter(|s| !s.is_empty())
                    .map(|s| *s)
                    .collect::<Vec<&str>>()
                    .join("\n\n")
            };

            if !prompt.trim().is_empty() {
                final_system_prompt = Some(prompt);
            }
        }

        let claude_request = ClaudeNativeRequest {
            model: model.to_string(),
            messages: claude_messages,
            system: final_system_prompt,
            max_tokens: unified_request.max_tokens.unwrap_or(1024),
            stream: Some(unified_request.stream),
            temperature: unified_request.temperature.map(|t| {
                // Adapt temperature from source protocol to Claude range
                adapt_temperature(t, ChatProtocol::Claude)
            }),
            top_p: unified_request.top_p,
            top_k: unified_request.top_k,
            stop_sequences: unified_request.stop_sequences.clone(), // Add missing field
            tools: claude_tools,
            tool_choice: claude_tool_choice,
            metadata: unified_request.metadata.as_ref().map(|m| {
                crate::ccproxy::types::claude::ClaudeMetadata {
                    user_id: m.user_id.clone(),
                }
            }),
            thinking: unified_request.thinking.as_ref().and_then(|t| {
                if matches!(t.include_thoughts, Some(true)) {
                    Some(crate::ccproxy::types::claude::ClaudeThinking {
                        thinking_type: "enabled".to_string(),
                        budget_tokens: t.budget_tokens.unwrap_or(1024), // Use 1024 as a reasonable default
                    })
                } else {
                    None
                }
            }),
            cache_control: unified_request.cache_control.as_ref().map(|c| {
                crate::ccproxy::types::claude::ClaudeCacheControl {
                    cache_type: c.cache_type.clone(),
                    ttl: c.ttl.clone(),
                }
            }),
        };

        let mut request_builder = client.post(full_provider_url);
        request_builder = request_builder.header("Content-Type", "application/json");
        request_builder = request_builder.header("x-api-key", api_key);
        request_builder = request_builder.header("anthropic-version", "2023-06-01");
        if unified_request.tools.is_some() || unified_request.tool_choice.is_some() {
            request_builder = request_builder.header("anthropic-beta", "tools-2024-04-04");
        }
        request_builder = request_builder.json(&claude_request);

        if log_proxy_to_file {
            // Log the request to a file
            log::info!(target: "ccproxy_logger","Claude Request Body: \n{}\n----------------\n", serde_json::to_string_pretty(&claude_request).unwrap_or_default());
        }

        // #[cfg(debug_assertions)]
        // {
        //     match serde_json::to_string_pretty(&claude_request) {
        //         Ok(request_json) => {
        //             log::debug!("Claude request: {}", request_json);
        //         }
        //         Err(e) => {
        //             log::error!("Failed to serialize Claude request: {}", e);
        //             // Try to serialize individual parts to identify the issue
        //             if let Some(tools) = &claude_request.tools {
        //                 for (i, tool) in tools.iter().enumerate() {
        //                     if let Err(tool_err) = serde_json::to_string(&tool) {
        //                         log::error!("Failed to serialize tool {}: {}", i, tool_err);
        //                         log::error!(
        //                             "Tool details - name: {}, params: {}",
        //                             tool.name,
        //                             tool.input_schema
        //                         );
        //                     }
        //                 }
        //             }
        //             return Err(anyhow::anyhow!("Failed to serialize Claude request: {}", e));
        //         }
        //     }
        // }

        Ok(request_builder)
    }

    async fn adapt_response(
        &self,
        backend_response: BackendResponse,
    ) -> Result<UnifiedResponse, anyhow::Error> {
        let claude_response: ClaudeNativeResponse = serde_json::from_slice(&backend_response.body)?;

        let mut content_blocks = Vec::new();

        if backend_response.tool_compat_mode {
            // In tool-compat mode, the response content is expected to be a single text block
            // containing potential XML tool calls.
            if let Some(ClaudeNativeContentBlock::Text { text }) = claude_response.content.get(0) {
                let mut processed_text = text.clone();
                let tool_tag_start = TOOL_TAG_START;
                let tool_tag_end = TOOL_TAG_END;

                // This loop finds and parses all tool_use blocks from the text.
                while let Some(start_pos) = processed_text.find(tool_tag_start) {
                    // Add text before the tool call as a separate text block
                    if start_pos > 0 {
                        content_blocks.push(UnifiedContentBlock::Text {
                            text: processed_text[..start_pos].to_string(),
                        });
                    }

                    // Search for the end tag
                    if let Some(relative_end_pos) = processed_text[start_pos..].find(tool_tag_end) {
                        let end_pos = start_pos + relative_end_pos;
                        let end_tag_len = tool_tag_end.len();
                        let tool_xml = &processed_text[start_pos..end_pos + end_tag_len];

                        match crate::ccproxy::helper::tool_use_xml::ToolUse::try_from(tool_xml) {
                            Ok(parsed_tool) => {
                                content_blocks.push(parsed_tool.into());
                            }
                            _ => {
                                log::warn!("parse tool xml failed, xml: {}", tool_xml);
                                // If parsing fails, treat the text as a regular text block and add reminder
                                content_blocks.push(UnifiedContentBlock::Text {
                                    text: tool_xml.to_string(),
                                });
                                content_blocks.push(UnifiedContentBlock::Text {
                                    text: TOOL_PARSE_ERROR_REMINDER.to_string(),
                                });
                            }
                        }
                        // Remove the parsed tool XML from the text
                        processed_text = processed_text[end_pos + end_tag_len..].to_string();
                    } else {
                        // Incomplete tool call, treat remaining as text
                        if !processed_text.is_empty() {
                            content_blocks.push(UnifiedContentBlock::Text {
                                text: processed_text.clone(),
                            });
                        }
                        processed_text.clear(); // Clear to exit loop
                        break;
                    }
                }
                // Add any remaining text after the last tool call
                if !processed_text.is_empty() {
                    content_blocks.push(UnifiedContentBlock::Text {
                        text: processed_text,
                    });
                }
            } else {
                // Fallback for unexpected content in tool-compat mode
                for block in claude_response.content {
                    match block {
                        ClaudeNativeContentBlock::Text { text } => {
                            content_blocks.push(UnifiedContentBlock::Text { text })
                        }
                        ClaudeNativeContentBlock::ToolUse { id, name, input } => {
                            content_blocks.push(UnifiedContentBlock::ToolUse { id, name, input })
                        }
                        _ => {}
                    }
                }
            }
        } else {
            // This is the original logic for native tool calls
            for block in claude_response.content {
                match block {
                    ClaudeNativeContentBlock::Text { text } => {
                        content_blocks.push(UnifiedContentBlock::Text { text })
                    }
                    ClaudeNativeContentBlock::ToolUse { id, name, input } => {
                        content_blocks.push(UnifiedContentBlock::ToolUse { id, name, input })
                    }
                    ClaudeNativeContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } => content_blocks.push(UnifiedContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error: is_error.unwrap_or(false),
                    }),
                    ClaudeNativeContentBlock::Image { source } => {
                        content_blocks.push(UnifiedContentBlock::Image {
                            media_type: source.media_type,
                            data: source.data,
                        })
                    }
                    ClaudeNativeContentBlock::Thinking { thinking } => {
                        content_blocks.push(UnifiedContentBlock::Thinking { thinking })
                    }
                }
            }
        }

        let usage = claude_response
            .usage
            .map(|u| UnifiedUsage {
                input_tokens: u.input_tokens,
                output_tokens: u.output_tokens,
                cache_creation_input_tokens: u.cache_creation_input_tokens,
                cache_read_input_tokens: u.cache_read_input_tokens,
                ..Default::default()
            })
            .unwrap_or_default();

        Ok(UnifiedResponse {
            id: claude_response.id,
            model: claude_response.model.unwrap_or_default(),
            content: content_blocks,
            stop_reason: claude_response.stop_reason,
            usage,
        })
    }

    /// Adapt a stream chunk into a unified stream chunk.
    /// @link https://docs.anthropic.com/en/docs/build-with-claude/streaming
    async fn adapt_stream_chunk(
        &self,
        chunk: bytes::Bytes,
        sse_status: Arc<RwLock<SseStatus>>,
    ) -> Result<Vec<UnifiedStreamChunk>, anyhow::Error> {
        let tool_compat_mode = sse_status.read().map_or(false, |s| s.tool_compat_mode);
        let mut unified_chunks = Vec::new();
        let chunk_str = String::from_utf8_lossy(&chunk);

        if tool_compat_mode {
            for line in chunk_str.lines() {
                if line.starts_with("event:") {
                    let data_line = chunk_str.lines().find(|l| l.starts_with("data:"));
                    if let Some(data_line) = data_line {
                        let data_str = data_line["data:".len()..].trim();
                        if data_str.is_empty() {
                            continue;
                        }
                        if let Ok(claude_event) =
                            serde_json::from_str::<ClaudeStreamEvent>(data_str)
                        {
                            match claude_event.event_type.as_str() {
                                "message_start" => {
                                    if let Some(msg) = claude_event.message {
                                        if let Ok(mut status) = sse_status.write() {
                                            status.message_start = true;
                                            status.message_id = msg.id.clone();
                                            status.model_id = msg.model.clone();
                                        }
                                        unified_chunks.push(UnifiedStreamChunk::MessageStart {
                                            id: msg.id,
                                            model: msg.model,
                                            usage: UnifiedUsage {
                                                input_tokens: msg.usage.input_tokens.unwrap_or(0),
                                                output_tokens: 0,
                                                ..Default::default()
                                            },
                                        });
                                    }
                                }
                                "content_block_delta" => {
                                    if let Some(delta) = claude_event.delta {
                                        if let Some("text_delta") = delta.delta_type.as_deref() {
                                            if let Some(text) = delta.text {
                                                self.process_tool_compat_content(
                                                    &text,
                                                    &sse_status,
                                                    &mut unified_chunks,
                                                );
                                            }
                                        }
                                    }
                                }
                                "message_delta" => {
                                    if let Some(delta) = claude_event.delta {
                                        if let Some(stop_reason) = delta.stop_reason {
                                            if let Ok(mut status) = sse_status.write() {
                                                // On stream finish, try to auto-complete any dangling tool tag.
                                                crate::ccproxy::adapter::backend::common::auto_complete_and_process_tool_tag(&mut status, &mut unified_chunks);

                                                if !status.tool_compat_buffer.is_empty()
                                                    || !status
                                                        .tool_compat_fragment_buffer
                                                        .is_empty()
                                                {
                                                    self.flush_tool_compat_buffer(
                                                        &mut status,
                                                        &mut unified_chunks,
                                                    );

                                                    if !status.tool_compat_buffer.is_empty()
                                                        || !status
                                                            .tool_compat_fragment_buffer
                                                            .is_empty()
                                                    {
                                                        unified_chunks.push(
                                                            UnifiedStreamChunk::Text {
                                                                delta: format!(
                                                                "{}{}",
                                                                status.tool_compat_buffer,
                                                                status.tool_compat_fragment_buffer
                                                            ),
                                                            },
                                                        );

                                                        status.tool_compat_buffer.clear();
                                                        status.tool_compat_fragment_buffer.clear();
                                                        status.in_tool_call_block = false;
                                                    }
                                                }
                                            }
                                            let usage = claude_event
                                                .usage
                                                .map(|u| UnifiedUsage {
                                                    input_tokens: u.input_tokens.unwrap_or(0),
                                                    output_tokens: u.output_tokens.unwrap_or(0),
                                                    ..Default::default()
                                                })
                                                .unwrap_or_default();
                                            unified_chunks.push(UnifiedStreamChunk::MessageStop {
                                                stop_reason,
                                                usage,
                                            });
                                        }
                                    }
                                }
                                "message_stop" => {
                                    if let Ok(mut status) = sse_status.write() {
                                        // On stream finish, try to auto-complete any dangling tool tag.
                                        crate::ccproxy::adapter::backend::common::auto_complete_and_process_tool_tag(&mut status, &mut unified_chunks);

                                        if !status.tool_compat_buffer.is_empty()
                                            || !status.tool_compat_fragment_buffer.is_empty()
                                        {
                                            self.flush_tool_compat_buffer(
                                                &mut status,
                                                &mut unified_chunks,
                                            );

                                            if !status.tool_compat_buffer.is_empty()
                                                || !status.tool_compat_fragment_buffer.is_empty()
                                            {
                                                unified_chunks.push(UnifiedStreamChunk::Text {
                                                    delta: format!(
                                                        "{}{}",
                                                        status.tool_compat_buffer,
                                                        status.tool_compat_fragment_buffer
                                                    ),
                                                });

                                                status.tool_compat_buffer.clear();
                                                status.tool_compat_fragment_buffer.clear();
                                                status.in_tool_call_block = false;
                                            }
                                        }
                                        status.message_start = false;
                                    }
                                }
                                "ping" => { /* Ignore */ }
                                "error" => {
                                    if let Some(err) = claude_event.error {
                                        unified_chunks.push(UnifiedStreamChunk::Error {
                                            message: err.message,
                                        });
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        } else {
            let model_id = sse_status.read().map(|x| x.model_id.clone());

            for line in chunk_str.lines() {
                if line.starts_with("event:") {
                    // let event_type = line["event:".len()..].trim();
                    // Assuming data: always follows event:
                    let data_line = chunk_str.lines().find(|l| l.starts_with("data:"));
                    if let Some(data_line) = data_line {
                        let data_str = data_line["data:".len()..].trim();
                        let claude_event: ClaudeStreamEvent = serde_json::from_str(data_str)?;

                        match claude_event.event_type.as_str() {
                            "message_start" => {
                                if let Some(msg) = claude_event.message {
                                    let model = model_id
                                        .as_deref()
                                        .unwrap_or(msg.model.as_str())
                                        .to_string();
                                    if let Ok(mut status) = sse_status.write() {
                                        status.message_start = true;
                                        status.message_id = msg.id.clone();
                                        status.model_id = model.clone();
                                    }
                                    unified_chunks.push(UnifiedStreamChunk::MessageStart {
                                        id: msg.id,
                                        model: model,
                                        usage: UnifiedUsage {
                                            input_tokens: msg.usage.input_tokens.unwrap_or(0),
                                            output_tokens: 0,
                                            ..Default::default()
                                        },
                                    });
                                }
                            }
                            "content_block_start" => {
                                unified_chunks.push(UnifiedStreamChunk::ContentBlockStart {
                                    index: claude_event.index.unwrap_or_default(),
                                    block: claude_event
                                        .content_block
                                        .as_ref()
                                        .map(|block| {
                                            serde_json::to_value(block.clone())
                                                .unwrap_or(Value::Null)
                                        })
                                        .unwrap_or(Value::Null),
                                });
                                if let Ok(mut status) = sse_status.write() {
                                    status.message_index = claude_event.index.unwrap_or_default();
                                }

                                if let Some(block) = claude_event.content_block {
                                    match block.block_type.as_str() {
                                        "text" => {
                                            if let Ok(mut status) = sse_status.write() {
                                                status.text_delta_count += 1;
                                                update_message_block(
                                                    &mut status,
                                                    "text".to_string(),
                                                );
                                            }
                                        }
                                        "thinking" => {
                                            if let Ok(mut status) = sse_status.write() {
                                                status.thinking_delta_count += 1;
                                                update_message_block(
                                                    &mut status,
                                                    "thinking".to_string(),
                                                );
                                            }
                                        }
                                        "tool_use" => {
                                            let tool_id = block.id.clone().unwrap_or(get_tool_id());
                                            if let Ok(mut status) = sse_status.write() {
                                                status.tool_id = tool_id.clone();
                                                update_message_block(&mut status, tool_id.clone());
                                            }
                                            unified_chunks.push(UnifiedStreamChunk::ToolUseStart {
                                                tool_type: "tool_use".to_string(),
                                                id: tool_id,
                                                name: block.name.unwrap_or_default(),
                                            });
                                        }
                                        // "server_tool_use" => {
                                        //     if let Ok(mut status) = sse_status.write() {
                                        //         status.tool_id = block.id.clone().unwrap_or_default();
                                        //         update_message_block(
                                        //             status,
                                        //             "server_tool_use".to_string(),
                                        //         );
                                        //     }
                                        //     unified_chunks.push(UnifiedStreamChunk::ToolUseStart {
                                        //         tool_type: "server_tool_use".to_string(),
                                        //         id: block.id.unwrap_or_default(),
                                        //         name: block.name.unwrap_or_default(),
                                        //     });
                                        // }
                                        _ => {}
                                    }
                                }
                            }
                            "content_block_delta" => {
                                if let Some(delta) = claude_event.delta {
                                    match delta.delta_type.as_deref() {
                                        Some("text_delta") => {
                                            if let Ok(mut status) = sse_status.write() {
                                                status.text_delta_count += 1;
                                                update_message_block(
                                                    &mut status,
                                                    "text".to_string(),
                                                );
                                            }
                                            if let Some(text) = delta.text {
                                                unified_chunks
                                                    .push(UnifiedStreamChunk::Text { delta: text });
                                            }
                                        }
                                        Some("thinking_delta") => {
                                            if let Some(text) = delta.text {
                                                if let Ok(mut status) = sse_status.write() {
                                                    status.thinking_delta_count += 1;
                                                    update_message_block(
                                                        &mut status,
                                                        "thinking".to_string(),
                                                    );
                                                }
                                                unified_chunks.push(UnifiedStreamChunk::Thinking {
                                                    delta: text,
                                                });
                                            }
                                        }
                                        Some("input_json_delta") => {
                                            let tool_id = if let Ok(mut status) = sse_status.write()
                                            {
                                                let id = status.tool_id.clone();
                                                update_message_block(
                                                    &mut status,
                                                    "tool_use".to_string(),
                                                );
                                                id
                                            } else {
                                                String::new()
                                            };
                                            if let Some(partial_json) = delta.partial_json {
                                                unified_chunks.push(
                                                    UnifiedStreamChunk::ToolUseDelta {
                                                        id: tool_id,
                                                        delta: partial_json,
                                                    },
                                                );
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            "content_block_stop" => {
                                unified_chunks.push(UnifiedStreamChunk::ContentBlockStop {
                                    index: claude_event.index.unwrap_or_default(),
                                });

                                // Check if we need to end a tool use based on current status
                                if let Ok(mut status) = sse_status.write() {
                                    if !status.tool_id.is_empty()
                                        && status.current_content_block.contains("tool")
                                    {
                                        unified_chunks.push(UnifiedStreamChunk::ToolUseEnd {
                                            id: status.tool_id.clone(),
                                        });
                                        status.tool_id = "".to_string();
                                    }
                                }
                            }
                            "message_delta" => {
                                if let Some(delta) = claude_event.delta {
                                    if let Some(stop_reason) = delta.stop_reason {
                                        let usage = claude_event
                                            .usage
                                            .map(|u| UnifiedUsage {
                                                input_tokens: u.input_tokens.unwrap_or(0),
                                                output_tokens: u.output_tokens.unwrap_or(0),
                                                ..Default::default()
                                            })
                                            .unwrap_or_default();
                                        unified_chunks.push(UnifiedStreamChunk::MessageStop {
                                            stop_reason,
                                            usage,
                                        });
                                    }
                                }
                            }
                            "message_stop" => {
                                if let Ok(mut status) = sse_status.write() {
                                    status.message_start = false;
                                    status.tool_id = "".to_string();
                                }
                            }
                            "ping" => { /* Ignore */ }
                            "error" => {
                                if let Some(err) = claude_event.error {
                                    unified_chunks.push(UnifiedStreamChunk::Error {
                                        message: err.message,
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        Ok(unified_chunks)
    }
}
