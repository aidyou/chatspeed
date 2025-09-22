use super::OutputAdapter;
use crate::ccproxy::{
    adapter::unified::{SseStatus, UnifiedFunctionCallPart, UnifiedResponse, UnifiedStreamChunk},
    gemini::{
        GeminiCandidate, GeminiContent, GeminiPart, GeminiResponse as GeminiNetworkResponse,
        GeminiUsageMetadata,
    },
    helper::sse::Event,
};

use axum::{
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use std::convert::Infallible;
use std::sync::{Arc, RwLock};

pub struct GeminiOutputAdapter;

impl OutputAdapter for GeminiOutputAdapter {
    fn adapt_response(
        &self,
        response: UnifiedResponse,
        sse_status: Arc<RwLock<SseStatus>>,
    ) -> Result<Response, anyhow::Error> {
        let mut gemini_parts = Vec::new();
        for block in response.content {
            match block {
                crate::ccproxy::adapter::unified::UnifiedContentBlock::Text { text } => {
                    gemini_parts.push(GeminiPart {
                        text: Some(text),
                        ..Default::default()
                    });
                }
                crate::ccproxy::adapter::unified::UnifiedContentBlock::ToolUse {
                    id: _,
                    name,
                    input,
                } => {
                    gemini_parts.push(GeminiPart {
                        function_call: Some(crate::ccproxy::gemini::GeminiFunctionCall {
                            name,
                            args: input,
                        }),
                        ..Default::default()
                    });
                }
                _ => {
                    // Gemini doesn't have direct equivalents for Image/ToolResult in assistant responses
                    // For simplicity, we might convert them to text or ignore.
                    // Here, we ignore other types for full responses.
                }
            }
        }

        let model = if let Ok(status) = sse_status.read() {
            status.model_id.clone()
        } else {
            response.model
        };
        let message_id = if let Ok(status) = sse_status.read() {
            status.message_id.clone()
        } else {
            response.id
        };

        let (estimated_input_tokens_f64, estimated_output_tokens_f64) = if let Ok(status) = sse_status.read() {
            (status.estimated_input_tokens, status.estimated_output_tokens)
        } else {
            (0.0, 0.0)
        };

        let input_tokens = if response.usage.input_tokens > 0 {
            response.usage.input_tokens
        } else {
            estimated_input_tokens_f64.ceil() as u64
        };
        let output_tokens = if response.usage.output_tokens > 0 {
            response.usage.output_tokens
        } else {
            estimated_output_tokens_f64.ceil() as u64
        };

        let usage = GeminiUsageMetadata {
            prompt_token_count: input_tokens,
            candidates_token_count: Some(output_tokens),
            total_token_count: input_tokens + output_tokens,
            tool_use_prompt_token_count: None, // Add missing field
            thoughts_token_count: None,        // Add missing field
            cached_content_token_count: None,  // Add missing field
            prompt_tokens_details: None,       // Add missing field
            candidates_tokens_details: None,   // Add missing field
        };

        let gemini_response = GeminiNetworkResponse {
            candidates: Some(vec![GeminiCandidate {
                index: Some(0), // Add missing field
                content: GeminiContent {
                    role: Some("model".to_string()),
                    parts: gemini_parts,
                },
                finish_reason: response.stop_reason.map(|s| s.into()), // Convert stop_reason string to GeminiFinishReason
                safety_ratings: None,                                  // Add missing field
                citation_metadata: None,                               // Add missing field
                grounding_metadata: None,                              // Add missing field
                avg_logprobs: None,                                    // Add missing field
                finish_message: None,                                  // Add missing field
            }]),
            prompt_feedback: None,
            usage_metadata: Some(usage),
            model_version: Some(model), // Add missing field
            create_time: Some(chrono::Utc::now().to_rfc3339()), // Add missing field
            response_id: Some(message_id), // Add missing field
        };

        Ok(Json(gemini_response).into_response())
    }

    fn adapt_stream_chunk(
        &self,
        chunk: UnifiedStreamChunk,
        sse_status: Arc<RwLock<SseStatus>>,
    ) -> Result<Vec<Event>, Infallible> {
        let event = Event::default().set_gemini();
        match chunk {
            UnifiedStreamChunk::MessageStart {
                id: _,
                model: _,
                usage: _,
            } => {
                // Gemini does not have a distinct message_start event in stream.
                // Usage info comes in message_delta or final chunk.
                // We'll return an empty data event, as Gemini expects a continuous stream of content.
                // let data = json!({
                //     "usageMetadata": {
                //         "promptTokenCount": usage.input_tokens,
                //         "candidatesTokenCount": 0,
                //         "totalTokenCount": usage.input_tokens
                //     }
                // });
                // Ok(vec![event.data(data.to_string())])
                Ok(vec![])
            }
            UnifiedStreamChunk::Text { delta } => {
                let data = json!({
                    "candidates": [{
                        "content": {
                            "role": "model",
                            "parts": [{ "text": delta }]
                        }
                    }]
                });
                Ok(vec![event.data(data.to_string())])
            }
            UnifiedStreamChunk::Thinking { delta } => {
                let data = json!({
                    "candidates": [{
                        "content": {
                            "role": "model",
                            "parts": [{
                                "text": delta,
                                "thought": serde_json::Value::Object(Default::default())
                            }]
                        }
                    }]
                });
                Ok(vec![event.data(data.to_string())])
            }
            UnifiedStreamChunk::ToolUseStart {
                tool_type: _,
                id,
                name,
            } => {
                if let Ok(mut status) = sse_status.write() {
                    status
                        .gemini_tools
                        .entry(id)
                        .or_insert(UnifiedFunctionCallPart {
                            name,
                            args: "".to_string(),
                        });
                }
                Ok(vec![])
            }
            UnifiedStreamChunk::ToolUseDelta { id, delta } => {
                if let Ok(mut status) = sse_status.write() {
                    if let Some(tool) = status.gemini_tools.get_mut(&id) {
                        tool.args.push_str(&delta);
                    }
                }
                Ok(vec![])
            }
            UnifiedStreamChunk::ToolUseEnd { id: _ } => {
                // Gemini doesn't have a distinct ToolUseEnd stream event.
                // The function_call is typically considered complete when no more deltas for it arrive.
                Ok(vec![])
            }
            UnifiedStreamChunk::MessageStop { stop_reason, usage } => {
                // Extract and process tool calls
                let mut parts = vec![];

                // Gemini's parallel tool calling requires all tools to be accumulated and sent together
                // in a single parts array, rather than sending them individually as separate messages
                // Example:
                // data:{"candidates":[{"content":{"parts":[{"functionCall":{"args":{"date":"2035-01-05","location":"shenzhen"},"name":"get_weather"}},{"functionCall":{"args":{"date":"2035-01-05","location":"beijing"},"name":"get_weather"}},{"functionCall":{"args":{"date":"2035-01-05","location":"shanghai"},"name":"get_weather"}}],"role":"model"},"finishReason":"STOP"}],"usageMetadata":{"candidatesTokenCount":48,"promptTokenCount":56,"totalTokenCount":104}}
                if let Ok(mut status) = sse_status.write() {
                    if !status.gemini_tools.is_empty() {
                        parts = status
                            .gemini_tools
                            .values()
                            .map(|tool| {
                                serde_json::json!({
                                    "functionCall": {
                                        "name": &tool.name,
                                        "args": serde_json::from_str::<serde_json::Value>(&tool.args)
                                            .unwrap_or_else(|_| serde_json::Value::String(tool.args.clone())),
                                    }
                                })
                            })
                            .collect::<Vec<serde_json::Value>>();

                        status.gemini_tools.clear();
                    }
                }

                let (estimated_input_tokens_f64, estimated_output_tokens_f64) = if let Ok(status) = sse_status.read() {
                    (status.estimated_input_tokens, status.estimated_output_tokens)
                } else {
                    (0.0, 0.0)
                };

                let input_tokens = if usage.input_tokens > 0 {
                    usage.input_tokens
                } else {
                    estimated_input_tokens_f64.ceil() as u64
                };
                let output_tokens = if usage.output_tokens > 0 {
                    usage.output_tokens
                } else {
                    estimated_output_tokens_f64.ceil() as u64
                };

                // Build the complete response in one go
                let end_event = serde_json::json!({
                    "candidates": [{
                        "content": {
                            "role": "model",
                            "parts": parts
                        },
                        "finishReason": stop_reason
                    }],
                    "usageMetadata": {
                        "promptTokenCount": input_tokens,
                        "candidatesTokenCount": output_tokens,
                        "totalTokenCount": input_tokens + output_tokens
                    }
                });

                Ok(vec![event.data(end_event.to_string())])
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
