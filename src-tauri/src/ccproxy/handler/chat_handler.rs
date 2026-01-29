use axum::response::{IntoResponse, Response};
use axum::Json;
use reqwest::header::HeaderMap;
use rust_i18n::t;
use serde_json::{json, Value};
use std::sync::{Arc, RwLock};

use crate::ccproxy::helper::get_msg_id;
use crate::ccproxy::ChatProtocol;
use crate::ccproxy::{
    adapter::{
        backend::{self, BackendAdapter},
        input::{from_claude, from_gemini, from_ollama, from_openai},
        output::{
            ClaudeOutputAdapter, GeminiOutputAdapter, OllamaOutputAdapter, OpenAIOutputAdapter,
            OutputAdapter, OutputAdapterEnum,
        },
        unified::{SseStatus, UnifiedRequest},
    },
    claude::ClaudeNativeRequest,
    errors::{CCProxyError, ProxyResult},
    gemini::GeminiRequest,
    helper::{
        get_provider_chat_full_url, stream_handler::handle_streamed_response, CcproxyQuery,
        ModelResolver,
    },
    openai::OpenAIChatCompletionRequest,
    types::ollama::OllamaChatCompletionRequest,
};
use crate::constants::{CFG_CCPROXY_LOG_PROXY_TO_FILE, CFG_CCPROXY_LOG_TO_FILE};
use crate::db::{CcproxyStat, MainStore};

fn get_proxy_alias_from_body(
    chat_protocol: &ChatProtocol,
    client_request_body: &bytes::Bytes,
    route_model_alias: &str,
) -> Result<String, CCProxyError> {
    match chat_protocol {
        ChatProtocol::OpenAI | ChatProtocol::HuggingFace => {
            let payload: OpenAIChatCompletionRequest = serde_json::from_slice(client_request_body)
                .map_err(|e| {
                    CCProxyError::InternalError(format!(
                        "Failed to deserialize OpenAI request: {}",
                        e
                    ))
                })?;
            Ok(payload.model)
        }
        ChatProtocol::Ollama => {
            let payload: OllamaChatCompletionRequest = serde_json::from_slice(client_request_body)
                .map_err(|e| {
                    CCProxyError::InternalError(format!(
                        "Failed to deserialize Ollama request: {}",
                        e
                    ))
                })?;
            Ok(payload.model)
        }
        ChatProtocol::Claude => {
            let payload: ClaudeNativeRequest = serde_json::from_slice(client_request_body)
                .map_err(|e| {
                    CCProxyError::InternalError(format!(
                        "Failed to deserialize Claude request: {}",
                        e
                    ))
                })?;
            Ok(payload.model)
        }
        ChatProtocol::Gemini => Ok(route_model_alias.to_string()),
    }
}

