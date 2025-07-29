use async_trait::async_trait;
use reqwest::{Client, RequestBuilder};
use serde_json::Value;
use std::sync::{Arc, RwLock};

use super::{BackendAdapter, BackendResponse};
use crate::ccproxy::adapter::{
    backend::update_message_block,
    range_adapter::{adapt_temperature, Protocol},
    unified::{
        SseStatus, UnifiedContentBlock, UnifiedRequest, UnifiedResponse, UnifiedRole,
        UnifiedStreamChunk, UnifiedToolChoice, UnifiedUsage,
    },
};
use crate::ccproxy::claude::{
    ClaudeNativeContentBlock, ClaudeNativeMessage, ClaudeNativeRequest, ClaudeNativeResponse,
    ClaudeNativeTool, ClaudeStreamEvent, ClaudeToolChoice,
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
}

#[async_trait]
impl BackendAdapter for ClaudeBackendAdapter {
    async fn adapt_request(
        &self,
        client: &Client,
        unified_request: &UnifiedRequest,
        api_key: &str,
        base_url: &str,
        model: &str,
    ) -> Result<RequestBuilder, anyhow::Error> {
        // Validate tool call sequence before processing
        self.validate_tool_call_sequence(&unified_request.messages)?;

        let mut claude_messages = Vec::new();

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
                        content_blocks.push(ClaudeNativeContentBlock::Text { text: text.clone() });
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

        let claude_request = ClaudeNativeRequest {
            model: model.to_string(),
            messages: claude_messages,
            system: unified_request.system_prompt.clone(),
            max_tokens: unified_request.max_tokens.unwrap_or(1024),
            stream: Some(unified_request.stream),
            temperature: unified_request.temperature.map(|t| {
                // Adapt temperature from source protocol to Claude range
                adapt_temperature(t, Protocol::OpenAI, Protocol::Claude)
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
            thinking: unified_request.thinking.as_ref().map(|t| {
                crate::ccproxy::types::claude::ClaudeThinking {
                    thinking_type: "enabled".to_string(),
                    budget_tokens: t.budget_tokens,
                }
            }),
            cache_control: unified_request.cache_control.as_ref().map(|c| {
                crate::ccproxy::types::claude::ClaudeCacheControl {
                    cache_type: c.cache_type.clone(),
                    ttl: c.ttl.clone(),
                }
            }),
        };

        let mut request_builder = client.post(format!("{}/messages", base_url));
        request_builder = request_builder.header("Content-Type", "application/json");
        request_builder = request_builder.header("x-api-key", api_key);
        request_builder = request_builder.header("anthropic-version", "2023-06-01");
        if unified_request.tools.is_some() || unified_request.tool_choice.is_some() {
            request_builder = request_builder.header("anthropic-beta", "tools-2024-04-04");
        }
        request_builder = request_builder.json(&claude_request);

        Ok(request_builder)
    }

    async fn adapt_response(
        &self,
        backend_response: BackendResponse,
    ) -> Result<UnifiedResponse, anyhow::Error> {
        let claude_response: ClaudeNativeResponse = serde_json::from_slice(&backend_response.body)?;

        let mut content_blocks = Vec::new();
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

        let usage = claude_response
            .usage
            .map(|u| UnifiedUsage {
                input_tokens: u.input_tokens,
                output_tokens: u.output_tokens,
                cache_creation_input_tokens: u.cache_creation_input_tokens,
                cache_read_input_tokens: u.cache_read_input_tokens,
                tool_use_prompt_tokens: None,
                thoughts_tokens: None,
                cached_content_tokens: None,
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
        let chunk_str = String::from_utf8_lossy(&chunk);
        let mut unified_chunks = Vec::new();
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
                                        cache_creation_input_tokens: None,
                                        cache_read_input_tokens: None,
                                        tool_use_prompt_tokens: None,
                                        thoughts_tokens: None,
                                        cached_content_tokens: None,
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
                                        serde_json::to_value(block.clone()).unwrap_or(Value::Null)
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
                                            update_message_block(&mut status, "text".to_string());
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
                                        let tool_id = block.id.clone().unwrap_or(
                                            format!("tool_{}", uuid::Uuid::new_v4()).to_string(),
                                        );
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
                                            update_message_block(&mut status, "text".to_string());
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
                                            unified_chunks
                                                .push(UnifiedStreamChunk::Thinking { delta: text });
                                        }
                                    }
                                    Some("input_json_delta") => {
                                        let tool_id = if let Ok(mut status) = sse_status.write() {
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
                                            unified_chunks.push(UnifiedStreamChunk::ToolUseDelta {
                                                id: tool_id,
                                                delta: partial_json,
                                            });
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
                                            cache_creation_input_tokens: None,
                                            cache_read_input_tokens: None,
                                            tool_use_prompt_tokens: None,
                                            thoughts_tokens: None,
                                            cached_content_tokens: None,
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

        Ok(unified_chunks)
    }
}
