use futures_util::{Stream, StreamExt};
use reqwest::Client;
use rust_i18n::t;
use serde_json::Value;
use std::collections::HashMap;
use std::convert::Infallible;
use std::error::Error;
use std::pin::Pin;
use std::str::FromStr as _;
// Added Pin import
use std::sync::{Arc, Mutex};
use uuid::Uuid;
use warp::{http::header::HeaderMap, sse::Event, Reply};

use super::adapter::protocol_adapter::AdaptedRequest;
use super::errors::ProxyResult;
use super::proxy_rotator::PROXY_BACKEND_ROTATOR;
use super::types::{BackendModelTarget, OpenAIListModelsResponse};
use crate::ai::interaction::chat_completion::ChatProtocol;
use crate::ai::interaction::constants::API_KEY_ROTATOR;
use crate::ai::network::{ProxyType, StreamProcessor};
use crate::ai::util::get_proxy_type;
use crate::commands::chat::setup_chat_proxy;
use crate::db::{AiModel, MainStore};
use crate::http::ccproxy::{
    adapter::{
        protocol_adapter::{ProtocolAdapter, RawBackendResponse, RawBackendStreamChunk},
        ClaudeAdapter, GeminiAdapter, OpenAIAdapter,
    },
    errors::ProxyAuthError,
    types::{
        ChatCompletionProxyConfig, ChatCompletionProxyKeysConfig, OpenAIChatCompletionRequest,
        OpenAIModel,
    }, // Renamed to avoid conflict with warp::sse::Event
};

/// Authenticates the request based on the Authorization Bearer token.
/// Reads `chat_completion_proxy_keys` from `MainStore`.
pub async fn authenticate_request(
    headers: HeaderMap,
    main_store: Arc<Mutex<MainStore>>,
) -> ProxyResult<()> {
    let auth_header = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok());

    let token_to_check = match auth_header {
        Some(header_val) if header_val.starts_with("Bearer ") => {
            header_val.trim_start_matches("Bearer ").to_string()
        }
        _ => {
            #[cfg(debug_assertions)]
            log::warn!("Proxy authentication: Missing or invalid Authorization header.");

            return Err(warp::reject::custom(ProxyAuthError::MissingToken));
        }
    };

    let proxy_keys: ChatCompletionProxyKeysConfig = {
        if let Ok(store_guard) = main_store.lock() {
            // Get the raw string value for chat_completion_proxy_keys
            let keys_config: ChatCompletionProxyKeysConfig =
                store_guard.get_config("chat_completion_proxy_keys", vec![]);
            drop(store_guard);

            keys_config
        } else {
            log::error!("{}", t!("db.failed_to_lock_main_store").to_string());
            vec![]
        }
    };

    if proxy_keys.is_empty() {
        log::warn!("Proxy authentication: 'chat_completion_proxy_keys' is configured but the list is empty.");
        return Err(warp::reject::custom(ProxyAuthError::NoKeysConfigured));
    }

    if proxy_keys.iter().any(|k| k.token == token_to_check) {
        #[cfg(debug_assertions)]
        log::debug!("Proxy authentication: Token is valid.");

        Ok(())
    } else {
        #[cfg(debug_assertions)]
        log::debug!(
            "Proxy authentication: Token is invalid. Token: {:?}******",
            token_to_check.get(..5).unwrap_or(&token_to_check)
        );

        Err(warp::reject::custom(ProxyAuthError::InvalidToken))
    }
}

