use async_trait::async_trait;
use quick_xml::de::from_str;
use reqwest::{Client, RequestBuilder};
use std::sync::{Arc, RwLock};

use crate::ccproxy::adapter::{
    backend::common::{generate_tool_prompt, ToolUse, TOOL_TAG_END, TOOL_TAG_START},
    unified::{
        SseStatus, UnifiedContentBlock, UnifiedRequest, UnifiedResponse, UnifiedRole,
        UnifiedStreamChunk, UnifiedUsage,
    },
};
use crate::ccproxy::types::ollama::{
    OllamaChatCompletionRequest, OllamaChatCompletionResponse, OllamaFunctionCall, OllamaMessage,
    OllamaOptions, OllamaStreamResponse, OllamaTool, OllamaToolCall,
};

use super::{BackendAdapter, BackendResponse};

pub struct OllamaBackendAdapter;

#[async_trait]
impl BackendAdapter for OllamaBackendAdapter {
    async fn adapt_request(
        &self,
        client: &Client,
        unified_request: &UnifiedRequest,
        _api_key: &str, // Ollama doesn't use API keys
        provider_full_url: &str,
        model: &str,
    ) -> Result<RequestBuilder, anyhow::Error> {
        let mut ollama_messages: Vec<OllamaMessage> = Vec::new();
        let mut unified_request = unified_request.clone();

        // --- Tool Compatibility Mode Handling ---
        if unified_request.tool_compat_mode {
            if let Some(tools) = &unified_request.tools {
                let tool_prompt = generate_tool_prompt(tools);
                if let Some(last_message) = unified_request
                    .messages
                    .iter_mut()
                    .rfind(|m| m.role == UnifiedRole::User)
                {
                    let new_text = if let Some(UnifiedContentBlock::Text { text }) =
                        last_message.content.get_mut(0)
                    {
                        format!("{}\n{}", tool_prompt, text)
                    } else {
                        tool_prompt
                    };

                    if !last_message.content.is_empty()
                        && matches!(last_message.content[0], UnifiedContentBlock::Text { .. })
                    {
                        last_message.content[0] = UnifiedContentBlock::Text { text: new_text };
                    } else {
                        last_message
                            .content
                            .insert(0, UnifiedContentBlock::Text { text: new_text });
                    }
                } else {
                    unified_request.messages.push(
                        crate::ccproxy::adapter::unified::UnifiedMessage {
                            role: UnifiedRole::User,
                            content: vec![UnifiedContentBlock::Text { text: tool_prompt }],
                            reasoning_content: None,
                        },
                    );
                }
                unified_request.tools = None;
            }
        }

        let mut current_content = String::new();
        let mut current_images: Vec<String> = Vec::new();
        let mut current_tool_calls: Vec<OllamaToolCall> = Vec::new();
        let mut current_role: Option<UnifiedRole> = None;

        let flush_message = |messages: &mut Vec<OllamaMessage>,
                             role: &Option<UnifiedRole>,
                             content: &mut String,
                             images: &mut Vec<String>,
                             tool_calls: &mut Vec<OllamaToolCall>| {
            if content.is_empty() && images.is_empty() && tool_calls.is_empty() {
                return;
            }

            let ollama_role = match role {
                Some(UnifiedRole::User) => "user".to_string(),
                Some(UnifiedRole::Assistant) => "assistant".to_string(),
                Some(UnifiedRole::Tool) => "tool".to_string(),
                _ => return, // System role is handled separately
            };

            messages.push(OllamaMessage {
                role: ollama_role,
                content: std::mem::take(content),
                images: if images.is_empty() {
                    None
                } else {
                    Some(std::mem::take(images))
                },
                tool_calls: if tool_calls.is_empty() {
                    None
                } else {
                    Some(std::mem::take(tool_calls))
                },
                tool_name: None,
            });
        };

        for msg in &unified_request.messages {
            if current_role.is_some() && current_role.as_ref() != Some(&msg.role) {
                flush_message(
                    &mut ollama_messages,
                    &current_role,
                    &mut current_content,
                    &mut current_images,
                    &mut current_tool_calls,
                );
                current_role = None;
            }

            if current_role.is_none() {
                current_role = Some(msg.role.clone());
            }

            for block in &msg.content {
                match block {
                    UnifiedContentBlock::Text { text } => {
                        if !current_content.is_empty() {
                            current_content.push_str(
                                "
",
                            );
                        }
                        current_content.push_str(text);
                    }
                    UnifiedContentBlock::Image { data, .. } => {
                        current_images.push(data.clone());
                    }
                    UnifiedContentBlock::ToolUse { name, input, .. } => {
                        current_tool_calls.push(OllamaToolCall {
                            function: OllamaFunctionCall {
                                name: name.clone(),
                                arguments: input.clone(),
                            },
                        });
                    }
                    UnifiedContentBlock::ToolResult { content, .. } => {
                        // A tool result must start a new tool message
                        flush_message(
                            &mut ollama_messages,
                            &current_role,
                            &mut current_content,
                            &mut current_images,
                            &mut current_tool_calls,
                        );
                        current_role = Some(UnifiedRole::Tool);
                        current_content = content.clone();
                        // Flush immediately as a tool response is a self-contained message
                        flush_message(
                            &mut ollama_messages,
                            &current_role,
                            &mut current_content,
                            &mut current_images,
                            &mut current_tool_calls,
                        );
                        current_role = None;
                    }
                    _ => {} // Ignore other block types for now
                }
            }
        }
        flush_message(
            &mut ollama_messages,
            &current_role,
            &mut current_content,
            &mut current_images,
            &mut current_tool_calls,
        );

        if let Some(system_prompt) = &unified_request.system_prompt {
            if !system_prompt.trim().is_empty() {
                ollama_messages.insert(
                    0,
                    OllamaMessage {
                        role: "system".to_string(),
                        content: system_prompt.clone(),
                        ..Default::default()
                    },
                );
            }
        }

        let ollama_request = OllamaChatCompletionRequest {
            model: model.to_string(),
            messages: ollama_messages,
            stream: Some(unified_request.stream),
            format: unified_request
                .response_mime_type
                .as_ref()
                .and_then(|mime| {
                    if mime == "application/json" {
                        Some("json".to_string())
                    } else {
                        None
                    }
                }),
            options: Some(OllamaOptions {
                temperature: unified_request.temperature,
                num_predict: unified_request.max_tokens,
                top_p: unified_request.top_p,
                top_k: unified_request.top_k.map(|k| k as i32),
                stop: unified_request.stop_sequences.clone(),
                presence_penalty: unified_request.presence_penalty,
                frequency_penalty: unified_request.frequency_penalty,
                seed: unified_request.seed,
                ..Default::default()
            }),
            keep_alive: Some("5m".to_string()),
            tools: unified_request.tools.as_ref().map(|tools| {
                tools
                    .iter()
                    .map(|tool| OllamaTool {
                        r#type: "function".to_string(),
                        function: crate::ccproxy::types::ollama::OllamaFunctionDefinition {
                            name: tool.name.clone(),
                            description: tool.description.clone().unwrap_or_default(),
                            parameters: tool.input_schema.clone(),
                        },
                    })
                    .collect()
            }),
        };

        let mut request_builder = client.post(provider_full_url);
        request_builder = request_builder.header("Content-Type", "application/json");
        request_builder = request_builder.json(&ollama_request);

        #[cfg(debug_assertions)]
        {
            match serde_json::to_string_pretty(&ollama_request) {
                Ok(request_json) => {
                    log::debug!("Ollama request: {}", request_json);
                }
                Err(e) => {
                    log::error!("Failed to serialize Ollama request: {}", e);
                    if let Some(tools) = &ollama_request.tools {
                        for (i, tool) in tools.iter().enumerate() {
                            if let Err(tool_err) = serde_json::to_string(&tool) {
                                log::error!("Failed to serialize tool {}: {}", i, tool_err);
                                log::error!(
                                    "Tool details - name: {}, type: {}",
                                    tool.function.name,
                                    tool.r#type
                                );
                            }
                        }
                    }
                    return Err(anyhow::anyhow!("Failed to serialize Ollama request: {}", e));
                }
            }
        }

        Ok(request_builder)
    }

