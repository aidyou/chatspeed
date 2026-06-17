use super::OutputAdapter;
use crate::ccproxy::adapter::unified::{
    SseStatus, UnifiedEmbeddingResponse, UnifiedResponse, UnifiedStreamChunk,
};
use crate::ccproxy::helper::sse::Event;
use crate::ccproxy::helper::{get_msg_id, get_tool_id};
use crate::ccproxy::types::openai::{OpenAIFunctionCall, UnifiedToolCall};
use crate::ccproxy::types::openai_responses::{
    OpenAIResponsesInputTokensDetails, OpenAIResponsesOutputContent, OpenAIResponsesOutputItem,
    OpenAIResponsesOutputTokensDetails, OpenAIResponsesReasoningSummary, OpenAIResponsesResponse,
    OpenAIResponsesUsage,
};
use crate::ccproxy::utils::token_estimator::resolve_usage_with_estimate;

use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use std::convert::Infallible;
use std::sync::{Arc, RwLock};

pub struct OpenAIResponsesOutputAdapter;

impl OutputAdapter for OpenAIResponsesOutputAdapter {
    fn adapt_response(
        &self,
        response: UnifiedResponse,
        sse_status: Arc<RwLock<SseStatus>>,
    ) -> Result<Response, anyhow::Error> {
        let mut text_content = String::new();
        let mut reasoning_content: Option<String> = None;
        let mut tool_calls: Vec<UnifiedToolCall> = Vec::new();

        for c in response.content {
            match c {
                crate::ccproxy::adapter::unified::UnifiedContentBlock::Thinking { thinking } => {
                    if let Some(existing) = &mut reasoning_content {
                        if !existing.is_empty() && !thinking.is_empty() {
                            existing.push('\n');
                        }
                        existing.push_str(&thinking);
                    } else {
                        reasoning_content = Some(thinking);
                    }
                }
                crate::ccproxy::adapter::unified::UnifiedContentBlock::Text { text } => {
                    text_content.push_str(&text);
                }
                crate::ccproxy::adapter::unified::UnifiedContentBlock::ToolUse {
                    id,
                    name,
                    input,
                } => {
                    tool_calls.push(UnifiedToolCall {
                        id: Some(id),
                        r#type: Some("function".to_string()),
                        function: Some(OpenAIFunctionCall {
                            name: Some(name),
                            arguments: Some(input.to_string()),
                        }),
                        index: None,
                    });
                }
                _ => {}
            }
        }

        let model = if let Ok(status) = sse_status.read() {
            status.model_id.clone()
        } else {
            response.model
        };
        let response_id = response_id(&response.id);

        let (estimated_input_tokens_f64, estimated_output_tokens_f64) =
            if let Ok(status) = sse_status.read() {
                (
                    status.estimated_input_tokens,
                    status.estimated_output_tokens,
                )
            } else {
                (0.0, 0.0)
            };

        let (input_tokens, output_tokens) = resolve_usage_with_estimate(
            "openai_responses",
            response.usage.input_tokens,
            response.usage.output_tokens,
            estimated_input_tokens_f64,
            estimated_output_tokens_f64,
            "response",
        );

        let created = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| anyhow::anyhow!("Failed to get system time: {}", e))?
            .as_secs();

        let mut output = Vec::new();
        if let Some(reasoning) = reasoning_content {
            if !reasoning.is_empty() {
                output.push(OpenAIResponsesOutputItem {
                    id: reasoning_id(),
                    item_type: "reasoning".to_string(),
                    status: "completed".to_string(),
                    role: None,
                    content: None,
                    call_id: None,
                    name: None,
                    arguments: None,
                    summary: Some(vec![OpenAIResponsesReasoningSummary {
                        summary_type: "summary_text".to_string(),
                        text: reasoning,
                    }]),
                });
            }
        }
        let mut message_content = Vec::new();
        if !text_content.is_empty() {
            message_content.push(OpenAIResponsesOutputContent {
                content_type: "output_text".to_string(),
                text: Some(text_content),
                annotations: Some(Vec::new()),
            });
        }
        // Reasoning text from chat-compatible providers is not emitted as message content here.
        // Responses clients expect reasoning to be represented by usage details unless the
        // backend returns native Responses reasoning items on the direct path.
        if !message_content.is_empty() {
            output.push(OpenAIResponsesOutputItem {
                id: get_msg_id(),
                item_type: "message".to_string(),
                status: "completed".to_string(),
                role: Some("assistant".to_string()),
                content: Some(message_content),
                call_id: None,
                name: None,
                arguments: None,
                summary: None,
            });
        }
        for tool_call in tool_calls {
            if let Some(function) = tool_call.function {
                let call_id = tool_call.id.unwrap_or_else(get_tool_id);
                output.push(OpenAIResponsesOutputItem {
                    id: call_id.clone(),
                    item_type: "function_call".to_string(),
                    status: "completed".to_string(),
                    role: None,
                    content: None,
                    call_id: Some(call_id),
                    name: function.name,
                    arguments: function.arguments,
                    summary: None,
                });
            }
        }