/// build unified request from http post
///
/// # Return
///     - UnifiedRequest: The unified request object.
///     - String: The route model alias.
///     - bool: The tool compatibility mode.
fn build_unified_request(
    chat_protocol: ChatProtocol,
    client_request_body: bytes::Bytes,
    tool_compat_mode: bool,
    route_model_alias: String,
    generate_action: String,
) -> Result<(UnifiedRequest, String, bool), CCProxyError> {
    match chat_protocol {
        ChatProtocol::OpenAI | ChatProtocol::HuggingFace => {
            // Manually deserialize the request body
            let client_request_payload: OpenAIChatCompletionRequest =
                match serde_json::from_slice(&client_request_body) {
                    Ok(payload) => payload,
                    Err(e) => {
                        log::error!(
                            "Failed to deserialize OpenAiRequest: {}, the request: {}",
                            e,
                            String::from_utf8_lossy(&client_request_body)
                        );
                        return Err(CCProxyError::InternalError(
                            t!("proxy.error.invalid_request_format", error = e.to_string())
                                .to_string(),
                        ));
                    }
                };

            let proxy_alias = client_request_payload.model.clone();

            // 1. Convert to UnifiedRequest
            let unified_request =
                from_openai(client_request_payload, tool_compat_mode).map_err(|e| {
                    CCProxyError::InternalError(
                        t!("proxy.error.invalid_request", error = e.to_string()).to_string(),
                    )
                })?;
            let is_streaming_request = unified_request.stream;
            Ok((unified_request, proxy_alias, is_streaming_request))
        }
        ChatProtocol::Ollama => {
            let client_request_payload: OllamaChatCompletionRequest =
                match serde_json::from_slice(&client_request_body) {
                    Ok(payload) => payload,
                    Err(e) => {
                        log::error!(
                            "Failed to deserialize OllamaRequest: {}, the request: {}",
                            e,
                            String::from_utf8_lossy(&client_request_body)
                        );
                        return Err(CCProxyError::InternalError(
                            t!("proxy.error.invalid_request_format", error = e.to_string())
                                .to_string(),
                        ));
                    }
                };

            let proxy_alias = client_request_payload.model.clone();

            let unified_request =
                from_ollama(client_request_payload, tool_compat_mode).map_err(|e| {
                    CCProxyError::InternalError(
                        t!("proxy.error.invalid_request", error = e.to_string()).to_string(),
                    )
                })?;
            let is_streaming_request = unified_request.stream;
            Ok((unified_request, proxy_alias, is_streaming_request))
        }
        ChatProtocol::Claude => {
            // Manually deserialize the request body
            let client_request_payload: ClaudeNativeRequest =
                match serde_json::from_slice(&client_request_body) {
                    Ok(payload) => payload,
                    Err(e) => {
                        log::error!(
                            "Failed to deserialize ClaudeRequest: {}, the request: {}",
                            e,
                            String::from_utf8_lossy(&client_request_body)
                        );
                        return Err(CCProxyError::InternalError(
                            t!("proxy.error.invalid_request_format", error = e.to_string())
                                .to_string(),
                        ));
                    }
                };

            let proxy_alias = client_request_payload.model.clone();

            // 1. Convert to UnifiedRequest
            let unified_request =
                from_claude(client_request_payload, tool_compat_mode).map_err(|e| {
                    CCProxyError::InternalError(
                        t!("proxy.error.invalid_request", error = e.to_string()).to_string(),
                    )
                })?;
            let is_streaming_request = unified_request.stream;
            Ok((unified_request, proxy_alias, is_streaming_request))
        }
        ChatProtocol::Gemini => {
            // Manually deserialize the request body
            let client_request_payload: GeminiRequest =
                match serde_json::from_slice(&client_request_body) {
                    Ok(payload) => payload,
                    Err(e) => {
                        log::error!(
                            "Failed to deserialize GeminiRequest: {}, the request: {}",
                            e,
                            String::from_utf8_lossy(&client_request_body)
                        );
                        return Err(CCProxyError::InternalError(
                            t!("proxy.error.invalid_request_format", error = e.to_string())
                                .to_string(),
                        ));
                    }
                };

            let proxy_alias = route_model_alias; // Use the model alias from the route

            // 1. Convert to UnifiedRequest
            let unified_request =
                from_gemini(client_request_payload, tool_compat_mode, generate_action).map_err(
                    |e| {
                        CCProxyError::InternalError(
                            t!("proxy.error.invalid_request", error = e.to_string()).to_string(),
                        )
                    },
                )?;

            // Determine if the original request was streaming
            let is_streaming_request = unified_request.stream;
            Ok((unified_request, proxy_alias, is_streaming_request))
        }
    }
}

