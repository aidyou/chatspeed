use crate::ccproxy::adapter::unified::{SseStatus, StreamLogRecorder, UnifiedFunctionCallPart};
use crate::ccproxy::helper::get_tool_id;
use crate::ccproxy::{
    errors::{CCProxyError, ProxyResult},
    helper::{get_provider_chat_full_url, should_forward_header, ModelResolver},
    types::ProxyModel,
    ChatProtocol,
};
use crate::db::MainStore;

use axum::body::Body;
use axum::response::Response;
use futures_util::stream::StreamExt;
use http::HeaderMap;
use reqwest::header::{HeaderName as ReqwestHeaderName, HeaderValue as ReqwestHeaderValue};
use rust_i18n::t;
use serde_json::{json, Value};
use std::str::FromStr;
use std::sync::{Arc, Mutex, RwLock};

pub async fn handle_direct_forward(
    client_headers: HeaderMap,
    client_request_body: bytes::Bytes,
    proxy_model: ProxyModel,
    is_streaming_request: bool,
    main_store_arc: Arc<std::sync::RwLock<MainStore>>,
    log_to_file: bool,
) -> ProxyResult<Response> {
    let http_client =
        ModelResolver::build_http_client(main_store_arc.clone(), proxy_model.metadata.clone())?;

    let full_url = get_provider_chat_full_url(
        proxy_model.chat_protocol.clone(),
        &proxy_model.base_url,
        &proxy_model.model,
        &proxy_model.api_key,
        is_streaming_request,
    );

    let mut reqwest_headers = reqwest::header::HeaderMap::new();

    // Forward relevant headers from client
    for (name, value) in client_headers.iter() {
        let name_str = name.as_str().to_lowercase();
        if should_forward_header(&name_str) {
            match ReqwestHeaderValue::from_bytes(value.as_bytes()) {
                Ok(h) => {
                    if let Ok(n) = ReqwestHeaderName::from_str(name.clone().as_str()) {
                        reqwest_headers.insert(n, h);
                    }
                }
                Err(_) => {}
            }
        }
    }

    // Add protocol-specific headers
    match proxy_model.chat_protocol {
        ChatProtocol::OpenAI | ChatProtocol::HuggingFace => {
            if !proxy_model.api_key.is_empty() {
                match ReqwestHeaderValue::from_str(&format!("Bearer {}", proxy_model.api_key)) {
                    Ok(h) => {
                        reqwest_headers.insert(reqwest::header::AUTHORIZATION, h);
                    }
                    Err(_) => {}
                }
            }
        }
        ChatProtocol::Claude => {
            match ReqwestHeaderValue::from_str(&proxy_model.api_key) {
                Ok(h) => {
                    reqwest_headers.insert(ReqwestHeaderName::from_static("x-api-key"), h);
                }
                Err(_) => {}
            }

            reqwest_headers.insert(
                ReqwestHeaderName::from_static("anthropic-version"),
                ReqwestHeaderValue::from_static("2023-06-01"),
            );
            if let Some(beta_header) = client_headers.get("anthropic-beta") {
                match ReqwestHeaderValue::from_bytes(beta_header.as_bytes()) {
                    Ok(h) => {
                        reqwest_headers.insert(ReqwestHeaderName::from_static("anthropic-beta"), h);
                    }
                    Err(_) => {}
                }
            }
        }
        ChatProtocol::Gemini => {
            // API key is in the URL for Gemini
        }
        ChatProtocol::Ollama => {
            // No auth for Ollama
        }
    }

    // Set Content-Type and Accept headers
    reqwest_headers.insert(
        reqwest::header::CONTENT_TYPE,
        ReqwestHeaderValue::from_static("application/json"),
    );
    if is_streaming_request {
        reqwest_headers.insert(
            reqwest::header::ACCEPT,
            ReqwestHeaderValue::from_static("text/event-stream"),
        );
    } else {
        reqwest_headers.insert(
            reqwest::header::ACCEPT,
            ReqwestHeaderValue::from_static("application/json"),
        );
    }

    // Modify the request body to replace the model alias
    let mut body_json: Value = serde_json::from_slice(&client_request_body).map_err(|e| {
        CCProxyError::InternalError(format!("Failed to deserialize request body: {}", e))
    })?;
    body_json = enhance_direct_request_body(body_json, &proxy_model, &proxy_model.chat_protocol);

    if let Some(model_field) = body_json.get_mut("model") {
        *model_field = Value::String(proxy_model.model.clone());
    }

    let modified_body = serde_json::to_vec(&body_json).map_err(|e| {
        CCProxyError::InternalError(format!("Failed to serialize modified body: {}", e))
    })?;

    let onward_request_builder = http_client
        .post(&full_url)
        .headers(reqwest_headers)
        .body(modified_body);

    // Send request
    let target_response = onward_request_builder
        .send()
        .await
        .map_err(|e| CCProxyError::InternalError(format!("Request to backend failed: {}", e)))?;

    // Handle response
    let status_code = target_response.status();
    let response_headers = target_response.headers().clone();

    if !status_code.is_success() {
        let error_body_bytes = target_response.bytes().await.map_err(|e| {
            CCProxyError::InternalError(
                t!("network.response_read_error", error = e.to_string()).to_string(),
            )
        })?;
        let mut response = Response::builder()
            .status(status_code)
            .body(Body::from(error_body_bytes))
            .map_err(|e| {
                CCProxyError::InternalError(format!("Failed to build error response: {}", e))
            })?;

        for (name, value) in response_headers.iter() {
            if let Ok(axum_name) = http::header::HeaderName::from_bytes(name.as_ref()) {
                if let Ok(axum_value) = http::header::HeaderValue::from_bytes(value.as_ref()) {
                    response.headers_mut().insert(axum_name, axum_value);
                }
            }
        }
        return Ok(response);
    }

    let mut response_builder = Response::builder().status(status_code);

    for (name, value) in response_headers.iter() {
        response_builder = response_builder.header(name.as_str(), value.as_bytes());
    }

    if is_streaming_request {
        let log_recorder = Arc::new(Mutex::new(StreamLogRecorder::new(
            format!("cid_{}", uuid::Uuid::new_v4().simple()),
            proxy_model.model.clone(),
        )));
        let chat_protocol = proxy_model.chat_protocol.clone();
        let log_recorder_clone = log_recorder.clone();

        let stream = target_response.bytes_stream().map(move |chunk| {
            let sse_status = Arc::new(RwLock::new(SseStatus::default()));
            let log_recorder = log_recorder_clone.clone();
            let chat_protocol = chat_protocol.clone();
            chunk.map(move |bytes| {
                if log_to_file {
                    chunk_parser_and_log(
                        &bytes,
                        log_recorder.clone(),
                        &chat_protocol,
                        sse_status.clone(),
                    );
                }

                bytes
            })
        });

        let body = Body::from_stream(stream);
        if let Ok(rsp) = response_builder.body(body) {
            Ok(rsp)
        } else {
            Err(CCProxyError::InternalError(
                "Failed to build response".to_string(),
            ))
        }
    } else {
        let body_bytes = target_response
            .bytes()
            .await
            .map_err(|e| CCProxyError::InternalError(e.to_string()))?;
        if log_to_file {
            log::info!(target: "ccproxy_logger", "[Direct] {} Response Body: {}\n================\n\n",proxy_model.chat_protocol.to_string(), String::from_utf8_lossy(&body_bytes));
        }

        if let Ok(response) = response_builder.body(Body::from(body_bytes)) {
            Ok(response)
        } else {
            Err(CCProxyError::InternalError(
                "Failed to build response".to_string(),
            ))
        }
    }
}

