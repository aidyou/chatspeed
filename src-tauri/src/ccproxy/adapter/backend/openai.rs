use async_trait::async_trait;
use reqwest::{Client, RequestBuilder};
use serde_json::json;
use std::sync::{Arc, RwLock};

use super::{BackendAdapter, BackendResponse};
use crate::ccproxy::get_tool_id;
use crate::ccproxy::openai::{
    OpenAIChatCompletionRequest, OpenAIChatCompletionResponse, OpenAIChatCompletionStreamResponse,
    OpenAIFunctionCall, OpenAIFunctionDefinition, OpenAIImageUrl, OpenAIMessageContent,
    OpenAIMessageContentPart, OpenAIResponseFormat, OpenAITool, OpenAIToolChoice,
    OpenAIToolChoiceFunction, OpenAIToolChoiceObject, UnifiedChatMessage, UnifiedToolCall,
};
use crate::ccproxy::types::{TOOL_PARSE_ERROR_REMINDER, TOOL_TAG_END, TOOL_TAG_START};
use crate::ccproxy::{
    adapter::{
        backend::{common, update_message_block},
        range_adapter::adapt_temperature,
        unified::{
            SseStatus, UnifiedContentBlock, UnifiedRequest, UnifiedResponse, UnifiedRole,
            UnifiedStreamChunk, UnifiedToolChoice, UnifiedUsage,
        },
    },
    types::ChatProtocol,
    utils::token_estimator::estimate_tokens,
};

pub struct OpenAIBackendAdapter;

