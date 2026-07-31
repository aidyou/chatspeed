use axum::{
    body::{to_bytes, Body},
    response::{IntoResponse, Response},
};
use reqwest::header::HeaderMap;
use rust_i18n::t;
use std::sync::{Arc, RwLock};

use crate::ccproxy::handler::request_preprocessor::{
    preprocess_client_request_body, preprocess_unified_request,
};
use crate::ccproxy::helper::{get_msg_id, send_with_retry, RetryConfig};
use crate::ccproxy::ChatProtocol;
use crate::ccproxy::{
    adapter::{
        backend::{self, BackendAdapter},
        input::{from_claude, from_gemini, from_ollama, from_openai},
        output::{
            ClaudeOutputAdapter, GeminiOutputAdapter, OllamaOutputAdapter, OpenAIOutputAdapter,
            OutputAdapter, OutputAdapterEnum,
        },
        unified::{SseStatus, UnifiedErrorResponse, UnifiedRequest},
    },
    claude::ClaudeNativeRequest,
    errors::{CCProxyError, ProxyResult},
    gemini::GeminiRequest,
    helper::{
        get_provider_chat_full_url, stream_handler::handle_streamed_response, CcproxyQuery,
        ModelResolver,
    },
    openai::OpenAIChatCompletionRequest,
    types::{ollama::OllamaChatCompletionRequest, ProxyModel},
};
use crate::constants::{
    CFG_CCPROXY_LOG_PROXY_TO_FILE, CFG_CCPROXY_LOG_TO_FILE, CFG_CCPROXY_RETRY_ON_429,
    CFG_CCPROXY_RETRY_ON_429_DEFAULT,
};
use crate::db::{CcproxyStat, MainStore};

async fn log_client_response(
    response: Response,
    client_protocol: &ChatProtocol,
    log_to_file: bool,
) -> ProxyResult<Response> {
    if !log_to_file {
        return Ok(response);
    }

    let (parts, body) = response.into_parts();
    let body_bytes = to_bytes(body, usize::MAX)
        .await
        .map_err(|e| CCProxyError::InternalError(e.to_string()))?;
    log::info!(
        target: "ccproxy_client_logger",
        "[Client] {} Response Body: \n{}\n================\n\n",
        client_protocol,
        String::from_utf8_lossy(&body_bytes)
    );

    Ok(Response::from_parts(parts, Body::from(body_bytes)))
}

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