        let cached_tokens = response
            .usage
            .prompt_cached_tokens
            .or(response.usage.cache_read_input_tokens)
            .or(response.usage.cached_content_tokens)
            .unwrap_or(0);
        let reasoning_tokens = response.usage.thoughts_tokens.unwrap_or(0);

        let responses_response = OpenAIResponsesResponse {
            id: response_id,
            object: "response".to_string(),
            created_at: created,
            status: "completed".to_string(),
            error: None,
            incomplete_details: None,
            model,
            output,
            instructions: None,
            previous_response_id: None,
            usage: Some(OpenAIResponsesUsage {
                input_tokens,
                output_tokens,
                total_tokens: input_tokens + output_tokens,
                input_tokens_details: Some(OpenAIResponsesInputTokensDetails { cached_tokens }),
                output_tokens_details: Some(OpenAIResponsesOutputTokensDetails {
                    reasoning_tokens,
                }),
            }),
        };

        Ok(Json(responses_response).into_response())
    }

    fn adapt_stream_chunk(
        &self,
        chunk: UnifiedStreamChunk,
        sse_status: Arc<RwLock<SseStatus>>,
    ) -> Result<Vec<Event>, Infallible> {
        match chunk {
            UnifiedStreamChunk::MessageStart { id, model, .. } => {
                let response_id = response_id(&id);
                let model = model_from_status(&sse_status, model);
                if let Ok(mut status) = sse_status.write() {
                    status.message_id = response_id.clone();
                    status.model_id = model.clone();
                }
                Ok(vec![Event::default().event("response.created").data(
                    json!({
                        "type": "response.created",
                        "response": base_stream_response(&response_id, &model, "in_progress", None, None)
                    })
                    .to_string(),
                )])
            }
            UnifiedStreamChunk::Text { delta } => {
                let (_, item_id) = ids_from_status(&sse_status);
                let mut events = Vec::new();
                let mut send_start = false;
                if let Ok(mut status) = sse_status.write() {
                    if !status.responses_text_started {
                        send_start = true;
                        status.responses_text_started = true;
                    }
                    status.responses_text_buffer.push_str(&delta);
                }
                if send_start {
                    events.push(
                        Event::default().event("response.output_item.added").data(
                            json!({
                                "type": "response.output_item.added",
                                "output_index": 0,
                                "item": {
                                    "id": item_id,
                                    "type": "message",
                                    "status": "in_progress",
                                    "role": "assistant",
                                    "content": []
                                }
                            })
                            .to_string(),
                        ),
                    );
                    events.push(
                        Event::default().event("response.content_part.added").data(
                            json!({
                                "type": "response.content_part.added",
                                "item_id": item_id,
                                "output_index": 0,
                                "content_index": 0,
                                "part": {
                                    "type": "output_text",
                                    "text": "",
                                    "annotations": []
                                }
                            })
                            .to_string(),
                        ),
                    );
                }
                events.push(
                    Event::default().event("response.output_text.delta").data(
                        json!({
                            "type": "response.output_text.delta",
                            "item_id": item_id,
                            "output_index": 0,
                            "content_index": 0,
                            "delta": delta
                        })
                        .to_string(),
                    ),
                );
                Ok(events)
            }
            UnifiedStreamChunk::Thinking { delta } => {
                let (_, reasoning_item_id) = reasoning_ids_from_status(&sse_status);
                let mut events = Vec::new();
                let mut send_start = false;
                if let Ok(mut status) = sse_status.write() {
                    if !status.responses_reasoning_started {
                        send_start = true;
                        status.responses_reasoning_started = true;
                    }
                    status.responses_reasoning_buffer.push_str(&delta);
                }
                if send_start {
                    events.push(
                        Event::default().event("response.output_item.added").data(
                            json!({
                                "type": "response.output_item.added",
                                "output_index": 0,
                                "item": {
                                    "id": reasoning_item_id,
                                    "type": "reasoning",
                                    "summary": []
                                }
                            })
                            .to_string(),
                        ),
                    );
                    events.push(
                        Event::default()
                            .event("response.reasoning_summary_part.added")
                            .data(
                                json!({
                                    "type": "response.reasoning_summary_part.added",
                                    "item_id": reasoning_item_id,
                                    "output_index": 0,
                                    "summary_index": 0,
                                    "part": {
                                        "type": "summary_text",
                                        "text": ""
                                    }
                                })
                                .to_string(),
                            ),
                    );
                }
                events.push(
                    Event::default()
                        .event("response.reasoning_summary_text.delta")
                        .data(
                            json!({
                                "type": "response.reasoning_summary_text.delta",
                                "item_id": reasoning_item_id,
                                "output_index": 0,
                                "summary_index": 0,
                                "delta": delta
                            })
                            .to_string(),
                        ),
                );
                Ok(events)
            }
            UnifiedStreamChunk::ToolUseStart {
                id, name, index, ..
            } => Ok(vec![Event::default()
                .event("response.output_item.added")
                .data(
                    json!({
                        "type": "response.output_item.added",
                        "output_index": index,
                        "item": {
                            "id": id,
                            "type": "function_call",
                            "status": "in_progress",
                            "call_id": id,
                            "name": name,
                            "arguments": ""
                        }
                    })
                    .to_string(),
                )]),
            UnifiedStreamChunk::ToolUseDelta { id, delta, index } => Ok(vec![Event::default()
                .event("response.function_call_arguments.delta")
                .data(
                    json!({
                        "type": "response.function_call_arguments.delta",
                        "item_id": id,
                        "output_index": index,
                        "delta": delta
                    })
                    .to_string(),
                )]),
            UnifiedStreamChunk::ToolUseEnd { id } => Ok(vec![Event::default()
                .event("response.output_item.done")
                .data(
                    json!({
                        "type": "response.output_item.done",
                        "output_index": 0,
                        "item": {
                            "id": id,
                            "type": "function_call",
                            "status": "completed"
                        }
                    })
                    .to_string(),
                )]),
            UnifiedStreamChunk::MessageStop {
                stop_reason: _,
                usage,
            } => {
                let (response_id, item_id) = ids_from_status(&sse_status);
                let model = model_from_status(&sse_status, String::new());
                let usage_json = stream_usage_json(&usage, &sse_status);
                let mut events = Vec::new();
                let has_text = sse_status
                    .read()
                    .map(|status| status.responses_text_started)
                    .unwrap_or(false);
                let output_text = sse_status
                    .read()
                    .map(|status| status.responses_text_buffer.clone())
                    .unwrap_or_default();
                let reasoning_text = sse_status
                    .read()
                    .map(|status| status.responses_reasoning_buffer.clone())
                    .unwrap_or_default();
                let has_reasoning = !reasoning_text.trim().is_empty();
                let (_, reasoning_item_id) = reasoning_ids_from_status(&sse_status);
                if has_reasoning {
                    events.push(
                        Event::default()
                            .event("response.reasoning_summary_text.done")
                            .data(
                                json!({
                                    "type": "response.reasoning_summary_text.done",
                                    "item_id": reasoning_item_id,
                                    "output_index": 0,
                                    "summary_index": 0,
                                    "text": reasoning_text
                                })
                                .to_string(),
                            ),
                    );
                    events.push(
                        Event::default()
                            .event("response.reasoning_summary_part.done")
                            .data(
                                json!({
                                    "type": "response.reasoning_summary_part.done",
                                    "item_id": reasoning_item_id,
                                    "output_index": 0,
                                    "summary_index": 0,
                                    "part": {
                                        "type": "summary_text",
                                        "text": reasoning_text
                                    }
                                })
                                .to_string(),
                            ),
                    );
                    events.push(Event::default().event("response.output_item.done").data(
                        json!({
                            "type": "response.output_item.done",
                            "output_index": 0,
                            "item": reasoning_output_item_json(&reasoning_item_id, &reasoning_text)
                        })
                        .to_string(),
                    ));
                }
                if has_text {
                    events.push(
                        Event::default().event("response.output_text.done").data(
                            json!({
                                "type": "response.output_text.done",
                                "item_id": item_id,
                                "output_index": 0,
                                "content_index": 0,
                                "text": output_text
                            })
                            .to_string(),
                        ),
                    );
                    events.push(
                        Event::default().event("response.content_part.done").data(
                            json!({
                                "type": "response.content_part.done",
                                "item_id": item_id,
                                "output_index": 0,
                                "content_index": 0,
                                "part": {
                                    "type": "output_text",
                                    "text": output_text,
                                    "annotations": []
                                }
                            })
                            .to_string(),
                        ),
                    );
                    events.push(
                        Event::default().event("response.output_item.done").data(
                            json!({
                                    "type": "response.output_item.done",
                                    "output_index": 0,
                                    "item": {
                                        "id": item_id,
                                        "type": "message",
                                    "status": "completed",
                                    "role": "assistant",
                                    "content": [{
                                        "type": "output_text",
                                        "text": output_text,
                                        "annotations": []
                                    }]
                                }
                            })
                            .to_string(),
                        ),
                    );
                }
                events.push(
                    Event::default().event("response.completed").data(
                        json!({
                                "type": "response.completed",
                            "response": base_stream_response(
                                &response_id,
                                &model,
                                "completed",
                                Some(usage_json),
                                Some(stream_output_items_json(
                                    has_reasoning,
                                    &reasoning_item_id,
                                    &reasoning_text,
                                    has_text,
                                    &item_id,
                                    &output_text,
                                ))
                            )
                        })
                        .to_string(),
                    ),
                );
                Ok(events)
            }
            UnifiedStreamChunk::Error { message } => {
                Ok(vec![Event::default().event("response.failed").data(
                    json!({
                        "type": "response.failed",
                        "response": {
                            "status": "failed",
                            "error": {
                                "message": message
                            }
                        }
                    })
                    .to_string(),
                )])
            }
            _ => Ok(Vec::new()),
        }
    }

    fn adapt_embedding_response(
        &self,
        _response: UnifiedEmbeddingResponse,
    ) -> Result<Response, anyhow::Error> {
        Err(anyhow::anyhow!(
            "OpenAI Responses output adapter does not support embeddings"
        ))
    }
}

