use super::OutputAdapter;
use crate::ccproxy::adapter::unified::{
    SseStatus, UnifiedEmbeddingResponse, UnifiedResponse, UnifiedStreamChunk,
};
use crate::ccproxy::helper::sse::Event;
use crate::ccproxy::helper::{get_msg_id, get_tool_id};
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
        let mut tool_calls = Vec::new();

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
                    tool_calls.push((id, name, input));
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
                    input: None,
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
                input: None,
                summary: None,
            });
        }
        for (call_id, name, tool_input) in tool_calls {
            let is_custom = responses_is_custom_tool(&sse_status, &name);
            output.push(OpenAIResponsesOutputItem {
                id: call_id.clone(),
                item_type: if is_custom {
                    "custom_tool_call".to_string()
                } else {
                    "function_call".to_string()
                },
                status: "completed".to_string(),
                role: None,
                content: None,
                call_id: Some(call_id),
                name: Some(name),
                arguments: (!is_custom).then(|| tool_input.to_string()),
                input: is_custom.then(|| custom_tool_input_from_value(&tool_input)),
                summary: None,
            });
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
            } => {
                let item_id = get_tool_id();
                let is_custom = responses_is_custom_tool(&sse_status, &name);
                if let Ok(mut status) = sse_status.write() {
                    status
                        .responses_tool_item_ids
                        .insert(id.clone(), item_id.clone());
                    status.responses_tool_names.insert(id.clone(), name.clone());
                    status
                        .responses_tool_arguments
                        .entry(id.clone())
                        .or_insert_with(String::new);
                    status.responses_tool_indexes.insert(id.clone(), index);
                }
                let item = if is_custom {
                    json!({
                        "id": item_id,
                        "type": "custom_tool_call",
                        "status": "in_progress",
                        "call_id": id,
                        "name": name,
                        "input": ""
                    })
                } else {
                    json!({
                        "id": item_id,
                        "type": "function_call",
                        "status": "in_progress",
                        "call_id": id,
                        "name": name,
                        "arguments": ""
                    })
                };
                Ok(vec![Event::default()
                    .event("response.output_item.added")
                    .data(
                        json!({
                            "type": "response.output_item.added",
                            "output_index": index,
                            "item": item
                        })
                        .to_string(),
                    )])
            }
            UnifiedStreamChunk::ToolUseDelta { id, delta, index } => {
                let (item_id, is_custom) = if let Ok(mut status) = sse_status.write() {
                    status
                        .responses_tool_arguments
                        .entry(id.clone())
                        .or_insert_with(String::new)
                        .push_str(&delta);
                    let item_id = status
                        .responses_tool_item_ids
                        .get(&id)
                        .cloned()
                        .unwrap_or_else(get_tool_id);
                    let is_custom = status
                        .responses_tool_names
                        .get(&id)
                        .is_some_and(|name| status.responses_custom_tool_names.contains(name));
                    (item_id, is_custom)
                } else {
                    (get_tool_id(), false)
                };
                if is_custom {
                    return Ok(Vec::new());
                }
                Ok(vec![Event::default()
                    .event("response.function_call_arguments.delta")
                    .data(
                        json!({
                            "type": "response.function_call_arguments.delta",
                            "item_id": item_id,
                            "output_index": index,
                            "delta": delta
                        })
                        .to_string(),
                    )])
            }
            UnifiedStreamChunk::ToolUseEnd { id } => {
                Ok(complete_response_tool_call(&sse_status, &id))
            }
            UnifiedStreamChunk::MessageStop {
                stop_reason: _,
                usage,
            } => {
                let (response_id, item_id) = ids_from_status(&sse_status);
                let model = model_from_status(&sse_status, String::new());
                let usage_json = stream_usage_json(&usage, &sse_status);
                let mut events = Vec::new();
                // Some backend streams stop immediately after the final tool delta.
                for id in pending_response_tool_ids(&sse_status) {
                    events.extend(complete_response_tool_call(&sse_status, &id));
                }
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
                let completed_tool_items = sse_status
                    .read()
                    .map(|status| {
                        status
                            .responses_completed_tool_items
                            .values()
                            .cloned()
                            .collect::<Vec<_>>()
                    })
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
                                    &completed_tool_items,
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

fn responses_is_custom_tool(sse_status: &Arc<RwLock<SseStatus>>, name: &str) -> bool {
    sse_status
        .read()
        .is_ok_and(|status| status.responses_custom_tool_names.contains(name))
}

fn custom_tool_input_from_value(input: &serde_json::Value) -> String {
    match input {
        serde_json::Value::Object(object) => object
            .get("input")
            .map(|input| match input {
                serde_json::Value::String(input) => input.clone(),
                input => input.to_string(),
            })
            .unwrap_or_else(|| input.to_string()),
        serde_json::Value::String(input) => input.clone(),
        input => input.to_string(),
    }
}

fn custom_tool_input_from_arguments(arguments: &str) -> String {
    serde_json::from_str(arguments)
        .map(|input| custom_tool_input_from_value(&input))
        .unwrap_or_else(|_| arguments.to_string())
}

fn pending_response_tool_ids(sse_status: &Arc<RwLock<SseStatus>>) -> Vec<String> {
    let mut pending = sse_status
        .read()
        .map(|status| {
            status
                .responses_tool_indexes
                .iter()
                .map(|(id, index)| (*index, id.clone()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    pending.sort_by_key(|(index, _)| *index);
    pending.into_iter().map(|(_, id)| id).collect()
}

fn complete_response_tool_call(sse_status: &Arc<RwLock<SseStatus>>, id: &str) -> Vec<Event> {
    let (item_id, name, arguments, output_index, is_custom) =
        if let Ok(mut status) = sse_status.write() {
            let item_id = status
                .responses_tool_item_ids
                .remove(id)
                .unwrap_or_else(get_tool_id);
            let name = status.responses_tool_names.remove(id).unwrap_or_default();
            let arguments = status
                .responses_tool_arguments
                .remove(id)
                .unwrap_or_default();
            let output_index = status.responses_tool_indexes.remove(id).unwrap_or(0);
            let is_custom = status.responses_custom_tool_names.contains(&name);
            (item_id, name, arguments, output_index, is_custom)
        } else {
            (get_tool_id(), String::new(), String::new(), 0, false)
        };

    if is_custom {
        let input = custom_tool_input_from_arguments(&arguments);
        let completed_item = json!({
            "id": item_id,
            "type": "custom_tool_call",
            "status": "completed",
            "call_id": id,
            "name": name,
            "input": input
        });
        if let Ok(mut status) = sse_status.write() {
            status
                .responses_completed_tool_items
                .insert(output_index, completed_item.clone());
        }
        return vec![
            Event::default()
                .event("response.custom_tool_call_input.delta")
                .data(
                    json!({
                        "type": "response.custom_tool_call_input.delta",
                        "item_id": item_id,
                        "output_index": output_index,
                        "delta": input
                    })
                    .to_string(),
                ),
            Event::default()
                .event("response.custom_tool_call_input.done")
                .data(
                    json!({
                        "type": "response.custom_tool_call_input.done",
                        "item_id": item_id,
                        "output_index": output_index,
                        "input": input
                    })
                    .to_string(),
                ),
            Event::default().event("response.output_item.done").data(
                json!({
                    "type": "response.output_item.done",
                    "output_index": output_index,
                    "item": completed_item
                })
                .to_string(),
            ),
        ];
    }

    let completed_item = json!({
        "id": item_id,
        "type": "function_call",
        "status": "completed",
        "call_id": id,
        "name": name,
        "arguments": arguments
    });
    if let Ok(mut status) = sse_status.write() {
        status
            .responses_completed_tool_items
            .insert(output_index, completed_item.clone());
    }
    vec![
        Event::default()
            .event("response.function_call_arguments.done")
            .data(
                json!({
                    "type": "response.function_call_arguments.done",
                    "item_id": item_id,
                    "output_index": output_index,
                    "arguments": arguments
                })
                .to_string(),
            ),
        Event::default().event("response.output_item.done").data(
            json!({
                "type": "response.output_item.done",
                "output_index": output_index,
                "item": completed_item
            })
            .to_string(),
        ),
    ]
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
    completed_tool_items: &[serde_json::Value],
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
    output.extend_from_slice(completed_tool_items);
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

    #[tokio::test]
    async fn adapted_response_restores_custom_tool_call() {
        let adapter = OpenAIResponsesOutputAdapter;
        let response = UnifiedResponse {
            id: "chatcmpl_custom".to_string(),
            model: "deepseek-v4-pro".to_string(),
            content: vec![UnifiedContentBlock::ToolUse {
                id: "call_123".to_string(),
                name: "exec".to_string(),
                input: json!({ "input": "const value = await run();" }),
            }],
            stop_reason: Some("tool_calls".to_string()),
            usage: UnifiedUsage::default(),
        };
        let mut status = SseStatus::default();
        status
            .responses_custom_tool_names
            .insert("exec".to_string());

        let http_response = adapter
            .adapt_response(response, Arc::new(RwLock::new(status)))
            .expect("response should adapt");
        let body = body::to_bytes(http_response.into_body(), usize::MAX)
            .await
            .expect("body should read");
        let payload: serde_json::Value =
            serde_json::from_slice(&body).expect("body should be json");

        assert_eq!(payload["output"][0]["type"], "custom_tool_call");
        assert_eq!(payload["output"][0]["call_id"], "call_123");
        assert_eq!(payload["output"][0]["name"], "exec");
        assert_eq!(payload["output"][0]["input"], "const value = await run();");
        assert!(payload["output"][0].get("arguments").is_none());
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
    fn tool_call_stream_emits_arguments_done_and_complete_item() {
        let adapter = OpenAIResponsesOutputAdapter;
        let status = Arc::new(RwLock::new(SseStatus::new(
            "msg_tool".to_string(),
            "glm-5".to_string(),
            false,
            10.0,
        )));

        let start = adapter
            .adapt_stream_chunk(
                UnifiedStreamChunk::ToolUseStart {
                    tool_type: "tool_use".to_string(),
                    id: "call_123".to_string(),
                    name: "write_stdin".to_string(),
                    index: 1,
                },
                status.clone(),
            )
            .expect("tool start should adapt")
            .into_iter()
            .map(|event| event.to_string())
            .collect::<String>();
        assert!(start.contains("response.output_item.added"));
        assert!(start.contains("\"type\":\"function_call\""));
        assert!(start.contains("\"call_id\":\"call_123\""));
        assert!(start.contains("\"name\":\"write_stdin\""));

        let delta = adapter
            .adapt_stream_chunk(
                UnifiedStreamChunk::ToolUseDelta {
                    id: "call_123".to_string(),
                    delta: "{\"chars\":\"n\\n\"}".to_string(),
                    index: 1,
                },
                status.clone(),
            )
            .expect("tool delta should adapt")
            .into_iter()
            .map(|event| event.to_string())
            .collect::<String>();
        assert!(delta.contains("response.function_call_arguments.delta"));
        assert!(delta.contains("\"delta\":\"{\\\"chars\\\":\\\"n\\\\n\\\"}\""));

        let end = adapter
            .adapt_stream_chunk(
                UnifiedStreamChunk::ToolUseEnd {
                    id: "call_123".to_string(),
                },
                status.clone(),
            )
            .expect("tool end should adapt")
            .into_iter()
            .map(|event| event.to_string())
            .collect::<String>();
        assert!(end.contains("response.function_call_arguments.done"));
        assert!(end.contains("\"arguments\":\"{\\\"chars\\\":\\\"n\\\\n\\\"}\""));
        assert!(end.contains("response.output_item.done"));
        assert!(end.contains("\"call_id\":\"call_123\""));
        assert!(end.contains("\"name\":\"write_stdin\""));
        assert!(end.contains("\"status\":\"completed\""));

        let completed = adapter
            .adapt_stream_chunk(
                UnifiedStreamChunk::MessageStop {
                    stop_reason: "tool_calls".to_string(),
                    usage: UnifiedUsage::default(),
                },
                status,
            )
            .expect("message stop should adapt")
            .into_iter()
            .map(|event| event.to_string())
            .collect::<String>();
        assert!(completed.contains("response.completed"));
        assert!(completed.contains("\"type\":\"function_call\""));
        assert!(completed.contains("\"call_id\":\"call_123\""));
    }

    #[test]
    fn custom_tool_call_stream_restores_custom_input_events() {
        let adapter = OpenAIResponsesOutputAdapter;
        let mut initial_status = SseStatus::new(
            "msg_custom".to_string(),
            "deepseek-v4-pro".to_string(),
            false,
            10.0,
        );
        initial_status
            .responses_custom_tool_names
            .insert("exec".to_string());
        let status = Arc::new(RwLock::new(initial_status));

        let start = adapter
            .adapt_stream_chunk(
                UnifiedStreamChunk::ToolUseStart {
                    tool_type: "tool_use".to_string(),
                    id: "call_custom".to_string(),
                    name: "exec".to_string(),
                    index: 1,
                },
                status.clone(),
            )
            .expect("tool start should adapt")
            .into_iter()
            .map(|event| event.to_string())
            .collect::<String>();
        assert!(start.contains("response.output_item.added"));
        assert!(start.contains("\"type\":\"custom_tool_call\""));
        assert!(start.contains("\"input\":\"\""));
        assert!(!start.contains("\"arguments\""));

        let delta = adapter
            .adapt_stream_chunk(
                UnifiedStreamChunk::ToolUseDelta {
                    id: "call_custom".to_string(),
                    delta: "{\"input\":\"const value = await run();\"}".to_string(),
                    index: 1,
                },
                status.clone(),
            )
            .expect("tool delta should buffer");
        assert!(delta.is_empty());

        let end = adapter
            .adapt_stream_chunk(
                UnifiedStreamChunk::ToolUseEnd {
                    id: "call_custom".to_string(),
                },
                status.clone(),
            )
            .expect("tool end should adapt")
            .into_iter()
            .map(|event| event.to_string())
            .collect::<String>();
        assert!(end.contains("response.custom_tool_call_input.delta"));
        assert!(end.contains("response.custom_tool_call_input.done"));
        assert!(end.contains("const value = await run();"));
        assert!(end.contains("\"type\":\"custom_tool_call\""));
        assert!(!end.contains("response.function_call_arguments"));

        let completed = adapter
            .adapt_stream_chunk(
                UnifiedStreamChunk::MessageStop {
                    stop_reason: "tool_calls".to_string(),
                    usage: UnifiedUsage::default(),
                },
                status,
            )
            .expect("message stop should adapt")
            .into_iter()
            .map(|event| event.to_string())
            .collect::<String>();
        assert!(completed.contains("response.completed"));
        assert!(completed.contains("\"type\":\"custom_tool_call\""));
        assert!(completed.contains("\"call_id\":\"call_custom\""));
        assert!(completed.contains("const value = await run();"));
    }

    #[test]
    fn message_stop_completes_pending_custom_tool_call_from_any_backend() {
        let adapter = OpenAIResponsesOutputAdapter;
        let mut initial_status = SseStatus::new(
            "msg_custom".to_string(),
            "backend-model".to_string(),
            false,
            10.0,
        );
        initial_status
            .responses_custom_tool_names
            .insert("exec".to_string());
        let status = Arc::new(RwLock::new(initial_status));

        adapter
            .adapt_stream_chunk(
                UnifiedStreamChunk::ToolUseStart {
                    tool_type: "tool_use".to_string(),
                    id: "call_pending".to_string(),
                    name: "exec".to_string(),
                    index: 1,
                },
                status.clone(),
            )
            .expect("tool start should adapt");
        adapter
            .adapt_stream_chunk(
                UnifiedStreamChunk::ToolUseDelta {
                    id: "call_pending".to_string(),
                    delta: "{\"input\":\"const value = await run();\"}".to_string(),
                    index: 1,
                },
                status.clone(),
            )
            .expect("tool delta should buffer");

        let completed = adapter
            .adapt_stream_chunk(
                UnifiedStreamChunk::MessageStop {
                    stop_reason: "tool_calls".to_string(),
                    usage: UnifiedUsage::default(),
                },
                status,
            )
            .expect("message stop should finalize pending tools")
            .into_iter()
            .map(|event| event.to_string())
            .collect::<String>();

        assert!(completed.contains("response.custom_tool_call_input.delta"));
        assert!(completed.contains("response.custom_tool_call_input.done"));
        assert!(completed.contains("response.output_item.done"));
        assert!(completed.contains("response.completed"));
        assert!(completed.contains("\"type\":\"custom_tool_call\""));
        assert!(completed.contains("\"call_id\":\"call_pending\""));
        assert!(completed.contains("const value = await run();"));
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