pub(crate) fn prepare_unified_request_for_proxy_model(
    unified_request: &mut UnifiedRequest,
    proxy_model: &ProxyModel,
) {
    unified_request.custom_params = proxy_model.custom_params.clone();

    // --- Inject Engine Defaults only if missing from client AND configured with valid non-default values ---
    ModelResolver::merge_parameters_unified(unified_request, proxy_model);

    preprocess_unified_request(unified_request, proxy_model);

    if proxy_model.tool_filter.len() > 0 {
        unified_request.tools = unified_request.tools.take().map(|tools| {
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

    if !proxy_model.prompt_replace.is_empty() {
        if let Some(system_prompt) = &mut unified_request.system_prompt {
            for (key, value) in &proxy_model.prompt_replace {
                if !key.is_empty() {
                    *system_prompt = system_prompt.replace(key, value);
                }
            }
        }
    }
}

pub(crate) async fn execute_unified_chat_request(
    client_protocol: ChatProtocol,
    client_headers: HeaderMap,
    mut unified_request: UnifiedRequest,
    proxy_alias: String,
    proxy_model: ProxyModel,
    is_streaming_request: bool,
    tool_compat_mode: bool,
    final_tool_compat_mode: bool,
    message_id: String,
    log_org_to_file: bool,
    log_proxy_to_file: bool,
    main_store_arc: Arc<MainStore>,
    output_adapter: OutputAdapterEnum,
) -> ProxyResult<Response> {
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

    let http_client = ModelResolver::build_http_client(
        main_store_arc.clone(),
        proxy_model.model_metadata.clone(),
    )?;

    let mut final_headers = reqwest::header::HeaderMap::new();
    ModelResolver::inject_proxy_headers(
        &mut final_headers,
        &client_headers,
        &proxy_model,
        &message_id,
    );

    final_headers.insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/json"),
    );

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

    onward_request_builder = onward_request_builder.headers(final_headers);

    let max_retries =
        main_store_arc.get_config(CFG_CCPROXY_RETRY_ON_429, CFG_CCPROXY_RETRY_ON_429_DEFAULT);
    let retry_config = RetryConfig::from_settings(max_retries);

    let target_response = match send_with_retry(onward_request_builder, &retry_config).await {
        Ok(response) => response,
        Err(CCProxyError::BackendRequestError(message)) => {
            log::warn!(
                "Backend request failed before receiving a response (alias: '{}', model: '{}', provider: '{}'): error={}",
                proxy_alias,
                proxy_model.model,
                proxy_model.provider,
                message
            );

            if log_proxy_to_file {
                log::info!(target: "ccproxy_upstream_logger", "[ERROR] Backend request failed before receiving a response, protocol: {}, model: {}\n{}\n---", proxy_model.chat_protocol.to_string(), &proxy_model.model, message);
            }

            {
                let store = main_store_arc.as_ref();
                let _ = store.record_ccproxy_stat(
                    CcproxyStat {
                        id: None,
                        workflow_session_id: None,
                        workflow_task_run_id: None,
                        workflow_segment_id: None,
                        root_session_id: None,
                        root_task_run_id: None,
                        request_kind: None,
                        client_model: proxy_model.client_alias.clone(),
                        backend_model: proxy_model.model.clone(),
                        provider_id: Some(proxy_model.provider_id),
                        provider: proxy_model.provider.clone(),
                        protocol: client_protocol.to_string(),
                        tool_compat_mode: if final_tool_compat_mode { 1 } else { 0 },
                        status_code: http::StatusCode::BAD_GATEWAY.as_u16() as i32,
                        error_message: Some(message.clone()),
                        input_tokens: 0,
                        output_tokens: 0,
                        cache_tokens: 0,
                        request_at: None,
                    }
                    .with_workflow_attribution(&client_headers),
                );
            }

            let response = output_adapter.adapt_error_response(UnifiedErrorResponse {
                status_code: http::StatusCode::BAD_GATEWAY.as_u16(),
                message,
                error_type: None,
                code: None,
                request_id: Some(message_id),
            });
            return log_client_response(response, &client_protocol, log_org_to_file).await;
        }
        Err(error) => return Err(error),
    };

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

        if log_proxy_to_file {
            log::info!(target: "ccproxy_upstream_logger", "[ERROR] {} Response Error, model: {}, Status: {}, Body: \n{}\n---", proxy_model.chat_protocol.to_string(), &proxy_model.model, status_code, error_body_str);
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

        let mut unified_error = crate::ccproxy::adapter::error::normalize_backend_error(
            &proxy_model.chat_protocol,
            status_code,
            &headers_from_target,
            &error_body_bytes,
        );
        if unified_error.request_id.is_none() {
            unified_error.request_id = Some(message_id.clone());
        }
        let message_content = unified_error.message.clone();

        {
            let store = main_store_arc.as_ref();
            let _ = store.record_ccproxy_stat(
                CcproxyStat {
                    id: None,
                    workflow_session_id: None,
                    workflow_task_run_id: None,
                    workflow_segment_id: None,
                    root_session_id: None,
                    root_task_run_id: None,
                    request_kind: None,
                    client_model: proxy_model.client_alias.clone(),
                    backend_model: proxy_model.model.clone(),
                    provider_id: Some(proxy_model.provider_id),
                    provider: proxy_model.provider.clone(),
                    protocol: client_protocol.to_string(),
                    tool_compat_mode: if final_tool_compat_mode { 1 } else { 0 },
                    status_code: status_code.as_u16() as i32,
                    error_message: Some(message_content),
                    input_tokens: 0,
                    output_tokens: 0,
                    cache_tokens: 0,
                    request_at: None,
                }
                .with_workflow_attribution(&client_headers),
            );
        }

        let mut response = output_adapter.adapt_error_response(unified_error);

        let filtered_headers =
            crate::ccproxy::utils::http::filter_proxy_headers(&headers_from_target);
        let final_headers = response.headers_mut();

        for (name, value) in filtered_headers.iter() {
            if name != http::header::CONTENT_TYPE {
                final_headers.insert(name.clone(), value.clone());
            }
        }

        if !final_headers.contains_key(http::header::CONTENT_TYPE) {
            final_headers.insert(
                http::header::CONTENT_TYPE,
                http::header::HeaderValue::from_static("application/json"),
            );
        }
        return log_client_response(response, &client_protocol, log_org_to_file).await;
    }

    let estimated_input_tokens =
        crate::ccproxy::utils::token_estimator::estimate_unified_request_tokens(&unified_request);

    let responses_custom_tool_names = unified_request.responses_custom_tool_names.clone();
    let mut status = SseStatus::new(
        message_id,
        proxy_alias.clone(),
        tool_compat_mode,
        estimated_input_tokens,
    );
    status.responses_custom_tool_names = responses_custom_tool_names;
    let sse_status = Arc::new(RwLock::new(status));

    if is_streaming_request {
        let res = handle_streamed_response(
            &client_headers,
            Arc::new(proxy_model.chat_protocol),
            client_protocol,
            target_response,
            backend_adapter,
            output_adapter,
            sse_status,
            log_org_to_file,
            log_proxy_to_file,
            main_store_arc.clone(),
            proxy_model.client_alias.clone(),
            proxy_model.model.clone(),
            proxy_model.provider_id,
            proxy_model.provider.clone(),
            final_tool_compat_mode,
        )
        .await?;
        Ok(res.into_response())
    } else {
        let response_headers_from_target = target_response.headers().clone();
        let body_bytes = target_response
            .bytes()
            .await
            .map_err(|e| CCProxyError::InternalError(e.to_string()))?;

        if log_proxy_to_file {
            log::info!(target: "ccproxy_upstream_logger", "[Upstream] {} Response Body: \n{}\n================\n\n", proxy_model.chat_protocol.to_string(), String::from_utf8_lossy(&body_bytes));
        }

        let backend_response = crate::ccproxy::adapter::backend::BackendResponse {
            body: body_bytes,
            tool_compat_mode,
        };
        let unified_response = backend_adapter
            .adapt_response(backend_response)
            .await
            .map_err(|e| CCProxyError::InternalError(e.to_string()))?;

        {
            let store = main_store_arc.as_ref();
            log::info!(
                "Recording ccproxy stats for non-streaming response: model={}, provider={}",
                &proxy_model.model,
                &proxy_model.provider
            );

            #[cfg(debug_assertions)]
            log::debug!(
                "Stat details: provider='{}', model='{}', client_alias='{}'",
                &proxy_model.provider,
                &proxy_model.model,
                &proxy_model.client_alias
            );

            let cache_tokens = unified_response
                .usage
                .cache_read_input_tokens
                .or(unified_response.usage.prompt_cached_tokens)
                .or(unified_response.usage.cached_content_tokens)
                .unwrap_or(0);

            let _ = store.record_ccproxy_stat(
                CcproxyStat {
                    id: None,
                    workflow_session_id: None,
                    workflow_task_run_id: None,
                    workflow_segment_id: None,
                    root_session_id: None,
                    root_task_run_id: None,
                    request_kind: None,
                    client_model: proxy_model.client_alias.clone(),
                    backend_model: proxy_model.model.clone(),
                    provider_id: Some(proxy_model.provider_id),
                    provider: proxy_model.provider.clone(),
                    protocol: client_protocol.to_string(),
                    tool_compat_mode: if final_tool_compat_mode { 1 } else { 0 },
                    status_code: 200,
                    error_message: None,
                    input_tokens: unified_response.usage.input_tokens as i64,
                    output_tokens: unified_response.usage.output_tokens as i64,
                    cache_tokens: cache_tokens as i64,
                    request_at: None,
                }
                .with_workflow_attribution(&client_headers),
            );
        }

        let mut response = output_adapter
            .adapt_response(unified_response, sse_status)
            .map_err(|e| CCProxyError::InternalError(e.to_string()))?
            .into_response();

        let filtered_headers =
            crate::ccproxy::utils::http::filter_proxy_headers(&response_headers_from_target);
        let final_headers = response.headers_mut();
        for (name, value) in filtered_headers.iter() {
            let name_str = name.as_str().to_lowercase();
            if name_str.starts_with("x-") || name_str == "retry-after" {
                final_headers.insert(name.clone(), value.clone());
            }
        }

        log_client_response(response, &client_protocol, log_org_to_file).await
    }
}

pub async fn handle_chat_completion(
    chat_protocol: ChatProtocol,
    client_headers: HeaderMap,
    _client_query: CcproxyQuery,
    client_request_body: bytes::Bytes,
    group_name: Option<String>,
    tool_compat_mode: bool,
    route_model_alias: String,
    generate_action: String,
    main_store_arc: Arc<MainStore>,
) -> ProxyResult<Response> {
    let protocol_string = chat_protocol.to_string();
    let message_id = get_msg_id();

    let log_org_to_file = main_store_arc.get_config(CFG_CCPROXY_LOG_TO_FILE, false);
    let log_proxy_to_file = main_store_arc.get_config(CFG_CCPROXY_LOG_PROXY_TO_FILE, false);

    let proxy_model = if let Some(provider_id) = client_headers
        .get("x-cs-provider-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok())
    {
        let model_id = client_headers
            .get("x-cs-model-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .or_else(|| {
                get_proxy_alias_from_body(&chat_protocol, &client_request_body, &route_model_alias)
                    .ok()
            })
            .ok_or_else(|| {
                CCProxyError::ModelAliasNotFound("Missing model id in header or body".to_string())
            })?;

        ModelResolver::get_ai_model_by_provider_and_model(
            main_store_arc.clone(),
            provider_id,
            model_id,
        )
        .await?
    } else {
        let proxy_alias_raw =
            get_proxy_alias_from_body(&chat_protocol, &client_request_body, &route_model_alias)?;

        // Support "group@alias" format in the model field
        let (proxy_alias, group_name) = if let Some((g, a)) = proxy_alias_raw.split_once('@') {
            (a.to_string(), Some(g.to_string()))
        } else {
            (proxy_alias_raw, group_name)
        };

        let group_name = group_name.as_deref();

        ModelResolver::get_ai_model_by_alias(main_store_arc.clone(), proxy_alias, group_name)
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

    let preprocessed_request_body =
        preprocess_client_request_body(client_request_body, &chat_protocol, &proxy_model)?;

    if chat_protocol == proxy_model.chat_protocol && !final_tool_compat_mode {
        let is_streaming = match chat_protocol {
            ChatProtocol::OpenAI | ChatProtocol::HuggingFace => {
                let req: OpenAIChatCompletionRequest =
                    serde_json::from_slice(&preprocessed_request_body).unwrap_or_default();
                req.stream.unwrap_or(false)
            }
            ChatProtocol::Claude => {
                let req: Result<ClaudeNativeRequest, _> =
                    serde_json::from_slice(&preprocessed_request_body);
                req.map(|r| r.stream.unwrap_or(false)).unwrap_or(false)
            }
            ChatProtocol::Ollama => {
                let req: OllamaChatCompletionRequest =
                    serde_json::from_slice(&preprocessed_request_body).unwrap_or_default();
                req.stream.unwrap_or(false)
            }
            ChatProtocol::Gemini => generate_action == "streamGenerateContent",
        };

        let result = super::handle_direct_forward(
            client_headers,
            preprocessed_request_body,
            proxy_model,
            is_streaming,
            main_store_arc,
            log_proxy_to_file,
        )
        .await?;
        return Ok(result.into_response());
    }

    if log_org_to_file {
        log::info!(target: "ccproxy_client_logger", "message id:{}\n{} Client Request Body: \n{}\n----------------\n", &message_id, &protocol_string, String::from_utf8_lossy(&preprocessed_request_body));
    }

    let (mut unified_request, proxy_alias, is_streaming_request) = build_unified_request(
        chat_protocol.clone(),
        preprocessed_request_body,
        final_tool_compat_mode,
        route_model_alias,
        generate_action,
    )?;

    prepare_unified_request_for_proxy_model(&mut unified_request, &proxy_model);

    let output_adapter: OutputAdapterEnum = match chat_protocol {
        ChatProtocol::OpenAI | ChatProtocol::HuggingFace => {
            OutputAdapterEnum::OpenAI(OpenAIOutputAdapter)
        }
        ChatProtocol::Claude => OutputAdapterEnum::Claude(ClaudeOutputAdapter),
        ChatProtocol::Gemini => OutputAdapterEnum::Gemini(GeminiOutputAdapter),
        ChatProtocol::Ollama => OutputAdapterEnum::Ollama(OllamaOutputAdapter),
    };

    execute_unified_chat_request(
        chat_protocol,
        client_headers,
        unified_request,
        proxy_alias,
        proxy_model,
        is_streaming_request,
        tool_compat_mode,
        final_tool_compat_mode,
        message_id,
        log_org_to_file,
        log_proxy_to_file,
        main_store_arc,
        output_adapter,
    )
    .await
}

#[cfg(test)]
mod usage_attribution_tests {
    use super::*;
    use axum::{extract::State, routing::post, Router};
    use tempfile::tempdir;
    use tokio::sync::mpsc;

    fn proxy_model(base_url: String) -> ProxyModel {
        ProxyModel {
            client_alias: "alias".to_string(),
            provider_id: 1,
            provider: "provider".to_string(),
            chat_protocol: ChatProtocol::OpenAI,
            base_url,
            model: "backend-model".to_string(),
            api_key: String::new(),
            model_metadata: None,
            custom_params: None,
            prompt_injection: "off".to_string(),
            prompt_injection_position: None,
            prompt_text: String::new(),
            tool_filter: Default::default(),
            prompt_replace: Vec::new(),
            temp_ratio: 1.0,
            max_tokens: None,
            temperature: None,
            presence_penalty: None,
            frequency_penalty: None,
            top_p: None,
            top_k: None,
            stop: Vec::new(),
            tool_compat_mode: None,
        }
    }

    fn attribution_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in [
            ("x-cs-workflow-session-id", "workflow-session"),
            ("x-cs-workflow-task-run-id", "workflow-session:task:1"),
            ("x-cs-workflow-segment-id", "3"),
            ("x-cs-root-session-id", "root-session"),
            ("x-cs-root-task-run-id", "root-session:task:1"),
            ("x-cs-request-kind", "react"),
        ] {
            headers.insert(name, value.parse().unwrap());
        }
        headers
    }

    #[tokio::test]
    async fn ccproxy_usage_attribution_unified_non_stream_persists_and_stays_off_upstream() {
        let (headers_tx, mut headers_rx) = mpsc::channel(1);
        let app = Router::new().route("/chat/completions", post(
            move |State(sender): State<mpsc::Sender<HeaderMap>>, headers: HeaderMap| async move {
                sender.send(headers).await.expect("receiver should remain open");
                axum::Json(serde_json::json!({
                    "id": "chatcmpl_test", "object": "chat.completion", "created": 1, "model": "backend-model",
                    "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}],
                    "usage": {"prompt_tokens": 12, "completion_tokens": 4, "total_tokens": 16}
                }))
            }
        )).with_state(headers_tx);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(tokio::net::TcpListener::from_std(listener).unwrap(), app)
                .await
                .unwrap();
        });
        let directory = tempdir().unwrap();
        let store = Arc::new(MainStore::new(directory.path().join("ccproxy.db")).unwrap());
        let mut request = UnifiedRequest::default();
        request.model = "alias".to_string();
        let response = execute_unified_chat_request(
            ChatProtocol::OpenAI,
            attribution_headers(),
            request,
            "alias".to_string(),
            proxy_model(format!("http://{address}")),
            false,
            false,
            false,
            "message-id".to_string(),
            false,
            false,
            store.clone(),
            OutputAdapterEnum::OpenAI(OpenAIOutputAdapter),
        )
        .await
        .expect("unified handler should succeed");
        assert_eq!(response.status(), http::StatusCode::OK);
        let upstream_headers = headers_rx.recv().await.unwrap();
        assert!(!upstream_headers.contains_key("x-cs-workflow-session-id"));
        assert!(!upstream_headers.contains_key("x-cs-request-kind"));
        let runtime = store.db_runtime().unwrap();
        let flush_runtime = runtime.clone();
        tokio::task::spawn_blocking(move || flush_runtime.drain_blocking())
            .await
            .unwrap()
            .unwrap();
        let stat = runtime.read(|conn| Ok(conn.query_row(
            "SELECT workflow_session_id, workflow_task_run_id, root_session_id, root_task_run_id, request_kind FROM ccproxy_stats", [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?)),
        )?)).await.unwrap();
        assert_eq!(
            stat,
            (
                "workflow-session".to_string(),
                "workflow-session:task:1".to_string(),
                "root-session".to_string(),
                "root-session:task:1".to_string(),
                "react".to_string()
            )
        );
        server.abort();
    }

    #[tokio::test]
    async fn ccproxy_usage_attribution_unified_stream_persists_after_body_consumption() {
        let app = Router::new().route("/chat/completions", post(|| async {
            axum::response::Response::builder()
                .header("content-type", "text/event-stream")
                .body(Body::from(concat!(
                    "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"index\":0}]}\n\n",
                    "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\",\"index\":0}],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":4,\"total_tokens\":16}}\n\n",
                    "data: [DONE]\n\n"
                )))
                .unwrap()
        }));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(tokio::net::TcpListener::from_std(listener).unwrap(), app)
                .await
                .unwrap();
        });
        let directory = tempdir().unwrap();
        let store =
            Arc::new(MainStore::new(directory.path().join("ccproxy-unified-stream.db")).unwrap());
        let mut request = UnifiedRequest::default();
        request.model = "alias".to_string();
        request.stream = true;
        let response = execute_unified_chat_request(
            ChatProtocol::OpenAI,
            attribution_headers(),
            request,
            "alias".to_string(),
            proxy_model(format!("http://{address}")),
            true,
            false,
            false,
            "message-id".to_string(),
            false,
            false,
            store.clone(),
            OutputAdapterEnum::OpenAI(OpenAIOutputAdapter),
        )
        .await
        .unwrap();
        let _body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let runtime = store.db_runtime().unwrap();
        let flush_runtime = runtime.clone();
        tokio::task::spawn_blocking(move || flush_runtime.drain_blocking())
            .await
            .unwrap()
            .unwrap();
        let stat = runtime.read(|conn| Ok(conn.query_row(
            "SELECT workflow_session_id, request_kind, input_tokens, output_tokens FROM ccproxy_stats", [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?)),
        )?)).await.unwrap();
        assert_eq!(
            stat,
            ("workflow-session".to_string(), "react".to_string(), 12, 4)
        );
        server.abort();
    }
}
