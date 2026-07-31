use reqwest::Response;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

use crate::ai::error::AiError;
use crate::ai::network::TokenUsage;
use crate::ai::traits::chat::{
    ChatMetadata, ChatResponse, FinishReason, MCPToolDeclaration, MessageType, ToolCallDeclaration,
};
use crate::ccproxy::adapter::input::helper::thinking_adapter::build_openai_compat_thinking_fields;
use crate::ccproxy::{StreamFormat, StreamProcessor};

use super::openai::OpenAIChat;

const RESPONSES_ENDPOINT: &str = "/v1/responses";

pub(crate) struct ResponsesRequestContext<'a> {
    pub model: &'a str,
    pub messages: &'a [Value],
    pub tools: &'a Option<Vec<MCPToolDeclaration>>,
    pub metadata: &'a ChatMetadata,
    pub model_metadata: &'a Option<Value>,
    pub params: &'a Value,
    pub stream: bool,
}

#[derive(Debug, Default)]
struct ResponsesToolAccumulator {
    index_by_item_id: HashMap<String, u32>,
    calls: HashMap<u32, ToolCallDeclaration>,
    next_index: u32,
}

impl ResponsesToolAccumulator {
    fn register_call(&mut self, item_id: Option<&str>, call_id: Option<&str>, name: Option<&str>) {
        self.call_entry(item_id, call_id, name);
    }

    fn append_arguments_delta(
        &mut self,
        item_id: Option<&str>,
        call_id: Option<&str>,
        name: Option<&str>,
        delta: Option<&str>,
    ) {
        if let Some(delta) = delta.filter(|delta| !delta.is_empty()) {
            let call = self.call_entry(item_id, call_id, name);
            call.arguments
                .get_or_insert_with(String::new)
                .push_str(delta);
        }
    }

    fn set_complete_arguments(
        &mut self,
        item_id: Option<&str>,
        call_id: Option<&str>,
        name: Option<&str>,
        arguments: Option<&str>,
    ) {
        let call = self.call_entry(item_id, call_id, name);
        if let Some(arguments) = arguments {
            call.arguments = Some(arguments.to_string());
        }
    }

    fn call_entry(
        &mut self,
        item_id: Option<&str>,
        call_id: Option<&str>,
        name: Option<&str>,
    ) -> &mut ToolCallDeclaration {
        let key = item_id.or(call_id).unwrap_or_default();
        let index = if key.is_empty() {
            let index = self.next_index;
            self.next_index += 1;
            index
        } else if let Some(index) = self.index_by_item_id.get(key) {
            *index
        } else {
            let index = self.next_index;
            self.next_index += 1;
            self.index_by_item_id.insert(key.to_string(), index);
            if let Some(call_id) = call_id.filter(|call_id| *call_id != key) {
                self.index_by_item_id.insert(call_id.to_string(), index);
            }
            index
        };

        let call = self
            .calls
            .entry(index)
            .or_insert_with(|| ToolCallDeclaration {
                index,
                id: call_id.or(item_id).unwrap_or_default().to_string(),
                name: name.unwrap_or_default().to_string(),
                arguments: Some(String::new()),
                results: None,
            });

        if call.id.is_empty() {
            call.id = call_id.or(item_id).unwrap_or_default().to_string();
        }
        if call.name.is_empty() {
            call.name = name.unwrap_or_default().to_string();
        }
        call
    }

    fn into_calls(self) -> HashMap<u32, ToolCallDeclaration> {
        self.calls
    }
}

pub(crate) fn responses_endpoint() -> &'static str {
    RESPONSES_ENDPOINT
}

pub(crate) fn supports_responses_api(metadata: &Option<Value>) -> bool {
    metadata
        .as_ref()
        .and_then(|metadata| {
            metadata
                .get("supportsResponsesApi")
                .or_else(|| metadata.get("supports_responses_api"))
        })
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

pub(crate) fn build_responses_payload(ctx: ResponsesRequestContext<'_>) -> Value {
    let mut payload = json!({
        "model": ctx.model,
        "input": convert_messages_to_responses_input(ctx.messages),
        "stream": ctx.stream,
    });

    if let Some(instructions) = collect_instructions(ctx.messages) {
        payload["instructions"] = Value::String(instructions);
    }

    crate::ai::util::merge_custom_params_value(&mut payload, ctx.model_metadata);
    crate::ai::util::merge_custom_params(&mut payload, &Some(ctx.metadata.clone()));

    if let Some(obj) = payload.as_object_mut() {
        // Keep the payload on the OpenAI Responses surface. Chat Completions-only
        // controls are removed here instead of being silently forwarded through
        // custom params with ambiguous provider behavior.
        for unsupported_key in [
            "messages",
            "stream_options",
            "response_format",
            "max_tokens",
            "stop",
            "stop_sequences",
            "n",
            "candidate_count",
            "presence_penalty",
            "frequency_penalty",
            "user_id",
        ] {
            obj.remove(unsupported_key);
        }

        if let Some(max_tokens) = ctx.metadata.max_tokens.filter(|value| *value > 0) {
            obj.insert("max_output_tokens".to_string(), json!(max_tokens));
        }
        if let Some(temperature) = ctx
            .metadata
            .temperature
            .filter(|value| *value >= 0.0 && *value <= 2.0)
        {
            obj.insert("temperature".to_string(), json!(temperature));
        }
        if let Some(top_p) = ctx
            .metadata
            .top_p
            .filter(|value| *value > 0.0 && *value <= 1.0)
        {
            obj.insert("top_p".to_string(), json!(top_p));
        }
        if let Some(user) = responses_user(&ctx) {
            obj.insert("user".to_string(), user);
        }
        if let Some(response_format) = responses_text_format(&ctx) {
            obj.insert("text".to_string(), json!({ "format": response_format }));
        }
        if let Some(tool_choice) = ctx
            .params
            .get("tool_choice")
            .or(ctx.metadata.tool_choice.as_ref())
        {
            obj.insert("tool_choice".to_string(), tool_choice.clone());
        }
        if let Some(reasoning_effort) = reasoning_effort(ctx.model, ctx.metadata, obj) {
            obj.insert(
                "reasoning".to_string(),
                json!({ "effort": reasoning_effort }),
            );
        }

        if let Some(tools) = ctx.tools.as_ref() {
            let responses_tools: Vec<Value> = tools
                .iter()
                .map(|tool| {
                    json!({
                        "type": "function",
                        "name": &tool.name,
                        "description": &tool.description,
                        "parameters": &tool.input_schema,
                    })
                })
                .collect();
            if !responses_tools.is_empty() {
                obj.insert("tools".to_string(), Value::Array(responses_tools));
            }
        }
    }

    payload
}

fn responses_user(ctx: &ResponsesRequestContext<'_>) -> Option<Value> {
    ctx.params
        .get("user")
        .or_else(|| ctx.params.get("user_id"))
        .filter(|value| !value.is_null())
        .cloned()
        .or_else(|| {
            ctx.metadata
                .user_id
                .as_ref()
                .map(|user| Value::String(user.clone()))
        })
}

fn responses_text_format(ctx: &ResponsesRequestContext<'_>) -> Option<Value> {
    if let Some(response_format) = ctx.params.get("response_format") {
        return Some(response_format.clone());
    }
    ctx.metadata.response_format.clone()
}

fn reasoning_effort(
    model: &str,
    metadata: &ChatMetadata,
    obj: &Map<String, Value>,
) -> Option<String> {
    obj.get("reasoning")
        .and_then(|reasoning| reasoning.get("effort"))
        .and_then(Value::as_str)
        .or_else(|| obj.get("reasoning_effort").and_then(Value::as_str))
        .map(str::to_string)
        .or_else(|| {
            build_openai_compat_thinking_fields(model, metadata.thinking.as_ref()).reasoning_effort
        })
}

fn collect_instructions(messages: &[Value]) -> Option<String> {
    let instructions = messages
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("system"))
        .filter_map(|message| message_text_content(message.get("content")?))
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

    (!instructions.is_empty()).then_some(instructions)
}