fn response_id(source_id: &str) -> String {
    if source_id.starts_with("resp_") {
        source_id.to_string()
    } else {
        format!("resp_{}", uuid::Uuid::new_v4().to_string().replace('-', ""))
    }
}

fn reasoning_id() -> String {
    format!("rs_{}", uuid::Uuid::new_v4().to_string().replace('-', ""))
}

fn ids_from_status(sse_status: &Arc<RwLock<SseStatus>>) -> (String, String) {
    if let Ok(mut status) = sse_status.write() {
        if !status.message_id.starts_with("resp_") {
            status.message_id = response_id(&status.message_id);
        }
        if status.responses_message_item_id.is_empty() {
            status.responses_message_item_id = get_msg_id();
        }
        (
            status.message_id.clone(),
            status.responses_message_item_id.clone(),
        )
    } else {
        (response_id(""), get_msg_id())
    }
}

fn reasoning_ids_from_status(sse_status: &Arc<RwLock<SseStatus>>) -> (String, String) {
    if let Ok(mut status) = sse_status.write() {
        if !status.message_id.starts_with("resp_") {
            status.message_id = response_id(&status.message_id);
        }
        if status.responses_reasoning_item_id.is_empty() {
            status.responses_reasoning_item_id = reasoning_id();
        }
        (
            status.message_id.clone(),
            status.responses_reasoning_item_id.clone(),
        )
    } else {
        (response_id(""), reasoning_id())
    }
}