    async fn adapt_response(
        &self,
        backend_response: BackendResponse,
    ) -> Result<UnifiedResponse, anyhow::Error> {
        let ollama_response: OllamaChatCompletionResponse =
            serde_json::from_slice(&backend_response.body)?;

        let mut content_blocks = Vec::new();

        if backend_response.tool_compat_mode {
            let mut processed_text = ollama_response.message.content.clone();
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
                if start_pos > 0 {
                    content_blocks.push(UnifiedContentBlock::Text {
                        text: processed_text[..start_pos].to_string(),
                    });
                }

                if let Some(relative_end_pos) = processed_text[start_pos..].find(TOOL_TAG_END) {
                    let end_pos = start_pos + relative_end_pos;
                    let tool_xml = &processed_text[start_pos..end_pos + end_tag_len];

                    if let Ok(parsed_tool) = from_str::<ToolUse>(tool_xml) {
                        let mut arguments = serde_json::Map::new();
                        for param in parsed_tool.params.param {
                            arguments.insert(
                                param.name.clone(),
                                serde_json::Value::String(param.get_value()),
                            );
                        }
                        content_blocks.push(UnifiedContentBlock::ToolUse {
                            id: format!("tool_{}", uuid::Uuid::new_v4()),
                            name: parsed_tool.name,
                            input: serde_json::Value::Object(arguments),
                        });
                    } else {
                        log::warn!("Failed to parse tool XML: {}", tool_xml);
                        content_blocks.push(UnifiedContentBlock::Text {
                            text: tool_xml.to_string(),
                        });
                    }
                    processed_text = processed_text[end_pos + end_tag_len..].to_string();
                } else {
                    if !processed_text.is_empty() {
                        content_blocks.push(UnifiedContentBlock::Text {
                            text: processed_text.clone(),
                        });
                    }
                    break;
                }
            }

            if !processed_text.is_empty() {
                content_blocks.push(UnifiedContentBlock::Text {
                    text: processed_text,
                });
            }
        } else {
            if !ollama_response.message.content.is_empty() {
                content_blocks.push(UnifiedContentBlock::Text {
                    text: ollama_response.message.content,
                });
            }
            if let Some(tool_calls) = ollama_response.message.tool_calls {
                for tc in tool_calls {
                    content_blocks.push(UnifiedContentBlock::ToolUse {
                        id: format!("tool_{}", uuid::Uuid::new_v4()),
                        name: tc.function.name,
                        input: tc.function.arguments,
                    });
                }
            }
        }