fn convert_messages_to_responses_input(messages: &[Value]) -> Value {
    let input = messages
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) != Some("system"))
        .flat_map(convert_message_to_input_items)
        .collect::<Vec<_>>();

    Value::Array(input)
}

fn convert_message_to_input_items(message: &Value) -> Vec<Value> {
    match message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "user" => vec![json!({
            "role": "user",
            "content": convert_content_parts(message.get("content"), true),
        })],
        "assistant" => {
            let mut items = Vec::new();
            if let Some(content) = message.get("content") {
                let parts = convert_content_parts(Some(content), false);
                if parts.as_array().is_some_and(|parts| !parts.is_empty()) {
                    items.push(json!({
                        "role": "assistant",
                        "content": parts,
                    }));
                }
            }
            if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
                for tool_call in tool_calls {
                    let function = tool_call.get("function").unwrap_or(&Value::Null);
                    items.push(json!({
                        "type": "function_call",
                        "call_id": tool_call.get("id").and_then(Value::as_str).unwrap_or_default(),
                        "name": function.get("name").and_then(Value::as_str).unwrap_or_default(),
                        "arguments": function.get("arguments").and_then(Value::as_str).unwrap_or_default(),
                    }));
                }
            }
            items
        }
        "tool" => vec![json!({
            "type": "function_call_output",
            "call_id": message
                .get("tool_call_id")
                .or_else(|| message.get("call_id"))
                .and_then(Value::as_str)
                .unwrap_or_default(),
            "output": message.get("content").cloned().unwrap_or(Value::String(String::new())),
        })],
        role if !role.is_empty() => vec![json!({
            "role": role,
            "content": convert_content_parts(message.get("content"), role == "user"),
        })],
        _ => Vec::new(),
    }
}

fn convert_content_parts(content: Option<&Value>, input_role: bool) -> Value {
    match content {
        Some(Value::Array(parts)) => Value::Array(
            parts
                .iter()
                .filter_map(|part| convert_content_part(part, input_role))
                .collect(),
        ),
        Some(Value::String(text)) => {
            let part_type = if input_role {
                "input_text"
            } else {
                "output_text"
            };
            Value::Array(vec![json!({ "type": part_type, "text": text })])
        }
        Some(other) if !other.is_null() => {
            let part_type = if input_role {
                "input_text"
            } else {
                "output_text"
            };
            Value::Array(vec![
                json!({ "type": part_type, "text": other.to_string() }),
            ])
        }
        _ => Value::Array(Vec::new()),
    }
}

fn convert_content_part(part: &Value, input_role: bool) -> Option<Value> {
    let part_type = part.get("type").and_then(Value::as_str).unwrap_or_default();
    match part_type {
        "text" | "input_text" | "output_text" => {
            let target_type = if input_role {
                "input_text"
            } else {
                "output_text"
            };
            Some(json!({
                "type": target_type,
                "text": part.get("text").and_then(Value::as_str).unwrap_or_default(),
            }))
        }
        "image_url" | "input_image" => {
            let image_url = part
                .get("image_url")
                .and_then(|image| image.get("url").or(Some(image)))
                .or_else(|| part.get("url"))
                .cloned()
                .unwrap_or(Value::Null);
            if image_url.is_null() {
                None
            } else {
                let mut converted = json!({ "type": "input_image", "image_url": image_url });
                if let Some(detail) = part.get("detail").and_then(Value::as_str).or_else(|| {
                    part.get("image_url")
                        .and_then(|image| image.get("detail"))
                        .and_then(Value::as_str)
                }) {
                    converted["detail"] = Value::String(detail.to_string());
                }
                Some(converted)
            }
        }
        _ => None,
    }
}

fn message_text_content(content: &Value) -> Option<String> {
    match content {
        Value::String(text) => Some(text.clone()),
        Value::Array(parts) => {
            let text = parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            (!text.is_empty()).then_some(text)
        }
        other if !other.is_null() => Some(other.to_string()),
        _ => None,
    }
}

pub(crate) async fn handle_response(
    chat_id: String,
    response: Response,
    callback: impl Fn(Arc<ChatResponse>) + Send + 'static,
    metadata_option: Option<ChatMetadata>,
    provider_name: String,
    is_stream: bool,
    stop_flag: Arc<Mutex<bool>>,
) -> Result<String, AiError> {
    if !is_stream {
        let response_text = match response.text().await {
            Ok(text) => text,
            Err(e) => {
                let err = AiError::ApiRequestFailed {
                    status_code: 500,
                    provider: provider_name.clone(),
                    details: format!("Failed to read response body: {}", e),
                };
                emit_error_response(&chat_id, &err.to_string(), &metadata_option, &callback);
                return Err(err);
            }
        };
        return handle_non_stream_response_text(
            chat_id,
            response_text,
            callback,
            metadata_option,
            provider_name,
        );
    }

    handle_stream_response(
        chat_id,
        response,
        callback,
        metadata_option,
        provider_name,
        stop_flag,
    )
    .await
}

fn responses_error_details(error: &Value) -> String {
    error
        .get("message")
        .or_else(|| error.get("error").and_then(|nested| nested.get("message")))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| error.to_string())
}

fn non_stream_terminal_failure_details(response: &Value, status: &str) -> String {
    if let Some(error) = response.get("error").filter(|error| !error.is_null()) {
        return responses_error_details(error);
    }

    if let Some(details) = response
        .get("incomplete_details")
        .filter(|details| !details.is_null())
    {
        return details.to_string();
    }

    format!("Responses request ended with status: {}", status)
}