/// Handles the `/v1/models` (list models) request.
/// Reads `chat_completion_proxy` from `MainStore`.
pub async fn list_proxied_models_handler(
    main_store: Arc<Mutex<MainStore>>,
) -> ProxyResult<impl Reply> {
    let config = {
        if let Ok(store_guard) = main_store.lock() {
            // Get the raw string value for chat_completion_proxy
            let cfg: ChatCompletionProxyConfig =
                store_guard.get_config("chat_completion_proxy", HashMap::new());
            drop(store_guard);

            cfg
        } else {
            log::error!("{}", t!("db.failed_to_lock_main_store").to_string());
            HashMap::new()
        }
    };

    #[cfg(debug_assertions)]
    log::debug!(
        "chat_completion_proxy get models: {}",
        serde_json::to_string_pretty(&config).unwrap_or_default()
    );

    let models: Vec<OpenAIModel> = config
        .keys()
        .map(|alias| OpenAIModel {
            id: alias.clone(),
            object: "model".to_string(),
            created: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            owned_by: "chatspeed-proxy".to_string(),
        })
        .collect();

    if models.is_empty() {
        log::info!("'chat_completion_proxy' configuration is empty or not found. Returning empty model list.");
    };

    let response = OpenAIListModelsResponse {
        object: "list".to_string(),
        data: models,
    };

    Ok(warp::reply::json(&response))
}

// =================================================
//  chat completion proxy
// =================================================

/// Get aid model detail from the database.
async fn get_ai_model(
    main_store_arc: Arc<Mutex<MainStore>>,
    proxy_alias: String,
) -> ProxyResult<(AiModel, BackendModelTarget)> {
    // 1. Get Proxy Configuration
    let backend_targets = {
        let store_guard = main_store_arc.lock().map_err(|e| {
            let err_msg = t!("db.failed_to_lock_main_store", error = e.to_string()).to_string();
            log::error!("Failed to lock MainStore: {}", &err_msg);

            warp::reject::custom(ProxyAuthError::StoreLockError(err_msg))
        })?;
        let proxy_config: ChatCompletionProxyConfig =
            store_guard.get_config("chat_completion_proxy", std::collections::HashMap::new());

        proxy_config.get(&proxy_alias).cloned().ok_or_else(|| {
            log::warn!("Proxy alias '{}' not found in configuration.", proxy_alias);
            warp::reject::custom(ProxyAuthError::ModelAliasNotFound(proxy_alias.clone()))
        })?
    };

    if backend_targets.is_empty() {
        log::warn!(
            "Proxy alias '{}' has no backend targets configured.",
            proxy_alias
        );
        return Err(warp::reject::custom(ProxyAuthError::NoBackendTargets(
            proxy_alias,
        )));
    }

    #[cfg(debug_assertions)]
    log::debug!(
        "chat_completion_proxy get backend_targets: {}",
        serde_json::to_string_pretty(&backend_targets).unwrap_or_default()
    );

    // 2. Select a backend using round-robin with the global rotator
    let selected_target = rotator_models(&proxy_alias, backend_targets).await;

    #[cfg(debug_assertions)]
    log::debug!(
        "chat_completion_proxy get selected_target: {}",
        serde_json::to_string_pretty(&selected_target).unwrap_or_default()
    );

    // 3. Get AI Model details for the selected target's provider_id
    let ai_model_details = {
        let store_guard = main_store_arc.lock().map_err(|e| {
            let err_msg = t!("db.failed_to_lock_main_store", error = e.to_string()).to_string();
            warp::reject::custom(ProxyAuthError::StoreLockError(err_msg))
        })?;
        // Assuming get_ai_model_by_id takes i64. Adjust if BackendModelTarget.id is String.
        // If BackendModelTarget.id is String, parse it to i64 or change get_ai_model_by_id.
        store_guard
            .config
            .get_ai_model_by_id(selected_target.id)
            .map_err(|db_err| {
                log::error!(
                    "Failed to get AI model by id '{}': {}",
                    selected_target.id,
                    db_err
                );
                warp::reject::custom(ProxyAuthError::ModelDetailsFetchError(format!(
                    "{}",
                    selected_target.id
                )))
            })?
    };

    log::info!(
        "chat_completion_proxy: alias={}, provider={}, base_url={}, protocol={}",
        &proxy_alias,
        &ai_model_details.name,
        &ai_model_details.base_url,
        &ai_model_details.api_protocol
    );

    if ai_model_details.base_url.is_empty() {
        // This case should ideally be prevented by configuration validation
        log::error!(
            "Target AI provider base_url is empty for alias '{}'",
            proxy_alias
        );
        return Err(warp::reject::custom(ProxyAuthError::InternalError(
            "Target base URL is empty".to_string(),
        )));
    }

    Ok((ai_model_details, selected_target))
}

