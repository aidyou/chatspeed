use crate::ccproxy::adapter::unified::{SseStatus, StreamLogRecorder, UnifiedFunctionCallPart};
use crate::ccproxy::helper::get_tool_id;
use crate::ccproxy::utils::token_estimator::estimate_tokens;
use crate::ccproxy::{
    errors::{CCProxyError, ProxyResult},
    helper::{get_provider_chat_full_url, ModelResolver},
    types::ProxyModel,
    ChatProtocol,
};
use crate::db::{CcproxyStat, MainStore};

use axum::body::Body;
use axum::response::Response;
use futures_util::stream::StreamExt;
use http::HeaderMap;
use reqwest::header::{HeaderName as ReqwestHeaderName, HeaderValue as ReqwestHeaderValue};
use rust_i18n::t;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, RwLock};

pub async fn handle_direct_forward(
    client_headers: HeaderMap,
    client_request_body: bytes::Bytes,
    proxy_model: ProxyModel,
    is_streaming_request: bool,
    main_store_arc: Arc<std::sync::RwLock<MainStore>>,
    log_to_file: bool,
) -> ProxyResult<Response> {
    let message_id = crate::ccproxy::helper::get_msg_id();
    let provider_name = proxy_model.provider.clone();
    let model_name = proxy_model.model.clone();
    let chat_protocol_for_stat = proxy_model.chat_protocol.clone();

    let http_client = ModelResolver::build_http_client(
        main_store_arc.clone(),
        proxy_model.model_metadata.clone(),
    )?;

    let full_url = get_provider_chat_full_url(
        proxy_model.chat_protocol.clone(),
        &proxy_model.base_url,
        &proxy_model.model,
        &proxy_model.api_key,
        is_streaming_request,
    );

    let mut reqwest_headers = reqwest::header::HeaderMap::new();

    // Inject all proxy headers (Metadata + Protocol Defaults + Client Overrides)
    ModelResolver::inject_proxy_headers(
        &mut reqwest_headers,
        &client_headers,
        &proxy_model,
        &message_id,
    );

    // Set mandatory content headers
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

    // Capture beta headers if present (Protocol specific override)
    if proxy_model.chat_protocol == ChatProtocol::Claude {
        if let Some(beta_header) = client_headers.get("anthropic-beta") {
            if let Ok(h) = ReqwestHeaderValue::from_bytes(beta_header.as_bytes()) {
                reqwest_headers.insert(ReqwestHeaderName::from_static("anthropic-beta"), h);
            }
        }
    }

    // Modify the request body to replace the model alias
    let mut body_json: Value = serde_json::from_slice(&client_request_body).map_err(|e| {
        CCProxyError::InternalError(format!("Failed to deserialize request body: {}", e))
    })?;

    // Estimate input tokens from text content in the request body
    let mut text_for_estimation = String::new();
    if let Some(messages) = body_json.get("messages").and_then(|m| m.as_array()) {
        for msg in messages {
            if let Some(content) = msg.get("content") {
                if let Some(text) = content.as_str() {
                    text_for_estimation.push_str(text);
                } else if let Some(parts) = content.as_array() {
                    for part in parts {
                        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                            text_for_estimation.push_str(text);
                        }
                    }
                }
            }
        }
    }
    if let Some(system) = body_json.get("system").and_then(|s| s.as_str()) {
        text_for_estimation.push_str(system);
    }

    let estimated_input_tokens =
        crate::ccproxy::utils::token_estimator::estimate_tokens(&text_for_estimation);

    // Merge proxy parameters into the body (Client > Model Config)
    ModelResolver::merge_parameters_json(&mut body_json, &proxy_model);

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
            .body(Body::from(error_body_bytes.clone()))
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

        // Record error for non-streaming direct forward
        if let Ok(store) = main_store_arc.read() {
            let error_msg = String::from_utf8_lossy(&error_body_bytes).to_string();
            let _ = store.record_ccproxy_stat(CcproxyStat {
                id: None,
                client_model: proxy_model.client_alias.clone(),
                backend_model: model_name.clone(),
                provider: provider_name.clone(),
                protocol: chat_protocol_for_stat.to_string(),
                tool_compat_mode: 0,
                status_code: status_code.as_u16() as i32,
                error_message: Some(error_msg),
                input_tokens: 0,
                output_tokens: 0,
                cache_tokens: 0,
                request_at: None,
            });
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
        let main_store_for_stream = main_store_arc.clone();
        let provider_for_stream = provider_name.clone();
        let client_model_for_stream = proxy_model.client_alias.clone();
        let backend_model_for_stream = model_name.clone();
        let sse_status = Arc::new(RwLock::new(SseStatus::new(
            "".to_string(),
            "".to_string(),
            false,
            estimated_input_tokens,
        )));

        let stream = target_response.bytes_stream().map(move |chunk| {
            let sse_status = sse_status.clone();
            let log_recorder = log_recorder_clone.clone();
            let chat_protocol = chat_protocol.clone();
            let main_store = main_store_for_stream.clone();
            let provider = provider_for_stream.clone();
            let client_model = client_model_for_stream.clone();
            let backend_model = backend_model_for_stream.clone();

            chunk.map(move |bytes| {
                // Always parse for statistics, but only log to file if enabled
                chunk_parser_and_log(
                    &bytes,
                    log_recorder.clone(),
                    &chat_protocol,
                    sse_status.clone(),
                    log_to_file,
                    main_store,
                    provider,
                    client_model,
                    backend_model,
                );

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

        // Record statistics for non-streaming direct forward
        if status_code.is_success() {
            if let Ok(store) = main_store_arc.read() {
                // For direct forward, we try a simple extraction of tokens from the JSON body
                let body_json: Value = serde_json::from_slice(&body_bytes).unwrap_or(Value::Null);
                let (input, output, cache) =
                    extract_usage_from_value(&body_json, &chat_protocol_for_stat);

                let _ = store.record_ccproxy_stat(CcproxyStat {
                    id: None,
                    client_model: proxy_model.client_alias.clone(),
                    backend_model: model_name.clone(),
                    provider: provider_name.clone(),
                    protocol: chat_protocol_for_stat.to_string(),
                    tool_compat_mode: 0, // Direct forward is never in compat mode
                    status_code: status_code.as_u16() as i32,
                    error_message: None,
                    input_tokens: input,
                    output_tokens: output,
                    cache_tokens: cache,
                    request_at: None,
                });
            }
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
    // 1. Tool filtering
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

    // 3. Prompt injection (System instruction behavior)
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

    // 4. Final step: Merge model-specific custom body params (Highest override for non-standard fields)
    crate::ai::util::merge_custom_params(&mut body, &proxy_model.custom_params);

    body
}

fn chunk_parser_and_log(
    bytes: &bytes::Bytes,
    log_recorder: Arc<Mutex<StreamLogRecorder>>,
    chat_protocol: &ChatProtocol,
    sse_status: Arc<RwLock<SseStatus>>,
    log_to_file: bool,
    main_store_arc: Arc<std::sync::RwLock<MainStore>>,
    provider: String,
    client_model: String,
    backend_model: String,
) {
    let chunk_str = String::from_utf8_lossy(bytes);
    let mut is_finished = false;

    for line in chunk_str.lines() {
        if line.starts_with("data:") {
            let data = &line["data:".len()..].trim();
            if data.is_empty() || *data == "[DONE]" {
                if *data == "[DONE]" {
                    is_finished = true;
                }
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
                                    if let Ok(mut status) = sse_status.write() {
                                        status.estimated_output_tokens += estimate_tokens(text);
                                    }
                                    if log_to_file {
                                        recorder.content.push_str(text.trim());
                                    }
                                }
                            }
                            // ... reasoning and tool_calls omitted for brevity but remain the same

                            if let Some(text) =
                                detlta.get("reasoning_content").and_then(|c| c.as_str())
                            {
                                if !text.is_empty() {
                                    if log_to_file {
                                        match recorder.thinking {
                                            Some(ref mut reasoning) => reasoning.push_str(text),
                                            None => recorder.thinking = Some(text.to_string()),
                                        }
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
                            if log_to_file {
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

                                                if log_to_file && !tool_name.is_empty() {
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
                                                        if let Ok(mut status) = sse_status.write() {
                                                            status.estimated_output_tokens +=
                                                                estimate_tokens(&text);
                                                        }
                                                        if log_to_file {
                                                            recorder.content.push_str(&text);
                                                        }
                                                    }
                                                }
                                            }
                                            Some("thinking_delta") => {
                                                if let Some(text) = delta.text {
                                                    if log_to_file && !text.is_empty() {
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
                                                    if log_to_file && !partial_json.is_empty() {
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
                                if let Ok(mut status) = sse_status.write() {
                                    status.estimated_output_tokens += estimate_tokens(text);
                                }
                                if log_to_file {
                                    recorder.content.push_str(text);
                                }
                            }
                        }
                        if let Some(tool_calls) = json_data
                            .get("candidates")
                            .and_then(|c| c.get(0))
                            .and_then(|c| c.get("content"))
                            .and_then(|c| c.get("parts"))
                            .and_then(|p| p.as_array())
                        {
                            if log_to_file {
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
                                            recorder_tool_calls
                                                .entry(id.to_string())
                                                .or_insert_with(|| UnifiedFunctionCallPart {
                                                    name: name.to_string(),
                                                    args: "".to_string(),
                                                });
                                            if let Some(args) =
                                                tc.get("functionCall").and_then(|f| f.get("args"))
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
                                    if let Ok(mut status) = sse_status.write() {
                                        status.estimated_output_tokens += estimate_tokens(text);
                                    }
                                    if log_to_file {
                                        recorder.content.push_str(text);
                                    }
                                }
                            }

                            if let Some(thinking) = message.get("thinking").and_then(|t| t.as_str())
                            {
                                if log_to_file && !thinking.is_empty() {
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
                                if log_to_file {
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
                                                if let Some(args) = tc
                                                    .get("function")
                                                    .and_then(|f| f.get("arguments"))
                                                {
                                                    if let Some(tool) =
                                                        recorder_tool_calls.get_mut(&id)
                                                    {
                                                        tool.args.push_str(&args.to_string());
                                                    }
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
        let (input, output) = {
            let recorder = log_recorder.lock().unwrap();
            if log_to_file {
                log::info!(target: "ccproxy_logger",
                    "[Direct]{} Stream Response: \n{}\n================\n\n",
                    chat_protocol.to_string(),
                    serde_json::to_string_pretty(&*recorder).unwrap_or_default()
                );
            }
            (
                recorder.input_tokens.unwrap_or(0),
                recorder.output_tokens.unwrap_or(0),
            )
        };

        // Record statistics to DB on finish
        if let Ok(store) = main_store_arc.read() {
            let (est_input, est_output) = if let Ok(status) = sse_status.read() {
                (
                    status.estimated_input_tokens,
                    status.estimated_output_tokens,
                )
            } else {
                (0.0, 0.0)
            };

            let final_input = if input > 0 {
                input
            } else {
                est_input.ceil() as u64
            };
            let final_output = if output > 0 {
                output
            } else {
                est_output.ceil() as u64
            };

            let _ = store.record_ccproxy_stat(CcproxyStat {
                id: None,
                client_model,
                backend_model,
                provider,
                protocol: chat_protocol.to_string(),
                tool_compat_mode: 0,
                status_code: 200,
                error_message: None,
                input_tokens: final_input as i64,
                output_tokens: final_output as i64,
                cache_tokens: 0, // Cache tokens are rarely provided in streaming direct forward
                request_at: None,
            });
        }
    }
}

fn extract_usage_from_value(value: &Value, protocol: &ChatProtocol) -> (i64, i64, i64) {
    match protocol {
        ChatProtocol::OpenAI | ChatProtocol::HuggingFace => {
            let usage = value.get("usage");
            let input = usage
                .and_then(|u| u.get("prompt_tokens"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let output = usage
                .and_then(|u| u.get("completion_tokens"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let cache = usage
                .and_then(|u| u.get("prompt_tokens_details"))
                .and_then(|d| d.get("cached_tokens"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            (input, output, cache)
        }
        ChatProtocol::Claude => {
            let usage = value.get("usage");
            let input = usage
                .and_then(|u| u.get("input_tokens"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let output = usage
                .and_then(|u| u.get("output_tokens"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let cache = usage
                .and_then(|u| u.get("cache_read_input_tokens"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            (input, output, cache)
        }
        ChatProtocol::Gemini => {
            let usage = value.get("usageMetadata");
            let input = usage
                .and_then(|u| u.get("promptTokenCount"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let output = usage
                .and_then(|u| u.get("candidatesTokenCount"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let cache = usage
                .and_then(|u| u.get("cachedContentTokenCount"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            (input, output, cache)
        }
        ChatProtocol::Ollama => {
            let input = value
                .get("prompt_eval_count")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let output = value
                .get("eval_count")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            (input, output, 0)
        }
    }
}
