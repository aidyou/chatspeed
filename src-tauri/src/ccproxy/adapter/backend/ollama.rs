use async_trait::async_trait;
use quick_xml::de::from_str;
use reqwest::{Client, RequestBuilder};
use serde_json::json;
use std::sync::{Arc, RwLock};

use crate::ccproxy::adapter::{
    backend::{generate_tool_prompt, update_message_block, ToolUse, TOOL_TAG_END, TOOL_TAG_START},
    unified::{
        SseStatus, UnifiedContentBlock, UnifiedMessage, UnifiedRequest, UnifiedResponse,
        UnifiedRole, UnifiedStreamChunk, UnifiedUsage,
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
        // If tool_compat_mode is enabled, we inject a system prompt with tool definitions
        // into the last user message. This is a specific adaptation for models that
        // don't support native tool calling APIs but can follow instructions.
        if unified_request.tool_compat_mode {
            if let Some(tools) = &unified_request.tools {
                let tool_prompt = generate_tool_prompt(tools);
                if let Some(last_message) = unified_request
                    .messages
                    .iter_mut()
                    .rfind(|m| m.role == UnifiedRole::User)
                {
                    // Prepend the tool prompt to the existing text content.
                    let new_text = if let Some(UnifiedContentBlock::Text { text }) =
                        last_message.content.get_mut(0)
                    {
                        format!("{}\n{}", tool_prompt, text)
                    } else {
                        tool_prompt
                    };
                    // Replace or insert the new text content.
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
                    // If no user message exists, create a new one with the tool prompt.
                    unified_request.messages.push(UnifiedMessage {
                        role: UnifiedRole::User,
                        content: vec![UnifiedContentBlock::Text { text: tool_prompt }],
                        reasoning_content: None,
                    });
                }
                // Tools are now part of the prompt, so remove them from the request object.
                unified_request.tools = None;
            }
        }

        // --- Advanced Message Processing with Proper Tool Call Handling ---
        // Build a map of tool call IDs to their corresponding tool results
        let mut tool_results: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for msg in &unified_request.messages {
            for block in &msg.content {
                if let UnifiedContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } = block
                {
                    tool_results.insert(tool_use_id.clone(), content.clone());
                }
            }
        }

        // Process messages with proper Ollama message structure
        let mut current_content = String::new();
        let mut current_images: Vec<String> = Vec::new();
        let mut current_tool_calls: Vec<OllamaToolCall> = Vec::new();
        let mut current_role: Option<UnifiedRole> = None;

        // Helper function to flush current message and add tool responses immediately
        let flush_current_message_with_tool_responses =
            |ollama_messages: &mut Vec<OllamaMessage>,
             current_role: &mut Option<UnifiedRole>,
             current_content: &mut String,
             current_images: &mut Vec<String>,
             current_tool_calls: &mut Vec<OllamaToolCall>,
             tool_results: &std::collections::HashMap<String, String>| {
                if current_content.is_empty()
                    && current_images.is_empty()
                    && current_tool_calls.is_empty()
                {
                    return;
                }

                let role_str = match current_role.as_ref().unwrap_or(&UnifiedRole::User) {
                    UnifiedRole::System => return,
                    UnifiedRole::User => "user",
                    UnifiedRole::Assistant => "assistant",
                    UnifiedRole::Tool => "tool",
                };

                // Add the main message
                if !current_content.is_empty()
                    || !current_images.is_empty()
                    || !current_tool_calls.is_empty()
                {
                    ollama_messages.push(OllamaMessage {
                        role: role_str.to_string(),
                        content: std::mem::take(current_content),
                        images: if current_images.is_empty() {
                            None
                        } else {
                            Some(std::mem::take(current_images))
                        },
                        thinking: None,
                        tool_calls: if current_tool_calls.is_empty() {
                            None
                        } else {
                            Some(current_tool_calls.clone())
                        },
                        tool_name: None,
                    });

                    // Immediately add tool responses for each tool call
                    for tool_call in current_tool_calls.iter() {
                        // Generate a tool call ID if not present
                        let tool_id = format!("tool_{}", uuid::Uuid::new_v4());
                        let tool_response_content =
                            tool_results.get(&tool_id).cloned().unwrap_or_else(|| {
                                "Tool execution was interrupted or failed.".to_string()
                            });

                        ollama_messages.push(OllamaMessage {
                            role: "tool".to_string(),
                            content: tool_response_content,
                            images: None,
                            thinking: None,
                            tool_calls: None,
                            tool_name: Some(tool_call.function.name.clone()),
                        });
                    }
                }

                current_tool_calls.clear();
                *current_role = None;
            };

        for msg in &unified_request.messages {
            for block in &msg.content {
                match block {
                    UnifiedContentBlock::ToolResult { .. } => {
                        // Tool results are handled when processing tool calls, skip them here
                        continue;
                    }
                    UnifiedContentBlock::Text { text } => {
                        // Check if we need to flush due to role change
                        if current_role.is_some() && current_role.as_ref().unwrap() != &msg.role {
                            flush_current_message_with_tool_responses(
                                &mut ollama_messages,
                                &mut current_role,
                                &mut current_content,
                                &mut current_images,
                                &mut current_tool_calls,
                                &tool_results,
                            );
                        }

                        if current_role.is_none() {
                            current_role = Some(msg.role.clone());
                        }
                        if !current_content.is_empty() {
                            current_content.push_str("\n\n");
                        }
                        current_content.push_str(text);
                    }
                    UnifiedContentBlock::Image { data, .. } => {
                        // Check if we need to flush due to role change
                        if current_role.is_some() && current_role.as_ref().unwrap() != &msg.role {
                            flush_current_message_with_tool_responses(
                                &mut ollama_messages,
                                &mut current_role,
                                &mut current_content,
                                &mut current_images,
                                &mut current_tool_calls,
                                &tool_results,
                            );
                        }

                        if current_role.is_none() {
                            current_role = Some(msg.role.clone());
                        }
                        current_images.push(data.clone());
                    }
                    UnifiedContentBlock::ToolUse { id: _, name, input } => {
                        if msg.role == UnifiedRole::System {
                            // If a system message contains a tool use, convert it to text
                            let tool_text = format!(
                                "System message contained a tool call: {{ name: {}, input: {} }}",
                                name, input
                            );
                            if current_role.is_none() {
                                current_role = Some(UnifiedRole::System);
                            }
                            if !current_content.is_empty() {
                                current_content.push_str("\n\n");
                            }
                            current_content.push_str(&tool_text);
                        } else {
                            // Check if we need to flush due to role change
                            if current_role.is_some() && current_role.as_ref().unwrap() != &msg.role
                            {
                                flush_current_message_with_tool_responses(
                                    &mut ollama_messages,
                                    &mut current_role,
                                    &mut current_content,
                                    &mut current_images,
                                    &mut current_tool_calls,
                                    &tool_results,
                                );
                            }

                            if current_role.is_none() {
                                current_role = Some(msg.role.clone());
                            }
                            current_tool_calls.push(OllamaToolCall {
                                function: OllamaFunctionCall {
                                    name: name.clone(),
                                    arguments: input.clone(),
                                },
                            });
                        }
                    }
                    _ => {} // Thinking blocks are ignored in the final request
                }
            }
        }

        // Flush any remaining message
        flush_current_message_with_tool_responses(
            &mut ollama_messages,
            &mut current_role,
            &mut current_content,
            &mut current_images,
            &mut current_tool_calls,
            &tool_results,
        );

        // Add system prompt if present
        if let Some(system_prompt) = &unified_request.system_prompt {
            if !system_prompt.trim().is_empty() {
                ollama_messages.insert(
                    0,
                    OllamaMessage {
                        role: "system".to_string(),
                        content: system_prompt.clone(),
                        images: None,
                        thinking: None,
                        tool_calls: None,
                        tool_name: None,
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
                                description: tool.description.clone().unwrap_or_default(),
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
                    if let Ok(parsed_tool) = from_str::<ToolUse>(tool_xml) {
                        let mut arguments = serde_json::Map::new();
                        for param in parsed_tool.params.param {
                            arguments.insert(
                                param.name.clone(),
                                serde_json::Value::String(param.get_value()),
                            );
                        }

                        log::debug!(
                            "tool_use parse result: name: {}, param: {:?}",
                            &parsed_tool.name,
                            &arguments
                        );

                        content_blocks.push(UnifiedContentBlock::ToolUse {
                            id: format!("tool_{}", uuid::Uuid::new_v4()),
                            name: parsed_tool.name,
                            input: serde_json::Value::Object(arguments),
                        });
                    } else {
                        let tool_xml = &processed_text[start_pos..end_pos + end_tag_len];

                        log::warn!("parse tool xml failed, xml: {}", tool_xml);
                        // If parsing fails, treat the text as a regular text block
                        content_blocks.push(UnifiedContentBlock::Text {
                            text: tool_xml.to_string(),
                        });
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
                        id: format!("tool_{}", uuid::Uuid::new_v4()),
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
            let tool_id = format!("tool_{}", uuid::Uuid::new_v4());

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
                let buffer = &status.tool_compat_buffer;
                let tool_start = TOOL_TAG_START;

                if let Some(start_pos) = buffer.find(tool_start) {
                    log::debug!(
                        "buffer has tool start tag `<ccp:tool_use>` at pos: {}",
                        start_pos
                    );
                    // Send text before the tool tag, but filter out markdown code blocks
                    let text_before = &buffer[..start_pos];
                    if !text_before.is_empty() {
                        unified_chunks.push(UnifiedStreamChunk::Text {
                            delta: text_before.to_string(),
                        });
                    }

                    status.tool_compat_buffer = buffer[start_pos..].to_string();
                    status.in_tool_call_block = true;
                } else {
                    break;
                }
            }

            if status.in_tool_call_block {
                let tool_end = TOOL_TAG_END;

                if let Some(end_pos) = status.tool_compat_buffer.find(tool_end) {
                    let tool_xml =
                        status.tool_compat_buffer[..end_pos + tool_end.len()].to_string();
                    let remaining_buffer =
                        status.tool_compat_buffer[end_pos + tool_end.len()..].to_string();

                    log::debug!(
                        "find tool end tag `{}` at pos: {}\n, tool xml: {}",
                        TOOL_TAG_END,
                        end_pos,
                        tool_xml
                    );

                    self.parse_and_emit_tool_call(status, &tool_xml, unified_chunks);

                    status.tool_compat_buffer = remaining_buffer;
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
        status: &mut std::sync::RwLockWriteGuard<SseStatus>,
        tool_xml: &str,
        unified_chunks: &mut Vec<UnifiedStreamChunk>,
    ) {
        if let Ok(parsed_tool) = from_str::<ToolUse>(tool_xml) {
            let tool_id = format!("tool_{}", uuid::Uuid::new_v4());
            if status.tool_id != "" {
                // send tool stop
                unified_chunks.push(UnifiedStreamChunk::ContentBlockStop {
                    index: status.message_index,
                })
            }
            status.tool_id = tool_id.clone();
            update_message_block(status, tool_id.clone());

            let mut arguments = serde_json::Map::new();
            for param in parsed_tool.params.param {
                arguments.insert(
                    param.name.clone(),
                    serde_json::Value::String(param.get_value()),
                );
            }

            // Send tool call start for claude only
            unified_chunks.push(UnifiedStreamChunk::ContentBlockStart {
                index: status.message_index,
                block: json!({
                    "type": "tool_use",
                    "id": tool_id.clone(),
                    "name": parsed_tool.name.clone(),
                    "input": {}
                }),
            });
            // Send tool call start for gemini and openai
            unified_chunks.push(UnifiedStreamChunk::ToolUseStart {
                tool_type: "function".to_string(),
                id: tool_id.clone(),
                name: parsed_tool.name.clone(),
            });

            // Send tool call parameters
            let args_json = serde_json::to_string(&arguments).unwrap_or_default();
            unified_chunks.push(UnifiedStreamChunk::ToolUseDelta {
                id: tool_id,
                delta: args_json.clone(),
            });

            log::info!(
                "tool parse success, name: {}, param: {}",
                parsed_tool.name.clone(),
                args_json
            );
        } else {
            unified_chunks.push(UnifiedStreamChunk::Error {
                message: format!("tool xml parse failed, xml: {}", tool_xml),
            });
            log::warn!("tool xml parse failed, xml: {}", tool_xml);
        }
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