fn model_from_status(sse_status: &Arc<RwLock<SseStatus>>, fallback: String) -> String {
    sse_status
        .read()
        .ok()
        .map(|status| status.model_id.clone())
        .filter(|model| !model.is_empty())
        .unwrap_or(fallback)
}

fn reasoning_output_item_json(item_id: &str, reasoning_text: &str) -> serde_json::Value {
    json!({
        "id": item_id,
        "type": "reasoning",
        "summary": [{
            "type": "summary_text",
            "text": reasoning_text
        }]
    })
}

fn message_output_item_json(item_id: &str, output_text: &str) -> serde_json::Value {
    json!({
        "id": item_id,
        "type": "message",
        "status": "completed",
        "role": "assistant",
        "content": [{
            "type": "output_text",
            "text": output_text,
            "annotations": []
        }]
    })
}

fn stream_output_items_json(
    has_reasoning: bool,
    reasoning_item_id: &str,
    reasoning_text: &str,
    has_text: bool,
    message_item_id: &str,
    output_text: &str,
) -> serde_json::Value {
    let mut output = Vec::new();
    if has_reasoning {
        output.push(reasoning_output_item_json(
            reasoning_item_id,
            reasoning_text,
        ));
    }
    if has_text {
        output.push(message_output_item_json(message_item_id, output_text));
    }
    serde_json::Value::Array(output)
}

