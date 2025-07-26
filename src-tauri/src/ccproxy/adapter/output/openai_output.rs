use super::OutputAdapter;
use crate::ccproxy::adapter::unified::{SseStatus, UnifiedResponse, UnifiedStreamChunk};
use crate::ccproxy::types::openai::{
    OpenAIChatCompletionChoice, OpenAIChatCompletionResponse, OpenAIMessageContent, OpenAIUsage,
    UnifiedChatMessage,
};

use serde_json::json;
use std::convert::Infallible;
use warp::{reply::json, sse::Event};

pub struct OpenAIOutputAdapter;

impl OutputAdapter for OpenAIOutputAdapter {
    fn adapt_response(&self, response: UnifiedResponse) -> Result<impl warp::Reply, anyhow::Error> {
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
        };

        let openai_response = OpenAIChatCompletionResponse {
            id: response.id,
            object: "chat.completion".to_string(),
            created: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| anyhow::anyhow!("Failed to get system time: {}", e))?
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
        _sse_status: std::sync::Arc<std::sync::RwLock<SseStatus>>,
    ) -> Result<Vec<Event>, Infallible> {
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
                Ok(vec![Event::default()
                    .id(id)
                    .event("message_start")
                    .data(data.to_string())])
            }
            UnifiedStreamChunk::Thinking { delta } => {
                let data = json!({ "choices": [{ "delta": { "reasoning_content": delta } }] });
                Ok(vec![Event::default().data(data.to_string())])
            }
            UnifiedStreamChunk::Text { delta } => {
                let data = json!({ "choices": [{ "delta": { "content": delta } }] });
                Ok(vec![Event::default().data(data.to_string())])
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
                Ok(vec![Event::default().data(data.to_string())])
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
                Ok(vec![Event::default().data(data.to_string())])
            }
            UnifiedStreamChunk::ToolUseEnd { id: _ } => {
                Ok(vec![Event::default().data("")]) // No direct OpenAI equivalent for tool_use_end delta
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
                Ok(vec![Event::default().data(data.to_string())])
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
