use async_trait::async_trait;
use quick_xml::de::from_str;
use reqwest::{Client, RequestBuilder};
use serde_json::json;
use std::sync::{Arc, RwLock};

use super::{BackendAdapter, BackendResponse};
use crate::ccproxy::adapter::{
    backend::{generate_tool_prompt, update_message_block, ToolUse, TOOL_TAG_END, TOOL_TAG_START},
    range_adapter::{adapt_temperature, Protocol},
    unified::{
        SseStatus, UnifiedContentBlock, UnifiedMessage, UnifiedRequest, UnifiedResponse,
        UnifiedRole, UnifiedStreamChunk, UnifiedToolChoice, UnifiedUsage,
    },
};
use crate::ccproxy::openai::{
    OpenAIChatCompletionRequest, OpenAIChatCompletionResponse, OpenAIChatCompletionStreamResponse,
    OpenAIFunctionCall, OpenAIFunctionDefinition, OpenAIImageUrl, OpenAIMessageContent,
    OpenAIMessageContentPart, OpenAIResponseFormat, OpenAITool, OpenAIToolChoice,
    OpenAIToolChoiceFunction, OpenAIToolChoiceObject, UnifiedChatMessage, UnifiedToolCall,
};

pub struct OpenAIBackendAdapter;

#[async_trait]
impl BackendAdapter for OpenAIBackendAdapter {
    async fn adapt_request(
        &self,
        client: &Client,
        unified_request: &UnifiedRequest,
        api_key: &str,
        full_provider_url: &str,
        model: &str,
    ) -> Result<RequestBuilder, anyhow::Error> {
        let mut openai_messages: Vec<UnifiedChatMessage> = Vec::new();
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

        // Process messages with proper OpenAI message structure
        let mut current_message_parts: Vec<OpenAIMessageContentPart> = Vec::new();
        let mut current_tool_calls: Vec<UnifiedToolCall> = Vec::new();
        let mut current_role: Option<UnifiedRole> = None;

        // Helper function to flush current message and add tool responses immediately
        let flush_current_message_with_tool_responses =
            |openai_messages: &mut Vec<UnifiedChatMessage>,
             current_role: &mut Option<UnifiedRole>,
             current_message_parts: &mut Vec<OpenAIMessageContentPart>,
             current_tool_calls: &mut Vec<UnifiedToolCall>,
             tool_results: &std::collections::HashMap<String, String>| {
                if current_message_parts.is_empty() && current_tool_calls.is_empty() {
                    return;
                }

                let role_str = match current_role.as_ref().unwrap_or(&UnifiedRole::User) {
                    UnifiedRole::System => return,
                    UnifiedRole::User => "user",
                    UnifiedRole::Assistant => "assistant",
                    UnifiedRole::Tool => "tool",
                };

                // Combine text parts into a single content if all are text
                let content = if current_message_parts.is_empty() {
                    None
                } else if current_message_parts
                    .iter()
                    .all(|p| matches!(p, OpenAIMessageContentPart::Text { .. }))
                {
                    let combined_text = current_message_parts
                        .iter()
                        .filter_map(|p| {
                            if let OpenAIMessageContentPart::Text { text } = p {
                                Some(text.as_str())
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<&str>>()
                        .join("\n\n");
                    Some(OpenAIMessageContent::Text(combined_text))
                } else {
                    Some(OpenAIMessageContent::Parts(std::mem::take(
                        current_message_parts,
                    )))
                };

                // Add the main message
                if content.is_some() || !current_tool_calls.is_empty() {
                    openai_messages.push(UnifiedChatMessage {
                        role: Some(role_str.to_string()),
                        content,
                        tool_calls: if current_tool_calls.is_empty() {
                            None
                        } else {
                            Some(current_tool_calls.clone())
                        },
                        tool_call_id: None,
                        reasoning_content: None,
                    });

                    // Immediately add tool responses for each tool call
                    for tool_call in current_tool_calls.iter() {
                        if let Some(tool_id) = &tool_call.id {
                            let tool_response_content =
                                tool_results.get(tool_id).cloned().unwrap_or_else(|| {
                                    "Tool execution was interrupted or failed.".to_string()
                                });

                            openai_messages.push(UnifiedChatMessage {
                                role: Some("tool".to_string()),
                                content: Some(OpenAIMessageContent::Text(tool_response_content)),
                                tool_calls: None,
                                tool_call_id: Some(tool_id.clone()),
                                reasoning_content: None,
                            });
                        }
                    }
                }

                current_message_parts.clear();
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
                                &mut openai_messages,
                                &mut current_role,
                                &mut current_message_parts,
                                &mut current_tool_calls,
                                &tool_results,
                            );
                        }

                        if current_role.is_none() {
                            current_role = Some(msg.role.clone());
                        }
                        current_message_parts
                            .push(OpenAIMessageContentPart::Text { text: text.clone() });
                    }
                    UnifiedContentBlock::Image { media_type, data } => {
                        // Check if we need to flush due to role change
                        if current_role.is_some() && current_role.as_ref().unwrap() != &msg.role {
                            flush_current_message_with_tool_responses(
                                &mut openai_messages,
                                &mut current_role,
                                &mut current_message_parts,
                                &mut current_tool_calls,
                                &tool_results,
                            );
                        }

                        if current_role.is_none() {
                            current_role = Some(msg.role.clone());
                        }
                        current_message_parts.push(OpenAIMessageContentPart::ImageUrl {
                            image_url: OpenAIImageUrl {
                                url: format!("data:{};base64,{}", media_type, data),
                                detail: None,
                            },
                        });
                    }
                    UnifiedContentBlock::ToolUse { id, name, input } => {
                        if msg.role == UnifiedRole::System {
                            // If a system message contains a tool use, convert it to text
                            let tool_text = format!(
                                "System message contained a tool call: {{ name: {}, input: {} }}",
                                name, input
                            );
                            if current_role.is_none() {
                                current_role = Some(UnifiedRole::System);
                            }
                            current_message_parts
                                .push(OpenAIMessageContentPart::Text { text: tool_text });
                        } else {
                            // Check if we need to flush due to role change
                            if current_role.is_some() && current_role.as_ref().unwrap() != &msg.role
                            {
                                flush_current_message_with_tool_responses(
                                    &mut openai_messages,
                                    &mut current_role,
                                    &mut current_message_parts,
                                    &mut current_tool_calls,
                                    &tool_results,
                                );
                            }

                            if current_role.is_none() {
                                current_role = Some(msg.role.clone());
                            }
                            current_tool_calls.push(UnifiedToolCall {
                                id: Some(id.clone()),
                                r#type: Some("function".to_string()),
                                function: OpenAIFunctionCall {
                                    name: Some(name.clone()),
                                    arguments: Some(input.to_string()),
                                },
                                index: None,
                            });
                        }
                    }
                    _ => {} // Thinking blocks are ignored in the final request
                }
            }
        }

        // Flush any remaining message
        flush_current_message_with_tool_responses(
            &mut openai_messages,
            &mut current_role,
            &mut current_message_parts,
            &mut current_tool_calls,
            &tool_results,
        );

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

        if let Some(sys_prompt_str) = unified_request.system_prompt {
            if !sys_prompt_str.trim().is_empty() {
                openai_messages.insert(
                    0,
                    UnifiedChatMessage {
                        role: Some("system".to_string()),
                        content: Some(OpenAIMessageContent::Text(sys_prompt_str)),
                        tool_calls: None,
                        tool_call_id: None,
                        reasoning_content: None,
                    },
                );
            }
        }

        let openai_request = OpenAIChatCompletionRequest {
            model: model.to_string(),
            messages: openai_messages,
            stream: Some(unified_request.stream),
            max_tokens: unified_request.max_tokens,
            temperature: unified_request.temperature.map(|t| {
                // Adapt temperature from source protocol to OpenAI range
                adapt_temperature(t, Protocol::Claude, Protocol::OpenAI)
            }),
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
        };

        let mut request_builder = client.post(full_provider_url);
        request_builder = request_builder.header("Content-Type", "application/json");
        if !api_key.is_empty() {
            request_builder =
                request_builder.header("Authorization", format!("Bearer {}", api_key));
        }

        // Try to serialize the request and log it for debugging
        request_builder = request_builder.json(&openai_request);

        #[cfg(debug_assertions)]
        {
            match serde_json::to_string_pretty(&openai_request) {
                Ok(request_json) => {
                    log::debug!("OpenAI request: {}", request_json);
                }
                Err(e) => {
                    log::error!("Failed to serialize OpenAI request: {}", e);
                    // Try to serialize individual parts to identify the issue
                    if let Some(tools) = &openai_request.tools {
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
                    return Err(anyhow::anyhow!("Failed to serialize OpenAI request: {}", e));
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
            .map(|u| UnifiedUsage {
                input_tokens: u.prompt_tokens,
                output_tokens: u.completion_tokens,
                ..Default::default()
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
        openai_chunk: &OpenAIChatCompletionStreamResponse,
        sse_status: &Arc<RwLock<SseStatus>>,
        unified_chunks: &mut Vec<UnifiedStreamChunk>,
    ) {
        if let Ok(mut status) = sse_status.write() {
            if !status.message_start {
                status.message_start = true;
                if let Some(id) = openai_chunk.id.as_ref() {
                    if !id.is_empty() {
                        status.message_id = id.clone();
                    }
                }
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
        tc: &UnifiedToolCall,
        unified_chunks: &mut Vec<UnifiedStreamChunk>,
    ) {
        if let Some(name) = &tc.function.name {
            if !name.is_empty() {
                let tool_id = tc
                    .id
                    .clone()
                    .unwrap_or_else(|| format!("tool_{}", uuid::Uuid::new_v4()));

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
                if let Ok(status) = sse_status.read() {
                    tool_id = status.tool_id.clone();
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

        let usage = openai_chunk
            .usage
            .clone()
            .map(|u| UnifiedUsage {
                input_tokens: u.prompt_tokens,
                output_tokens: u.completion_tokens,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
                tool_use_prompt_tokens: None,
                thoughts_tokens: None,
                cached_content_tokens: None,
                ..Default::default()
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
                    // Send text before the tool tag
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
                // Flush the text part that is definitely not part of a tag
                let text_to_flush = &buffer[..text_to_flush_len];
                unified_chunks.push(UnifiedStreamChunk::Text {
                    delta: text_to_flush.to_string(),
                });
                // Update the buffer to only contain the partial tag (or be empty)
                status.tool_compat_buffer = buffer[text_to_flush_len..].to_string();
            }
        }
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
