use rust_i18n::t;
use serde_json::Value;
use std::sync::{Arc, Mutex, RwLock};
use uuid::Uuid;
use warp::{http::header::HeaderMap, Reply};

use crate::ai::interaction::ChatProtocol;
use crate::ccproxy::adapter::backend;
use crate::ccproxy::adapter::input::{from_claude, from_gemini, from_ollama};
use crate::ccproxy::adapter::output::{
    ClaudeOutputAdapter, GeminiOutputAdapter, OllamaOutputAdapter, OutputAdapterEnum,
};
use crate::ccproxy::adapter::unified::UnifiedRequest;
use crate::ccproxy::claude::ClaudeNativeRequest;
use crate::ccproxy::gemini::GeminiRequest;
use crate::ccproxy::types::ollama::OllamaChatCompletionRequest;
use crate::ccproxy::{
    adapter::{
        backend::BackendAdapter,
        input::from_openai,
        output::{OpenAIOutputAdapter, OutputAdapter},
        unified::SseStatus,
    },
    errors::{CCProxyError, ProxyResult},
    helper::stream_handler::handle_streamed_response,
    helper::{get_provider_full_url, CcproxyQuery, ModelResolver},
    openai::OpenAIChatCompletionRequest,
};
use crate::db::MainStore;

fn get_proxy_alias_from_body(
    chat_protocol: &ChatProtocol,
    client_request_body: &bytes::Bytes,
    route_model_alias: &str,
) -> Result<String, warp::reject::Rejection> {
    match chat_protocol {
        ChatProtocol::OpenAI | ChatProtocol::HuggingFace => {
            let payload: OpenAIChatCompletionRequest = serde_json::from_slice(client_request_body)
                .map_err(|e| {
                    warp::reject::custom(CCProxyError::InternalError(format!(
                        "Failed to deserialize OpenAI request: {}",
                        e
                    )))
                })?;
            Ok(payload.model)
        }
        ChatProtocol::Ollama => {
            let payload: OllamaChatCompletionRequest = serde_json::from_slice(client_request_body)
                .map_err(|e| {
                    warp::reject::custom(CCProxyError::InternalError(format!(
                        "Failed to deserialize Ollama request: {}",
                        e
                    )))
                })?;
            Ok(payload.model)
        }
        ChatProtocol::Claude => {
            let payload: ClaudeNativeRequest = serde_json::from_slice(client_request_body)
                .map_err(|e| {
                    warp::reject::custom(CCProxyError::InternalError(format!(
                        "Failed to deserialize Claude request: {}",
                        e
                    )))
                })?;
            Ok(payload.model)
        }
        ChatProtocol::Gemini => Ok(route_model_alias.to_string()),
    }
}