pub(crate) fn handle_non_stream_response_text(
    chat_id: String,
    response_text: String,
    callback: impl Fn(Arc<ChatResponse>) + Send + 'static,
    metadata_option: Option<ChatMetadata>,
    provider_name: String,
) -> Result<String, AiError> {
    let parsed: Value = match serde_json::from_str(&response_text) {
        Ok(parsed) => parsed,
        Err(e) => {
            let err = AiError::ApiRequestFailed {
                status_code: 500,
                provider: provider_name.clone(),
                details: format!("Failed to parse response: {}", e),
            };
            emit_error_response(&chat_id, &err.to_string(), &metadata_option, &callback);
            return Err(err);
        }
    };

    if let Some(error) = parsed.get("error").filter(|error| !error.is_null()) {
        let details = responses_error_details(error);
        let err = AiError::ApiRequestFailed {
            status_code: 500,
            provider: provider_name,
            details: details.clone(),
        };
        callback(ChatResponse::new_with_arc(
            chat_id,
            details,
            MessageType::Error,
            metadata_option.as_ref().and_then(|m| m.to_value()),
            Some(FinishReason::Error),
        ));
        return Err(err);
    }

    if let Some(status) = parsed.get("status").and_then(Value::as_str) {
        if !status.eq_ignore_ascii_case("completed") {
            let details = non_stream_terminal_failure_details(&parsed, status);
            let err = AiError::ApiRequestFailed {
                status_code: 500,
                provider: provider_name,
                details: details.clone(),
            };
            emit_error_response(&chat_id, &details, &metadata_option, &callback);
            return Err(err);
        }
    }

    let mut content = String::new();
    let mut reasoning_content = String::new();
    let mut accumulated_tool_calls = HashMap::new();
    collect_response_output(
        &parsed,
        &mut content,
        &mut reasoning_content,
        &mut accumulated_tool_calls,
    );

    emit_collected_response(
        chat_id,
        content,
        reasoning_content,
        accumulated_tool_calls,
        usage_from_response(&parsed),
        metadata_option,
        FinishReason::Complete,
        callback,
    )
}

async fn handle_stream_response(
    chat_id: String,
    response: Response,
    callback: impl Fn(Arc<ChatResponse>) + Send + 'static,
    metadata_option: Option<ChatMetadata>,
    provider_name: String,
    stop_flag: Arc<Mutex<bool>>,
) -> Result<String, AiError> {
    let processor = StreamProcessor::new();
    let mut event_receiver = processor
        .process_stream(response, &StreamFormat::OpenAI)
        .await;
    let mut content = String::new();
    let mut reasoning_content = String::new();
    let mut token_usage = TokenUsage::default();
    let mut tool_accumulator = ResponsesToolAccumulator::default();
    let start_time = Instant::now();

    let mut stopped = false;
    let mut completed = false;

    while let Some(event) = event_receiver.recv().await {
        if *stop_flag.lock().await {
            processor.stop();
            stopped = true;
            break;
        }

        let event = match event {
            Ok(event) => event,
            Err(e) => {
                let err = AiError::StreamProcessingFailed {
                    provider: provider_name.clone(),
                    details: e,
                };
                emit_error_response(&chat_id, &err.to_string(), &metadata_option, &callback);
                return Err(err);
            }
        };
        let event_text = String::from_utf8_lossy(&event);
        match process_stream_event_text(
            &event_text,
            &mut content,
            &mut reasoning_content,
            &mut token_usage,
            &mut tool_accumulator,
            &chat_id,
            &metadata_option,
            &provider_name,
            &callback,
        ) {
            Ok(StreamEventOutcome::Continue) => {}
            Ok(StreamEventOutcome::Completed) => {
                completed = true;
            }
            Ok(StreamEventOutcome::Failed(details)) => {
                let err = AiError::StreamProcessingFailed {
                    provider: provider_name.clone(),
                    details,
                };
                return Err(err);
            }
            Err(err) => {
                emit_error_response(&chat_id, &err.to_string(), &metadata_option, &callback);
                return Err(err);
            }
        }
    }

    if stopped {
        log::info!(
            "Skipping Responses completion and tool call processing for chat_id {} because stop_flag is set",
            chat_id
        );
        return Ok(json!({
            "reasoning": reasoning_content,
            "content": content
        })
        .to_string());
    }

    if !completed {
        let details = "Responses stream ended before response.completed".to_string();
        emit_error_response(&chat_id, &details, &metadata_option, &callback);
        return Err(AiError::StreamProcessingFailed {
            provider: provider_name.clone(),
            details,
        });
    }

    if token_usage.tokens_per_second == 0.0 {
        let duration = start_time.elapsed();
        token_usage.tokens_per_second = if duration.as_secs_f64() > 0.1 {
            token_usage.completion_tokens as f64 / duration.as_secs_f64()
        } else {
            0.0
        };
    }

    let mut accumulated_tool_calls = tool_accumulator.into_calls();
    if !accumulated_tool_calls.is_empty() {
        OpenAIChat::emit_tool_calls(
            &mut accumulated_tool_calls,
            &content,
            &chat_id,
            &metadata_option,
            &provider_name,
            &callback,
        );
    }

    callback(ChatResponse::new_with_arc(
        chat_id.clone(),
        String::new(),
        MessageType::Finished,
        {
            let mut meta = metadata_option.unwrap_or_default();
            meta.update_usage(
                token_usage.total_tokens,
                token_usage.prompt_tokens,
                token_usage.completion_tokens,
                token_usage.tokens_per_second,
            );
            meta.to_value()
        },
        Some(FinishReason::Complete),
    ));

    Ok(json!({
        "reasoning": reasoning_content,
        "content": content
    })
    .to_string())
}

fn process_stream_event_text<F: Fn(Arc<ChatResponse>) + Send + 'static>(
    event_text: &str,
    content: &mut String,
    reasoning_content: &mut String,
    token_usage: &mut TokenUsage,
    tool_accumulator: &mut ResponsesToolAccumulator,
    chat_id: &str,
    metadata_option: &Option<ChatMetadata>,
    provider_name: &str,
    callback: &F,
) -> Result<StreamEventOutcome, AiError> {
    for data in event_text
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .filter(|data| !data.is_empty() && *data != "[DONE]")
    {
        let value: Value =
            serde_json::from_str(data).map_err(|e| AiError::StreamProcessingFailed {
                provider: provider_name.to_string(),
                details: format!("Failed to parse Responses stream event: {}", e),
            })?;
        match process_stream_event(
            &value,
            content,
            reasoning_content,
            token_usage,
            tool_accumulator,
            chat_id,
            metadata_option,
            provider_name,
            callback,
        )? {
            StreamEventOutcome::Continue => {}
            StreamEventOutcome::Completed => return Ok(StreamEventOutcome::Completed),
            StreamEventOutcome::Failed(details) => return Ok(StreamEventOutcome::Failed(details)),
        }
    }
    Ok(StreamEventOutcome::Continue)
}

