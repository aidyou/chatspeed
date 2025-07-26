use crate::ai::network::{StreamFormat, StreamProcessor};
use crate::ccproxy::adapter::unified::SseStatus;
use crate::ccproxy::common::CcproxyQuery;
use crate::db::{AiModel, MainStore};
use crate::{
    ai::interaction::chat_completion::ChatProtocol,
    ccproxy::{
        adapter::{
            backend::{BackendAdapter, BackendResponse},
            input::from_gemini,
            output::{GeminiOutputAdapter, OutputAdapter},
        },
        common::ModelResolver,
        errors::{CCProxyError, ProxyResult},
        gemini::GeminiRequest,
        types::openai::BackendModelTarget,
    },
};
use futures_util::{stream::iter, StreamExt};
use rust_i18n::t;

use crate::ccproxy::connection_monitor::{ConnectionMonitor, HeartbeatMonitor, MonitoredStream};
use serde_json::Value;
use std::str::FromStr;
use std::sync::{Arc, Mutex, RwLock};
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;
use warp::{http::header::HeaderMap, Reply};

/// Handle Gemini native format chat completion requests
pub async fn handle_gemini_native_chat(
    // The model name is often part of the URL for Gemini, so we might need to extract it
    // or assume a default if not explicitly in the payload.
    // For now, we'll let `from_gemini` set a default or rely on the handler's routing.
    route_model_alias: String,
    generate_action: String,
    query: CcproxyQuery,
    original_client_headers: HeaderMap,
    client_request_body: bytes::Bytes, // Changed from GeminiRequest
    tool_compat_mode: bool,
    main_store_arc: Arc<Mutex<MainStore>>,
) -> ProxyResult<impl Reply> {
    let log_to_file = query
        .debug
        .unwrap_or(crate::ccproxy::common::DEFAULT_LOG_TO_FILE);

    // Log the raw request body
    if log_to_file {
        log::info!(target: "ccproxy_logger", "Request Body: \n{}\n---", String::from_utf8_lossy(&client_request_body));
    }

    // Manually deserialize the request body
    let client_request_payload: GeminiRequest = match serde_json::from_slice(&client_request_body) {
        Ok(payload) => payload,
        Err(e) => {
            log::error!("Failed to deserialize GeminiRequest: {}", e);
            return Err(warp::reject::custom(CCProxyError::InternalError(
                t!("proxy.error.invalid_request_format", error = e.to_string()).to_string(),
            )));
        }
    };

    let proxy_alias = route_model_alias; // Use the model alias from the route

    // 1. Convert to UnifiedRequest
    let mut unified_request =
        from_gemini(client_request_payload, tool_compat_mode).map_err(|e| {
            CCProxyError::InternalError(
                t!("proxy.error.invalid_request", error = e.to_string()).to_string(),
            )
        })?;
    if generate_action == "streamGenerateContent" {
        unified_request.stream = true
    } else {
        unified_request.stream = false
    }

    // Determine if the original request was streaming
    let is_streaming_request = unified_request.stream;

    // 2. Resolve model and get backend adapter
    let (ai_model_details, selected_target) =
        get_ai_model(main_store_arc.clone(), proxy_alias.clone()).await?;
    let target_api_key =
        ModelResolver::rotator_keys(&ai_model_details.base_url, ai_model_details.api_key.clone())
            .await;
    let actual_model_name = selected_target.model.clone();

    let backend_chat_protocol = ChatProtocol::from_str(&ai_model_details.api_protocol)
        .map_err(|e| warp::reject::custom(CCProxyError::InvalidProtocolError(e.to_string())))?;

    let backend_adapter: Arc<dyn BackendAdapter> = match backend_chat_protocol {
        ChatProtocol::OpenAI | ChatProtocol::Ollama | ChatProtocol::HuggingFace => {
            Arc::new(crate::ccproxy::adapter::backend::OpenAIBackendAdapter)
        }
        ChatProtocol::Claude => Arc::new(crate::ccproxy::adapter::backend::ClaudeBackendAdapter),
        ChatProtocol::Gemini => Arc::new(crate::ccproxy::adapter::backend::GeminiBackendAdapter),
    };

    // Build HTTP client with proxy settings
    let http_client =
        ModelResolver::build_http_client(main_store_arc.clone(), ai_model_details.metadata.clone())
            .await?;

    // 3. Adapt and send request to backend
    let mut onward_request_builder = backend_adapter
        .adapt_request(
            &http_client,
            &unified_request,
            &target_api_key,
            &ai_model_details.base_url,
            &actual_model_name,
        )
        .await
        .map_err(|e| CCProxyError::InternalError(e.to_string()))?;
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
    for (name, value) in original_client_headers.iter() {
        let name_str = name.as_str().to_lowercase();
        if crate::ccproxy::common::should_forward_header(&name_str) {
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
                log::error!(
                    "Failed to read error response body from backend (alias: {}): {}",
                    proxy_alias,
                    e
                );
                let err_msg = t!("network.response_read_error", error = e.to_string()).to_string();
                return Err(warp::reject::custom(CCProxyError::InternalError(err_msg)));
            }
        };
        let error_body_str = String::from_utf8_lossy(&error_body_bytes);
        if log_to_file {
            log::info!(target: "ccproxy_logger", "Gemini Response Error Status: {}, Body: \n{}\n---\n", status_code, error_body_str);
        }

        log::warn!(
            "Backend API error (alias: '{}', model: '{}', provider: '{}'): status_code={}, response={}",
            proxy_alias,
            actual_model_name,
            ai_model_details.name,
            status_code,
            error_body_str
        );

        // Try to parse the error as Gemini-like JSON error, otherwise wrap the raw string.
        let final_error_json: Value =
            if let Ok(json_error) = serde_json::from_str::<Value>(&error_body_str) {
                // If the backend already returned a Gemini-like error, just forward it.
                if json_error.get("error").is_some() {
                    json_error
                } else {
                    // Otherwise, wrap the error in the Gemini structure.
                    serde_json::json!({
                        "error": {
                            "code": status_code.as_u16(),
                            "message": json_error.to_string(), // Use the full JSON as a string message
                            "status": "UNKNOWN"
                        }
                    })
                }
            } else {
                // Not JSON, wrap it.
                serde_json::json!({
                    "error": {
                        "code": status_code.as_u16(),
                        "message": error_body_str.to_string(),
                        "status": "UNKNOWN"
                    }
                })
            };

        let warp_status_code = warp::http::StatusCode::from_u16(status_code.as_u16())
            .unwrap_or(warp::http::StatusCode::INTERNAL_SERVER_ERROR);

        let mut response = warp::reply::json(&final_error_json).into_response();
        *response.status_mut() = warp_status_code;

        // Forward relevant headers from the target's error response
        let final_headers = response.headers_mut();
        for (reqwest_name, reqwest_value) in headers_from_target.iter() {
            let name_str = reqwest_name.as_str().to_lowercase();
            if name_str.starts_with("x-ratelimit-")
                || name_str == "retry-after"
                || name_str == "content-type"
            {
                if let (Ok(warp_name), Ok(warp_value)) = (
                    warp::http::header::HeaderName::from_bytes(reqwest_name.as_ref()),
                    warp::http::header::HeaderValue::from_bytes(reqwest_value.as_ref()),
                ) {
                    final_headers.insert(warp_name, warp_value);
                }
            }
        }
        // Ensure Content-Type is application/json if not already set
        if !final_headers.contains_key(warp::http::header::CONTENT_TYPE) {
            final_headers.insert(
                warp::http::header::CONTENT_TYPE,
                warp::http::header::HeaderValue::from_static("application/json"),
            );
        }
        return Ok(response);
    }

    // 4. Adapt response back to client format
    let output_adapter = GeminiOutputAdapter; // Gemini requests always get Gemini output

    if is_streaming_request {
        let status_code = target_response.status();
        let response_headers_from_target = target_response.headers().clone();

        // Create a StreamProcessor instance to handle reassembly of backend chunks
        let stream_processor = StreamProcessor::new();
        let connection_monitor = ConnectionMonitor::new();

        // Set up callback for client disconnection
        {
            let stream_processor_clone = stream_processor.clone();
            connection_monitor
                .on_disconnect(move || {
                    log::info!("Client disconnected, stopping backend stream processing");
                    stream_processor_clone.stop();
                })
                .await;
        }

        // Start heartbeat monitoring (check every 30 seconds)
        let heartbeat_monitor = HeartbeatMonitor::new(connection_monitor.clone(), 30);
        let _heartbeat_handle = heartbeat_monitor.start_heartbeat();

        let stream_format_for_processor = match backend_chat_protocol {
            ChatProtocol::Gemini => StreamFormat::Gemini,
            ChatProtocol::Claude => StreamFormat::Claude,
            _ => StreamFormat::OpenAI,
        };

        let backend_event_receiver = stream_processor
            .process_stream(target_response, &stream_format_for_processor)
            .await;

        let processed_backend_stream = ReceiverStream::new(backend_event_receiver);
        let monitored_stream = MonitoredStream::new_with_heartbeat(
            processed_backend_stream,
            connection_monitor.clone(),
            heartbeat_monitor,
        );
        let sse_status = Arc::new(RwLock::new(SseStatus::new(
            format!("msg_{}", Uuid::new_v4().simple()),
            proxy_alias.clone(),
            tool_compat_mode,
        )));

        if log_to_file {
            log::info!(target: "ccproxy_logger", "Gemini Stream Response Chunk Start: \n-----\n");
        }

        let sse_status_clone = sse_status.clone();
        let unified_stream = monitored_stream
            .then(move |result_from_processor| {
                let adapter = backend_adapter.clone();
                let status = sse_status_clone.clone();
                async move {
                    match result_from_processor {
                        Ok(chunk) => {
                            #[cfg(debug_assertions)]
                            {
                                log::debug!("recv: {}", String::from_utf8_lossy(&chunk));
                            }
                            adapter
                            .adapt_stream_chunk(chunk, status)
                            .await
                            .unwrap_or_else(|e| {
                                vec![crate::ccproxy::adapter::unified::UnifiedStreamChunk::Error {
                                    message: e.to_string(),
                                }]
                            })
                        }
                        Err(e) => {
                            log::error!("Error from StreamProcessor: {}", e);
                            vec![
                                crate::ccproxy::adapter::unified::UnifiedStreamChunk::Error {
                                    message: e.to_string(),
                                },
                            ]
                        }
                    }
                }
            })
            .flat_map(iter);

        let sse_stream = unified_stream.flat_map(move |unified_chunk| {
            let events = output_adapter
                .adapt_stream_chunk(unified_chunk, sse_status.clone())
                .unwrap_or_else(|_| {
                    log::error!(
                        "adapt_stream_chunk failed unexpectedly despite Infallible error type"
                    );
                    Vec::new()
                });

            if log_to_file && events.len() > 0 {
                log::info!(target:"ccproxy_logger","{}", events.iter().map(|event| event.to_string()).collect::<Vec<String>>().join(""));
            }

            futures_util::stream::iter(
                events
                    .into_iter()
                    .map(|event| Ok::<warp::sse::Event, std::convert::Infallible>(event)),
            )
        });

        let sse_status_code = if status_code.is_client_error() || status_code.is_server_error() {
            warp::http::StatusCode::from_u16(status_code.as_u16())
                .unwrap_or(warp::http::StatusCode::INTERNAL_SERVER_ERROR)
        } else {
            warp::http::StatusCode::OK
        };

        // 创建一个可以检测客户端断开的流
        let client_disconnect_stream = sse_stream.inspect({
            let connection_monitor = connection_monitor.clone();
            move |event| {
                // 如果发送事件失败，说明客户端可能已断开
                if event.is_err() {
                    log::info!("SSE event send failed, client likely disconnected");
                    let monitor = connection_monitor.clone();
                    tokio::spawn(async move {
                        monitor.mark_disconnected().await;
                    });
                }
            }
        });

        let sse_reply_inner =
            warp::sse::reply(warp::sse::keep_alive().stream(client_disconnect_stream));
        let mut response =
            warp::reply::with_status(sse_reply_inner, sse_status_code).into_response();

        // Forward relevant headers from the target's response
        let final_headers = response.headers_mut();
        for (reqwest_name, reqwest_value) in response_headers_from_target.iter() {
            let name_str = reqwest_name.as_str().to_lowercase();
            if name_str == "content-type"
                || name_str == "cache-control"
                || name_str == "x-request-id"
                || name_str.starts_with("x-ratelimit-")
            {
                if let (Ok(warp_name), Ok(warp_value)) = (
                    warp::http::header::HeaderName::from_bytes(reqwest_name.as_ref()),
                    warp::http::header::HeaderValue::from_bytes(reqwest_value.as_ref()),
                ) {
                    final_headers.insert(warp_name, warp_value);
                }
            }
        }
        // Ensure Content-Type is text/event-stream if not already set
        if !response_headers_from_target.contains_key(reqwest::header::CONTENT_TYPE) {
            final_headers.insert(
                warp::http::header::CONTENT_TYPE,
                warp::http::header::HeaderValue::from_static("text/event-stream"),
            );
        }
        Ok(response)
    } else {
        let body_bytes = target_response
            .bytes()
            .await
            .map_err(|e| warp::reject::custom(CCProxyError::InternalError(e.to_string())))?;

        if log_to_file {
            log::info!(target: "ccproxy_logger", "Gemini Response Body: \n{}\n---", String::from_utf8_lossy(&body_bytes));
        }

        let backend_response = BackendResponse {
            body: body_bytes,
            tool_compat_mode,
        };
        let unified_response = backend_adapter
            .adapt_response(backend_response)
            .await
            .map_err(|e| warp::reject::custom(CCProxyError::InternalError(e.to_string())))?;
        let response = output_adapter
            .adapt_response(unified_response)
            .map_err(|e| warp::reject::custom(CCProxyError::InternalError(e.to_string())))?;
        Ok(response.into_response())
    }
}

async fn get_ai_model(
    main_store_arc: Arc<Mutex<MainStore>>,
    proxy_alias: String,
) -> ProxyResult<(AiModel, BackendModelTarget)> {
    ModelResolver::get_ai_model_by_alias(main_store_arc, proxy_alias).await
}
