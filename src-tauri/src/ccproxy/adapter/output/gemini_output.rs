use super::OutputAdapter;
use crate::ccproxy::adapter::unified::{SseStatus, UnifiedResponse, UnifiedStreamChunk};
use crate::ccproxy::gemini::{
    GeminiCandidate, GeminiContent, GeminiPart, GeminiResponse as GeminiNetworkResponse,
    GeminiUsageMetadata,
};
use serde_json::{json, Value};
use std::convert::Infallible;
use std::sync::{Arc, RwLock};
use warp::sse::Event;

pub struct GeminiOutputAdapter;

impl OutputAdapter for GeminiOutputAdapter {
    fn adapt_response(&self, response: UnifiedResponse) -> Result<impl warp::Reply, anyhow::Error> {
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

        let usage = GeminiUsageMetadata {
            prompt_token_count: response.usage.input_tokens,
            candidates_token_count: Some(response.usage.output_tokens),
            total_token_count: response.usage.input_tokens + response.usage.output_tokens,
        };

        let gemini_response = GeminiNetworkResponse {
            candidates: Some(vec![GeminiCandidate {
                content: GeminiContent {
                    role: "model".to_string(),
                    parts: gemini_parts,
                },
                finish_reason: response.stop_reason.map(|s| s.into()), // Convert stop_reason string to GeminiFinishReason
            }]),
            prompt_feedback: None,
            usage_metadata: Some(usage),
        };

        Ok(warp::reply::json(&gemini_response))
    }

    fn adapt_stream_chunk(
        &self,
        chunk: UnifiedStreamChunk,
        _sse_status: Arc<RwLock<SseStatus>>,
    ) -> Result<Vec<Event>, Infallible> {
        let event = Event::default();
        match chunk {
            UnifiedStreamChunk::MessageStart {
                id: _,
                model: _,
                usage,
            } => {
                // Gemini does not have a distinct message_start event in stream.
                // Usage info comes in message_delta or final chunk.
                // We'll return an empty data event, as Gemini expects a continuous stream of content.
                let data = json!({
                    "usageMetadata": {
                        "promptTokenCount": usage.input_tokens,
                        "candidatesTokenCount": 0,
                        "totalTokenCount": usage.input_tokens
                    }
                });
                Ok(vec![event.data(data.to_string())])
            }
            UnifiedStreamChunk::Thinking { delta } | UnifiedStreamChunk::Text { delta } => {
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
            UnifiedStreamChunk::ToolUseStart {
                tool_type: _,
                id: _,
                name,
            } => {
                // ToolUseStart for Gemini can be modeled as a function_call part in the stream
                // Arguments are typically streamed via ToolUseDelta
                let data = json!({
                    "candidates": [{
                        "content": {
                            "role": "model",
                            "parts": [{
                                "functionCall": {
                                    "name": name,
                                    "args": {} // Start with empty args, delta will fill
                                }
                            }]
                        }
                    }]
                });
                Ok(vec![event.data(data.to_string())])
            }
            UnifiedStreamChunk::ToolUseDelta { id: _, delta } => {
                // Stream tool arguments. Gemini expects args as a single JSON object for function_call.
                // This might require accumulating deltas and sending a complete JSON.
                // For simplicity now, we assume delta is the full argument string, or we append.
                // A more robust solution would involve stateful accumulation.
                let data = json!({
                    "candidates": [{
                        "content": {
                            "role": "model",
                            "parts": [{
                                "functionCall": {
                                    "args": serde_json::from_str::<Value>(&delta).unwrap_or(Value::String(delta))
                                }
                            }]
                        }
                    }]
                });
                Ok(vec![event.data(data.to_string())])
            }
            UnifiedStreamChunk::ToolUseEnd { id: _ } => {
                // Gemini doesn't have a distinct ToolUseEnd stream event.
                // The function_call is typically considered complete when no more deltas for it arrive.
                Ok(vec![event.data("")])
            }
            UnifiedStreamChunk::MessageStop { stop_reason, usage } => {
                let data = json!({
                    "candidates": [{
                        "finishReason": stop_reason,
                        "content": {
                            "role": "model",
                            "parts": []
                        }
                    }],
                    "usageMetadata": {
                        "promptTokenCount": usage.input_tokens,
                        "candidatesTokenCount": usage.output_tokens,
                        "totalTokenCount": usage.input_tokens + usage.output_tokens
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