/// Enhances the request body sent directly to the AI provider.
///
/// Applies proxy-level transformations before forwarding:
///   - **temperature scaling**: If `proxy_model.temperature` is not `1.0`,
///     multiplies any existing `"temperature"` value in the body by the proxy setting.
///   - **tool filtering**: Removes any function/tool whose name appears in
///     `proxy_model.tool_filter`.
///   - **prompt injection**: When tools are present (`"tools"` array is non-empty)
///     and `proxy_model.prompt_injection` is not `"off"`, inserts or overwrites
///     the system prompt with `proxy_model.prompt_text`.
///     - `"enhance"` appends it.
///     - `"replace"` wholly replaces the original system prompt.
///     Protocol-specific locations:
///       - **Claude**: `"system"` key.
///       - **Gemini**: `"system_instruction"` object with `"parts"`.
///       - **OpenAI / Ollama / HuggingFace**: `"messages"` array, inserting a new
///         `"system"` message at position 0 if none exists.
///
/// Args:
/// - `body`: Original request body as JSON.
/// - `proxy_model`: Proxy settings influencing transformations.
/// - `chat_protocol`: The target provider protocol clarifying field semantics.
///
/// Returns the updated JSON body ready for forwarding.
fn enhance_direct_request_body(
    mut body: Value,
    proxy_model: &ProxyModel,
    chat_protocol: &ChatProtocol,
) -> Value {
    if proxy_model.temperature != 1.0 {
        if let Some(temperature) = body.get("temperature").and_then(|t| t.as_f64()) {
            if let Some(body_map) = body.as_object_mut() {
                body_map.insert(
                    "temperature".to_string(),
                    json!((proxy_model.temperature as f64) * temperature),
                );
            }
        }
    }

    if !proxy_model.tool_filter.is_empty() {
        if let Some(tools) = body.get_mut("tools").and_then(|t| t.as_array_mut()) {
            tools.retain(|tool| {
                let tool_name = match chat_protocol {
                    ChatProtocol::Claude => tool.get("name").and_then(|n| n.as_str()),
                    ChatProtocol::Gemini => tool
                        .get("function_declarations")
                        .and_then(|f| f.as_array())
                        .and_then(|f| f.first())
                        .and_then(|f| f.get("name"))
                        .and_then(|n| n.as_str()),
                    _ => tool
                        .get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(|n| n.as_str()),
                };

                if let Some(name) = tool_name {
                    return !proxy_model.tool_filter.contains_key(name);
                }
                true
            });
        }
    }

    let has_tools = body
        .get("tools")
        .and_then(|t| t.as_array())
        .map_or(false, |t| !t.is_empty());

    if proxy_model.prompt_injection != "off" && !proxy_model.prompt_text.is_empty() && has_tools {
        if let Some(body_map) = body.as_object_mut() {
            match chat_protocol {
                ChatProtocol::Claude => {
                    let system_prompt_field = "system";
                    let original_prompt = body_map
                        .get(system_prompt_field)
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();

                    if proxy_model.prompt_injection == "enhance" {
                        let new_prompt = if original_prompt.is_empty() {
                            proxy_model.prompt_text.clone()
                        } else {
                            format!("{}\n\n{}", original_prompt, proxy_model.prompt_text)
                        };
                        body_map.insert(system_prompt_field.to_string(), json!(new_prompt));
                    } else if proxy_model.prompt_injection == "replace" {
                        body_map.insert(
                            system_prompt_field.to_string(),
                            json!(proxy_model.prompt_text.clone()),
                        );
                    }
                }
                ChatProtocol::Gemini => {
                    let system_prompt_field = "system_instruction";
                    let original_prompt = body_map
                        .get(system_prompt_field)
                        .and_then(|v| v.get("parts"))
                        .and_then(|p| p.as_array())
                        .and_then(|p| p.first())
                        .and_then(|p| p.get("text"))
                        .and_then(|t| t.as_str())
                        .unwrap_or_default()
                        .to_string();

                    let new_prompt = if proxy_model.prompt_injection == "enhance" {
                        format!("{}\n\n{}", original_prompt, proxy_model.prompt_text)
                    } else {
                        proxy_model.prompt_text.clone()
                    };

                    body_map.insert(
                        system_prompt_field.to_string(),
                        json!({ "parts": [{ "text": new_prompt }] }),
                    );
                }
                ChatProtocol::OpenAI | ChatProtocol::Ollama | ChatProtocol::HuggingFace => {
                    if let Some(messages) =
                        body_map.get_mut("messages").and_then(|m| m.as_array_mut())
                    {
                        let mut system_message_found = false;
                        if let Some(system_message) = messages
                            .iter_mut()
                            .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("system"))
                        {
                            if let Some(content) =
                                system_message.get("content").and_then(|c| c.as_str())
                            {
                                let new_content = if proxy_model.prompt_injection == "enhance" {
                                    format!("{}\n\n{}", content, proxy_model.prompt_text)
                                } else {
                                    // replace
                                    proxy_model.prompt_text.clone()
                                };
                                if let Some(msg_obj) = system_message.as_object_mut() {
                                    msg_obj.insert("content".to_string(), json!(new_content));
                                }
                                system_message_found = true;
                            }
                        }

                        if !system_message_found {
                            messages.insert(
                                0,
                                json!({
                                    "role": "system",
                                    "content": proxy_model.prompt_text.clone()
                                }),
                            );
                        }
                    }
                }
            }
        }
    }

    body
}