        Ok(UnifiedResponse {
            id: format!("chatcmpl-{}", uuid::Uuid::new_v4()),
            model: ollama_response.model,
            content: content_blocks,
            stop_reason: Some("stop".to_string()),
            usage: UnifiedUsage {
                input_tokens: ollama_response.prompt_eval_count.unwrap_or(0) as u64,
                output_tokens: ollama_response.eval_count.unwrap_or(0) as u64,
                ..Default::default()
            },
        })
    }

    async fn adapt_stream_chunk(
        &self,
        chunk: bytes::Bytes,
        sse_status: Arc<RwLock<SseStatus>>,
    ) -> Result<Vec<UnifiedStreamChunk>, anyhow::Error> {
        let chunk_str = String::from_utf8_lossy(&chunk);
        let mut unified_chunks = Vec::new();

        for line in chunk_str.lines() {
            if line.trim().is_empty() {
                continue;
            }

            let ollama_chunk: OllamaStreamResponse = match serde_json::from_str(line) {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("Failed to parse Ollama stream chunk: {}, line: {}", e, line);
                    continue;
                }
            };

            if ollama_chunk.done {
                // Flush any remaining buffer content when the stream ends
                if let Ok(mut status) = sse_status.write() {
                    if status.tool_compat_mode {
                        self.flush_tool_compat_buffer(&mut status, &mut unified_chunks);
                        if !status.tool_compat_buffer.is_empty() {
                            unified_chunks.push(UnifiedStreamChunk::Text {
                                delta: status.tool_compat_buffer.clone(),
                            });
                            status.tool_compat_buffer.clear();
                        }
                    } else if status.tool_name.is_some() {
                        // If we were in a tool call, send the end block
                        unified_chunks.push(UnifiedStreamChunk::ToolUseDelta {
                            id: status.tool_id.clone(),
                            delta: status.tool_arguments.clone().unwrap_or_default(),
                        });
                        unified_chunks.push(UnifiedStreamChunk::ToolUseEnd {
                            id: status.tool_id.clone(),
                        });
                        status.tool_name = None;
                        status.tool_arguments = None;
                        status.tool_id = String::new();
                    }
                }

                unified_chunks.push(UnifiedStreamChunk::MessageStop {
                    stop_reason: "stop".to_string(),
                    usage: UnifiedUsage {
                        input_tokens: ollama_chunk.prompt_eval_count.unwrap_or(0) as u64,
                        output_tokens: ollama_chunk.eval_count.unwrap_or(0) as u64,
                        ..Default::default()
                    },
                });
            } else {
                let tool_compat_mode = {
                    sse_status
                        .read()
                        .map(|s| s.tool_compat_mode)
                        .unwrap_or(false)
                };

                if tool_compat_mode {
                    self.process_tool_compat_content(
                        &ollama_chunk.message.content,
                        &sse_status,
                        &mut unified_chunks,
                    );
                } else {
                    // Normal mode processing
                    if !ollama_chunk.message.content.is_empty() {
                        unified_chunks.push(UnifiedStreamChunk::Text {
                            delta: ollama_chunk.message.content,
                        });
                    }

                    if let Some(tool_calls) = ollama_chunk.message.tool_calls {
                        for tc in tool_calls {
                            let name = tc.function.name;
                            if let Ok(mut status) = sse_status.write() {
                                if status.tool_name.is_none() {
                                    status.tool_id = format!("tool_{}", uuid::Uuid::new_v4());
                                    status.tool_name = Some(name.clone());

                                    unified_chunks.push(UnifiedStreamChunk::ToolUseStart {
                                        tool_type: "function".to_string(),
                                        id: status.tool_id.clone(),
                                        name: name.clone(),
                                    });
                                }
                            }

                            let args = tc.function.arguments;
                            if let Ok(mut status) = sse_status.write() {
                                if status.tool_arguments.is_none() {
                                    status.tool_arguments = Some(String::new());
                                }
                                let current_args = status.tool_arguments.as_mut().unwrap();
                                current_args.push_str(&args.to_string());
                            }
                        }
                    }
                }
            }
        }

        Ok(unified_chunks)
    }
}