enum StreamEventOutcome {
    Continue,
    Completed,
    Failed(String),
}

fn process_stream_event<F: Fn(Arc<ChatResponse>) + Send + 'static>(
    value: &Value,
    content: &mut String,
    reasoning_content: &mut String,
    token_usage: &mut TokenUsage,
    tool_accumulator: &mut ResponsesToolAccumulator,
    chat_id: &str,
    metadata_option: &Option<ChatMetadata>,
    provider_name: &str,
    callback: &F,
) -> Result<StreamEventOutcome, AiError> {
    let event_type = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match event_type {
        "response.output_text.delta" => {
            if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                content.push_str(delta);
                callback(ChatResponse::new_with_arc(
                    chat_id.to_string(),
                    delta.to_string(),
                    MessageType::Text,
                    metadata_option.as_ref().and_then(|m| m.to_value()),
                    None,
                ));
            }
        }
        "response.reasoning_summary_text.delta" => {
            if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                reasoning_content.push_str(delta);
                callback(ChatResponse::new_with_arc(
                    chat_id.to_string(),
                    delta.to_string(),
                    MessageType::Reasoning,
                    metadata_option.as_ref().and_then(|m| m.to_value()),
                    None,
                ));
            }
        }
        "response.output_item.added" => {
            if let Some(item) = value.get("item") {
                register_stream_tool_item(tool_accumulator, item, false);
            }
        }
        "response.output_item.done" => {
            if let Some(item) = value.get("item") {
                register_stream_tool_item(tool_accumulator, item, true);
            }
        }
        "response.function_call_arguments.delta" => {
            tool_accumulator.append_arguments_delta(
                value.get("item_id").and_then(Value::as_str),
                value.get("call_id").and_then(Value::as_str),
                value.get("name").and_then(Value::as_str),
                value.get("delta").and_then(Value::as_str),
            );
        }
        "response.function_call_arguments.done" => {
            tool_accumulator.set_complete_arguments(
                value.get("item_id").and_then(Value::as_str),
                value.get("call_id").and_then(Value::as_str),
                value.get("name").and_then(Value::as_str),
                value.get("arguments").and_then(Value::as_str),
            );
        }
        "response.completed" => {
            if let Some(response) = value.get("response") {
                if token_usage.total_tokens == 0 {
                    *token_usage = usage_from_response(response);
                }
            }
            return Ok(StreamEventOutcome::Completed);
        }
        "response.incomplete" => {
            let details = response_terminal_details(value, "Responses stream incomplete");
            emit_error_response(chat_id, &details, metadata_option, callback);
            return Ok(StreamEventOutcome::Failed(details));
        }
        "response.failed" => {
            let details = response_terminal_details(value, "Responses stream failed");
            emit_error_response(chat_id, &details, metadata_option, callback);
            return Ok(StreamEventOutcome::Failed(details));
        }
        _ => {}
    }

    log::trace!(
        "{} Responses stream event handled: {}",
        provider_name,
        event_type
    );
    Ok(StreamEventOutcome::Continue)
}

fn response_terminal_details(value: &Value, fallback: &str) -> String {
    if let Some(message) = value
        .get("response")
        .and_then(|response| response.get("error"))
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
    {
        return message.to_string();
    }

    if let Some(details) = value
        .get("response")
        .and_then(|response| response.get("incomplete_details"))
        .filter(|details| !details.is_null())
    {
        return details.to_string();
    }

    value
        .get("response")
        .and_then(|response| response.get("error"))
        .cloned()
        .unwrap_or_else(|| Value::String(fallback.to_string()))
        .to_string()
}

fn register_stream_tool_item(
    tool_accumulator: &mut ResponsesToolAccumulator,
    item: &Value,
    finalize_arguments: bool,
) {
    if item.get("type").and_then(Value::as_str) != Some("function_call") {
        return;
    }

    if finalize_arguments {
        tool_accumulator.set_complete_arguments(
            item.get("id").and_then(Value::as_str),
            item.get("call_id").and_then(Value::as_str),
            item.get("name").and_then(Value::as_str),
            item.get("arguments").and_then(Value::as_str),
        );
    } else {
        tool_accumulator.register_call(
            item.get("id").and_then(Value::as_str),
            item.get("call_id").and_then(Value::as_str),
            item.get("name").and_then(Value::as_str),
        );
    }
}

fn emit_error_response<F: Fn(Arc<ChatResponse>) + Send + 'static>(
    chat_id: &str,
    details: &str,
    metadata_option: &Option<ChatMetadata>,
    callback: &F,
) {
    callback(ChatResponse::new_with_arc(
        chat_id.to_string(),
        details.to_string(),
        MessageType::Error,
        metadata_option.as_ref().and_then(|m| m.to_value()),
        Some(FinishReason::Error),
    ));
}

fn collect_response_output(
    response: &Value,
    content: &mut String,
    reasoning_content: &mut String,
    accumulated_tool_calls: &mut HashMap<u32, ToolCallDeclaration>,
) {
    if let Some(output) = response.get("output").and_then(Value::as_array) {
        for item in output {
            collect_response_item(item, content, reasoning_content, accumulated_tool_calls);
        }
    }
}