fn chunk_parser_and_log(
    bytes: &bytes::Bytes,
    log_recorder: Arc<Mutex<StreamLogRecorder>>,
    chat_protocol: &ChatProtocol,
    sse_status: Arc<RwLock<SseStatus>>,
) {
    let chunk_str = String::from_utf8_lossy(bytes);
    let mut is_finished = false;

    for line in chunk_str.lines() {
        if line.starts_with("data:") {
            let data = &line["data:".len()..].trim();
            if data.is_empty() || *data == "[DONE]" {
                continue;
            }

            if let Ok(json_data) = serde_json::from_str::<Value>(data) {
                let mut recorder = log_recorder.lock().unwrap();

                // Update chat_id if it's still the initial generated one
                if recorder.chat_id.starts_with("cid_") {
                    let api_chat_id = json_data
                        .get("id")
                        .and_then(|id| id.as_str())
                        .map(|x| x.trim());
                    if let Some(id) = api_chat_id {
                        if !id.is_empty() {
                            recorder.chat_id = id.to_string();
                        }
                    }
                }

                match chat_protocol {
                    ChatProtocol::OpenAI | ChatProtocol::HuggingFace => {
                        if let Some(detlta) = json_data
                            .get("choices")
                            .and_then(|c| c.get(0))
                            .and_then(|c| c.get("delta"))
                        {
                            if let Some(text) = detlta.get("content").and_then(|c| c.as_str()) {
                                if !text.is_empty() {
                                    recorder.content.push_str(text.trim());
                                }
                            }

                            if let Some(text) =
                                detlta.get("reasoning_content").and_then(|c| c.as_str())
                            {
                                if !text.is_empty() {
                                    match recorder.thinking {
                                        Some(ref mut reasoning) => reasoning.push_str(text),
                                        None => recorder.thinking = Some(text.to_string()),
                                    }
                                }
                            }
                        }

                        // tool call
                        if let Some(tool_calls) = json_data
                            .get("choices")
                            .and_then(|c| c.get(0))
                            .and_then(|c| c.get("delta"))
                            .and_then(|d| d.get("tool_calls"))
                            .and_then(|t| t.as_array())
                        {
                            if recorder.tool_calls.is_none() {
                                recorder.tool_calls = Some(Default::default());
                            }

                            if let Some(recorder_tool_calls) = &mut recorder.tool_calls {
                                for tc in tool_calls {
                                    let tool_id = tc
                                        .get("id")
                                        .and_then(|i| i.as_str())
                                        .map(|x| x.to_string())
                                        .unwrap_or(get_tool_id());

                                    if let Some(func) = tc.get("function") {
                                        if let Some(name) =
                                            func.get("name").and_then(|n| n.as_str())
                                        {
                                            recorder_tool_calls
                                                .entry(tool_id.to_string())
                                                .or_insert_with(|| UnifiedFunctionCallPart {
                                                    name: name.to_string(),
                                                    args: "".to_string(),
                                                });
                                        }

                                        if let Some(args) =
                                            func.get("arguments").and_then(|a| a.as_str())
                                        {
                                            if let Some(tool) =
                                                recorder_tool_calls.get_mut(&tool_id)
                                            {
                                                tool.args.push_str(args);
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        // usage
                        if let Some(usage) = json_data.get("usage") {
                            if let Some(prompt_tokens) =
                                usage.get("prompt_tokens").and_then(|t| t.as_u64())
                            {
                                recorder.input_tokens = Some(prompt_tokens);
                            }
                            if let Some(completion_tokens) =
                                usage.get("completion_tokens").and_then(|t| t.as_u64())
                            {
                                recorder.output_tokens = Some(completion_tokens);
                            }
                        }

                        if let Some(reason) = json_data
                            .get("choices")
                            .and_then(|c| c.get(0))
                            .and_then(|c| c.get("finish_reason"))
                            .and_then(|r| r.as_str())
                        {
                            if !reason.is_empty() {
                                is_finished = true;
                            }
                        }
                    }
                    ChatProtocol::Claude => {
                        if let Ok(claude_event) =
                            serde_json::from_str::<crate::ccproxy::claude::ClaudeStreamEvent>(data)
                        {
                            match claude_event.event_type.as_str() {
                                "message_start" => {
                                    if let Some(msg) = claude_event.message {
                                        if recorder.model.is_empty() {
                                            recorder.model = msg.model;
                                        }
                                        recorder.input_tokens =
                                            Some(msg.usage.input_tokens.unwrap_or(0));
                                    }
                                }
                                "content_block_start" => {
                                    if let Some(block) = claude_event.content_block {
                                        match block.block_type.as_str() {
                                            "tool_use" => {
                                                let tool_name = block.name.unwrap_or_default();
                                                let tool_id =
                                                    block.id.clone().unwrap_or(get_tool_id());
                                                if let Ok(mut status) = sse_status.write() {
                                                    status.tool_id = tool_id.clone();
                                                }

                                                if !tool_name.is_empty() {
                                                    if recorder.tool_calls.is_none() {
                                                        recorder.tool_calls =
                                                            Some(Default::default());
                                                    }
                                                    if let Some(tool_calls) =
                                                        &mut recorder.tool_calls
                                                    {
                                                        tool_calls
                                                            .entry(tool_id.clone())
                                                            .or_insert_with(|| {
                                                                UnifiedFunctionCallPart {
                                                                    name: tool_name,
                                                                    args: "".to_string(),
                                                                }
                                                            });
                                                    }
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                                "content_block_delta" => {
                                    if let Some(delta) = claude_event.delta {
                                        match delta.delta_type.as_deref() {
                                            Some("text_delta") => {
                                                if let Some(text) = delta.text {
                                                    if !text.is_empty() {
                                                        recorder.content.push_str(&text);
                                                    }
                                                }
                                            }
                                            Some("thinking_delta") => {
                                                if let Some(text) = delta.text {
                                                    if !text.is_empty() {
                                                        if let Some(thinking) =
                                                            &mut recorder.thinking
                                                        {
                                                            thinking.push_str(&text);
                                                        } else {
                                                            recorder.thinking =
                                                                Some(text.to_string());
                                                        }
                                                    }
                                                }
                                            }
                                            Some("input_json_delta") => {
                                                if let Some(partial_json) = delta.partial_json {
                                                    if !partial_json.is_empty() {
                                                        if let Ok(status) = sse_status.read() {
                                                            let tool_id = status.tool_id.clone();

                                                            if let Some(tool_calls) =
                                                                &mut recorder.tool_calls
                                                            {
                                                                if let Some(tool) =
                                                                    tool_calls.get_mut(&tool_id)
                                                                {
                                                                    tool.args
                                                                        .push_str(&partial_json);
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                                "message_delta" => {
                                    if let Some(usage) = claude_event.usage {
                                        recorder.output_tokens =
                                            Some(usage.output_tokens.unwrap_or(0));
                                    }
                                    if let Some(delta) = claude_event.delta {
                                        if delta.stop_reason.is_some() {
                                            is_finished = true;
                                        }
                                    }
                                }
                                "message_stop" => {
                                    is_finished = true;
                                }
                                _ => {}
                            }
                        }
                    }
                    ChatProtocol::Gemini => {
                        if let Some(text) = json_data
                            .get("candidates")
                            .and_then(|c| c.get(0))
                            .and_then(|c| c.get("content"))
                            .and_then(|c| c.get("parts"))
                            .and_then(|p| p.get(0))
                            .and_then(|p| p.get("text"))
                            .and_then(|t| t.as_str())
                        {
                            if !text.is_empty() {
                                recorder.content.push_str(text);
                            }
                        }
                        if let Some(tool_calls) = json_data
                            .get("candidates")
                            .and_then(|c| c.get(0))
                            .and_then(|c| c.get("content"))
                            .and_then(|c| c.get("parts"))
                            .and_then(|p| p.as_array())
                        {
                            if recorder.tool_calls.is_none() {
                                recorder.tool_calls = Some(Default::default());
                            }
                            if let Some(recorder_tool_calls) = &mut recorder.tool_calls {
                                for tc in tool_calls {
                                    if let Some(name) = tc
                                        .get("functionCall")
                                        .and_then(|f| f.get("name"))
                                        .and_then(|n| n.as_str())
                                    {
                                        let id = get_tool_id();
                                        recorder_tool_calls.entry(id.to_string()).or_insert_with(
                                            || UnifiedFunctionCallPart {
                                                name: name.to_string(),
                                                args: "".to_string(),
                                            },
                                        );
                                        if let Some(args) =
                                            tc.get("functionCall").and_then(|f| f.get("args"))
                                        {
                                            if let Some(tool) = recorder_tool_calls.get_mut(&id) {
                                                tool.args.push_str(&args.to_string());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if let Some(usage_metadata) = json_data.get("usage_metadata") {
                            if let Some(prompt_token_count) = usage_metadata
                                .get("prompt_token_count")
                                .and_then(|t| t.as_u64())
                            {
                                recorder.input_tokens = Some(prompt_token_count);
                            }
                            if let Some(candidates_token_count) = usage_metadata
                                .get("candidates_token_count")
                                .and_then(|t| t.as_u64())
                            {
                                recorder.output_tokens = Some(candidates_token_count);
                            }
                        }
                        if let Some(reason) = json_data
                            .get("candidates")
                            .and_then(|c| c.get(0))
                            .and_then(|c| c.get("finishReason"))
                            .and_then(|r| r.as_str())
                        {
                            if !reason.is_empty() {
                                is_finished = true;
                            }
                        }
                    }
                    ChatProtocol::Ollama => {
                        if let Some(message) = json_data.get("message") {
                            if let Some(text) = message.get("content").and_then(|c| c.as_str()) {
                                if !text.is_empty() {
                                    recorder.content.push_str(text);
                                }
                            }

                            if let Some(thinking) = message.get("thinking").and_then(|t| t.as_str())
                            {
                                if !thinking.is_empty() {
                                    if let Some(reasoning) = &mut recorder.thinking {
                                        reasoning.push_str(thinking);
                                    } else {
                                        recorder.thinking = Some(thinking.to_string());
                                    }
                                }
                            }

                            if let Some(tool_calls) =
                                message.get("tool_calls").and_then(|t| t.as_array())
                            {
                                if recorder.tool_calls.is_none() {
                                    recorder.tool_calls = Some(Default::default());
                                }
                                if let Some(recorder_tool_calls) = &mut recorder.tool_calls {
                                    for tc in tool_calls {
                                        if let Some(name) = tc
                                            .get("function")
                                            .and_then(|f| f.get("name"))
                                            .and_then(|n| n.as_str())
                                        {
                                            let id = get_tool_id();
                                            recorder_tool_calls
                                                .entry(id.to_string())
                                                .or_insert_with(|| UnifiedFunctionCallPart {
                                                    name: name.to_string(),
                                                    args: "".to_string(),
                                                });
                                            if let Some(args) =
                                                tc.get("function").and_then(|f| f.get("arguments"))
                                            {
                                                if let Some(tool) = recorder_tool_calls.get_mut(&id)
                                                {
                                                    tool.args.push_str(&args.to_string());
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if let Some(prompt_eval_count) =
                            json_data.get("prompt_eval_count").and_then(|t| t.as_u64())
                        {
                            recorder.input_tokens = Some(prompt_eval_count);
                        }
                        if let Some(eval_count) =
                            json_data.get("eval_count").and_then(|t| t.as_u64())
                        {
                            recorder.output_tokens = Some(eval_count);
                        }
                        if let Some(done) = json_data.get("done").and_then(|d| d.as_bool()) {
                            if done {
                                is_finished = true;
                            }
                        }
                    }
                }
            }
        }
    }

    if is_finished {
        if let Ok(final_log) = log_recorder.clone().lock() {
            log::info!(target: "ccproxy_logger",
                "[Direct]{} Stream Response: \n{}\n================\n\n",
                chat_protocol.to_string(),
                serde_json::to_string_pretty(&*final_log).unwrap_or_default()
            );
        }
    }
}