fn build_unified_request(
    chat_protocol: ChatProtocol,
    client_request_body: bytes::Bytes,
    tool_compat_mode: bool,
    route_model_alias: String,
    generate_action: String,
) -> Result<(UnifiedRequest, String, bool), warp::reject::Rejection> {
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
                        return Err(warp::reject::custom(CCProxyError::InternalError(
                            t!("proxy.error.invalid_request_format", error = e.to_string())
                                .to_string(),
                        )));
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
                        return Err(warp::reject::custom(CCProxyError::InternalError(
                            t!("proxy.error.invalid_request_format", error = e.to_string())
                                .to_string(),
                        )));
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
                        return Err(warp::reject::custom(CCProxyError::InternalError(
                            t!("proxy.error.invalid_request_format", error = e.to_string())
                                .to_string(),
                        )));
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
                        return Err(warp::reject::custom(CCProxyError::InternalError(
                            t!("proxy.error.invalid_request_format", error = e.to_string())
                                .to_string(),
                        )));
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

pub async fn handle_openai_chat_completion(
    chat_protocol: ChatProtocol,
    client_headers: HeaderMap,
    client_query: CcproxyQuery,
    client_request_body: bytes::Bytes,
    tool_compat_mode: bool,
    route_model_alias: String,
    generate_action: String,
    main_store_arc: Arc<Mutex<MainStore>>,
) -> ProxyResult<impl Reply> {
    let protocol_string = chat_protocol.to_string();

    let log_to_file = client_query
        .debug
        .unwrap_or(crate::ccproxy::helper::DEFAULT_LOG_TO_FILE);
    if log_to_file {
        // Log the raw request body
        log::info!(target: "ccproxy_logger", "{} Request Body: \n{}\n---", &protocol_string, String::from_utf8_lossy(&client_request_body));
    }

    let proxy_alias =
        get_proxy_alias_from_body(&chat_protocol, &client_request_body, &route_model_alias)?;

    let proxy_model =
        ModelResolver::get_ai_model_by_alias(main_store_arc.clone(), proxy_alias.clone()).await?;

    if chat_protocol == proxy_model.chat_protocol {
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
            log_to_file,
        )
        .await?;
        return Ok(result.into_response());
    }

    let (unified_request, proxy_alias, is_streaming_request) = build_unified_request(
        chat_protocol.clone(),
        client_request_body.clone(),
        tool_compat_mode,
        route_model_alias,
        generate_action,
    )?;

    // 2. Resolve model and get backend adapter
    let proxy_model =
        ModelResolver::get_ai_model_by_alias(main_store_arc.clone(), proxy_alias.clone()).await?;

    let full_url = get_provider_full_url(
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
    let http_client =
        ModelResolver::build_http_client(main_store_arc.clone(), proxy_model.metadata.clone())
            .await?;

    // 3. Adapt and send request to backend
    let mut onward_request_builder = backend_adapter
        .adapt_request(
            &http_client,
            &unified_request,
            &proxy_model.api_key,
            &full_url,
            &proxy_model.model,
        )
        .await
        .map_err(|e| warp::reject::custom(CCProxyError::InternalError(e.to_string())))?;

    // Set common headers
    onward_request_builder = onward_request_builder.header("Content-Type", "application/json");

    // Set streaming request headers
    if is_streaming_request {
        onward_request_builder =
            onward_request_builder.header(reqwest::header::ACCEPT, "text/event-stream");
    } else {
        onward_request_builder =
            onward_request_builder.header(reqwest::header::ACCEPT, "application/json");
    }

    // Forward relevant headers
    for (name, value) in client_headers.iter() {
        let name_str = name.as_str().to_lowercase();
        if crate::ccproxy::helper::should_forward_header(&name_str) {
            if let (Ok(reqwest_name), Ok(reqwest_value)) = (
                reqwest::header::HeaderName::from_bytes(name.as_ref()),
                reqwest::header::HeaderValue::from_bytes(value.as_ref()),
            ) {
                onward_request_builder = onward_request_builder.header(reqwest_name, reqwest_value);
            }
        }
    }

    let target_response = onward_request_builder.send().await.map_err(|e| {
        warp::reject::custom(CCProxyError::InternalError(format!(
            "Request to backend failed: {}",
            e
        )))
    })?;

    // Handle error response
    if !target_response.status().is_success() {
        let status_code = target_response.status();
        let headers_from_target = target_response.headers().clone();
        let error_body_bytes = match target_response.bytes().await {
            Ok(bytes) => bytes,
            Err(e) => {
                log::error!("Failed to read backend error response: {}", e);
                let err_msg = t!("network.response_read_error", error = e.to_string()).to_string();
                return Err(warp::reject::custom(CCProxyError::InternalError(err_msg)));
            }
        };
        let error_body_str = String::from_utf8_lossy(&error_body_bytes);

        if log_to_file {
            log::info!(target: "ccproxy_logger", "OpenAI-Compatible Response Error, Status: {}, Body: \n{}\n---", status_code, error_body_str);
        }

        log::warn!(
            "Backend API error (alias: '{}', model: '{}', provider: '{}'): status_code={}, response={}",
            proxy_alias,
            proxy_model.model,
            proxy_model.provider,
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

        let warp_status_code = warp::http::StatusCode::from_u16(status_code.as_u16())
            .unwrap_or(warp::http::StatusCode::INTERNAL_SERVER_ERROR);

        let mut response =
            warp::reply::json(&serde_json::json!({ "error": final_error_json })).into_response();
        *response.status_mut() = warp_status_code;

        let final_headers = response.headers_mut();
        for (reqwest_name, reqwest_value) in headers_from_target.iter() {
            let name_str = reqwest_name.as_str().to_lowercase();
            if name_str.starts_with("x-ratelimit-")
                || name_str == "retry-after"
                || name_str == "content-type"
            {
                if let Ok(warp_name) =
                    warp::http::header::HeaderName::from_bytes(reqwest_name.as_ref())
                {
                    if let Ok(warp_value) =
                        warp::http::header::HeaderValue::from_bytes(reqwest_value.as_ref())
                    {
                        final_headers.insert(warp_name, warp_value);
                    }
                }
            }
        }
        if !final_headers.contains_key(warp::http::header::CONTENT_TYPE) {
            final_headers.insert(
                warp::http::header::CONTENT_TYPE,
                warp::http::header::HeaderValue::from_static("application/json"),
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

    let sse_status = Arc::new(RwLock::new(SseStatus::new(
        format!("msg_{}", Uuid::new_v4().simple()),
        proxy_alias.clone(),
        tool_compat_mode,
    )));

    if is_streaming_request {
        let res = handle_streamed_response(
            &proxy_model.chat_protocol,
            target_response,
            backend_adapter,
            output_adapter,
            sse_status,
            log_to_file,
        )
        .await?;
        Ok(res.into_response())
    } else {
        let body_bytes = target_response
            .bytes()
            .await
            .map_err(|e| warp::reject::custom(CCProxyError::InternalError(e.to_string())))?;

        if log_to_file {
            log::info!(target: "ccproxy_logger", "OpenAI-Compatible Response Body: \n{}\n---", String::from_utf8_lossy(&body_bytes));
        }

        let backend_response = crate::ccproxy::adapter::backend::BackendResponse {
            body: body_bytes,
            tool_compat_mode,
        };
        let unified_response = backend_adapter
            .adapt_response(backend_response)
            .await
            .map_err(|e| warp::reject::custom(CCProxyError::InternalError(e.to_string())))?;

        let response = output_adapter
            .adapt_response(unified_response, sse_status)
            .map_err(|e| warp::reject::custom(CCProxyError::InternalError(e.to_string())))?;
        Ok(response.into_response())
    }
}
