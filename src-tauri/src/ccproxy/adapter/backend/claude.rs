use crate::ccproxy::adapter::backend::update_message_block;
use crate::ccproxy::adapter::unified::SseStatus;
use async_trait::async_trait;
use reqwest::{Client, RequestBuilder};
use serde_json::Value;
use std::sync::{Arc, RwLock};

use super::{BackendAdapter, BackendResponse};
use crate::ccproxy::adapter::unified::{
    UnifiedContentBlock, UnifiedRequest, UnifiedResponse, UnifiedRole, UnifiedStreamChunk,
    UnifiedToolChoice, UnifiedUsage,
};
use crate::ccproxy::types::claude::{
    ClaudeNativeContentBlock, ClaudeNativeMessage, ClaudeNativeRequest, ClaudeNativeResponse,
    ClaudeNativeTool, ClaudeStreamEvent, ClaudeToolChoice,
};

pub struct ClaudeBackendAdapter;

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
            temperature: unified_request.temperature,
            top_p: unified_request.top_p,
            top_k: unified_request.top_k,
            tools: claude_tools,
            tool_choice: claude_tool_choice,
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

        for line in chunk_str.lines() {
            if line.starts_with("event: ") {
                let event_type = line["event: ".len()..].trim();
                // Assuming data: always follows event:
                let data_line = chunk_str.lines().find(|l| l.starts_with("data: "));
                if let Some(data_line) = data_line {
                    let data_str = data_line["data: ".len()..].trim();
                    let claude_event: ClaudeStreamEvent = serde_json::from_str(data_str)?;

                    match event_type {
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
                                            update_message_block(status, "text".to_string());
                                        }
                                    }
                                    "thinking" => {
                                        if let Ok(mut status) = sse_status.write() {
                                            status.thinking_delta_count += 1;
                                            update_message_block(status, "thinking".to_string());
                                        }
                                    }
                                    // 对于 claude -> claude 来说，我们不需要处理 tool_use 和server_tool_use 事件，
                                    // 他们被包含在 content_block_start 事件中
                                    // "tool_use" => {
                                    //     if let Ok(mut status) = sse_status.write() {
                                    //         status.tool_id = block.id.clone().unwrap_or_default();
                                    //         update_message_block(status, "tool_use".to_string());
                                    //     }
                                    //     unified_chunks.push(UnifiedStreamChunk::ToolUseStart {
                                    //         tool_type: "tool_use".to_string(),
                                    //         id: block.id.unwrap_or_default(),
                                    //         name: block.name.unwrap_or_default(),
                                    //     });
                                    // }
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
                                            update_message_block(status, "text".to_string());
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
                                                    status,
                                                    "thinking".to_string(),
                                                );
                                            }
                                            unified_chunks
                                                .push(UnifiedStreamChunk::Thinking { delta: text });
                                        }
                                    }
                                    Some("input_json_delta") => {
                                        let tool_id = if let Ok(status) = sse_status.write() {
                                            let id = status.tool_id.clone();
                                            update_message_block(status, "tool_use".to_string());
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
                            if let Some(block) = claude_event.content_block {
                                match block.block_type.as_str() {
                                    "text" => {
                                        if let Ok(mut status) = sse_status.write() {
                                            status.text_delta_count += 1;
                                            update_message_block(status, "text".to_string());
                                        }
                                    }
                                    "thinking" => {
                                        if let Ok(mut status) = sse_status.write() {
                                            status.thinking_delta_count += 1;
                                            update_message_block(status, "thinking".to_string());
                                        }
                                    }
                                    // "tool_use" => {
                                    //     if let Ok(mut status) = sse_status.write() {
                                    //         unified_chunks.push(UnifiedStreamChunk::ToolUseEnd {
                                    //             id: status.tool_id.clone(),
                                    //         });
                                    //         status.tool_id = "".to_string();
                                    //         update_message_block(status, "tool_use".to_string());
                                    //     }
                                    // }
                                    _ => {}
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
