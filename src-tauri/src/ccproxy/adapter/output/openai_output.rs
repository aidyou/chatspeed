use super::OutputAdapter;
use crate::ccproxy::adapter::unified::{SseStatus, UnifiedResponse, UnifiedStreamChunk};
use crate::ccproxy::helper::get_msg_id;
use crate::ccproxy::helper::sse::Event;
use crate::ccproxy::types::openai::{
    OpenAIChatCompletionChoice, OpenAIChatCompletionResponse, OpenAIMessageContent, OpenAIUsage,
    UnifiedChatMessage,
};

use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use std::convert::Infallible;
use std::sync::{Arc, RwLock};

pub struct OpenAIOutputAdapter;

impl OutputAdapter for OpenAIOutputAdapter {
    fn adapt_response(
        &self,
        response: UnifiedResponse,
        sse_status: Arc<RwLock<SseStatus>>,
    ) -> Result<Response, anyhow::Error> {
        let mut text_content = String::new();
        let mut reasoning_content: Option<String> = None;
        let mut tool_calls: Vec<crate::ccproxy::types::openai::UnifiedToolCall> = Vec::new();

        for c in response.content {
            match c {
                crate::ccproxy::adapter::unified::UnifiedContentBlock::Thinking { thinking } => {
                    reasoning_content = Some(thinking);
                }
                crate::ccproxy::adapter::unified::UnifiedContentBlock::Text { text } => {
                    text_content.push_str(&text);
                }
                crate::ccproxy::adapter::unified::UnifiedContentBlock::ToolUse {
                    id,
                    name,
                    input,
                } => {
                    tool_calls.push(crate::ccproxy::types::openai::UnifiedToolCall {
                        id: Some(id),
                        r#type: Some("function".to_string()),
                        function: crate::ccproxy::types::openai::OpenAIFunctionCall {
                            name: Some(name),
                            arguments: Some(input.to_string()),
                        },
                        index: None,
                    });
                }
                _ => {}
            }
        }

        let choice = OpenAIChatCompletionChoice {
            index: 0,
            message: UnifiedChatMessage {
                role: Some("assistant".to_string()),
                content: Some(OpenAIMessageContent::Text(text_content)),
                tool_calls: if tool_calls.is_empty() {
                    None
                } else {
                    Some(tool_calls)
                },
                reasoning_content,
                ..Default::default()
            },
            finish_reason: response.stop_reason,
            logprobs: None, // Add missing logprobs field
        };

        let model = if let Ok(status) = sse_status.read() {
            status.model_id.clone()
        } else {
            response.model
        };
        let response_id = if let Ok(status) = sse_status.read() {
            status.message_id.clone()
        } else {
            response.id.clone()
        };

        let openai_response = OpenAIChatCompletionResponse {
            id: response_id,
            object: "chat.completion".to_string(),
            created: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| anyhow::anyhow!("Failed to get system time: {}", e))?
                .as_secs(),
            model: model,
            choices: vec![choice],
            usage: Some(OpenAIUsage {
                prompt_tokens: response.usage.input_tokens,
                completion_tokens: response.usage.output_tokens,
                total_tokens: response.usage.input_tokens + response.usage.output_tokens,
            }),
        };