impl OllamaBackendAdapter {
    /// Process content in tool compatibility mode
    fn process_tool_compat_content(
        &self,
        content: &str,
        sse_status: &Arc<RwLock<SseStatus>>,
        unified_chunks: &mut Vec<UnifiedStreamChunk>,
    ) {
        if let Ok(mut status) = sse_status.write() {
            // Check buffer size before adding new content
            if status.tool_compat_fragment_buffer.len() > 1024 * 1024 {
                log::warn!("Fragment buffer size limit exceeded");
                let fragment = status.tool_compat_fragment_buffer.clone();
                status.tool_compat_buffer.push_str(&fragment);

                status.tool_compat_fragment_buffer.clear();
                status.tool_compat_fragment_count = 0;
                status.tool_compat_last_flush_time = std::time::Instant::now();
            }

            // Add content to fragment buffer
            status.tool_compat_fragment_buffer.push_str(content);
            status.tool_compat_fragment_count += 1;

            let now = std::time::Instant::now();
            let time_since_flush = now
                .duration_since(status.tool_compat_last_flush_time)
                .as_millis();

            // Force flush if conditions are met
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

        // Process tool calls in the buffer
        self.process_tool_calls_in_buffer(status, unified_chunks);

        // Handle remaining buffer content
        self.handle_remaining_buffer_content(status, unified_chunks);
    }

    /// Process tool calls found in the buffer
    fn process_tool_calls_in_buffer(
        &self,
        status: &mut std::sync::RwLockWriteGuard<SseStatus>,
        unified_chunks: &mut Vec<UnifiedStreamChunk>,
    ) {
        loop {
            if !status.in_tool_call_block {
                if let Some(start_pos) = status.tool_compat_buffer.find(TOOL_TAG_START) {
                    let text_before = &status.tool_compat_buffer[..start_pos];
                    if !text_before.is_empty() {
                        unified_chunks.push(UnifiedStreamChunk::Text {
                            delta: text_before.to_string(),
                        });
                    }
                    status.tool_compat_buffer = status.tool_compat_buffer[start_pos..].to_string();
                    status.in_tool_call_block = true;
                } else {
                    break;
                }
            }

            if status.in_tool_call_block {
                if let Some(end_pos) = status.tool_compat_buffer.find(TOOL_TAG_END) {
                    let tool_xml = &status.tool_compat_buffer[..end_pos + TOOL_TAG_END.len()];
                    self.parse_and_emit_tool_call(tool_xml, unified_chunks);
                    status.tool_compat_buffer =
                        status.tool_compat_buffer[end_pos + TOOL_TAG_END.len()..].to_string();
                    status.in_tool_call_block = false;
                } else {
                    break;
                }
            }
        }
    }

    /// Parse tool XML and emit tool call chunks
    fn parse_and_emit_tool_call(
        &self,
        tool_xml: &str,
        unified_chunks: &mut Vec<UnifiedStreamChunk>,
    ) {
        if let Ok(parsed_tool) = from_str::<ToolUse>(tool_xml) {
            let tool_id = format!("tool_{}", uuid::Uuid::new_v4());
            let mut arguments = serde_json::Map::new();
            for param in parsed_tool.params.param {
                arguments.insert(
                    param.name.clone(),
                    serde_json::Value::String(param.get_value()),
                );
            }
            let args_json = serde_json::to_string(&arguments).unwrap_or_default();

            unified_chunks.push(UnifiedStreamChunk::ToolUseStart {
                tool_type: "function".to_string(),
                id: tool_id.clone(),
                name: parsed_tool.name.clone(),
            });
            unified_chunks.push(UnifiedStreamChunk::ToolUseDelta {
                id: tool_id.clone(),
                delta: args_json,
            });
            unified_chunks.push(UnifiedStreamChunk::ToolUseEnd { id: tool_id });
        } else {
            log::warn!("Failed to parse tool XML, treating as text: {}", tool_xml);
            unified_chunks.push(UnifiedStreamChunk::Text {
                delta: tool_xml.to_string(),
            });
        }
    }

    /// Handle remaining content in buffer that's not part of tool calls
    fn handle_remaining_buffer_content(
        &self,
        status: &mut std::sync::RwLockWriteGuard<SseStatus>,
        unified_chunks: &mut Vec<UnifiedStreamChunk>,
    ) {
        if !status.in_tool_call_block && !status.tool_compat_buffer.is_empty() {
            let buffer = &status.tool_compat_buffer;
            let tool_start = TOOL_TAG_START;

            // Check for a partial start tag at the end of the buffer
            let mut partial_tag_len = 0;
            for i in (1..=std::cmp::min(buffer.len(), tool_start.len())).rev() {
                if buffer.ends_with(&tool_start[..i]) {
                    partial_tag_len = i;
                    break;
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
