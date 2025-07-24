use futures_util::{stream::iter, StreamExt};
use rust_i18n::t;
use std::sync::{Arc, Mutex, RwLock};
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;
use warp::{http::header::HeaderMap, Reply};

use crate::ai::interaction::ChatProtocol;
use crate::ai::network::{StreamFormat, StreamProcessor};
use crate::ccproxy::adapter::unified::SseStatus;
use crate::ccproxy::common::ModelResolver;
use crate::ccproxy::{
    adapter::{
        backend::{BackendAdapter, BackendResponse},
        input::from_claude,
        output::{ClaudeOutputAdapter, OutputAdapter},
    },
    errors::{ProxyAuthError, ProxyResult},
    types::claude::ClaudeNativeRequest,
    types::openai::BackendModelTarget,
};
use crate::db::{AiModel, MainStore};
use serde_json::Value;
use std::str::FromStr;

/// Handle Claude native format chat completion requests
pub async fn handle_claude_native_chat(
    original_client_headers: HeaderMap,
    client_request_payload: ClaudeNativeRequest,
    main_store_arc: Arc<Mutex<MainStore>>,
) -> ProxyResult<impl Reply> {
    let proxy_alias = client_request_payload.model.clone();
    let is_streaming_request = client_request_payload.stream.unwrap_or(false);

    // 1. Convert to UnifiedRequest
    let unified_request = from_claude(client_request_payload).map_err(|e| {
        ProxyAuthError::InternalError(
            t!("proxy.error.invalid_request", error = e.to_string()).to_string(),
        )
    })?;

    // 2. Resolve model and get backend adapter
    let (ai_model_details, selected_target) =
        get_ai_model(main_store_arc.clone(), proxy_alias.clone()).await?;
    let target_api_key =
        ModelResolver::rotator_keys(&ai_model_details.base_url, ai_model_details.api_key.clone())
            .await;
    let actual_model_name = selected_target.model.clone();

    let backend_chat_protocol = ChatProtocol::from_str(&ai_model_details.api_protocol)
        .map_err(|e| warp::reject::custom(ProxyAuthError::InvalidProtocolError(e.to_string())))?;

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
        .map_err(|e| ProxyAuthError::InternalError(e.to_string()))?;

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
        warp::reject::custom(ProxyAuthError::InternalError(format!(
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
                return Err(warp::reject::custom(ProxyAuthError::InternalError(err_msg)));
            }
        };
        let error_body_str = String::from_utf8_lossy(&error_body_bytes);

        log::warn!(
            "Proxy target returned an error. Status: {}, Body: {}",
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
    let output_adapter = ClaudeOutputAdapter;

    if is_streaming_request {
        let stream_format = match backend_chat_protocol {
            ChatProtocol::Gemini => StreamFormat::Gemini,
            ChatProtocol::Claude => StreamFormat::Claude,
            _ => StreamFormat::OpenAI,
        };

        let stream_processor = StreamProcessor::new();
        let reassembled_receiver = stream_processor
            .process_stream(target_response, &stream_format)
            .await;

        // Wrap the receiver in a stream to make it compatible with futures_util::StreamExt
        let reassembled_stream = ReceiverStream::new(reassembled_receiver);
        let sse_status = Arc::new(RwLock::new(SseStatus {
            message_id: format!("msg_{}", Uuid::new_v4().simple()),
            model_id: proxy_alias.clone(),
            ..Default::default()
        }));

        let sse_status_clone = sse_status.clone();
        let unified_stream = reassembled_stream
            .then(move |item| {
                let adapter = backend_adapter.clone();
                let status = sse_status_clone.clone();
                async move {
                    match item {
                        // The item from the channel is a Result<Bytes, String>
                        Ok(chunk) => {
                            // Attempt to adapt the complete chunk
                            adapter
                                .adapt_stream_chunk(chunk, status)
                                .await
                                .unwrap_or_else(|e| {
                                    // If adaptation fails, create an error chunk
                                    vec![crate::ccproxy::adapter::unified::UnifiedStreamChunk::Error {
                                        message: e.to_string(),
                                    }]
                                })
                        }
                        Err(e) => {
                            // This is an error from the StreamProcessor itself
                            vec![crate::ccproxy::adapter::unified::UnifiedStreamChunk::Error {
                                message: e.to_string(),
                            }]
                        }
                    }
                }
            })
            .flat_map(iter);

        let sse_stream = unified_stream.flat_map(move |unified_chunk| {
            let events = output_adapter
                .adapt_stream_chunk(unified_chunk, sse_status.clone())
                .unwrap();
            futures_util::stream::iter(
                events
                    .into_iter()
                    .map(|event| Ok::<warp::sse::Event, std::convert::Infallible>(event)),
            )
        });
        let mut response =
            warp::sse::reply(warp::sse::keep_alive().stream(sse_stream)).into_response();
        *response.status_mut() = warp::http::StatusCode::OK;

        Ok(response)
    } else {
        let body_bytes = target_response
            .bytes()
            .await
            .map_err(|e| warp::reject::custom(ProxyAuthError::InternalError(e.to_string())))?;

        let backend_response = BackendResponse { body: body_bytes };
        let unified_response = backend_adapter
            .adapt_response(backend_response)
            .await
            .map_err(|e| warp::reject::custom(ProxyAuthError::InternalError(e.to_string())))?;
        let response = output_adapter
            .adapt_response(unified_response)
            .map_err(|e| warp::reject::custom(ProxyAuthError::InternalError(e.to_string())))?;
        Ok(response.into_response())
    }
}

async fn get_ai_model(
    main_store_arc: Arc<Mutex<MainStore>>,
    proxy_alias: String,
) -> ProxyResult<(AiModel, BackendModelTarget)> {
    ModelResolver::get_ai_model_by_alias(main_store_arc, proxy_alias).await
}
