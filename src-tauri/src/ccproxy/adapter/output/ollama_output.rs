use super::OutputAdapter;
use crate::ccproxy::{
    adapter::unified::{SseStatus, UnifiedResponse, UnifiedStreamChunk},
    helper::sse::Event,
    types::ollama::{
        OllamaChatCompletionResponse, OllamaFunctionCall, OllamaMessage, OllamaStreamResponse,
        OllamaToolCall,
    },
};

use anyhow::Result;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use std::convert::Infallible;
use std::sync::{Arc, RwLock};

pub struct OllamaOutputAdapter;

impl OutputAdapter for OllamaOutputAdapter {
    fn adapt_response(
        &self,
        response: UnifiedResponse,
        _sse_status: Arc<RwLock<SseStatus>>,
    ) -> Result<Response, anyhow::Error> {
        let mut text_content = String::new();
        let mut tool_calls = Vec::new();
        let mut reasoning_content = String::new();

        for block in response.content {
            match block {
                crate::ccproxy::adapter::unified::UnifiedContentBlock::Text { text } => {
                    text_content.push_str(&text);
                }
                crate::ccproxy::adapter::unified::UnifiedContentBlock::Thinking { thinking } => {
                    reasoning_content.push_str(&thinking);
                }
                crate::ccproxy::adapter::unified::UnifiedContentBlock::ToolUse {
                    id: _, // Ollama does not use tool ID
                    name,
                    input,
                } => {
                    tool_calls.push(OllamaToolCall {
                        function: OllamaFunctionCall {
                            name,
                            arguments: input,
                        },
                    });
                }
                _ => {}
            }
        }

        let message = OllamaMessage {
            role: "assistant".to_string(),
            content: text_content,
            images: None,
            thinking: if reasoning_content.is_empty() {
                None
            } else {
                Some(reasoning_content)
            },
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
            tool_name: None,
        };

        let ollama_response = OllamaChatCompletionResponse {
            model: response.model,
            created_at: chrono::Utc::now().to_rfc3339(),
            message,
            done: true,
            total_duration: response.usage.total_duration,
            load_duration: response.usage.load_duration,
            prompt_eval_count: Some(response.usage.input_tokens as u32),
            prompt_eval_duration: response.usage.prompt_eval_duration,
            eval_count: Some(response.usage.output_tokens as u32),
            eval_duration: response.usage.eval_duration,
        };

        Ok(Json(ollama_response).into_response())
    }

    fn adapt_stream_chunk(
        &self,
        chunk: UnifiedStreamChunk,
        sse_status: Arc<RwLock<SseStatus>>,
    ) -> Result<Vec<Event>, Infallible> {
        let model_id = if let Ok(status) = sse_status.read() {
            status.model_id.clone()
        } else {
            crate::ccproxy::helper::get_msg_id()
        };

        let stream_response = match chunk {
            UnifiedStreamChunk::Text { delta } => Some(OllamaStreamResponse {
                model: model_id,
                created_at: chrono::Utc::now().to_rfc3339(),
                message: OllamaMessage {
                    role: "assistant".to_string(),
                    content: delta,
                    ..Default::default()
                },
                done: false,
                ..Default::default()
            }),

            UnifiedStreamChunk::ToolUseStart { id: _, name, .. } => {
                if let Ok(mut status) = sse_status.write() {
                    status.tool_name = Some(name.clone());
                    status.tool_arguments = Some(String::new());
                }

                Some(OllamaStreamResponse {
                    model: model_id,
                    created_at: chrono::Utc::now().to_rfc3339(),
                    message: OllamaMessage {
                        role: "assistant".to_string(),
                        content: "".to_string(),
                        tool_calls: Some(vec![OllamaToolCall {
                            function: OllamaFunctionCall {
                                name,
                                arguments: json!({}),
                            },
                        }]),
                        ..Default::default()
                    },
                    done: false,
                    ..Default::default()
                })
            }

            UnifiedStreamChunk::ToolUseDelta { delta, .. } => {
                let (tool_name, _) = if let Ok(mut status) = sse_status.write() {
                    (status.tool_name.take(), status.tool_arguments.take())
                } else {
                    (None, None)
                };

                if let Some(name) = tool_name {
                    let arguments: serde_json::Value = serde_json::from_str(&delta)
                        .unwrap_or_else(|_| json!({ "partial_data": delta }));

                    Some(OllamaStreamResponse {
                        model: model_id,
                        created_at: chrono::Utc::now().to_rfc3339(),
                        message: OllamaMessage {
                            role: "assistant".to_string(),
                            content: "".to_string(),
                            tool_calls: Some(vec![OllamaToolCall {
                                function: OllamaFunctionCall { name, arguments },
                            }]),
                            ..Default::default()
                        },
                        done: false,
                        ..Default::default()
                    })
                } else {
                    None
                }
            }

            UnifiedStreamChunk::ToolUseEnd { .. } => {
                let (tool_name, tool_arguments) = if let Ok(mut status) = sse_status.write() {
                    (status.tool_name.take(), status.tool_arguments.take())
                } else {
                    (None, None)
                };

                if let (Some(name), Some(arguments_str)) = (tool_name, tool_arguments) {
                    let arguments: serde_json::Value = serde_json::from_str(&arguments_str)
                        .map_err(|e| {
                            log::error!(
                                "Failed to parse tool arguments, error: {}, rawString: {}",
                                e,
                                arguments_str
                            );
                            e
                        })
                        .unwrap_or_else(|_| json!({ "partial_data": arguments_str }));

                    Some(OllamaStreamResponse {
                        model: model_id,
                        created_at: chrono::Utc::now().to_rfc3339(),
                        message: OllamaMessage {
                            role: "assistant".to_string(),
                            content: "".to_string(),
                            tool_calls: Some(vec![OllamaToolCall {
                                function: OllamaFunctionCall { name, arguments },
                            }]),
                            ..Default::default()
                        },
                        done: false,
                        ..Default::default()
                    })
                } else {
                    None
                }
            }

            UnifiedStreamChunk::MessageStop { usage, .. } => {
                let output = if let Ok(status) = sse_status.read() {
                    (status.text_delta_count
                        + status.tool_delta_count
                        + status.thinking_delta_count) as u64
                } else {
                    1
                };
                Some(OllamaStreamResponse {
                    model: model_id,
                    created_at: chrono::Utc::now().to_rfc3339(),
                    message: OllamaMessage {
                        role: "assistant".to_string(),
                        content: "".to_string(),
                        ..Default::default()
                    },
                    done: true,
                    total_duration: Some(usage.total_duration.unwrap_or(0)),
                    load_duration: Some(usage.load_duration.unwrap_or(0)),
                    prompt_eval_count: Some(usage.input_tokens.max(1) as u32),
                    prompt_eval_duration: Some(usage.prompt_eval_duration.unwrap_or(0)),
                    eval_count: Some(usage.output_tokens.max(output) as u32),
                    eval_duration: Some(usage.eval_duration.unwrap_or(0)),
                })
            }

            UnifiedStreamChunk::Error { message } => {
                let error_response = json!({ "error": message });
                return Ok(vec![Event::default().text(error_response.to_string())]);
            }

            _ => None,
        };

        if let Some(response) = stream_response {
            let json_str = serde_json::to_string(&response)
                .map_err(|e| {
                    log::error!(
                        "Failed to serialize Ollama response, error: {}, response: {:?}",
                        e.to_string(),
                        &response
                    );
                })
                .unwrap_or_default();
            Ok(vec![Event::default().text(json_str)])
        } else {
            Ok(vec![])
        }
    }
}