#[async_trait]
impl BackendAdapter for OpenAIBackendAdapter {
    async fn adapt_request(
        &self,
        client: &Client,
        unified_request: &mut UnifiedRequest,
        api_key: &str,
        full_provider_url: &str,
        model: &str,
        log_proxy_to_file: bool,
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

        let mut openai_messages: Vec<UnifiedChatMessage> = Vec::new();
        // --- Message Processing ---
        if unified_request.tool_compat_mode {
            // Special handling for tool compatibility mode to mimic a more natural
            // conversation flow for models that don't natively support tool calls.
            let mut processed_messages: Vec<UnifiedChatMessage> = Vec::new();
            let mut tool_results_buffer: Vec<String> = Vec::new();

            for msg in &unified_request.messages {
                match msg.role {
                    UnifiedRole::Assistant => {
                        // If there are pending tool results, flush them as a single user message first.
                        if !tool_results_buffer.is_empty() {
                            processed_messages.push(UnifiedChatMessage {
                                role: Some("user".to_string()),
                                content: Some(OpenAIMessageContent::Text(
                                    tool_results_buffer.join("\n"),
                                )),
                                ..Default::default()
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
                            processed_messages.push(UnifiedChatMessage {
                                role: Some("assistant".to_string()),
                                content: Some(OpenAIMessageContent::Text(final_content)),
                                ..Default::default()
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
                            processed_messages.push(UnifiedChatMessage {
                                role: Some("user".to_string()),
                                content: Some(OpenAIMessageContent::Text(
                                    tool_results_buffer.join("\n"),
                                )),
                                ..Default::default()
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
                            processed_messages.push(UnifiedChatMessage {
                                role: Some("user".to_string()),
                                content: Some(OpenAIMessageContent::Text(content_text)),
                                ..Default::default()
                            });
                        }
                    }
                    _ => {} // Ignore System role in this context, it's handled separately.
                }
            }

            // Flush any remaining tool results at the end of the message list.
            if !tool_results_buffer.is_empty() {
                processed_messages.push(UnifiedChatMessage {
                    role: Some("user".to_string()),
                    content: Some(OpenAIMessageContent::Text(tool_results_buffer.join("\n"))),
                    ..Default::default()
                });
            }
            openai_messages = processed_messages;
        } else {
            // Standard processing for non-tool-compatibility mode.
            for msg in &unified_request.messages {
                let role_str = match msg.role {
                    UnifiedRole::System => "system",
                    UnifiedRole::User => "user",
                    UnifiedRole::Assistant => "assistant",
                    UnifiedRole::Tool => "tool",
                };

                let mut primary_message_parts: Vec<OpenAIMessageContentPart> = Vec::new();
                let mut primary_tool_calls: Vec<UnifiedToolCall> = Vec::new();
                let mut tool_result_messages: Vec<UnifiedChatMessage> = Vec::new();

                for block in &msg.content {
                    match block {
                        UnifiedContentBlock::Text { text } => {
                            primary_message_parts
                                .push(OpenAIMessageContentPart::Text { text: text.clone() });
                        }
                        UnifiedContentBlock::Image { media_type, data } => {
                            primary_message_parts.push(OpenAIMessageContentPart::ImageUrl {
                                image_url: OpenAIImageUrl {
                                    url: format!("data:{};base64,{}", media_type, data),
                                    detail: None,
                                },
                            });
                        }
                        UnifiedContentBlock::ToolUse { id, name, input } => {
                            primary_tool_calls.push(UnifiedToolCall {
                                id: Some(id.clone()),
                                r#type: Some("function".to_string()),
                                function: OpenAIFunctionCall {
                                    name: Some(name.clone()),
                                    arguments: Some(input.to_string()),
                                },
                                index: None,
                            });
                        }
                        UnifiedContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            ..
                        } => {
                            // Find the tool name from history, as some servers require it in the tool response.
                            let tool_name = unified_request
                                .messages
                                .iter()
                                .flat_map(|m| &m.content)
                                .find_map(|block| {
                                    if let UnifiedContentBlock::ToolUse { id, name, .. } = block {
                                        if id == tool_use_id {
                                            Some(name.clone())
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    }
                                });

                            tool_result_messages.push(UnifiedChatMessage {
                                role: Some("tool".to_string()),
                                content: Some(OpenAIMessageContent::Text(content.clone())),
                                tool_call_id: Some(tool_use_id.clone()),
                                name: tool_name,
                                ..Default::default()
                            });
                        }
                        _ => {} // Ignore Thinking blocks
                    }
                }

                if !primary_message_parts.is_empty() || !primary_tool_calls.is_empty() {
                    let content = if primary_message_parts.is_empty() {
                        None
                    } else if primary_message_parts
                        .iter()
                        .all(|p| matches!(p, OpenAIMessageContentPart::Text { .. }))
                    {
                        let combined_text = primary_message_parts
                            .iter()
                            .filter_map(|p| {
                                if let OpenAIMessageContentPart::Text { text } = p {
                                    Some(text.as_str())
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<&str>>()
                            .join("\n");
                        Some(OpenAIMessageContent::Text(combined_text))
                    } else {
                        Some(OpenAIMessageContent::Parts(primary_message_parts))
                    };

                    openai_messages.push(UnifiedChatMessage {
                        role: Some(role_str.to_string()),
                        content,
                        tool_calls: if primary_tool_calls.is_empty() {
                            None
                        } else {
                            Some(primary_tool_calls)
                        },
                        ..Default::default()
                    });
                }

                openai_messages.extend(tool_result_messages);
            }
        }

        let openai_tools = unified_request.tools.as_ref().and_then(|tools| {
            let collected_tools: Vec<OpenAITool> = tools
                .iter()
                .filter_map(|tool| {
                    // Ensure input_schema is an object, otherwise skip the tool
                    if tool.input_schema.is_object() {
                        Some(OpenAITool {
                            r#type: "function".to_string(),
                            function: OpenAIFunctionDefinition {
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

        let openai_tool_choice = unified_request
            .tool_choice
            .as_ref()
            .map(|choice| match choice {
                UnifiedToolChoice::None => OpenAIToolChoice::String("none".to_string()),
                UnifiedToolChoice::Auto => OpenAIToolChoice::String("auto".to_string()),
                UnifiedToolChoice::Required => OpenAIToolChoice::String("required".to_string()),
                UnifiedToolChoice::Tool { name } => {
                    OpenAIToolChoice::Object(OpenAIToolChoiceObject {
                        choice_type: "function".to_string(),
                        function: OpenAIToolChoiceFunction { name: name.clone() },
                    })
                }
            });

        let reasoning_effort = if let Some(thinking) = &unified_request.thinking {
            if matches!(thinking.include_thoughts, Some(true)) {
                let effort = match thinking.budget_tokens {
                    Some(budget) if budget < 4096 => "low",
                    Some(budget) if budget >= 4096 && budget <= 16384 => "medium",
                    Some(budget) if budget > 16384 => "high",
                    _ => "medium", // Default for Some(0) or None
                };
                Some(effort.to_string())
            } else {
                None
            }
        } else {
            None
        };

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
            if let Some(last_user_msg) = openai_messages
                .iter_mut()
                .rfind(|m| m.role.as_deref() == Some("user"))
            {
                // Append to the content of the last user message
                if let Some(content) = &mut last_user_msg.content {
                    match content {
                        OpenAIMessageContent::Text(text) => {
                            *text = format!("{}\n\n{}", combined_prompt_text, text);
                        }
                        OpenAIMessageContent::Parts(parts) => {
                            parts.insert(
                                0,
                                OpenAIMessageContentPart::Text {
                                    text: combined_prompt_text.to_string(),
                                },
                            );
                        }
                    }
                } else {
                    // If the last user message has no content, set it.
                    last_user_msg.content =
                        Some(OpenAIMessageContent::Text(combined_prompt_text.to_string()));
                }
            }

            // After injecting into user message, handle the original system prompt separately.
            if let Some(sys_prompt_str) = unified_request.system_prompt.as_deref() {
                if !sys_prompt_str.trim().is_empty() {
                    openai_messages.insert(
                        0,
                        UnifiedChatMessage {
                            role: Some("system".to_string()),
                            content: Some(OpenAIMessageContent::Text(sys_prompt_str.to_string())),
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
                openai_messages.insert(
                    0,
                    UnifiedChatMessage {
                        role: Some("system".to_string()),
                        content: Some(OpenAIMessageContent::Text(final_system_prompt)),
                        ..Default::default()
                    },
                );
            }
        }

        let openai_request = OpenAIChatCompletionRequest {
            model: model.to_string(),
            messages: openai_messages,
            stream: Some(unified_request.stream),
            max_tokens: unified_request.max_tokens,
            temperature: unified_request
                .temperature
                .map(|t| adapt_temperature(t, ChatProtocol::OpenAI)),
            top_p: unified_request.top_p,
            presence_penalty: unified_request.presence_penalty,
            frequency_penalty: unified_request.frequency_penalty,
            response_format: unified_request.response_format.as_ref().and_then(|rf| {
                match serde_json::from_value::<OpenAIResponseFormat>(rf.clone()) {
                    Ok(format) => Some(format),
                    Err(e) => {
                        log::warn!("Failed to parse response_format: {}", e);
                        None
                    }
                }
            }),
            stop: unified_request.stop_sequences.clone(),
            seed: unified_request.seed,
            user: unified_request.user.clone(),
            tools: openai_tools,
            tool_choice: openai_tool_choice,
            logprobs: unified_request.logprobs,
            top_logprobs: unified_request.top_logprobs,
            stream_options: None,
            logit_bias: None, // Not supported in unified request yet
            reasoning_effort,
            store: None, // Not supported in unified request yet
        };

        let mut request_builder = client.post(full_provider_url);
        request_builder = request_builder.header("Content-Type", "application/json");
        if !api_key.is_empty() {
            request_builder =
                request_builder.header("Authorization", format!("Bearer {}", api_key));
        }

        // Try to serialize the request and log it for debugging
        let mut request_json = serde_json::to_value(&openai_request)?;

        // Merge custom params from model config
        crate::ai::util::merge_custom_params(&mut request_json, &unified_request.custom_params);

        request_builder = request_builder.json(&request_json);

        if log_proxy_to_file {
            // Log the request to a file
            log::info!(target: "ccproxy_logger","Openai Request Body: \n{}\n----------------\n", serde_json::to_string_pretty(&request_json).unwrap_or_default());
        }

        // #[cfg(debug_assertions)]
        // {
        //     match serde_json::to_string_pretty(&openai_request) {
        //         Ok(request_json) => {
        //             log::debug!("OpenAI request: {}", request_json);
        //         }
        //         Err(e) => {
        //             log::error!("Failed to serialize OpenAI request: {}", e);
        //             // Try to serialize individual parts to identify the issue
        //             if let Some(tools) = &openai_request.tools {
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
        //             return Err(anyhow::anyhow!("Failed to serialize OpenAI request: {}", e));
        //         }
        //     }
        // }

        Ok(request_builder)
    }

    async fn adapt_response(
        &self,
        backend_response: BackendResponse,
    ) -> Result<UnifiedResponse, anyhow::Error> {
        #[cfg(debug_assertions)]
        log::debug!(
            "openai response: {}",
            String::from_utf8_lossy(&backend_response.body)
        );

        let openai_response: Result<OpenAIChatCompletionResponse, serde_json::Error> =
            serde_json::from_slice(&backend_response.body);

        let openai_response = match openai_response {
            Ok(response) => response,
            Err(e) => {
                log::error!("Failed to parse OpenAI response: {}", e);
                log::error!(
                    "Response body: {}",
                    String::from_utf8_lossy(&backend_response.body)
                );
                return Err(anyhow::anyhow!("Failed to parse OpenAI response: {}", e));
            }
        };

        let first_choice = openai_response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No choices in OpenAI response"))?;

        let mut content_blocks = Vec::new();

        // Handle tool compatibility mode parsing
        if backend_response.tool_compat_mode {
            if let Some(content) = first_choice.message.content {
                let text = match content {
                    OpenAIMessageContent::Text(text) => text,
                    OpenAIMessageContent::Parts(parts) => parts
                        .into_iter()
                        .filter_map(|part| match part {
                            OpenAIMessageContentPart::Text { text } => Some(text),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join(""),
                };

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
            }
        } else {
            // Original parsing logic for non-tool compatibility mode
            if let Some(content) = first_choice.message.content {
                match content {
                    OpenAIMessageContent::Text(text) => {
                        content_blocks.push(UnifiedContentBlock::Text { text })
                    }
                    OpenAIMessageContent::Parts(parts) => {
                        for part in parts {
                            match part {
                                OpenAIMessageContentPart::Text { text } => {
                                    content_blocks.push(UnifiedContentBlock::Text { text })
                                }
                                OpenAIMessageContentPart::ImageUrl { image_url: _ } => {
                                    anyhow::bail!("Image URL in assistant response not supported for UnifiedResponse");
                                }
                            }
                        }
                    }
                }
            }
        }

        if let Some(reasoning_content) = first_choice.message.reasoning_content {
            if !reasoning_content.is_empty() {
                content_blocks.push(UnifiedContentBlock::Thinking {
                    thinking: reasoning_content,
                });
            }
        }

        // Handle native tool calls (non-tool compatibility mode)
        if !backend_response.tool_compat_mode {
            if let Some(tool_calls) = first_choice.message.tool_calls {
                for tc in tool_calls {
                    content_blocks.push(UnifiedContentBlock::ToolUse {
                        id: tc.id.clone().unwrap_or_default(),
                        name: tc.function.name.unwrap_or_default(),
                        input: serde_json::from_str(
                            &tc.function.arguments.unwrap_or_default().replace("'", "\""),
                        )?,
                    });
                }
            }
        }

        let usage = openai_response
            .usage
            .map(|u| {
                let thoughts_tokens = u
                    .completion_tokens_details
                    .as_ref()
                    .and_then(|d| d.reasoning_tokens);
                let prompt_cached_tokens = u
                    .prompt_tokens_details
                    .as_ref()
                    .and_then(|d| d.cached_tokens);

                UnifiedUsage {
                    input_tokens: u.prompt_tokens,
                    output_tokens: u.completion_tokens,
                    thoughts_tokens,
                    prompt_cached_tokens,
                    ..Default::default()
                }
            })
            .unwrap_or_default();

        Ok(UnifiedResponse {
            id: openai_response.id,
            model: openai_response.model,
            content: content_blocks,
            stop_reason: first_choice.finish_reason,
            usage,
        })
    }

    /// Adapt a openai stream chunk into a unified stream chunk.
    /// @link https://platform.openai.com/docs/api-reference/chat-streaming
    async fn adapt_stream_chunk(
        &self,
        chunk: bytes::Bytes,
        sse_status: Arc<RwLock<SseStatus>>,
    ) -> Result<Vec<UnifiedStreamChunk>, anyhow::Error> {
        let chunk_str = String::from_utf8_lossy(&chunk);
        let mut unified_chunks = Vec::new();

        for event_block_str in chunk_str.split("\n\n") {
            if event_block_str.trim().is_empty() {
                continue;
            }

            if let Some(data) = self.extract_sse_data(event_block_str) {
                if data == "[DONE]" {
                    // Handled by MessageStop or stream end signal
                    continue;
                }

                let openai_chunk: OpenAIChatCompletionStreamResponse = serde_json::from_str(&data)
                    .map_err(|e| {
                        log::error!("Failed to parse OpenAI chunk, error: {}, data: {}", e, data);
                        e
                    })?;
                // Handle message start
                self.handle_message_start(&openai_chunk, &sse_status, &mut unified_chunks);

                for (_, choice) in openai_chunk.choices.iter().enumerate() {
                    let delta = &choice.delta;

                    // Process reasoning content
                    if let Some(content) = &delta.reasoning_content {
                        self.process_reasoning_content(
                            content.clone(),
                            &sse_status,
                            &mut unified_chunks,
                        );
                    }

                    if let Some(content) = &delta.reference {
                        self.process_reference_content(
                            content.clone(),
                            &sse_status,
                            &mut unified_chunks,
                        );
                    }

                    if let Some(content) = &delta.content {
                        let has_text = self.content_has_text(content);

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
                                content,
                                &sse_status,
                                &mut unified_chunks,
                            );
                        } else {
                            self.process_normal_content(
                                content,
                                has_text,
                                &sse_status,
                                &mut unified_chunks,
                            );
                        }
                    }

                    if let Some(tool_calls) = &delta.tool_calls {
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

                    // Process finish reason
                    if let Some(finish_reason) = choice.finish_reason.as_ref() {
                        self.process_finish_reason(
                            finish_reason.clone(),
                            &openai_chunk,
                            &sse_status,
                            &mut unified_chunks,
                        );
                    }
                }
            }
        }

        Ok(unified_chunks)
    }
}

// Private helper methods for OpenAIBackendAdapter
impl OpenAIBackendAdapter {
    /// Extract SSE data from event block
    fn extract_sse_data(&self, event_block: &str) -> Option<String> {
        for line in event_block.lines() {
            if line.starts_with("data:") {
                return Some(line["data:".len()..].trim().to_string());
            }
        }
        None
    }

    /// Check if content has text
    fn content_has_text(&self, content: &OpenAIMessageContent) -> bool {
        match content {
            OpenAIMessageContent::Text(text) => !text.is_empty(),
            OpenAIMessageContent::Parts(parts) => parts.iter().any(
                |part| matches!(part, OpenAIMessageContentPart::Text { text } if !text.is_empty()),
            ),
        }
    }

    /// Handle message start event
    fn handle_message_start(
        &self,
        _openai_chunk: &OpenAIChatCompletionStreamResponse,
        sse_status: &Arc<RwLock<SseStatus>>,
        unified_chunks: &mut Vec<UnifiedStreamChunk>,
    ) {
        if let Ok(mut status) = sse_status.write() {
            if !status.message_start {
                status.message_start = true;
                // if let Some(id) = openai_chunk.id.as_ref() {
                //     if status.message_id.is_empty() && !id.is_empty() {
                //         status.message_id = id.clone();
                //     }
                // }
                unified_chunks.push(UnifiedStreamChunk::MessageStart {
                    id: status.message_id.clone(),
                    model: status.model_id.clone(),
                    usage: UnifiedUsage {
                        input_tokens: 0, // OpenAI stream doesn't provide input tokens in the first chunk
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
                status.estimated_output_tokens += estimate_tokens(&content);
                update_message_block(&mut status, "thinking".to_string());
            } else {
                log::warn!(
                    "adapt_stream_chunk: failed to acquire write lock for reasoning_content"
                );
            }
        }
        unified_chunks.push(UnifiedStreamChunk::Thinking { delta: content });
    }

    fn process_reference_content(
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
                if status.text_delta_count == 0 {
                    log::debug!("adapt_stream_chunk: sending thinking start block");
                    unified_chunks.push(UnifiedStreamChunk::ContentBlockStart {
                        index: 0,
                        block: json!({
                            "type":"thinking",
                            "thinking":"",
                        }),
                    })
                }
                status.text_delta_count += 1;
                status.estimated_output_tokens += estimate_tokens(&content);
                update_message_block(&mut status, "reference".to_string());
            } else {
                log::warn!(
                    "adapt_stream_chunk: failed to acquire write lock for reasoning_content"
                );
            }
        }
        unified_chunks.push(UnifiedStreamChunk::Reference { delta: content });
    }

    fn process_tool_use(
        &self,
        sse_status: &Arc<RwLock<SseStatus>>,
        tc: &UnifiedToolCall,
        unified_chunks: &mut Vec<UnifiedStreamChunk>,
    ) {
        if let Some(name) = &tc.function.name {
            if !name.is_empty() {
                let tool_id = tc.id.clone().unwrap_or_else(|| get_tool_id());

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
        }

        if let Some(args) = &tc.function.arguments {
            if !args.is_empty() {
                let mut tool_id = String::new();
                if let Ok(mut status) = sse_status.write() {
                    tool_id = status.tool_id.clone();
                    status.estimated_output_tokens += estimate_tokens(args);
                };
                // Replace single quotes with double quotes for JSON compatibility
                let cleaned_args = args.replace("'", "\"");
                unified_chunks.push(UnifiedStreamChunk::ToolUseDelta {
                    id: tool_id,
                    delta: cleaned_args,
                });
            }
        }
    }

    /// Process finish reason
    fn process_finish_reason(
        &self,
        finish_reason: String,
        openai_chunk: &OpenAIChatCompletionStreamResponse,
        sse_status: &Arc<RwLock<SseStatus>>,
        unified_chunks: &mut Vec<UnifiedStreamChunk>,
    ) {
        if let Ok(mut status) = sse_status.write() {
            if status.tool_compat_mode {
                // On stream finish, try to auto-complete any dangling tool tag.
                common::auto_complete_and_process_tool_tag(&mut status, unified_chunks);
            }

            // After attempting to complete and process tags, send any remaining text.
            if !status.tool_compat_buffer.is_empty() {
                unified_chunks.push(UnifiedStreamChunk::Text {
                    delta: status.tool_compat_buffer.clone(),
                });
                status.tool_compat_buffer.clear();
            }
        }

        let stop_reason = match finish_reason.to_lowercase().as_str() {
            "stop" => "stop".to_string(),
            "length" => "max_tokens".to_string(),
            "tool_calls" => "tool_use".to_string(),
            _ => "unknown".to_string(),
        };

        let usage = openai_chunk
            .usage
            .clone()
            .map(|u| {
                let thoughts_tokens = u
                    .completion_tokens_details
                    .as_ref()
                    .and_then(|d| d.reasoning_tokens);
                let prompt_cached_tokens = u
                    .prompt_tokens_details
                    .as_ref()
                    .and_then(|d| d.cached_tokens);

                UnifiedUsage {
                    input_tokens: u.prompt_tokens,
                    output_tokens: u.completion_tokens,
                    thoughts_tokens,
                    prompt_cached_tokens,
                    ..Default::default()
                }
            })
            .unwrap_or_default();

        unified_chunks.push(UnifiedStreamChunk::MessageStop { stop_reason, usage });
    }

    /// Process content in tool compatibility mode
    fn process_tool_compat_content(
        &self,
        content: &OpenAIMessageContent,
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
            status.tool_compat_fragment_buffer.push_str(&match content {
                OpenAIMessageContent::Text(text) => text.as_str(),
                OpenAIMessageContent::Parts(parts) => parts
                    .iter()
                    .find_map(|part| match part {
                        OpenAIMessageContentPart::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .unwrap_or(""),
            });
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
    }

    /// Process content in normal (non-tool compatibility) mode
    fn process_normal_content(
        &self,
        content: &OpenAIMessageContent,
        has_text: bool,
        sse_status: &Arc<RwLock<SseStatus>>,
        unified_chunks: &mut Vec<UnifiedStreamChunk>,
    ) {
        if let Ok(mut status) = sse_status.write() {
            if has_text {
                update_message_block(&mut status, "text".to_string());
            }

            if status.text_delta_count == 0 && has_text {
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
        self.add_text_chunks(content, sse_status, unified_chunks);
    }

    /// Add text chunks to unified_chunks
    fn add_text_chunks(
        &self,
        content: &OpenAIMessageContent,
        sse_status: &Arc<RwLock<SseStatus>>,
        unified_chunks: &mut Vec<UnifiedStreamChunk>,
    ) {
        match content {
            OpenAIMessageContent::Text(text) => {
                if !text.is_empty() {
                    // Update text delta count using existing lock
                    if let Ok(mut status) = sse_status.write() {
                        status.text_delta_count += 1;
                        status.estimated_output_tokens += estimate_tokens(text);
                    }
                    unified_chunks.push(UnifiedStreamChunk::Text {
                        delta: text.clone(),
                    });
                }
            }
            OpenAIMessageContent::Parts(parts) => {
                if !parts.is_empty() {
                    // Update text delta count using existing lock
                    if let Ok(mut status) = sse_status.write() {
                        status.text_delta_count += parts.len() as u32;
                        for part in parts {
                            if let OpenAIMessageContentPart::Text { text } = part {
                                if !text.is_empty() {
                                    status.estimated_output_tokens += estimate_tokens(text);
                                }
                            }
                        }
                    }
                    for part in parts {
                        if let OpenAIMessageContentPart::Text { text } = part {
                            if !text.is_empty() {
                                unified_chunks.push(UnifiedStreamChunk::Text {
                                    delta: text.clone(),
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}