pub async fn handle_chat_completion(
    chat_protocol: ChatProtocol,
    client_headers: HeaderMap,
    client_query: CcproxyQuery,
    client_request_body: bytes::Bytes,
    group_name: Option<String>,
    tool_compat_mode: bool,
    route_model_alias: String,
    generate_action: String,
    main_store_arc: Arc<std::sync::RwLock<MainStore>>,
) -> ProxyResult<Response> {
    let protocol_string = chat_protocol.to_string();
    let message_id = get_msg_id();

    let log_org_to_file = if let Ok(store) = main_store_arc.read() {
        store.get_config(CFG_CCPROXY_LOG_TO_FILE, false)
    } else {
        client_query.debug.unwrap_or(false)
    };
    let log_proxy_to_file = if let Ok(store) = main_store_arc.read() {
        store.get_config(CFG_CCPROXY_LOG_PROXY_TO_FILE, false)
    } else {
        false
    };

    if log_org_to_file {
        // Log the raw request body
        log::info!(target: "ccproxy_logger", "message id:{}\n{} Origin Request Body: \n{}\n----------------\n", &message_id, &protocol_string, String::from_utf8_lossy(&client_request_body));
    }

    let proxy_model = if let (Some(provider_id), Some(model_id)) = (
        client_headers
            .get("X-CS-Provider-Id")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<i64>().ok()),
        client_headers
            .get("X-CS-Model-Id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string()),
    ) {
        ModelResolver::get_ai_model_by_provider_and_model(
            main_store_arc.clone(),
            provider_id,
            model_id,
        )
        .await?
    } else {
        let proxy_alias =
            get_proxy_alias_from_body(&chat_protocol, &client_request_body, &route_model_alias)?;

        let group_name = group_name.as_deref();

        ModelResolver::get_ai_model_by_alias(
            main_store_arc.clone(),
            proxy_alias.clone(),
            group_name,
        )
        .await?
    };

    //======================================================
    // Direct send request to ai server
    //======================================================
    // Determine final tool_compat_mode based on metadata override
    let final_tool_compat_mode = match proxy_model.tool_compat_mode.as_deref() {
        Some("compat") => true,
        Some("native") => false,
        Some("auto") | None => tool_compat_mode, // Use route parameter for auto mode
        _ => tool_compat_mode,                   // Fallback to route parameter
    };

    if chat_protocol == proxy_model.chat_protocol && !final_tool_compat_mode {
        let is_streaming = match chat_protocol {
            ChatProtocol::OpenAI | ChatProtocol::HuggingFace => {
                let req: OpenAIChatCompletionRequest =
                    serde_json::from_slice(&client_request_body).unwrap_or_default();
                req.stream.unwrap_or(false)
            }
            ChatProtocol::Claude => {
                let req: Result<ClaudeNativeRequest, _> =
                    serde_json::from_slice(&client_request_body);
                req.map(|r| r.stream.unwrap_or(false)).unwrap_or(false)
            }
            ChatProtocol::Ollama => {
                let req: OllamaChatCompletionRequest =
                    serde_json::from_slice(&client_request_body).unwrap_or_default();
                req.stream.unwrap_or(false)
            }
            ChatProtocol::Gemini => generate_action == "streamGenerateContent",
        };

        let result = super::handle_direct_forward(
            client_headers,
            client_request_body,
            proxy_model,
            is_streaming,
            main_store_arc,
            log_org_to_file,
        )
        .await?;
        return Ok(result.into_response());
    }

    let (mut unified_request, proxy_alias, is_streaming_request) = build_unified_request(
        chat_protocol.clone(),
        client_request_body.clone(),
        final_tool_compat_mode,
        route_model_alias,
        generate_action,
    )?;

    unified_request.custom_params = proxy_model.custom_params.clone();

    // --- Inject Engine Defaults only if missing from client AND configured with valid non-default values ---
    ModelResolver::merge_parameters_unified(&mut unified_request, &proxy_model);

    // Tool filtering logic remains same
    if proxy_model.tool_filter.len() > 0 {
        unified_request.tools = unified_request.tools.map(|tools| {
            tools
                .into_iter()
                .filter(|tool| !proxy_model.tool_filter.contains_key(&tool.name))
                .collect()
        });
    }

    let has_tools = unified_request
        .tools
        .as_ref()
        .map_or(false, |t| t.len() > 0);

    if proxy_model.prompt_injection != "off" && !proxy_model.prompt_text.is_empty() && has_tools {
        unified_request.prompt_injection = Some(proxy_model.prompt_injection.clone());
        unified_request.prompt_enhance_text = Some(proxy_model.prompt_text.clone());
        unified_request.prompt_injection_position = proxy_model.prompt_injection_position.clone();
    }

    // --- Apply Prompt Replacement (Metadata KV) ---
    if !proxy_model.prompt_replace.is_empty() {
        if let Some(system_prompt) = &mut unified_request.system_prompt {
            for (key, value) in &proxy_model.prompt_replace {
                if !key.is_empty() {
                    *system_prompt = system_prompt.replace(key, value);
                }
            }
        }
    }

    // 2. Use the already resolved proxy_model from above

    let full_url = get_provider_chat_full_url(
        proxy_model.chat_protocol.clone(),
        &proxy_model.base_url,
        &proxy_model.model,
        &proxy_model.api_key,
        is_streaming_request,
    );

    let backend_adapter: Arc<dyn BackendAdapter> = match proxy_model.chat_protocol {
        ChatProtocol::OpenAI | ChatProtocol::HuggingFace => {
            Arc::new(crate::ccproxy::adapter::backend::OpenAIBackendAdapter)
        }
        ChatProtocol::Ollama => Arc::new(backend::OllamaBackendAdapter),
        ChatProtocol::Claude => Arc::new(backend::ClaudeBackendAdapter),
        ChatProtocol::Gemini => Arc::new(backend::GeminiBackendAdapter),
    };

    // Build HTTP client with proxy settings
    let http_client = ModelResolver::build_http_client(
        main_store_arc.clone(),
        proxy_model.model_metadata.clone(),
    )?;

    // Build consolidated headers using HeaderMap to handle overrides correctly
    let mut final_headers = reqwest::header::HeaderMap::new();

    // Inject all proxy headers (Metadata + Protocol Defaults + Client Overrides)
    ModelResolver::inject_proxy_headers(
        &mut final_headers,
        &client_headers,
        &proxy_model,
        &message_id,
    );

    // Set mandatory content headers (these shouldn't be overridden)
    final_headers.insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/json"),
    );

    // Set streaming request headers
    if is_streaming_request {
        final_headers.insert(
            reqwest::header::ACCEPT,
            reqwest::header::HeaderValue::from_static("text/event-stream"),
        );
    } else {
        final_headers.insert(
            reqwest::header::ACCEPT,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
    }

    // 3. Adapt request (Adapter will handle body transformation and specific protocol adjustments)
    let mut onward_request_builder = backend_adapter
        .adapt_request(
            &http_client,
            &mut unified_request,
            &proxy_model.api_key,
            &full_url,
            &proxy_model.model,
            log_proxy_to_file,
            &mut final_headers,
        )
        .await
        .map_err(|e| CCProxyError::InternalError(e.to_string()))?;

    // Apply consolidated headers to the request builder
    onward_request_builder = onward_request_builder.headers(final_headers);

    let target_response = onward_request_builder
        .send()
        .await
        .map_err(|e| CCProxyError::InternalError(format!("Request to backend failed: {}", e)))?;

    // Handle error response
    if !target_response.status().is_success() {
        let status_code = target_response.status();
        let headers_from_target = target_response.headers().clone();
        let error_body_bytes = match target_response.bytes().await {
            Ok(bytes) => bytes,
            Err(e) => {
                log::error!("Failed to read backend error response: {}", e);
                let err_msg = t!("network.response_read_error", error = e.to_string()).to_string();
                return Err(CCProxyError::InternalError(err_msg));
            }
        };
        let error_body_str = String::from_utf8_lossy(&error_body_bytes);

        if log_org_to_file {
            log::info!(target: "ccproxy_logger", "[ERROR] {} Response Error, model: {}, Status: {}, Body: \n{}\n---", proxy_model.chat_protocol.to_string(), &proxy_model.model, status_code, error_body_str);
        }

        log::warn!(
            "Backend API error (alias: '{}', model: '{}', provider: '{}'): url={}, status_code={}, response={}",
            proxy_alias,
            proxy_model.model,
            proxy_model.provider,
            &full_url,
            status_code,
            error_body_str
        );

        let final_error_json: Value;
        let mut error_type_str = "upstream_api_error".to_string();

        let message_content = if let Ok(json_error) = serde_json::from_str::<Value>(&error_body_str)
        {
            if let Some(err_obj) = json_error.get("error") {
                error_type_str = err_obj
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or(&error_type_str)
                    .to_string();
                err_obj
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or(&error_body_str)
                    .to_string()
            } else {
                error_body_str.to_string()
            }
        } else {
            error_body_str.to_string()
        };
        final_error_json = serde_json::json!({
            "message": message_content,
            "type": error_type_str,
            "param": null,
            "code": status_code.as_u16().to_string()
        });

        // Record error statistics
        if let Ok(store) = main_store_arc.read() {
            let _ = store.record_ccproxy_stat(CcproxyStat {
                id: None,
                client_model: proxy_model.client_alias.clone(),
                backend_model: proxy_model.model.clone(),
                provider: proxy_model.provider.clone(),
                protocol: chat_protocol.to_string(),
                tool_compat_mode: if final_tool_compat_mode { 1 } else { 0 },
                status_code: status_code.as_u16() as i32,
                error_message: Some(message_content),
                input_tokens: 0,
                output_tokens: 0,
                cache_tokens: 0,
                request_at: None,
            });
        }

        let mut response =
            (status_code, Json(json!({ "error": final_error_json }))).into_response();

        let final_headers = response.headers_mut();
        for (reqwest_name, reqwest_value) in headers_from_target.iter() {
            let name_str = reqwest_name.as_str().to_lowercase();
            if name_str.starts_with("x-ratelimit-")
                || name_str == "retry-after"
                || name_str == "content-type"
            {
                if let Ok(axum_name) =
                    reqwest::header::HeaderName::from_bytes(reqwest_name.as_ref())
                {
                    if let Ok(axum_value) =
                        reqwest::header::HeaderValue::from_bytes(reqwest_value.as_ref())
                    {
                        final_headers.insert(axum_name, axum_value);
                    }
                }
            }
        }
        if !final_headers.contains_key(reqwest::header::CONTENT_TYPE) {
            final_headers.insert(
                reqwest::header::CONTENT_TYPE,
                reqwest::header::HeaderValue::from_static("application/json"),
            );
        }
        return Ok(response);
    }

    // 4. Adapt response back to client format
    let output_adapter: OutputAdapterEnum = match chat_protocol {
        ChatProtocol::OpenAI | ChatProtocol::HuggingFace => {
            OutputAdapterEnum::OpenAI(OpenAIOutputAdapter)
        }
        ChatProtocol::Claude => OutputAdapterEnum::Claude(ClaudeOutputAdapter),
        ChatProtocol::Gemini => OutputAdapterEnum::Gemini(GeminiOutputAdapter),
        ChatProtocol::Ollama => OutputAdapterEnum::Ollama(OllamaOutputAdapter),
    };

    // Estimate input tokens
    let mut full_prompt_text = String::new();

    // Add system prompt
    if let Some(system_prompt) = &unified_request.system_prompt {
        full_prompt_text.push_str(system_prompt);
        full_prompt_text.push_str("\n");
    }

    // Add message content
    for message in &unified_request.messages {
        for content_block in &message.content {
            if let crate::ccproxy::adapter::unified::UnifiedContentBlock::Text { text } =
                content_block
            {
                full_prompt_text.push_str(text);
                full_prompt_text.push_str("\n");
            }
        }
    }

    // Add tool definitions if in native tool mode
    if !unified_request.tool_compat_mode {
        if let Some(tools) = &unified_request.tools {
            for tool in tools {
                full_prompt_text.push_str(&tool.name);
                full_prompt_text.push_str("\n");
                if let Some(description) = &tool.description {
                    full_prompt_text.push_str(description);
                    full_prompt_text.push_str("\n");
                }
                full_prompt_text.push_str(&tool.input_schema.to_string());
                full_prompt_text.push_str("\n");
            }
        }
    }
    // Add combined_prompt (which includes tool definitions in compat mode, or other enhancements)
    else if let Some(combined_prompt) = &unified_request.combined_prompt {
        full_prompt_text.push_str(combined_prompt);
        full_prompt_text.push_str("\n");
    }

    let estimated_input_tokens =
        crate::ccproxy::utils::token_estimator::estimate_tokens(&full_prompt_text);

    let sse_status = Arc::new(RwLock::new(SseStatus::new(
        message_id,
        proxy_alias.clone(),
        tool_compat_mode,
        estimated_input_tokens,
    )));

    if is_streaming_request {
        let res = handle_streamed_response(
            Arc::new(proxy_model.chat_protocol),
            chat_protocol,
            target_response,
            backend_adapter,
            output_adapter,
            sse_status,
            log_org_to_file,
            main_store_arc.clone(),
            proxy_alias.clone(),
            proxy_model.model.clone(),
            proxy_model.provider.clone(),
            final_tool_compat_mode,
        )
        .await?;
        Ok(res.into_response())
    } else {
        let body_bytes = target_response
            .bytes()
            .await
            .map_err(|e| CCProxyError::InternalError(e.to_string()))?;

        if log_org_to_file {
            log::info!(target: "ccproxy_logger", "[Proxy] {} Response Body: \n{}\n================\n\n",chat_protocol.to_owned(), String::from_utf8_lossy(&body_bytes));
        }

        let backend_response = crate::ccproxy::adapter::backend::BackendResponse {
            body: body_bytes,
            tool_compat_mode,
        };
        let unified_response = backend_adapter
            .adapt_response(backend_response)
            .await
            .map_err(|e| CCProxyError::InternalError(e.to_string()))?;

        // Record success statistics
        if let Ok(store) = main_store_arc.read() {
            log::info!(
                "Recording ccproxy stats for non-streaming response: model={}, provider={}",
                &proxy_model.model,
                &proxy_model.provider
            );
            let cache_tokens = unified_response
                .usage
                .cache_read_input_tokens
                .or(unified_response.usage.prompt_cached_tokens)
                .or(unified_response.usage.cached_content_tokens)
                .unwrap_or(0);

            let _ = store.record_ccproxy_stat(CcproxyStat {
                id: None,
                client_model: proxy_model.client_alias.clone(),
                backend_model: proxy_model.model.clone(),
                provider: proxy_model.provider.clone(),
                protocol: chat_protocol.to_string(),
                tool_compat_mode: if final_tool_compat_mode { 1 } else { 0 },
                status_code: 200,
                error_message: None,
                input_tokens: unified_response.usage.input_tokens as i64,
                output_tokens: unified_response.usage.output_tokens as i64,
                cache_tokens: cache_tokens as i64,
                request_at: None,
            });
        }

        let response = output_adapter
            .adapt_response(unified_response, sse_status)
            .map_err(|e| CCProxyError::InternalError(e.to_string()))?;
        Ok(response.into_response())
    }
}