async fn rotator_models(
    proxy_alias: &str,
    backend_targets: Vec<BackendModelTarget>,
) -> BackendModelTarget {
    let selected_index = PROXY_BACKEND_ROTATOR
        .get_next_index(proxy_alias, backend_targets.len())
        .await;
    backend_targets[selected_index].clone()
}

async fn rotator_keys(base_url: &str, target_api_key: String) -> ProxyResult<String> {
    if target_api_key.contains('\n') {
        let keys = target_api_key
            .split('\n')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<Vec<String>>();
        Ok(API_KEY_ROTATOR
            .get_next_key(&base_url, keys)
            .await
            .unwrap_or_default())
    } else {
        Ok(target_api_key)
    }
}

async fn build_chat_completion_client(
    main_store_arc: Arc<Mutex<MainStore>>,
    adapted_request_parts: AdaptedRequest,
    target_api_key: &str,
    actual_model_name: &str,
    proxy_alias: &str,
    mut metadata: Option<Value>,
    is_streaming_request: bool,
    original_client_headers: HeaderMap,
    adapter: Arc<dyn ProtocolAdapter>,
) -> ProxyResult<reqwest::RequestBuilder> {
    let mut client_builder = Client::builder();

    // Set proxy if configured
    {
        let _ = setup_chat_proxy(&main_store_arc, &mut metadata);
        let proxy_type = get_proxy_type(metadata);
        match proxy_type {
            ProxyType::None => {
                client_builder = client_builder.no_proxy();
            }
            ProxyType::Http(proxy_url, proxy_username, proxy_password) => {
                log::info!(
                    "chat_completion_proxy: proxy_type=http, proxy_url={}, proxy_username={}",
                    &proxy_url,
                    proxy_username.as_deref().unwrap_or_default()
                );

                if let Ok(mut proxy) = reqwest::Proxy::all(proxy_url) {
                    let username = proxy_username.as_deref().unwrap_or_default();
                    let password = proxy_password.as_deref().unwrap_or_default();
                    if !username.is_empty() && !password.is_empty() {
                        proxy = proxy.basic_auth(username, password);
                    }
                    client_builder = client_builder.proxy(proxy);
                }
            }
            ProxyType::System => {}
        }
    }

    // It's generally better to reuse a reqwest::Client if this handler is called frequently.
    // For simplicity here, we create a new one each time.
    // Consider creating it once and passing it via `with_main_store` or similar if performance becomes an issue.
    let http_client = client_builder.build().map_err(|e| {
        let err = t!("network.client_build_error", error = e.to_string()).to_string();
        log::error!("{}", &err);
        warp::reject::custom(ProxyAuthError::InternalError(err))
    })?;

    let mut onward_request_builder = http_client.post(&adapted_request_parts.url);

    // Apply adapter-specific modifications (like body, headers)
    onward_request_builder = adapter
        .modify_request_builder(onward_request_builder, &adapted_request_parts)
        .map_err(|e| {
            log::error!("Adapter failed to modify request builder: {:?}", e);
            warp::reject::custom(e) // Assuming ProxyAuthError can be converted from this error or is this error
        })?;

    // Set Authorization header for the target provider
    if target_api_key.is_empty() {
        log::warn!("No API key configured for target model '{}' (alias '{}'). Proceeding without Bearer token.", actual_model_name, proxy_alias);
    }

    if is_streaming_request {
        onward_request_builder =
            onward_request_builder.header(reqwest::header::ACCEPT, "text/event-stream");
    } else {
        onward_request_builder =
            onward_request_builder.header(reqwest::header::ACCEPT, "application/json");
    }
    // Selectively forward relevant headers from the original client request
    for (name, value) in original_client_headers.iter() {
        let name_str = name.as_str().to_lowercase();
        // Forward common non-sensitive headers. Avoid forwarding "host", "authorization", "content-length", etc.
        // Adjust this list based on what you want to pass through.
        if name_str == "x-request-id"
            || name_str == "user-agent"
            || name_str == "accept-language"
            || name_str == "x-title"
            || name_str == "http-referer"
            // Add other custom headers you want to forward, e.g., "x-custom-tenant-id"
            || name_str.starts_with("x-custom-")
        {
            #[cfg(debug_assertions)]
            log::debug!(
                "Forwarding header: {}={}",
                name_str,
                value.to_str().unwrap_or_default()
            );

            // Convert warp::http::HeaderName to reqwest::header::HeaderName
            // and warp::http::HeaderValue to reqwest::header::HeaderValue
            if let Ok(reqwest_name) = reqwest::header::HeaderName::from_bytes(name.as_ref()) {
                if let Ok(reqwest_value) = reqwest::header::HeaderValue::from_bytes(value.as_ref())
                {
                    onward_request_builder =
                        onward_request_builder.header(reqwest_name, reqwest_value);
                } else {
                    log::warn!("Failed to convert header value for: {}", name_str);
                }
            } else {
                log::warn!("Failed to convert header name: {}", name_str);
            }
        } else {
            #[cfg(debug_assertions)]
            log::debug!("Ignore client header: {}", name_str,);
        }
    }

    Ok(onward_request_builder)
}