fn collect_response_item(
    item: &Value,
    content: &mut String,
    reasoning_content: &mut String,
    accumulated_tool_calls: &mut HashMap<u32, ToolCallDeclaration>,
) {
    match item.get("type").and_then(Value::as_str).unwrap_or_default() {
        "message" => {
            if let Some(parts) = item.get("content").and_then(Value::as_array) {
                for part in parts {
                    if part.get("type").and_then(Value::as_str) == Some("output_text") {
                        if let Some(text) = part.get("text").and_then(Value::as_str) {
                            content.push_str(text);
                        }
                    }
                }
            }
        }
        "reasoning" => {
            if let Some(summary) = item.get("summary").and_then(Value::as_array) {
                let text = summary
                    .iter()
                    .filter_map(|part| part.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n");
                if !text.is_empty() {
                    if !reasoning_content.is_empty() {
                        reasoning_content.push('\n');
                    }
                    reasoning_content.push_str(&text);
                }
            }
        }
        "function_call" => {
            let index = accumulated_tool_calls.len() as u32;
            accumulated_tool_calls.insert(
                index,
                ToolCallDeclaration {
                    index,
                    id: item
                        .get("call_id")
                        .or_else(|| item.get("id"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    name: item
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    arguments: Some(
                        item.get("arguments")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    ),
                    results: None,
                },
            );
        }
        _ => {}
    }
}

fn usage_from_response(response: &Value) -> TokenUsage {
    let Some(usage) = response.get("usage") else {
        return TokenUsage::default();
    };

    TokenUsage {
        total_tokens: usage
            .get("total_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        prompt_tokens: usage
            .get("input_tokens")
            .or_else(|| usage.get("prompt_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        completion_tokens: usage
            .get("output_tokens")
            .or_else(|| usage.get("completion_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        tokens_per_second: 0.0,
    }
}

fn emit_collected_response(
    chat_id: String,
    content: String,
    reasoning_content: String,
    mut accumulated_tool_calls: HashMap<u32, ToolCallDeclaration>,
    token_usage: TokenUsage,
    metadata_option: Option<ChatMetadata>,
    finish_reason: FinishReason,
    callback: impl Fn(Arc<ChatResponse>) + Send + 'static,
) -> Result<String, AiError> {
    if !reasoning_content.is_empty() {
        callback(ChatResponse::new_with_arc(
            chat_id.clone(),
            reasoning_content.clone(),
            MessageType::Reasoning,
            metadata_option.as_ref().and_then(|m| m.to_value()),
            None,
        ));
    }

    if !content.is_empty() {
        callback(ChatResponse::new_with_arc(
            chat_id.clone(),
            content.clone(),
            MessageType::Text,
            metadata_option.as_ref().and_then(|m| m.to_value()),
            None,
        ));
    }

    if !accumulated_tool_calls.is_empty() {
        OpenAIChat::emit_tool_calls(
            &mut accumulated_tool_calls,
            &content,
            &chat_id,
            &metadata_option,
            "OpenAI Responses",
            &callback,
        );
    }

    callback(ChatResponse::new_with_arc(
        chat_id.clone(),
        String::new(),
        MessageType::Finished,
        {
            let mut meta = metadata_option.unwrap_or_default();
            meta.update_usage(
                token_usage.total_tokens,
                token_usage.prompt_tokens,
                token_usage.completion_tokens,
                token_usage.tokens_per_second,
            );
            meta.to_value()
        },
        Some(finish_reason),
    ));

    Ok(json!({
        "reasoning": reasoning_content,
        "content": content
    })
    .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex as StdMutex;

    #[test]
    fn routing_helpers_preserve_responses_and_compat_contract() {
        assert_eq!(responses_endpoint(), "/v1/responses");
        assert!(supports_responses_api(&Some(
            json!({ "supportsResponsesApi": true })
        )));
        assert!(supports_responses_api(&Some(
            json!({ "supports_responses_api": true })
        )));
        assert!(!supports_responses_api(&Some(
            json!({ "supportsResponsesApi": false })
        )));
        assert!(!supports_responses_api(&None));
    }

    #[test]
    fn supports_responses_api_accepts_camel_and_snake_case() {
        assert!(supports_responses_api(&Some(
            json!({ "supportsResponsesApi": true })
        )));
        assert!(supports_responses_api(&Some(
            json!({ "supports_responses_api": true })
        )));
        assert!(!supports_responses_api(&Some(
            json!({ "supportsResponsesApi": false })
        )));
        assert!(!supports_responses_api(&None));
    }

    #[test]
    fn build_payload_maps_messages_tools_and_workflow_tool_choice() {
        let metadata = ChatMetadata {
            max_tokens: Some(1024),
            temperature: Some(0.7),
            tool_choice: Some(json!("required")),
            ..Default::default()
        };
        let params = json!({ "tool_choice": "required" });
        let tools = Some(vec![MCPToolDeclaration {
            name: "lookup".to_string(),
            description: "Look up data".to_string(),
            input_schema: json!({ "type": "object", "properties": { "query": { "type": "string" } } }),
            output_schema: None,
            disabled: false,
            scope: None,
        }]);
        let messages = vec![
            json!({ "role": "system", "content": "You are helpful." }),
            json!({ "role": "user", "content": [{ "type": "text", "text": "hello" }, { "type": "image_url", "image_url": { "url": "data:image/png;base64,abc", "detail": "low" } }] }),
        ];

        let payload = build_responses_payload(ResponsesRequestContext {
            model: "gpt-4.1",
            messages: &messages,
            tools: &tools,
            metadata: &metadata,
            model_metadata: &None,
            params: &params,
            stream: true,
        });

        assert_eq!(payload["model"], "gpt-4.1");
        assert_eq!(payload["instructions"], "You are helpful.");
        assert_eq!(payload["max_output_tokens"], 1024);
        assert_eq!(payload["tool_choice"], "required");
        assert_eq!(payload["tools"][0]["name"], "lookup");
        assert_eq!(payload["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(payload["input"][0]["content"][1]["type"], "input_image");
        assert!(payload.get("messages").is_none());
    }

    #[test]
    fn build_payload_maps_assistant_tool_history_and_tool_output() {
        let metadata = ChatMetadata::default();
        let params = json!({});
        let tools = None;
        let messages = vec![
            json!({
                "role": "assistant",
                "content": "Need a lookup",
                "tool_calls": [{
                    "id": "call_1",
                    "type": "function",
                    "function": { "name": "lookup", "arguments": "{\"query\":\"x\"}" }
                }]
            }),
            json!({ "role": "tool", "tool_call_id": "call_1", "content": "found" }),
        ];

        let payload = build_responses_payload(ResponsesRequestContext {
            model: "gpt-4.1",
            messages: &messages,
            tools: &tools,
            metadata: &metadata,
            model_metadata: &None,
            params: &params,
            stream: false,
        });

        assert_eq!(payload["input"][0]["role"], "assistant");
        assert_eq!(payload["input"][1]["type"], "function_call");
        assert_eq!(payload["input"][1]["call_id"], "call_1");
        assert_eq!(payload["input"][1]["name"], "lookup");
        assert_eq!(payload["input"][2]["type"], "function_call_output");
        assert_eq!(payload["input"][2]["call_id"], "call_1");
    }

    #[test]
    fn build_payload_maps_request_controls_and_filters_chat_only_fields() {
        let metadata = ChatMetadata {
            max_tokens: Some(2048),
            temperature: Some(0.8),
            top_p: Some(0.9),
            presence_penalty: Some(1.1),
            frequency_penalty: Some(1.2),
            response_format: Some(json!({ "type": "json_object" })),
            stop: Some(vec!["END".to_string()]),
            candidate_count: Some(3),
            user_id: Some("metadata-user".to_string()),
            tool_choice: Some(json!("auto")),
            custom_params: Some(vec![
                crate::ai::traits::chat::CustomParam {
                    key: "parallel_tool_calls".to_string(),
                    value: json!(false),
                },
                crate::ai::traits::chat::CustomParam {
                    key: "presence_penalty".to_string(),
                    value: json!(1.5),
                },
                crate::ai::traits::chat::CustomParam {
                    key: "candidate_count".to_string(),
                    value: json!(4),
                },
            ]),
            ..Default::default()
        };
        let params = json!({
            "response_format": { "type": "json_schema", "json_schema": { "name": "answer", "schema": { "type": "object" } } },
            "user": "params-user",
            "tool_choice": "required",
            "frequency_penalty": 0.7,
            "presence_penalty": 0.6,
            "stop": ["STOP"],
            "n": 2,
            "candidate_count": 2,
            "user_id": "legacy-user"
        });
        let messages = vec![json!({ "role": "user", "content": "hello" })];

        let payload = build_responses_payload(ResponsesRequestContext {
            model: "gpt-4.1",
            messages: &messages,
            tools: &None,
            metadata: &metadata,
            model_metadata: &Some(json!({
                "customParams": [
                    { "key": "store", "value": false },
                    { "key": "stop", "value": ["MODEL_STOP"] },
                    { "key": "frequency_penalty", "value": 1.8 }
                ]
            })),
            params: &params,
            stream: false,
        });

        assert_eq!(payload["max_output_tokens"], 2048);
        assert!((payload["temperature"].as_f64().unwrap_or_default() - 0.8).abs() < 0.0001);
        assert!((payload["top_p"].as_f64().unwrap_or_default() - 0.9).abs() < 0.0001);
        assert_eq!(payload["user"], "params-user");
        assert_eq!(payload["tool_choice"], "required");
        assert_eq!(payload["text"]["format"]["type"], "json_schema");
        assert_eq!(payload["parallel_tool_calls"], false);
        assert_eq!(payload["store"], false);
        assert!(payload.get("presence_penalty").is_none());
        assert!(payload.get("frequency_penalty").is_none());
        assert!(payload.get("stop").is_none());
        assert!(payload.get("stop_sequences").is_none());
        assert!(payload.get("n").is_none());
        assert!(payload.get("candidate_count").is_none());
        assert!(payload.get("user_id").is_none());
        assert!(payload.get("response_format").is_none());
        assert!(payload.get("max_tokens").is_none());
    }

    #[test]
    fn build_payload_maps_thinking_to_responses_reasoning_effort() {
        let metadata = ChatMetadata {
            thinking: Some(crate::db::ThinkingConfig {
                r#type: "enabled".to_string(),
                budget_tokens: Some(2048),
            }),
            ..Default::default()
        };
        let messages = vec![json!({ "role": "user", "content": "hello" })];

        let payload = build_responses_payload(ResponsesRequestContext {
            model: "gpt-4.1",
            messages: &messages,
            tools: &None,
            metadata: &metadata,
            model_metadata: &None,
            params: &json!({}),
            stream: false,
        });

        assert_eq!(payload["reasoning"]["effort"], "medium");
        assert!(payload.get("reasoning_effort").is_none());
        assert!(payload.get("thinking").is_none());
        assert!(payload.get("thinking_budget").is_none());
    }

    #[test]
    fn build_payload_prefers_explicit_responses_reasoning_effort() {
        let metadata = ChatMetadata {
            thinking: Some(crate::db::ThinkingConfig {
                r#type: "enabled".to_string(),
                budget_tokens: Some(2048),
            }),
            ..Default::default()
        };
        let messages = vec![json!({ "role": "user", "content": "hello" })];

        let payload = build_responses_payload(ResponsesRequestContext {
            model: "gpt-4.1",
            messages: &messages,
            tools: &None,
            metadata: &metadata,
            model_metadata: &Some(json!({
                "customParams": [
                    { "key": "reasoning", "value": { "effort": "high" } }
                ]
            })),
            params: &json!({}),
            stream: false,
        });

        assert_eq!(payload["reasoning"]["effort"], "high");
    }

    #[test]
    fn non_stream_success_with_null_error_emits_semantic_chunks_and_finished() {
        let emitted = Arc::new(StdMutex::new(Vec::<Arc<ChatResponse>>::new()));
        let emitted_for_callback = emitted.clone();
        let callback = move |response: Arc<ChatResponse>| {
            emitted_for_callback
                .lock()
                .expect("test mutex")
                .push(response);
        };
        let response = json!({
            "id": "resp_1",
            "error": null,
            "output": [
                { "type": "reasoning", "summary": [{ "type": "summary_text", "text": "think" }] },
                { "type": "message", "content": [{ "type": "output_text", "text": "answer" }] },
                { "type": "function_call", "call_id": "call_1", "name": "lookup", "arguments": "{\"query\":\"x\"}" }
            ],
            "usage": { "input_tokens": 3, "output_tokens": 5, "total_tokens": 8 }
        });

        let result = handle_non_stream_response_text(
            "chat_1".to_string(),
            response.to_string(),
            callback,
            None,
            "test".to_string(),
        )
        .expect("error:null success should parse");

        assert!(result.contains("answer"));
        let emitted = emitted.lock().expect("test mutex");
        assert_eq!(
            emitted
                .iter()
                .filter(|r| r.r#type == MessageType::Reasoning)
                .count(),
            1
        );
        assert_eq!(
            emitted
                .iter()
                .filter(|r| r.r#type == MessageType::Text)
                .count(),
            1
        );
        assert_eq!(
            emitted
                .iter()
                .filter(|r| r.r#type == MessageType::ToolCalls)
                .count(),
            1
        );
        let finished = emitted
            .iter()
            .find(|response| response.r#type == MessageType::Finished)
            .expect("finished response");
        assert_eq!(finished.finish_reason, Some(FinishReason::Complete));
        let metadata = finished.metadata.as_ref().expect("usage metadata");
        assert_eq!(metadata["tokens"]["total"], 8);
        assert_eq!(metadata["tokens"]["prompt"], 3);
        assert_eq!(metadata["tokens"]["completion"], 5);
        assert!(emitted.iter().all(|r| r.r#type != MessageType::Error));
    }

    #[test]
    fn non_stream_non_null_error_emits_error_callback() {
        let emitted = Arc::new(StdMutex::new(Vec::<Arc<ChatResponse>>::new()));
        let emitted_for_callback = emitted.clone();
        let callback = move |response: Arc<ChatResponse>| {
            emitted_for_callback
                .lock()
                .expect("test mutex")
                .push(response);
        };
        let response = json!({
            "error": { "message": "Responses failed", "type": "invalid_request_error" }
        });

        let result = handle_non_stream_response_text(
            "chat_1".to_string(),
            response.to_string(),
            callback,
            None,
            "test".to_string(),
        );

        assert!(result.is_err());
        let emitted = emitted.lock().expect("test mutex");
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].r#type, MessageType::Error);
        assert_eq!(emitted[0].chunk, "Responses failed");
        assert_eq!(emitted[0].finish_reason, Some(FinishReason::Error));
    }

    #[test]
    fn non_stream_incomplete_status_emits_only_error() {
        assert_non_stream_terminal_status_emits_only_error(
            json!({
                "id": "resp_1",
                "status": "incomplete",
                "error": null,
                "incomplete_details": { "reason": "max_output_tokens" },
                "output": partial_non_stream_output(),
            }),
            "max_output_tokens",
        );
    }

    #[test]
    fn non_stream_failed_status_without_error_emits_only_error() {
        assert_non_stream_terminal_status_emits_only_error(
            json!({
                "id": "resp_1",
                "status": "failed",
                "output": partial_non_stream_output(),
            }),
            "failed",
        );
    }

    #[test]
    fn non_stream_failed_status_with_null_error_emits_only_error() {
        assert_non_stream_terminal_status_emits_only_error(
            json!({
                "id": "resp_1",
                "status": "failed",
                "error": null,
                "output": partial_non_stream_output(),
            }),
            "failed",
        );
    }

    #[test]
    fn non_stream_failed_status_with_error_message_emits_only_error() {
        assert_non_stream_terminal_status_emits_only_error(
            json!({
                "id": "resp_1",
                "status": "failed",
                "error": { "message": "upstream failed" },
                "output": partial_non_stream_output(),
            }),
            "upstream failed",
        );
    }

    #[test]
    fn non_stream_cancelled_status_emits_only_error() {
        assert_non_stream_terminal_status_emits_only_error(
            json!({
                "id": "resp_1",
                "status": "cancelled",
                "output": partial_non_stream_output(),
            }),
            "cancelled",
        );
    }

    #[test]
    fn non_stream_in_progress_status_emits_only_error() {
        assert_non_stream_terminal_status_emits_only_error(
            json!({
                "id": "resp_1",
                "status": "in_progress",
                "output": partial_non_stream_output(),
            }),
            "in_progress",
        );
    }

    #[test]
    fn non_stream_completed_status_is_success_case_insensitively() {
        let emitted = Arc::new(StdMutex::new(Vec::<Arc<ChatResponse>>::new()));
        let emitted_for_callback = emitted.clone();
        let callback = move |response: Arc<ChatResponse>| {
            emitted_for_callback
                .lock()
                .expect("test mutex")
                .push(response);
        };
        let response = json!({
            "id": "resp_1",
            "status": "COMPLETED",
            "error": null,
            "output": [
                { "type": "message", "content": [{ "type": "output_text", "text": "answer" }] }
            ]
        });

        let result = handle_non_stream_response_text(
            "chat_1".to_string(),
            response.to_string(),
            callback,
            None,
            "test".to_string(),
        )
        .expect("completed status should parse");

        assert!(result.contains("answer"));
        let emitted = emitted.lock().expect("test mutex");
        assert!(emitted.iter().any(|r| r.r#type == MessageType::Text));
        assert!(emitted.iter().any(|r| {
            r.r#type == MessageType::Finished && r.finish_reason == Some(FinishReason::Complete)
        }));
        assert!(emitted.iter().all(|r| r.r#type != MessageType::Error));
    }

    fn assert_non_stream_terminal_status_emits_only_error(response: Value, expected_detail: &str) {
        let emitted = Arc::new(StdMutex::new(Vec::<Arc<ChatResponse>>::new()));
        let emitted_for_callback = emitted.clone();
        let callback = move |response: Arc<ChatResponse>| {
            emitted_for_callback
                .lock()
                .expect("test mutex")
                .push(response);
        };

        let result = handle_non_stream_response_text(
            "chat_1".to_string(),
            response.to_string(),
            callback,
            None,
            "test".to_string(),
        );

        assert!(result.is_err());
        let emitted = emitted.lock().expect("test mutex");
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].r#type, MessageType::Error);
        assert_eq!(emitted[0].finish_reason, Some(FinishReason::Error));
        assert!(
            emitted[0].chunk.contains(expected_detail),
            "error chunk should include {expected_detail:?}, got {:?}",
            emitted[0].chunk
        );
        assert!(emitted.iter().all(|r| r.r#type != MessageType::Finished));
        assert!(emitted.iter().all(|r| r.r#type != MessageType::ToolCalls));
        assert!(emitted.iter().all(|r| r.r#type != MessageType::Text));
        assert!(emitted.iter().all(|r| r.r#type != MessageType::Reasoning));
    }

    fn partial_non_stream_output() -> Value {
        json!([
            { "type": "reasoning", "summary": [{ "type": "summary_text", "text": "partial thinking" }] },
            { "type": "message", "content": [{ "type": "output_text", "text": "partial" }] },
            { "type": "function_call", "call_id": "call_1", "name": "lookup", "arguments": "{\"query\":\"x\"}" }
        ])
    }

    #[test]
    fn non_stream_parse_error_emits_error_callback() {
        let emitted = Arc::new(StdMutex::new(Vec::<Arc<ChatResponse>>::new()));
        let emitted_for_callback = emitted.clone();
        let callback = move |response: Arc<ChatResponse>| {
            emitted_for_callback
                .lock()
                .expect("test mutex")
                .push(response);
        };

        let result = handle_non_stream_response_text(
            "chat_1".to_string(),
            "not json".to_string(),
            callback,
            None,
            "test".to_string(),
        );

        assert!(result.is_err());
        let emitted = emitted.lock().expect("test mutex");
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].r#type, MessageType::Error);
        assert_eq!(emitted[0].finish_reason, Some(FinishReason::Error));
    }

    #[test]
    fn stream_event_text_handles_text_reasoning_usage_and_done_marker() {
        let mut content = String::new();
        let mut reasoning = String::new();
        let mut usage = TokenUsage::default();
        let mut accumulator = ResponsesToolAccumulator::default();
        let emitted = Arc::new(StdMutex::new(Vec::<Arc<ChatResponse>>::new()));
        let emitted_for_callback = emitted.clone();
        let callback = move |response: Arc<ChatResponse>| {
            emitted_for_callback
                .lock()
                .expect("test mutex")
                .push(response);
        };
        let event_text = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hello\"}\n\n",
            "data: {\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"think\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":2,\"output_tokens\":3,\"total_tokens\":5}}}\n\n",
            "data: [DONE]\n\n"
        );

        assert!(matches!(
            process_stream_event_text(
                event_text,
                &mut content,
                &mut reasoning,
                &mut usage,
                &mut accumulator,
                "chat_1",
                &None,
                "test",
                &callback,
            )
            .expect("stream event text should parse"),
            StreamEventOutcome::Completed
        ));

        assert_eq!(content, "hello");
        assert_eq!(reasoning, "think");
        assert_eq!(usage.prompt_tokens, 2);
        assert_eq!(usage.completion_tokens, 3);
        assert_eq!(usage.total_tokens, 5);
        let emitted = emitted.lock().expect("test mutex");
        assert_eq!(emitted.len(), 2);
        assert_eq!(emitted[0].r#type, MessageType::Text);
        assert_eq!(emitted[1].r#type, MessageType::Reasoning);
    }

    #[test]
    fn malformed_stream_event_text_emits_no_partial_error_by_itself() {
        let mut content = String::new();
        let mut reasoning = String::new();
        let mut usage = TokenUsage::default();
        let mut accumulator = ResponsesToolAccumulator::default();
        let callback = |response: Arc<ChatResponse>| drop(response);

        let result = process_stream_event_text(
            "data: {not-json}\n\n",
            &mut content,
            &mut reasoning,
            &mut usage,
            &mut accumulator,
            "chat_1",
            &None,
            "test",
            &callback,
        );

        assert!(result.is_err());
    }

    #[test]
    fn stopped_stream_accumulator_is_not_emitted_as_complete() {
        let mut accumulator = ResponsesToolAccumulator::default();
        accumulator.register_call(Some("fc_1"), Some("call_1"), Some("lookup"));
        accumulator.append_arguments_delta(Some("fc_1"), None, None, Some("{\"query\""));
        let partial_calls = accumulator.into_calls();
        assert_eq!(partial_calls.len(), 1);
        assert_eq!(partial_calls[&0].arguments.as_deref(), Some("{\"query\""));
        // handle_stream_response returns before converting this accumulator into ToolCalls when stopped.
    }

    #[test]
    fn non_stream_output_collection_extracts_text_reasoning_tool_and_usage() {
        let response = json!({
            "output": [
                { "type": "reasoning", "summary": [{ "type": "summary_text", "text": "think" }] },
                { "type": "message", "content": [{ "type": "output_text", "text": "answer" }] },
                { "type": "function_call", "call_id": "call_1", "name": "lookup", "arguments": "{\"query\":\"x\"}" }
            ],
            "usage": { "input_tokens": 3, "output_tokens": 5, "total_tokens": 8 }
        });
        let mut content = String::new();
        let mut reasoning = String::new();
        let mut calls = HashMap::new();

        collect_response_output(&response, &mut content, &mut reasoning, &mut calls);
        let usage = usage_from_response(&response);

        assert_eq!(content, "answer");
        assert_eq!(reasoning, "think");
        assert_eq!(calls.get(&0).map(|call| call.name.as_str()), Some("lookup"));
        assert_eq!(usage.prompt_tokens, 3);
        assert_eq!(usage.completion_tokens, 5);
        assert_eq!(usage.total_tokens, 8);
    }

    #[test]
    fn stream_tool_accumulator_replaces_complete_arguments_without_duplication() {
        let mut content = String::new();
        let mut reasoning = String::new();
        let mut usage = TokenUsage::default();
        let mut accumulator = ResponsesToolAccumulator::default();
        let callback = |response: Arc<ChatResponse>| drop(response);

        for event in [
            json!({
                "type": "response.output_item.added",
                "item": { "id": "fc_1", "type": "function_call", "call_id": "call_1", "name": "lookup" }
            }),
            json!({ "type": "response.function_call_arguments.delta", "item_id": "fc_1", "delta": "{\"query\"" }),
            json!({ "type": "response.function_call_arguments.delta", "item_id": "fc_1", "delta": ":\"x\"}" }),
            json!({ "type": "response.function_call_arguments.done", "item_id": "fc_1", "arguments": "{\"query\":\"x\"}" }),
            json!({
                "type": "response.output_item.done",
                "item": { "id": "fc_1", "type": "function_call", "call_id": "call_1", "name": "lookup", "arguments": "{\"query\":\"x\"}" }
            }),
        ] {
            assert!(matches!(
                process_stream_event(
                    &event,
                    &mut content,
                    &mut reasoning,
                    &mut usage,
                    &mut accumulator,
                    "chat_1",
                    &None,
                    "test",
                    &callback,
                )
                .expect("event should parse"),
                StreamEventOutcome::Continue
            ));
        }

        let calls = accumulator.into_calls();
        let call = calls.get(&0).expect("tool call should be accumulated");
        assert_eq!(call.id, "call_1");
        assert_eq!(call.name, "lookup");
        assert_eq!(call.arguments.as_deref(), Some("{\"query\":\"x\"}"));
    }

    #[test]
    fn stream_incomplete_event_emits_error_and_does_not_complete() {
        let mut content = String::new();
        let mut reasoning = String::new();
        let mut usage = TokenUsage::default();
        let mut accumulator = ResponsesToolAccumulator::default();
        accumulator.register_call(Some("fc_1"), Some("call_1"), Some("lookup"));
        accumulator.append_arguments_delta(Some("fc_1"), None, None, Some("{\"query\""));
        let emitted = Arc::new(StdMutex::new(Vec::<Arc<ChatResponse>>::new()));
        let emitted_for_callback = emitted.clone();
        let callback = move |response: Arc<ChatResponse>| {
            emitted_for_callback
                .lock()
                .expect("test mutex")
                .push(response);
        };
        let outcome = process_stream_event(
            &json!({
                "type": "response.incomplete",
                "response": { "incomplete_details": { "reason": "max_output_tokens" } }
            }),
            &mut content,
            &mut reasoning,
            &mut usage,
            &mut accumulator,
            "chat_1",
            &None,
            "test",
            &callback,
        )
        .expect("incomplete event should be handled");

        assert!(matches!(outcome, StreamEventOutcome::Failed(_)));
        let emitted = emitted.lock().expect("test mutex");
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].r#type, MessageType::Error);
        assert_eq!(emitted[0].finish_reason, Some(FinishReason::Error));
    }

    #[test]
    fn stream_failed_event_emits_error_once() {
        let mut content = String::new();
        let mut reasoning = String::new();
        let mut usage = TokenUsage::default();
        let mut accumulator = ResponsesToolAccumulator::default();
        let emitted = Arc::new(StdMutex::new(Vec::<Arc<ChatResponse>>::new()));
        let emitted_for_callback = emitted.clone();
        let callback = move |response: Arc<ChatResponse>| {
            emitted_for_callback
                .lock()
                .expect("test mutex")
                .push(response);
        };
        let outcome = process_stream_event(
            &json!({
                "type": "response.failed",
                "response": { "error": { "message": "boom" } }
            }),
            &mut content,
            &mut reasoning,
            &mut usage,
            &mut accumulator,
            "chat_1",
            &None,
            "test",
            &callback,
        )
        .expect("failed event should be handled");

        assert!(matches!(outcome, StreamEventOutcome::Failed(_)));
        let emitted = emitted.lock().expect("test mutex");
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].r#type, MessageType::Error);
        assert_eq!(emitted[0].finish_reason, Some(FinishReason::Error));
    }
}
