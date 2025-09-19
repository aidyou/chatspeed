use async_trait::async_trait;
use reqwest::{Client, RequestBuilder};
use serde_json::json;
use std::sync::{Arc, RwLock};

use crate::ccproxy::adapter::range_adapter::adapt_temperature;
use crate::ccproxy::get_tool_id;
use crate::ccproxy::types::ollama::{
    OllamaChatCompletionRequest, OllamaChatCompletionResponse, OllamaFunctionCall, OllamaMessage,
    OllamaOptions, OllamaStreamResponse, OllamaTool, OllamaToolCall,
};
use crate::ccproxy::types::{TOOL_PARSE_ERROR_REMINDER, TOOL_TAG_END, TOOL_TAG_START};
use crate::ccproxy::{
    adapter::{
        backend::update_message_block,
        unified::{
            SseStatus, UnifiedContentBlock, UnifiedRequest, UnifiedResponse, UnifiedRole,
            UnifiedStreamChunk, UnifiedUsage,
        },
    },
    types::ChatProtocol,
};

use super::{BackendAdapter, BackendResponse};
use crate::ccproxy::adapter::backend::common;

pub struct OllamaBackendAdapter;

#[async_trait]
impl BackendAdapter for OllamaBackendAdapter {
    async fn adapt_request(
        &self,
        client: &Client,
        unified_request: &mut UnifiedRequest,
        _api_key: &str, // Ollama doesn't use API keys
        provider_full_url: &str,
        model: &str,
        log_proxy_to_file: bool,
    ) -> Result<RequestBuilder, anyhow::Error> {
        // --- Tool Compatibility Mode Handling ---
        // If tool_compat_mode is enabled, we inject a system prompt with tool definitions
        // into the system message. This is a specific adaptation for models that
        // don't support native tool calling APIs but can follow instructions.
        unified_request.enhance_prompt();

        // --- Message Processing ---
        let mut ollama_messages: Vec<OllamaMessage> = Vec::new();
        if unified_request.tool_compat_mode {
            // Special handling for tool compatibility mode to mimic a more natural
            // conversation flow for models that don't natively support tool calls.
            let mut processed_messages: Vec<OllamaMessage> = Vec::new();
            let mut tool_results_buffer: Vec<String> = Vec::new();

            for msg in &unified_request.messages {
                match msg.role {
                    UnifiedRole::Assistant => {
                        // If there are pending tool results, flush them as a single user message first.
                        if !tool_results_buffer.is_empty() {
                            processed_messages.push(OllamaMessage {
                                role: "user".to_string(),
                                content: tool_results_buffer.join("\n"),
                                images: None,
                                tool_calls: None,
                                thinking: None,
                                tool_name: None,
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
                                _ => {} // Ignore Thinking blocks
                            }
                        }

                        // Join all parts with a consistent separator
                        let final_content = content_parts.join("\n\n");

                        if !final_content.is_empty() {
                            processed_messages.push(OllamaMessage {
                                role: "assistant".to_string(),
                                content: final_content,
                                images: None,
                                tool_calls: None,
                                thinking: None,
                                tool_name: None,
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
                            processed_messages.push(OllamaMessage {
                                role: "user".to_string(),
                                content: tool_results_buffer.join("\n"),
                                images: None,
                                tool_calls: None,
                                thinking: None,
                                tool_name: None,
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
                            processed_messages.push(OllamaMessage {
                                role: "user".to_string(),
                                content: content_text,
                                images: None,
                                tool_calls: None,
                                thinking: None,
                                tool_name: None,
                            });
                        }
                    }
                    _ => {} // Ignore System role in this context, it's handled separately.
                }
            }

            // Flush any remaining tool results at the end of the message list.
            if !tool_results_buffer.is_empty() {
                processed_messages.push(OllamaMessage {
                    role: "user".to_string(),
                    content: tool_results_buffer.join("\n"),
                    images: None,
                    tool_calls: None,
                    thinking: None,
                    tool_name: None,
                });
            }
            ollama_messages = processed_messages;
        } else {
            // This logic correctly converts the UnifiedRequest message structure into the
            // Ollama-compatible format for non-tool-compat mode.
            for msg in &unified_request.messages {
                let role_str = match msg.role {
                    UnifiedRole::System => "system",
                    UnifiedRole::User => "user",
                    UnifiedRole::Assistant => "assistant",
                    UnifiedRole::Tool => "tool",
                };

                let mut content_parts = Vec::new();
                let mut image_parts: Vec<String> = Vec::new();
                let mut tool_calls = Vec::new();

                for block in &msg.content {
                    match block {
                        UnifiedContentBlock::Text { text } => {
                            content_parts.push(text.clone());
                        }
                        UnifiedContentBlock::Image { data, .. } => {
                            image_parts.push(data.clone());
                        }
                        UnifiedContentBlock::ToolUse { name, input, .. } => {
                            tool_calls.push(OllamaToolCall {
                                function: OllamaFunctionCall {
                                    name: name.clone(),
                                    arguments: input.clone(),
                                },
                            });
                        }
                        UnifiedContentBlock::ToolResult { content, .. } => {
                            // For a 'tool' role message, the content is the result.
                            content_parts.push(content.clone());
                        }
                        _ => {} // Ignore Thinking blocks
                    }
                }

                // Don't add empty messages
                if content_parts.is_empty() && image_parts.is_empty() && tool_calls.is_empty() {
                    continue;
                }

                ollama_messages.push(OllamaMessage {
                    role: role_str.to_string(),
                    content: content_parts.join("\n\n"),
                    images: if image_parts.is_empty() {
                        None
                    } else {
                        Some(image_parts)
                    },
                    tool_calls: if tool_calls.is_empty() {
                        None
                    } else {
                        Some(tool_calls)
                    },
                    thinking: None,
                    tool_name: None,
                });
            }
        }

        // --- New Prompt Injection Logic ---
        let injection_pos = unified_request
            .prompt_injection_position
            .as_deref()
            .unwrap_or("system");
        let combined_prompt_text = unified_request
            .combined_prompt
            .as_deref()
            .unwrap_or_default();

        if injection_pos == "user" && !combined_prompt_text.is_empty() {
            // Find the last user message and append the prompt.
            if let Some(last_user_msg) = ollama_messages.iter_mut().rfind(|m| m.role == "user") {
                last_user_msg.content =
                    format!("{}\n\n{}", combined_prompt_text, last_user_msg.content);
            }

            // After injecting into user message, handle the original system prompt separately.
            if let Some(sys_prompt_str) = &unified_request.system_prompt {
                if !sys_prompt_str.trim().is_empty() {
                    ollama_messages.insert(
                        0,
                        OllamaMessage {
                            role: "system".to_string(),
                            content: sys_prompt_str.clone(),
                            ..Default::default()
                        },
                    );
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

            let final_system_prompt = if injection_mode == "replace" {
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

            if !final_system_prompt.trim().is_empty() {
                ollama_messages.insert(
                    0,
                    OllamaMessage {
                        role: "system".to_string(),
                        content: final_system_prompt,
                        ..Default::default()
                    },
                );
            }
        }

        // Convert tools to Ollama format
        let ollama_tools = unified_request.tools.as_ref().and_then(|tools| {
            let collected_tools: Vec<OllamaTool> = tools
                .iter()
                .filter_map(|tool| {
                    // Ensure input_schema is an object, otherwise skip the tool
                    if tool.input_schema.is_object() {
                        Some(OllamaTool {
                            r#type: "function".to_string(),
                            function: crate::ccproxy::types::ollama::OllamaFunctionDefinition {
                                name: tool.name.clone(),
                                description: tool.description.clone(),
                                parameters: tool.input_schema.clone(),
                            },
                        })
                    } else {
                        log::warn!(
                            "Skipping tool '{}' because input_schema is not a JSON object",
                            tool.name
                        );
                        None
                    }
                })
                .collect();

            if collected_tools.is_empty() {
                None
            } else {
                Some(collected_tools)
            }
        });

        let ollama_request = OllamaChatCompletionRequest {
            model: model.to_string(),
            messages: ollama_messages,
            stream: Some(unified_request.stream),
            format: unified_request
                .response_format
                .as_ref()
                .and_then(|rf| {
                    // Try to extract format from response_format
                    if let Some(format_type) = rf.get("type") {
                        if format_type == "json_object" {
                            Some("json".to_string())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .or_else(|| {
                    // Fallback to response_mime_type for backward compatibility
                    unified_request
                        .response_mime_type
                        .as_ref()
                        .and_then(|mime| {
                            if mime == "application/json" {
                                Some("json".to_string())
                            } else {
                                None
                            }
                        })
                }),
            options: Some(OllamaOptions {
                temperature: unified_request
                    .temperature
                    .map(|t| adapt_temperature(t, ChatProtocol::Ollama)),
                num_predict: unified_request.max_tokens,
                top_p: unified_request.top_p,
                top_k: unified_request.top_k.map(|k| k as i32),
                stop: unified_request.stop_sequences.clone(),
                presence_penalty: unified_request.presence_penalty,
                frequency_penalty: unified_request.frequency_penalty,
                seed: unified_request.seed,
                ..Default::default()
            }),
            keep_alive: unified_request
                .keep_alive
                .as_deref()
                .map(|ka| serde_json::Value::from(ka)),
            tools: ollama_tools,
        };

        let mut request_builder = client.post(provider_full_url);
        request_builder = request_builder.header("Content-Type", "application/json");
        // Ollama doesn't use API keys, but we keep the pattern consistent
        // if !api_key.is_empty() {
        //     request_builder = request_builder.header("Authorization", format!("Bearer {}", api_key));
        // }

        // Try to serialize the request and log it for debugging
        request_builder = request_builder.json(&ollama_request);

        if log_proxy_to_file {
            // Log the request to a file
            log::info!(target: "ccproxy_logger","Ollama Request Body: \n{}\n----------------\n", serde_json::to_string_pretty(&ollama_request).unwrap_or_default());
        }

        // #[cfg(debug_assertions)]
        // {
        //     match serde_json::to_string_pretty(&ollama_request) {
        //         Ok(request_json) => {
        //             log::debug!("Ollama request: {}", request_json);
        //         }
        //         Err(e) => {
        //             log::error!("Failed to serialize Ollama request: {}", e);
        //             if let Some(tools) = &ollama_request.tools {
        //                 for (i, tool) in tools.iter().enumerate() {
        //                     if let Err(tool_err) = serde_json::to_string(&tool) {
        //                         log::error!("Failed to serialize tool {}: {}", i, tool_err);
        //                         log::error!(
        //                             "Tool details - name: {}, type: {}",
        //                             tool.function.name,
        //                             tool.r#type
        //                         );
        //                     }
        //                 }
        //             }
        //             return Err(anyhow::anyhow!("Failed to serialize Ollama request: {}", e));
        //         }
        //     }
        // }

        Ok(request_builder)
    }

    async fn adapt_response(
        &self,
        backend_response: BackendResponse,
    ) -> Result<UnifiedResponse, anyhow::Error> {
        log::debug!(
            "ollama response: {}",
            String::from_utf8_lossy(&backend_response.body)
        );

        let ollama_response: Result<OllamaChatCompletionResponse, serde_json::Error> =
            serde_json::from_slice(&backend_response.body);

        let ollama_response = match ollama_response {
            Ok(response) => response,
            Err(e) => {
                log::error!("Failed to parse Ollama response: {}", e);
                log::error!(
                    "Response body: {}",
                    String::from_utf8_lossy(&backend_response.body)
                );
                return Err(anyhow::anyhow!("Failed to parse Ollama response: {}", e));
            }
        };

        let mut content_blocks = Vec::new();

        // Handle tool compatibility mode parsing
        if backend_response.tool_compat_mode {
            let text = ollama_response.message.content.clone();
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
                if let Some(relative_end_pos) = processed_text[start_pos..].find(TOOL_TAG_END) {
                    let end_pos = start_pos + relative_end_pos;
                    let tool_xml = &processed_text[start_pos..end_pos + end_tag_len];
                    match crate::ccproxy::helper::tool_use_xml::ToolUse::try_from(tool_xml) {
                        Ok(parsed_tool) => {
                            log::debug!(
                                "tool_use parse result: name: {}, param: {:?}",
                                &parsed_tool.name,
                                &parsed_tool.args
                            );
                            content_blocks.push(parsed_tool.into());
                        }
                        Err(e) => {
                            let tool_xml = &processed_text[start_pos..end_pos + end_tag_len];
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
                    processed_text = processed_text[(end_pos + end_tag_len)..].to_string();
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
        } else {
            // Original parsing logic for non-tool compatibility mode
            if !ollama_response.message.content.is_empty() {
                content_blocks.push(UnifiedContentBlock::Text {
                    text: ollama_response.message.content,
                });
            }
        }

        // Handle reasoning content (thinking)
        if let Some(thinking_content) = ollama_response.message.thinking {
            if !thinking_content.is_empty() {
                content_blocks.push(UnifiedContentBlock::Thinking {
                    thinking: thinking_content,
                });
            }
        }

        // Handle native tool calls (non-tool compatibility mode)
        if !backend_response.tool_compat_mode {
            if let Some(tool_calls) = ollama_response.message.tool_calls {
                for tc in tool_calls {
                    content_blocks.push(UnifiedContentBlock::ToolUse {
                        id: get_tool_id(),
                        name: tc.function.name,
                        input: tc.function.arguments,
                    });
                }
            }
        }

        let usage = UnifiedUsage {
            input_tokens: ollama_response.prompt_eval_count.unwrap_or(0) as u64,
            output_tokens: ollama_response.eval_count.unwrap_or(0) as u64,
            ..Default::default()
        };

        Ok(UnifiedResponse {
            id: format!("chatcmpl-{}", uuid::Uuid::new_v4()),
            model: ollama_response.model,
            content: content_blocks,
            stop_reason: Some("stop".to_string()),
            usage,
        })
    }

    /// Adapt a ollama stream chunk into a unified stream chunk.
    /// @link https://github.com/ollama/ollama/blob/main/docs/api.md#generate-a-chat-completion
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

            // Handle message start
            self.handle_message_start(&ollama_chunk, &sse_status, &mut unified_chunks);

            if ollama_chunk.done {
                // Process finish reason
                self.process_finish_reason(
                    "stop".to_string(),
                    &ollama_chunk,
                    &sse_status,
                    &mut unified_chunks,
                );
            } else {
                // Process reasoning content (thinking)
                if let Some(thinking_content) = &ollama_chunk.message.thinking {
                    self.process_reasoning_content(
                        thinking_content.clone(),
                        &sse_status,
                        &mut unified_chunks,
                    );
                }

                // Handle content
                if !ollama_chunk.message.content.is_empty() {
                    // Check tool compatibility mode first
                    let tool_compat_mode = {
                        if let Ok(status) = sse_status.read() {
                            status.tool_compat_mode
                        } else {
                            false
                        }
                    };

                    if tool_compat_mode {
                        self.process_tool_compat_content(
                            &ollama_chunk.message.content,
                            &sse_status,
                            &mut unified_chunks,
                        );
                    } else {
                        self.process_normal_content(
                            &ollama_chunk.message.content,
                            &sse_status,
                            &mut unified_chunks,
                        );
                    }
                }

                // Handle tool calls
                if let Some(tool_calls) = &ollama_chunk.message.tool_calls {
                    // Acquire lock once for tool call initialization
                    if let Ok(mut status) = sse_status.write() {
                        if status.tool_delta_count == 0 {
                            if status.text_delta_count > 0 || status.thinking_delta_count > 0 {
                                unified_chunks.push(UnifiedStreamChunk::ContentBlockStop {
                                    index: status.message_index,
                                });
                            }
                        }
                        status.tool_delta_count += 1;
                    }

                    for tc in tool_calls {
                        self.process_tool_use(&sse_status, tc, &mut unified_chunks);
                    }
                }
            }
        }

        Ok(unified_chunks)
    }
}

// Private helper methods for OllamaBackendAdapter
impl OllamaBackendAdapter {
    /// Handle message start event
    fn handle_message_start(
        &self,
        _ollama_chunk: &OllamaStreamResponse,
        sse_status: &Arc<RwLock<SseStatus>>,
        unified_chunks: &mut Vec<UnifiedStreamChunk>,
    ) {
        if let Ok(mut status) = sse_status.write() {
            if !status.message_start {
                status.message_start = true;
                // Ollama doesn't provide message ID in stream, generate one
                if status.message_id.is_empty() {
                    status.message_id = format!("chatcmpl-{}", uuid::Uuid::new_v4());
                }
                unified_chunks.push(UnifiedStreamChunk::MessageStart {
                    id: status.message_id.clone(),
                    model: status.model_id.clone(),
                    usage: UnifiedUsage {
                        input_tokens: 0, // Ollama stream doesn't provide input tokens in the first chunk
                        output_tokens: 0,
                        ..Default::default()
                    },
                });
            }
        }
    }

    /// Process reasoning content
    fn process_reasoning_content(
        &self,
        content: String,
        sse_status: &Arc<RwLock<SseStatus>>,
        unified_chunks: &mut Vec<UnifiedStreamChunk>,
    ) {
        if content.is_empty() {
            return;
        }

        // Scope the lock to avoid holding it too long
        {
            if let Ok(mut status) = sse_status.write() {
                // Send the thinking start flag
                if status.thinking_delta_count == 0 {
                    log::debug!("adapt_stream_chunk: sending thinking start block");
                    unified_chunks.push(UnifiedStreamChunk::ContentBlockStart {
                        index: 0,
                        block: json!({
                            "type":"thinking",
                            "thinking":"",
                        }),
                    })
                }
                status.thinking_delta_count += 1;
                update_message_block(&mut status, "thinking".to_string());
            } else {
                log::warn!(
                    "adapt_stream_chunk: failed to acquire write lock for reasoning_content"
                );
            }
        }
        unified_chunks.push(UnifiedStreamChunk::Thinking { delta: content });
    }

    fn process_tool_use(
        &self,
        sse_status: &Arc<RwLock<SseStatus>>,
        tc: &OllamaToolCall,
        unified_chunks: &mut Vec<UnifiedStreamChunk>,
    ) {
        let name = &tc.function.name;
        if !name.is_empty() {
            let tool_id = get_tool_id();

            // Update tool_id in status
            let mut message_index = 0;
            if let Ok(mut status) = sse_status.write() {
                if status.tool_id != "" {
                    // send tool stop
                    unified_chunks.push(UnifiedStreamChunk::ContentBlockStop {
                        index: status.message_index,
                    })
                }
                status.tool_id = tool_id.clone();
                update_message_block(&mut status, tool_id.clone());
                message_index = status.message_index;
            }

            // for claude only
            unified_chunks.push(UnifiedStreamChunk::ContentBlockStart {
                index: message_index,
                block: json!({
                    "type":"tool_use",
                    "id": tool_id.clone(),
                    "name": name.clone(),
                    "input":{}
                }),
            });
            // for gemini and openai
            unified_chunks.push(UnifiedStreamChunk::ToolUseStart {
                tool_type: "tool_use".to_string(),
                id: tool_id.clone(),
                name: name.clone(),
            });
        }

        let args = &tc.function.arguments;
        if !args.is_null() {
            let mut tool_id = String::new();
            if let Ok(status) = sse_status.read() {
                tool_id = status.tool_id.clone();
            };
            // Convert arguments to string
            let args_str = args.to_string();
            unified_chunks.push(UnifiedStreamChunk::ToolUseDelta {
                id: tool_id,
                delta: args_str,
            });
        }
    }

    /// Process finish reason
    fn process_finish_reason(
        &self,
        finish_reason: String,
        ollama_chunk: &OllamaStreamResponse,
        sse_status: &Arc<RwLock<SseStatus>>,
        unified_chunks: &mut Vec<UnifiedStreamChunk>,
    ) {
        if let Ok(mut status) = sse_status.write() {
            if status.tool_compat_mode {
                // On stream finish, try to auto-complete any dangling tool tag.
                common::auto_complete_and_process_tool_tag(&mut status, unified_chunks);
            }

            if !status.tool_compat_buffer.is_empty()
                || !status.tool_compat_fragment_buffer.is_empty()
            {
                log::debug!("Flushing remaining buffers at stream end - buffer: {} chars, fragment: {} chars, in_tool_block: {}",
                    status.tool_compat_buffer.len(),
                    status.tool_compat_fragment_buffer.len(),
                    status.in_tool_call_block
                );

                // First, try to flush and process any remaining tool calls
                self.flush_tool_compat_buffer(&mut status, unified_chunks);

                // If there's still content in the buffer after processing, output it as text
                if !status.tool_compat_buffer.is_empty()
                    || !status.tool_compat_fragment_buffer.is_empty()
                {
                    unified_chunks.push(UnifiedStreamChunk::Text {
                        delta: format!(
                            "{}{}",
                            status.tool_compat_buffer, status.tool_compat_fragment_buffer
                        ),
                    });

                    status.tool_compat_buffer.clear();
                    status.tool_compat_fragment_buffer.clear();
                    status.in_tool_call_block = false;
                }
            }
        }

        let stop_reason = match finish_reason.to_lowercase().as_str() {
            "stop" => "stop".to_string(),
            "length" => "max_tokens".to_string(),
            "tool_calls" => "tool_use".to_string(),
            _ => "unknown".to_string(),
        };

        let usage = UnifiedUsage {
            input_tokens: ollama_chunk.prompt_eval_count.unwrap_or(0) as u64,
            output_tokens: ollama_chunk.eval_count.unwrap_or(0) as u64,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
            tool_use_prompt_tokens: None,
            thoughts_tokens: None,
            cached_content_tokens: None,
            ..Default::default()
        };

        unified_chunks.push(UnifiedStreamChunk::MessageStop { stop_reason, usage });
    }

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
        common::process_tool_calls_in_buffer(status, unified_chunks);

        // Handle remaining buffer content
        self.handle_remaining_buffer_content(status, unified_chunks);
    }

    /// Process content in normal (non-tool compatibility) mode
    fn process_normal_content(
        &self,
        content: &str,
        sse_status: &Arc<RwLock<SseStatus>>,
        unified_chunks: &mut Vec<UnifiedStreamChunk>,
    ) {
        if let Ok(mut status) = sse_status.write() {
            if !content.is_empty() {
                update_message_block(&mut status, "text".to_string());
            }

            if status.text_delta_count == 0 && !content.is_empty() {
                if status.thinking_delta_count > 0 {
                    unified_chunks.push(UnifiedStreamChunk::ContentBlockStop {
                        index: (status.message_index - 1).max(0),
                    });
                }

                if status.tool_delta_count > 0 {
                    unified_chunks.push(UnifiedStreamChunk::ToolUseEnd {
                        id: status.tool_id.clone(),
                    });
                    status.tool_delta_count = 0;
                }

                unified_chunks.push(UnifiedStreamChunk::ContentBlockStart {
                    index: status.message_index,
                    block: json!({
                         "type": "text",
                         "text": ""
                    }),
                });
            }
        }

        // Add text chunks
        if !content.is_empty() {
            // Update text delta count using existing lock
            if let Ok(mut status) = sse_status.write() {
                status.text_delta_count += 1;
            }
            unified_chunks.push(UnifiedStreamChunk::Text {
                delta: content.to_string(),
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
            let tool_end = TOOL_TAG_END;

            let mut partial_tag_len = 0;

            // Check for a partial start tag at the end of the buffer
            for i in (1..=std::cmp::min(buffer.len(), tool_start.len())).rev() {
                if buffer.ends_with(&tool_start[..i]) {
                    partial_tag_len = i;
                    break;
                }
            }

            // Also check for a partial end tag if no partial start tag was found
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
                // Flush the text part that is definitely not part of a tag, but clean markdown
                let text_to_flush = &buffer[..text_to_flush_len];
                if !text_to_flush.is_empty() {
                    unified_chunks.push(UnifiedStreamChunk::Text {
                        delta: text_to_flush.to_string(),
                    });
                }
                // Update the buffer to only contain the partial tag (or be empty)
                status.tool_compat_buffer = buffer[text_to_flush_len..].to_string();
            }
        }
    }
}
