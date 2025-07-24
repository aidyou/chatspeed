use super::OutputAdapter;
use crate::ccproxy::adapter::unified::{SseStatus, UnifiedResponse, UnifiedStreamChunk};
use serde_json::json;
use std::convert::Infallible;
use std::sync::{Arc, RwLock};
use warp::{reply::json, sse::Event};

pub struct OpenAIOutputAdapter;

use crate::ccproxy::types::openai::{
    OpenAIChatCompletionChoice, OpenAIChatCompletionResponse, OpenAIMessageContent, OpenAIUsage,
    UnifiedChatMessage,
};

impl OutputAdapter for OpenAIOutputAdapter {
    fn adapt_response(&self, response: UnifiedResponse) -> Result<impl warp::Reply, anyhow::Error> {
        let mut text_content = String::new();
        let mut reasoning_content: Option<String> = None;
        let mut tool_calls: Vec<crate::ccproxy::types::openai::UnifiedToolCall> = Vec::new();

        for c in response.content {
            match c {
                crate::ccproxy::adapter::unified::UnifiedContentBlock::Text { text } => {
                    text_content.push_str(&text);
                }
                crate::ccproxy::adapter::unified::UnifiedContentBlock::Thinking { thinking } => {
                    reasoning_content = Some(thinking);
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
                crate::ccproxy::adapter::unified::UnifiedContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error: _,
                } => {
                    // OpenAI doesn't have a direct equivalent for tool results in assistant responses.
                    // We'll append it as text content for now.
                    text_content.push_str(&format!("\nTool Result ({}) {}", tool_use_id, content));
                }
                crate::ccproxy::adapter::unified::UnifiedContentBlock::Image {
                    media_type, ..
                } => {
                    // OpenAI doesn't have a direct equivalent for image content in assistant responses.
                    // We'll append it as text content for now.
                    text_content
                        .push_str(&format!("\nImage ({}) [base64 data omitted]", media_type));
                }
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
        };

        let openai_response = OpenAIChatCompletionResponse {
            id: response.id,
            object: "chat.completion".to_string(),
            created: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            model: response.model,
            choices: vec![choice],
            usage: Some(OpenAIUsage {
                prompt_tokens: response.usage.input_tokens,
                completion_tokens: response.usage.output_tokens,
                total_tokens: response.usage.input_tokens + response.usage.output_tokens,
            }),
        };

        Ok(json(&openai_response))
    }

    fn adapt_stream_chunk(
        &self,
        chunk: UnifiedStreamChunk,
        _sse_status: Arc<RwLock<SseStatus>>,
    ) -> Result<Vec<Event>, Infallible> {
        let event = Event::default();
        match chunk {
            UnifiedStreamChunk::MessageStart { id, model, usage } => {
                // Convert to OpenAI-compatible message_start event
                let data = json!({
                    "id": id.clone(),
                    "model": model,
                    "usage": {
                        "input_tokens": usage.input_tokens,
                        "output_tokens": usage.output_tokens
                    }
                });
                Ok(vec![event
                    .id(id)
                    .event("message_start")
                    .data(data.to_string())])
            }
            UnifiedStreamChunk::Thinking { delta } => {
                let data = json!({ "choices": [{ "delta": { "reasoning_content": delta } }] });
                Ok(vec![event.data(data.to_string())])
            }
            UnifiedStreamChunk::Text { delta } => {
                let data = json!({ "choices": [{ "delta": { "content": delta } }] });
                Ok(vec![event.data(data.to_string())])
            }
            UnifiedStreamChunk::ToolUseStart {
                tool_type: _,
                id,
                name,
            } => {
                let data = json!({
                    "choices": [{
                        "delta": {
                            "tool_calls": [{
                                "index": 0,
                                "id": id,
                                "function": {
                                    "name": name,
                                    "arguments": ""
                                }
                            }]
                        }
                    }]
                });
                Ok(vec![event.data(data.to_string())])
            }
            UnifiedStreamChunk::ToolUseDelta { id: _, delta } => {
                let data = json!({
                    "choices": [{
                        "delta": {
                            "tool_calls": [{
                                "index": 0,
                                "function": {
                                    "arguments": delta
                                }
                            }]
                        }
                    }]
                });
                Ok(vec![event.data(data.to_string())])
            }
            UnifiedStreamChunk::ToolUseEnd { id: _ } => {
                Ok(vec![event.data("")]) // No direct OpenAI equivalent for tool_use_end delta
            }
            UnifiedStreamChunk::MessageStop { stop_reason, usage } => {
                let data = json!({
                    "choices": [{
                        "finish_reason": stop_reason
                    }],
                    "usage": {
                        "prompt_tokens": usage.input_tokens,
                        "completion_tokens": usage.output_tokens,
                        "total_tokens": usage.input_tokens + usage.output_tokens
                    }
                });
                Ok(vec![event.data(data.to_string())])
            }
            UnifiedStreamChunk::Error { message } => {
                // Map internal errors to a data event for the client
                let data = json!({ "error": { "message": message } });
                Ok(vec![event.data(data.to_string())])
            }
            _ => Ok(vec![]),
        }
    }
}