fn stream_usage_json(
    usage: &crate::ccproxy::adapter::unified::UnifiedUsage,
    sse_status: &Arc<RwLock<SseStatus>>,
) -> serde_json::Value {
    let (estimated_input_tokens_f64, estimated_output_tokens_f64) =
        if let Ok(status) = sse_status.read() {
            (
                status.estimated_input_tokens,
                status.estimated_output_tokens,
            )
        } else {
            (0.0, 0.0)
        };
    let (input_tokens, output_tokens) = resolve_usage_with_estimate(
        "openai_responses",
        usage.input_tokens,
        usage.output_tokens,
        estimated_input_tokens_f64,
        estimated_output_tokens_f64,
        "stream_stop",
    );
    let cached_tokens = usage
        .prompt_cached_tokens
        .or(usage.cache_read_input_tokens)
        .or(usage.cached_content_tokens)
        .unwrap_or(0);
    let reasoning_tokens = usage.thoughts_tokens.unwrap_or(0);

    json!({
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "total_tokens": input_tokens + output_tokens,
        "input_tokens_details": {
            "cached_tokens": cached_tokens
        },
        "output_tokens_details": {
            "reasoning_tokens": reasoning_tokens
        }
    })
}

fn base_stream_response(
    response_id: &str,
    model: &str,
    status: &str,
    usage: Option<serde_json::Value>,
    output: Option<serde_json::Value>,
) -> serde_json::Value {
    json!({
        "id": response_id,
        "object": "response",
        "created_at": chrono::Utc::now().timestamp(),
        "status": status,
        "error": null,
        "incomplete_details": null,
        "model": model,
        "output": output.unwrap_or_else(|| json!([])),
        "parallel_tool_calls": true,
        "previous_response_id": null,
        "usage": usage
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ccproxy::adapter::unified::{UnifiedContentBlock, UnifiedResponse, UnifiedUsage};
    use axum::body;

    #[tokio::test]
    async fn adapted_response_emits_reasoning_item_and_message_item() {
        let adapter = OpenAIResponsesOutputAdapter;
        let response = UnifiedResponse {
            id: "chatcmpl_test".to_string(),
            model: "glm-5".to_string(),
            content: vec![
                UnifiedContentBlock::Text {
                    text: "answer".to_string(),
                },
                UnifiedContentBlock::Thinking {
                    thinking: "internal reasoning".to_string(),
                },
            ],
            stop_reason: Some("stop".to_string()),
            usage: UnifiedUsage {
                input_tokens: 10,
                output_tokens: 3,
                thoughts_tokens: Some(2),
                ..Default::default()
            },
        };

        let http_response = adapter
            .adapt_response(response, Arc::new(RwLock::new(SseStatus::default())))
            .expect("response should adapt");
        let body = body::to_bytes(http_response.into_body(), usize::MAX)
            .await
            .expect("body should read");
        let payload: serde_json::Value =
            serde_json::from_slice(&body).expect("body should be json");

        assert!(payload["id"]
            .as_str()
            .expect("response id should be a string")
            .starts_with("resp_"));
        assert_eq!(payload["output"][0]["type"], "reasoning");
        assert!(payload["output"][0]["id"]
            .as_str()
            .expect("reasoning id should be a string")
            .starts_with("rs_"));
        assert_eq!(payload["output"][0]["summary"][0]["type"], "summary_text");
        assert_eq!(
            payload["output"][0]["summary"][0]["text"],
            "internal reasoning"
        );
        assert!(payload["output"][1]["id"]
            .as_str()
            .expect("message id should be a string")
            .starts_with("msg_"));
        assert_eq!(payload["output"][1]["type"], "message");
        assert_eq!(payload["output"][1]["content"][0]["type"], "output_text");
        assert_eq!(
            payload["usage"]["output_tokens_details"]["reasoning_tokens"],
            2
        );
        assert!(payload["output"][1]["content"]
            .as_array()
            .expect("content should be an array")
            .iter()
            .all(|item| item["type"] != "reasoning_text"));
    }

    #[test]
    fn stream_chunks_are_emitted_as_responses_events() {
        let adapter = OpenAIResponsesOutputAdapter;
        let status = Arc::new(RwLock::new(SseStatus::new(
            "msg_test".to_string(),
            "glm-5".to_string(),
            false,
            10.0,
        )));

        let created = adapter
            .adapt_stream_chunk(
                UnifiedStreamChunk::MessageStart {
                    id: "chatcmpl_test".to_string(),
                    model: "glm-5".to_string(),
                    usage: UnifiedUsage::default(),
                },
                status.clone(),
            )
            .expect("created event should adapt");
        assert!(created[0].to_string().contains("response.created"));

        let text = adapter
            .adapt_stream_chunk(
                UnifiedStreamChunk::Text {
                    delta: "hello".to_string(),
                },
                status.clone(),
            )
            .expect("text event should adapt")
            .into_iter()
            .map(|event| event.to_string())
            .collect::<String>();
        assert!(text.contains("response.output_item.added"));
        assert!(text.contains("response.output_text.delta"));
        assert!(text.contains("hello"));

        let completed = adapter
            .adapt_stream_chunk(
                UnifiedStreamChunk::MessageStop {
                    stop_reason: "stop".to_string(),
                    usage: UnifiedUsage {
                        input_tokens: 10,
                        output_tokens: 2,
                        thoughts_tokens: Some(1),
                        ..Default::default()
                    },
                },
                status,
            )
            .expect("completed event should adapt")
            .into_iter()
            .map(|event| event.to_string())
            .collect::<String>();
        assert!(completed.contains("response.completed"));
        assert!(completed.contains("\"input_tokens\":10"));
        assert!(completed.contains("\"reasoning_tokens\":1"));
        assert!(completed.contains("\"output\":["));
        assert!(completed.contains("\"text\":\"hello\""));
    }

    #[test]
    fn reasoning_only_stream_completes_with_reasoning_item() {
        let adapter = OpenAIResponsesOutputAdapter;
        let status = Arc::new(RwLock::new(SseStatus::new(
            "msg_reasoning".to_string(),
            "deepseek-v4-flash".to_string(),
            false,
            10.0,
        )));

        adapter
            .adapt_stream_chunk(
                UnifiedStreamChunk::MessageStart {
                    id: "chatcmpl_reasoning".to_string(),
                    model: "deepseek-v4-flash".to_string(),
                    usage: UnifiedUsage::default(),
                },
                status.clone(),
            )
            .expect("created event should adapt");
        adapter
            .adapt_stream_chunk(
                UnifiedStreamChunk::Thinking {
                    delta: "reasoning only answer".to_string(),
                },
                status.clone(),
            )
            .expect("thinking event should adapt");

        let completed = adapter
            .adapt_stream_chunk(
                UnifiedStreamChunk::MessageStop {
                    stop_reason: "stop".to_string(),
                    usage: UnifiedUsage::default(),
                },
                status,
            )
            .expect("completed event should adapt")
            .into_iter()
            .map(|event| event.to_string())
            .collect::<String>();

        assert!(completed.contains("response.completed"));
        assert!(completed.contains("\"output\":["));
        assert!(completed.contains("\"type\":\"reasoning\""));
        assert!(completed.contains("\"type\":\"summary_text\""));
        assert!(completed.contains("\"text\":\"reasoning only answer\""));
        assert!(!completed.contains("\"type\":\"message\""));
    }
}
