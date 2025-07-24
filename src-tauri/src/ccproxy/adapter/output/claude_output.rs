use super::OutputAdapter;
use crate::ccproxy::adapter::unified::{SseStatus, UnifiedResponse, UnifiedStreamChunk};
use serde_json::json;
use std::convert::Infallible;
use std::sync::{Arc, RwLock};
use warp::{reply::json, sse::Event};

pub struct ClaudeOutputAdapter;

use crate::ccproxy::types::claude::{
    ClaudeNativeContentBlock, ClaudeNativeResponse, ClaudeNativeUsage,
};

impl OutputAdapter for ClaudeOutputAdapter {
    fn adapt_response(&self, response: UnifiedResponse) -> Result<impl warp::Reply, anyhow::Error> {
        let content = response
            .content
            .into_iter()
            .map(|c| match c {
                crate::ccproxy::adapter::unified::UnifiedContentBlock::Text { text } => {
                    ClaudeNativeContentBlock::Text { text }
                }
                crate::ccproxy::adapter::unified::UnifiedContentBlock::Thinking { thinking } => {
                    ClaudeNativeContentBlock::Thinking { thinking }
                }
                crate::ccproxy::adapter::unified::UnifiedContentBlock::ToolUse {
                    id,
                    name,
                    input,
                } => ClaudeNativeContentBlock::ToolUse { id, name, input },
                crate::ccproxy::adapter::unified::UnifiedContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } => ClaudeNativeContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error: Some(is_error),
                },
                crate::ccproxy::adapter::unified::UnifiedContentBlock::Image {
                    media_type,
                    data,
                } => ClaudeNativeContentBlock::Image {
                    source: crate::ccproxy::types::claude::ClaudeImageSource {
                        source_type: "base64".to_string(),
                        media_type,
                        data,
                    },
                },
            })
            .collect();

        let claude_response = ClaudeNativeResponse {
            id: response.id,
            response_type: "message".to_string(),
            role: Some("assistant".to_string()),
            content,
            model: Some(response.model),
            stop_reason: response.stop_reason,
            usage: Some(ClaudeNativeUsage {
                input_tokens: response.usage.input_tokens,
                output_tokens: response.usage.output_tokens,
            }),
            error: None,
        };

        Ok(json(&claude_response))
    }

    fn adapt_stream_chunk(
        &self,
        chunk: UnifiedStreamChunk,
        sse_status: Arc<RwLock<SseStatus>>,
    ) -> Result<Vec<Event>, Infallible> {
        match chunk {
            UnifiedStreamChunk::MessageStart { id, model, usage } => {
                Ok(vec![gen_message_start_event(id, model, usage.input_tokens)])
            }
            UnifiedStreamChunk::Thinking { delta } => {
                Ok(vec![Event::default().event("content_block_delta").data(
                    json!({
                        "type": "content_block_delta",
                        "index": 0,
                        "delta": { "type": "thinking_delta", "thinking": delta }
                    })
                    .to_string(),
                )])
            }
            UnifiedStreamChunk::Text { delta } => {
                let message_index = if let Ok(status) = sse_status.read() {
                    status.message_index
                } else {
                    0
                };

                Ok(vec![Event::default().event("content_block_delta").data(
                    json!({
                        "type": "content_block_delta",
                        "index":  message_index,
                        "delta": { "type": "text_delta", "text": delta }
                    })
                    .to_string(),
                )])
            }
            UnifiedStreamChunk::ToolUseDelta { id: _, delta } => {
                let message_index = if let Ok(status) = sse_status.read() {
                    status.message_index
                } else {
                    0
                };
                Ok(vec![Event::default().event("content_block_delta").data(
                    json!({
                        "type": "content_block_delta",
                        "index": message_index,
                        "delta": { "type": "input_json_delta", "partial_json": delta }
                    })
                    .to_string(),
                )])
            }
            UnifiedStreamChunk::ContentBlockStart { index, block } => {
                Ok(vec![Event::default().event("content_block_start").data(
                    json!({
                        "type": "content_block_start",
                        "index": index,
                        "content_block": block
                    })
                    .to_string(),
                )])
            }

            UnifiedStreamChunk::ContentBlockStop { index } => {
                Ok(vec![Event::default().event("content_block_stop").data(
                    json!({
                        "type": "content_block_stop",
                        "index": index,
                    })
                    .to_string(),
                )])
            }

            UnifiedStreamChunk::MessageStop {
                stop_reason: _,
                usage,
            } => {
                let message_index = if let Ok(status) = sse_status.read() {
                    status.message_index
                } else {
                    0
                };
                Ok(vec![
                    Event::default().event("content_block_stop").data(
                        json!({
                            "type": "content_block_stop",
                            "index": message_index
                        })
                        .to_string(),
                    ),
                    Event::default().event("message_delta").data(
                        json!({
                            "type": "message_delta",
                            "delta": {
                                "stop_reason": "end_turn".to_string()
                            },
                            "usage": {
                                "output_tokens": usage.output_tokens
                            }
                        })
                        .to_string(),
                    ),
                    Event::default().event("message_stop"),
                ])
            }
            UnifiedStreamChunk::Error { message } => {
                let data = json!({
                    "type": "error",
                    "error": {
                        "type": "internal_error",
                        "message": message
                    }
                });
                Ok(vec![Event::default().event("error").data(data.to_string())])
            }
            // Claude 的工具开始信息包含在 content_block_start 事件中，如：
            // event: content_block_start
            // data: {"type":"content_block_start","index":1,"content_block":{"type":"server_tool_use","id":"srvtoolu_014hJH82Qum7Td6UV8gDXThB","name":"web_search","input":{}}}
            // 所以我们不处理 ToolUseStart 和 ToolUseEnd

            // UnifiedStreamChunk::ToolUseStart {
            //     tool_type,
            //     id,
            //     name,
            // } => {
            //     let message_index = if let Ok(status) = sse_status.read() {
            //         status.message_index
            //     } else {
            //         0
            //     };

            //     Ok(vec![Event::default().event("content_block_start").data(
            //         json!({
            //             "type": "content_block_start",
            //             "index": message_index,
            //             "content_block": {
            //                 "type": tool_type,
            //                 "id": id,
            //                 "name": name,
            //                 "input": {}
            //             }
            //         })
            //         .to_string(),
            //     )])
            // }
            // UnifiedStreamChunk::ToolUseEnd { id: _ } => {
            //     let message_index = if let Ok(status) = sse_status.read() {
            //         status.message_index
            //     } else {
            //         0
            //     };
            //     let data = json!({ "type": "content_block_stop", "index": message_index });
            //     Ok(vec![Event::default()
            //         .event("content_block_stop")
            //         .data(data.to_string())])
            // }
            _ => Ok(vec![]),
        }
    }
}

fn gen_message_start_event(id: String, model: String, input_token: u64) -> Event {
    Event::default().event("message_start").data(
        json!({
            "type": "message_start",
            "message": {
                "id": id,
                "type": "message",
                "role": "assistant",
                "content": [],
                "model": model,
                "stop_reason": null,
                "stop_sequence": null,
                "usage": {
                    "input_tokens": input_token,
                    "output_tokens": 0
                }
            }
        })
        .to_string(),
    )
}