        Ok(Json(openai_response).into_response())
    }

    fn adapt_stream_chunk(
        &self,
        chunk: UnifiedStreamChunk,
        sse_status: std::sync::Arc<std::sync::RwLock<SseStatus>>,
    ) -> Result<Vec<Event>, Infallible> {
        match chunk {
            UnifiedStreamChunk::MessageStart {
                id,
                model,
                usage: _,
            } => {
                let model_id = if let Ok(status) = sse_status.read() {
                    status.model_id.clone()
                } else {
                    model
                };
                // Convert to OpenAI-compatible message_start event
                let data = json!({
                    "id": id.clone(),
                    "model": model_id,
                    "object":"chat.completion.chunk",
                    "created": chrono::Utc::now().timestamp(),
                    "choices": [
                      {
                        "index": 0,
                        "delta": {
                          "role": "assistant",
                          "content": "",
                        },
                      }
                    ]
                });
                Ok(vec![Event::default().data(data.to_string())])
            }
            UnifiedStreamChunk::Thinking { delta } => {
                let message_id = if let Ok(status) = sse_status.read() {
                    status.message_id.clone()
                } else {
                    get_msg_id()
                };
                let model = if let Ok(status) = sse_status.read() {
                    status.model_id.clone()
                } else {
                    String::new()
                };
                let data = json!({
                    "id": message_id,
                    "model": model,
                    "object":"chat.completion.chunk",
                    "created": chrono::Utc::now().timestamp(),
                    "choices": [{
                        "index":0,
                        "delta": {
                            "reasoning_content": delta,
                        }
                    }]
                });
                Ok(vec![Event::default().data(data.to_string())])
            }
            UnifiedStreamChunk::Text { delta } => {
                let message_id = if let Ok(status) = sse_status.read() {
                    status.message_id.clone()
                } else {
                    get_msg_id()
                };
                let model = if let Ok(status) = sse_status.read() {
                    status.model_id.clone()
                } else {
                    String::new()
                };
                let data = json!({
                    "id": message_id,
                    "model": model,
                    "object":"chat.completion.chunk",
                    "created": chrono::Utc::now().timestamp(),
                    "choices": [{
                        "index":0,
                        "delta": {
                            "content": delta,
                        }
                    }]
                });
                Ok(vec![Event::default().data(data.to_string())])
            }
            UnifiedStreamChunk::ToolUseStart {
                tool_type: _,
                id,
                name,
            } => {
                let message_index = if let Ok(status) = sse_status.read() {
                    status.message_index
                } else {
                    0
                };
                let message_id = if let Ok(status) = sse_status.read() {
                    status.message_id.clone()
                } else {
                    get_msg_id()
                };
                let model = if let Ok(status) = sse_status.read() {
                    status.model_id.clone()
                } else {
                    String::new()
                };
                let data = json!({
                    "id": message_id,
                    "model": model,
                    "object":"chat.completion.chunk",
                    "created": chrono::Utc::now().timestamp(),
                    "choices": [{
                        "index":0,
                        "delta": {
                            "tool_calls": [{
                                "index": message_index,
                                "id": id,
                                "function": {
                                    "name": name,
                                    "arguments": ""
                                }
                            }]
                        },
                    }]
                });
                Ok(vec![Event::default().data(data.to_string())])
            }
            UnifiedStreamChunk::ToolUseDelta { id: _, delta } => {
                let message_index = if let Ok(status) = sse_status.read() {
                    status.message_index
                } else {
                    0
                };
                let message_id = if let Ok(status) = sse_status.read() {
                    status.message_id.clone()
                } else {
                    get_msg_id()
                };
                let model = if let Ok(status) = sse_status.read() {
                    status.model_id.clone()
                } else {
                    String::new()
                };
                let data = json!({
                    "id": message_id,
                    "model": model,
                    "object":"chat.completion.chunk",
                    "created": chrono::Utc::now().timestamp(),
                    "choices": [{
                        "index":0,
                        "delta": {
                            "tool_calls": [{
                                "index": message_index,
                                "function": {
                                    "arguments": delta
                                }
                            }]
                        },
                    }]
                });
                Ok(vec![Event::default().data(data.to_string())])
            }
            UnifiedStreamChunk::ToolUseEnd { id: _ } => Ok(vec![]),
            UnifiedStreamChunk::MessageStop { stop_reason, usage } => {
                let message_id = if let Ok(status) = sse_status.read() {
                    status.message_id.clone()
                } else {
                    get_msg_id()
                };
                let model = if let Ok(status) = sse_status.read() {
                    status.model_id.clone()
                } else {
                    String::new()
                };
                let input_tokens = if usage.input_tokens > 0 {
                    usage.input_tokens
                } else {
                    99
                };
                let output_tokens = if usage.output_tokens > 0 {
                    usage.output_tokens
                } else {
                    if let Ok(status) = sse_status.read() {
                        (status.text_delta_count
                            + status.tool_delta_count
                            + status.thinking_delta_count) as u64
                    } else {
                        99
                    }
                };
                let has_tool = if let Ok(status) = sse_status.read() {
                    if !status.tool_id.is_empty()
                        || status.tool_delta_count > 0
                        || stop_reason == "tool_use"
                        || stop_reason == "tool_calls"
                    {
                        true
                    } else {
                        false
                    }
                } else {
                    false
                };

                let data = json!({
                    "id": message_id,
                    "model": model,
                    "object":"chat.completion.chunk",
                    "created": chrono::Utc::now().timestamp(),
                    "choices": [{
                        "index":0,
                        "delta": {},
                        "finish_reason": if has_tool {
                            "tool_calls"
                        } else {
                            &stop_reason
                        }
                    }],
                    "usage": {
                        "prompt_tokens": input_tokens,
                        "completion_tokens": output_tokens,
                        "total_tokens": input_tokens + output_tokens
                    }
                });
                Ok(vec![
                    Event::default().data(data.to_string()),
                    Event::default().data("[DONE]"),
                ])
            }
            UnifiedStreamChunk::Error { message } => {
                // Map internal errors to a data event for the client
                let data = json!({ "error": { "message": message } });
                Ok(vec![Event::default().data(data.to_string())])
            }
            _ => Ok(vec![]),
        }
    }
}