/// Handles the `/v1/chat/completions` proxy request.
pub async fn handle_chat_completion_proxy(
    original_client_headers: HeaderMap, // Added to receive original headers
    mut client_request_payload: OpenAIChatCompletionRequest, // Renamed for clarity
    main_store_arc: Arc<Mutex<MainStore>>,
) -> ProxyResult<impl Reply> {
    let proxy_alias = client_request_payload.model.clone();

    let (ai_model_details, selected_target) =
        get_ai_model(main_store_arc.clone(), proxy_alias.clone()).await?;

    // rotator the keys
    let target_api_key =
        rotator_keys(&ai_model_details.base_url, ai_model_details.api_key.clone()).await?;
    let actual_model_name = selected_target.model.clone();

    // Select Protocol Adapter based on ai_model_details.api_protocol
    // Convert the string protocol from AiModel to the ChatProtocol enum
    let backend_chat_protocol =
        ChatProtocol::from_str(&ai_model_details.api_protocol).map_err(|parse_err| {
            // Renamed _parse_err to parse_err for clarity if used in log
            log::error!(
                "Invalid or unsupported API protocol string ('{}') in AiModel configuration: {}",
                ai_model_details.api_protocol,
                parse_err // Log the parsing error
            );
            // The closure for map_err should return the error type itself, not a Result.
            // The `?` operator will then wrap this into a Rejection.
            warp::reject::custom(ProxyAuthError::InvalidProtocolError(
                ai_model_details.api_protocol.clone(),
            ))
        })?;

    let adapter: Arc<dyn ProtocolAdapter> = match backend_chat_protocol {
        ChatProtocol::OpenAI => Arc::new(OpenAIAdapter),
        ChatProtocol::Claude => {
            log::debug!(
                "Using ClaudeAdapter for protocol: {}",
                ai_model_details.api_protocol
            );
            Arc::new(ClaudeAdapter)
        }
        ChatProtocol::Gemini => {
            // Assuming GeminiAdapter is now available
            log::debug!(
                "Using GeminiAdapter for protocol: {}",
                ai_model_details.api_protocol
            );
            Arc::new(GeminiAdapter)
        }
        ChatProtocol::Ollama | ChatProtocol::HuggingFace => {
            // Grouping similar placeholders
            // TODO: Replace with actual adapters when implemented
            Arc::new(OpenAIAdapter) // Placeholder
        }
    };
    // Modify the incoming request to use the actual model name for the target provider
    client_request_payload.model = actual_model_name.clone();

    // Serialize the (potentially modified) incoming request to be sent to the target
    // Now, use the adapter to get the adapted request parts
    let adapted_request_parts = match adapter.adapt_request(
        &ai_model_details.base_url,
        &actual_model_name,
        &target_api_key, // Pass the API key to the adapter
        &client_request_payload,
    ) {
        // Ok(parts: AdaptedRequest) => parts, // Type annotation for clarity
        Ok(parts) => parts,
        Err(e) => {
            log::error!("Failed to serialize onward request body: {:?}", e);
            return Err(warp::reject::custom(ProxyAuthError::InternalError(
                "Failed to serialize request".to_string(),
            )));
        }
    };
    #[cfg(debug_assertions)]
    {
        log::debug!("proxy request url: {}", &adapted_request_parts.url);
        for (header, value) in adapted_request_parts.headers_to_add.iter() {
            let v = if header.to_lowercase() == "authorization" {
                "<redacted>"
            } else {
                value
            };
            log::debug!("proxy request headers: {}={}", header, v);
        }
        log::debug!("proxy request body: {:?}", &adapted_request_parts.body)
    }

    let stream_requested = client_request_payload.stream.unwrap_or(false);
    let onward_request_builder = build_chat_completion_client(
        main_store_arc.clone(),
        adapted_request_parts,
        &target_api_key,
        &actual_model_name,
        &proxy_alias,
        ai_model_details.metadata.clone(),
        stream_requested,
        original_client_headers,
        adapter.clone(),
    )
    .await?;

    let target_response = match onward_request_builder.send().await {
        Ok(res) => res,
        Err(e) => {
            log::error!(
                "Failed to send request to target AI provider. Error: {}, Source: {:?}",
                e,
                e.source()
            );
            return Err(warp::reject::custom(ProxyAuthError::InternalError(
                format!("Request to target failed: {}", e),
            )));
        }
    };

    // --- BEGIN: Immediate Error Handling for Target Response ---
    if !target_response.status().is_success() {
        let status_code = target_response.status();
        let headers_from_target = target_response.headers().clone();
        let error_body_bytes = match target_response.bytes().await {
            Ok(bytes) => bytes,
            Err(e) => {
                log::error!(
                    "Failed to read error response body from target AI provider (alias: {}): {}",
                    proxy_alias,
                    e
                );
                let err_msg = t!("network.response_read_error", error = e.to_string()).to_string();
                return Err(warp::reject::custom(ProxyAuthError::InternalError(err_msg)));
            }
        };
        let error_body_str = String::from_utf8_lossy(&error_body_bytes);

        log::warn!(
            "Proxy target (alias: '{}', model: '{}', provider: '{}') returned an error. Status: {}, Body: {}",
            proxy_alias,
            actual_model_name,
            ai_model_details.name,
            status_code,
            error_body_str
        );

        let final_error_json: Value;
        let mut error_type_str = "upstream_api_error".to_string();

        // Try to parse the error as OpenAI-compatible JSON error, otherwise use raw string.
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
                error_body_str.to_string() // Not an OpenAI error structure, but still JSON
            }
        } else {
            error_body_str.to_string() // Not JSON
        };
        final_error_json = serde_json::json!({
            "message": message_content,
            "type": error_type_str,
            "param": null,
            "code": status_code.as_u16().to_string() // Keep as string for OpenAI compatibility
        });

        let warp_status_code = warp::http::StatusCode::from_u16(status_code.as_u16())
            .unwrap_or(warp::http::StatusCode::INTERNAL_SERVER_ERROR);

        let mut response =
            warp::reply::json(&serde_json::json!({ "error": final_error_json })).into_response();
        *response.status_mut() = warp_status_code;

        // Forward relevant headers from the target's error response
        let final_headers = response.headers_mut();
        for (reqwest_name, reqwest_value) in headers_from_target.iter() {
            let name_str = reqwest_name.as_str().to_lowercase();
            // Forward common error-related headers or rate-limiting headers
            if name_str.starts_with("x-ratelimit-")
                || name_str == "retry-after"
                || name_str == "content-type"
            // Forward content-type if set by upstream error
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
        // Ensure Content-Type is application/json if not already set by target for the error
        if !final_headers.contains_key(warp::http::header::CONTENT_TYPE) {
            final_headers.insert(
                warp::http::header::CONTENT_TYPE,
                warp::http::header::HeaderValue::from_static("application/json"),
            );
        }
        return Ok(response);
    }
    // --- END: Immediate Error Handling for Target Response ---

    if stream_requested {
        // Extract status and headers BEFORE `target_response` is consumed by StreamProcessor
        let status_code = target_response.status();
        let response_headers_from_target = target_response.headers().clone();

        let stream_id = format!("ccp-{}", Uuid::new_v4());
        let next_tool_call_stream_index_arc = Arc::new(Mutex::new(0u32));

        // Determine the stream format for the StreamProcessor based on the backend protocol
        let stream_format_for_processor = match backend_chat_protocol {
            ChatProtocol::Gemini => crate::ai::network::StreamFormat::Gemini,
            ChatProtocol::Claude => crate::ai::network::StreamFormat::Claude,
            _ => crate::ai::network::StreamFormat::OpenAI, // Default to OpenAI SSE format
        };

        // Create a StreamProcessor instance to handle reassembly of backend chunks
        let stream_processor = StreamProcessor::new();
        let backend_event_receiver = stream_processor
            .process_stream(target_response, &stream_format_for_processor) // target_response is consumed here
            .await;

        let processed_backend_stream =
            tokio_stream::wrappers::ReceiverStream::new(backend_event_receiver);

        let sse_stream = processed_backend_stream.filter_map({
            let adapter_ref_for_filter_map = Arc::clone(&adapter);
            let stream_id_ref_for_filter_map = stream_id.clone();
            let model_name_ref_for_filter_map = actual_model_name.clone();

            move |result_from_processor| { // Renamed for clarity: this comes from StreamProcessor
                let adapter_for_async_block = Arc::clone(&adapter_ref_for_filter_map);
                let stream_id_for_async_block = stream_id_ref_for_filter_map.clone();
                let model_name_for_async_block = model_name_ref_for_filter_map.clone();
                let index_arc_for_async_block = Arc::clone(&next_tool_call_stream_index_arc);

                async move {
                    match result_from_processor { // This is Result<Bytes, String>
                        Ok(complete_backend_chunk_data) => { // This is Bytes (a complete event)
                            let mut index_guard =
                                match index_arc_for_async_block.lock() {
                                    Ok(guard) => guard,
                                    Err(poison_error) => {
                                        log::error!(
                                        "Mutex for next_tool_call_stream_index is poisoned: {}. Skipping chunk.",
                                        poison_error
                                    );
                                        return None;
                                    }
                                };
                            match adapter_for_async_block.adapt_stream_chunk(
                                RawBackendStreamChunk { data: complete_backend_chunk_data },
                                &stream_id_for_async_block,
                                &model_name_for_async_block,
                                &mut *index_guard, // Pass &mut u32 from the MutexGuard
                            ) {
                                Ok(Some(parsed_sse_event)) => {
                                    let mut warp_event = Event::default();
                                    if let Some(id) = parsed_sse_event.id {
                                        warp_event = warp_event.id(id);
                                    }
                                    if let Some(event_type) = parsed_sse_event.event_type {
                                        warp_event = warp_event.event(event_type);
                                    }
                                    if let Some(data) = parsed_sse_event.data {
                                        warp_event = warp_event.data(data); // warp adds "data: " prefix
                                    }
                                    if let Some(retry_str) = parsed_sse_event.retry {
                                        if let Ok(retry_ms) = retry_str.parse::<u64>() {
                                            warp_event = warp_event.retry(std::time::Duration::from_millis(retry_ms));
                                        } else {
                                            log::warn!("Failed to parse retry value: {}", retry_str);
                                        }
                                    }
                                    Some(Ok::<_, std::convert::Infallible>(warp_event))
                                }
                                Ok(None) => None, // Adapter indicated no event to send for this chunk
                                Err(e) => {
                                    log::error!("Error adapting stream chunk: {:?}", e);
                                    None // Or send an error event to client if desired
                                }
                            }
                        }
                        Err(e) => {
                            log::error!("Error from StreamProcessor (likely backend stream error or parsing issue): {}, dropping event.", e);
                            None
                        }
                    }
                }
            }
        });

        // If the adapter specifies a stream end signal, append it.
        let final_sse_stream: Pin<
            Box<dyn Stream<Item = Result<Event, Infallible>> + Send + Sync + 'static>,
        > = if let Some(end_signal_content) = adapter.adapt_stream_end() {
            // Assuming adapt_stream_end() returns the content for the data field,
            // like "[DONE]" for OpenAI.
            // If adapt_stream_end() were to return a full SseEvent, the logic here would be similar
            // to the main filter_map block.
            let end_event = futures_util::stream::once(async move {
                // For [DONE] signal, it's typically just a data event.
                // Event::default().data("[DONE]")
                Ok::<_, Infallible>(Event::default().data(end_signal_content))
            });
            Box::pin(sse_stream.chain(end_event))
        } else {
            Box::pin(sse_stream)
        };

        // Construct the SSE response for Warp
        // Start with the status code from the target if it's an error, otherwise default to 200 OK for SSE.
        let sse_status_code = if status_code.is_client_error() || status_code.is_server_error() {
            // Convert reqwest::StatusCode to warp::http::StatusCode
            warp::http::StatusCode::from_u16(status_code.as_u16())
                .unwrap_or(warp::http::StatusCode::INTERNAL_SERVER_ERROR)
        } else {
            warp::http::StatusCode::OK
        };

        let sse_reply_inner = warp::sse::reply(warp::sse::keep_alive().stream(final_sse_stream));
        let mut response =
            warp::reply::with_status(sse_reply_inner, sse_status_code).into_response();

        // Forward relevant headers from the target's response
        let final_headers = response.headers_mut();
        for (reqwest_name, reqwest_value) in response_headers_from_target.iter() {
            // Only forward safe and relevant headers.
            // `Content-Type` is crucial for SSE.
            let name_str = reqwest_name.as_str().to_lowercase();
            if name_str == "content-type" // reqwest::header::CONTENT_TYPE.as_str()
                || name_str == "cache-control"
                || name_str == "x-request-id"
                // Add other relevant headers like X-RateLimit-*
                || name_str.starts_with("x-ratelimit-")
            {
                // Convert reqwest headers to warp headers
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
        // Ensure Content-Type is text/event-stream if not already set by target
        if !response_headers_from_target.contains_key(reqwest::header::CONTENT_TYPE) {
            final_headers.insert(
                warp::http::header::CONTENT_TYPE,
                warp::http::header::HeaderValue::from_static("text/event-stream"), // charset=utf-8 is often default or implied
            );
        }
        Ok(response)
    } else {
        // Non-streaming: forward status, headers, and body
        let status_code = target_response.status();
        let headers_from_target = target_response.headers().clone();
        let body_bytes = match target_response.bytes().await {
            Ok(bytes) => bytes,
            Err(e) => {
                log::error!(
                    "Failed to read non-streaming response body from target: {}",
                    e
                );
                return Err(warp::reject::custom(ProxyAuthError::InternalError(
                    format!("Failed to read response body: {}", e),
                )));
            }
        };

        // Adapt the response body
        let adapted_response_body_result = adapter.adapt_response_body(
            RawBackendResponse {
                status_code, // reqwest::StatusCode
                headers: headers_from_target.clone(),
                body_bytes,
            },
            &actual_model_name, // Pass the target_model_name
        );

        let adapted_response_body_bytes = match adapted_response_body_result {
            Ok(adapted_resp) => serde_json::to_vec(&adapted_resp).map_err(|e| {
                log::error!("Failed to serialize adapted OpenAI response: {}", e);
                ProxyAuthError::InternalError("Failed to serialize adapted response".to_string())
            })?,
            Err(e) => {
                log::error!("Failed to adapt response body: {:?}", e);
                return Err(warp::reject::custom(e));
            }
        };

        let warp_status_code = warp::http::StatusCode::from_u16(status_code.as_u16())
            .unwrap_or(warp::http::StatusCode::INTERNAL_SERVER_ERROR);
        let mut response_builder = warp::http::Response::builder().status(warp_status_code);
        if let Some(headers_map) = response_builder.headers_mut() {
            for (reqwest_name, reqwest_value) in headers_from_target.iter() {
                // Forward relevant headers
                let name_str = reqwest_name.as_str().to_lowercase();
                if name_str == "content-type"
                    || name_str == "content-length"
                    || name_str == "x-request-id"
                    || name_str.starts_with("x-ratelimit-")
                    || name_str == "cache-control"
                    || name_str == "expires"
                    || name_str == "date"
                    || name_str == "etag"
                {
                    if let Ok(warp_name) =
                        warp::http::header::HeaderName::from_bytes(reqwest_name.as_ref())
                    {
                        if let Ok(warp_value) =
                            warp::http::header::HeaderValue::from_bytes(reqwest_value.as_ref())
                        {
                            headers_map.insert(warp_name, warp_value);
                        }
                    }
                }
            }
            // Ensure Content-Type is application/json if not already set by target and status is OK-ish
            // The adapter is responsible for the final Content-Type of the *adapted* response.
            // For OpenAI compatible non-streaming, it should always be application/json.
            if !headers_map.contains_key(warp::http::header::CONTENT_TYPE) // Check if already set by forwarded headers
                && status_code.is_success()
            {
                headers_map.insert(
                    warp::http::header::CONTENT_TYPE,
                    warp::http::header::HeaderValue::from_static("application/json"),
                );
            }
        }
        // The main attempt to build the response
        match response_builder.body(adapted_response_body_bytes.into()) {
            Ok(response) => Ok(response),
            Err(e) => {
                log::error!(
                    "Failed to build primary proxy response with adapted body: {}",
                    e
                );
                // Fallback response
                match warp::http::Response::builder()
                    .status(warp::http::StatusCode::INTERNAL_SERVER_ERROR)
                    .header(
                        warp::http::header::CONTENT_TYPE,
                        "text/plain; charset=utf-8",
                    )
                    .body(bytes::Bytes::from_static(b"Internal Server Error").into())
                {
                    Ok(fallback_response) => Ok(fallback_response),
                    Err(fallback_e) => {
                        log::error!(
                            "CRITICAL: Failed to build even the fallback HTTP response: {}",
                            fallback_e
                        );
                        // For maximum robustness, construct the simplest possible response.
                        // This is the ultimate fallback if the previous one (with headers) failed.
                        match warp::http::Response::builder()
                            .status(warp::http::StatusCode::INTERNAL_SERVER_ERROR)
                            .body(bytes::Bytes::from_static(b"Internal Server Error").into())
                        {
                            Ok(ultimate_fallback_response) => Ok(ultimate_fallback_response),
                            Err(ultimate_fallback_error) => {
                                // If even building a Response with a static body and status fails,
                                // something is fundamentally wrong with the http crate or environment.
                                // Return the simplest possible warp::Reply.
                                log::error!(
                                        "CRITICAL: Failed to build the ultimate fallback (http::Response::Builder) HTTP response: {}. Returning simplest warp reply.",
                                        ultimate_fallback_error
                                    );
                                // Convert WithStatus to a Response to match other Ok arms
                                let ultimate_reply = warp::reply::with_status(
                                    "Internal Server Error",
                                    warp::http::StatusCode::INTERNAL_SERVER_ERROR,
                                );
                                Ok(ultimate_reply.into_response())
                            }
                        }
                    }
                }
            }
        }
    }
}
