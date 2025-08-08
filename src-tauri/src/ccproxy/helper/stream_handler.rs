use crate::{
    ai::interaction::ChatProtocol,
    ccproxy::{
        adapter::{
            backend::BackendAdapter,
            output::OutputAdapter,
            unified::{SseStatus, StreamLogRecorder, UnifiedFunctionCallPart, UnifiedStreamChunk},
        },
        errors::{CCProxyError, ProxyResult},
        StreamFormat, StreamProcessor,
    },
};

use axum::body::Body;
use axum::response::Response;
use futures_util::{stream::iter, StreamExt};
use http::StatusCode;
use std::sync::{Arc, Mutex, RwLock};
use tokio_stream::wrappers::ReceiverStream;

pub async fn handle_streamed_response(
    chat_protocol: Arc<ChatProtocol>,
    target_response: reqwest::Response,
    backend_adapter: Arc<dyn BackendAdapter>,
    output_adapter: impl OutputAdapter + Send + Sync + 'static,
    sse_status: Arc<RwLock<SseStatus>>,
    log_to_file: bool,
) -> ProxyResult<Response> {
    let stream_format = match chat_protocol.as_ref() {
        ChatProtocol::Gemini => StreamFormat::Gemini,
        ChatProtocol::Claude => StreamFormat::Claude,
        _ => StreamFormat::OpenAI,
    };

    let status_code = target_response.status();
    let response_headers_from_target = target_response.headers().clone();

    let reassembled_receiver = StreamProcessor::new()
        .process_stream(target_response, &stream_format)
        .await;

    let reassembled_stream = ReceiverStream::new(reassembled_receiver);

    let chat_protocol_str = chat_protocol.to_string();
    let sse_status_clone = sse_status.clone();
    let message_id = if let Ok(state) = sse_status.read() {
        state.message_id.clone()
    } else {
        "".to_string()
    };
    let model_id = if let Ok(state) = sse_status.read() {
        state.model_id.clone()
    } else {
        "".to_string()
    };
    let log_recorder = Arc::new(Mutex::new(StreamLogRecorder::new(message_id, model_id)));

    let unified_stream = reassembled_stream
        .then(move |item| {
            let adapter = backend_adapter.clone();
            let status = sse_status_clone.clone();
            let chat_protocol_str_clone = chat_protocol_str.clone();
            async move {
                match item {
                    Ok(chunk) => {
                        #[cfg(debug_assertions)]
                        {
                            log::debug!(
                                "{} recv: {}",
                                chat_protocol_str_clone,
                                String::from_utf8_lossy(&chunk)
                            );
                        }
                        // Attempt to adapt the complete chunk
                        let result = adapter
                            .adapt_stream_chunk(chunk, status)
                            .await
                            .unwrap_or_else(|e| {
                                log::error!("handler: adapt_stream_chunk failed: {}", e);
                                // If adaptation fails, create an error chunk
                                vec![
                                    crate::ccproxy::adapter::unified::UnifiedStreamChunk::Error {
                                        message: e.to_string(),
                                    },
                                ]
                            });
                        result
                    }
                    Err(e) => {
                        log::error!("error on backend stream: {}", e.to_string());
                        vec![UnifiedStreamChunk::Error {
                            message: e.to_string(),
                        }]
                    }
                }
            }
        })
        .flat_map(iter);

    let chat_protocol_outer = chat_protocol.clone();
    let output_adapter = Arc::new(output_adapter);
    let log_recorder_clone = log_recorder.clone();
    let byte_stream = unified_stream.then(move |unified_chunk| {
        let chat_protocol_inner = chat_protocol_outer.clone();
        let output_adapter = output_adapter.clone();
        let sse_status = sse_status.clone();
        let inner_log_recorder = log_recorder_clone.clone();
        async move {
            if let Ok(mut log_recorder) = inner_log_recorder.lock() {
                adapt_stream_chunk_to_log(
                    chat_protocol_inner,
                    &unified_chunk,
                    &mut log_recorder,
                    log_to_file,
                );
            }

            let events = output_adapter
                .adapt_stream_chunk(unified_chunk, sse_status)
                .unwrap_or_else(|_| {
                    log::error!(
                        "adapt_stream_chunk failed unexpectedly despite Infallible error type"
                    );
                    Vec::new()
                });

            let bytes: Vec<u8> = events
                .iter()
                .flat_map(|event| event.to_string().into_bytes())
                .collect();
            Ok::<_, std::io::Error>(bytes)
        }
    });

    let body = Body::from_stream(byte_stream);
    let response_builder = Response::builder()
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache");

    let mut response = match response_builder.body(body) {
        Ok(res) => res,
        Err(e) => {
            return Err(CCProxyError::InternalError(e.to_string()));
        }
    };

    let sse_status_code = if status_code.is_client_error() || status_code.is_server_error() {
        StatusCode::from_u16(status_code.as_u16()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
    } else {
        StatusCode::OK
    };
    *response.status_mut() = sse_status_code;

    let final_headers = response.headers_mut();
    for (reqwest_name, reqwest_value) in response_headers_from_target.iter() {
        let name_str = reqwest_name.as_str().to_lowercase();
        if name_str == "content-type"
            || name_str == "cache-control"
            || name_str == "x-request-id"
            || name_str.starts_with("x-ratelimit-")
            || name_str == "retry-after"
        {
            if let (Ok(axum_name), Ok(axum_value)) = (
                http::header::HeaderName::from_bytes(reqwest_name.as_ref()),
                http::header::HeaderValue::from_bytes(reqwest_value.as_ref()),
            ) {
                final_headers.insert(axum_name, axum_value);
            }
        }
    }
    if !final_headers.contains_key(http::header::CONTENT_TYPE) {
        final_headers.insert(
            http::header::CONTENT_TYPE,
            http::header::HeaderValue::from_static("text/event-stream"),
        );
    }

    Ok(response)
}

pub fn adapt_stream_chunk_to_log(
    chat_protocol: Arc<ChatProtocol>,
    chunk: &UnifiedStreamChunk,
    recorder: &mut StreamLogRecorder,
    log_to_file: bool,
) {
    match chunk {
        UnifiedStreamChunk::MessageStart { id, .. } => {
            recorder.chat_id = id.clone();
        }
        UnifiedStreamChunk::Thinking { delta } => {
            if !delta.trim().is_empty() {
                if let Some(reasoning) = &mut recorder.thinking {
                    reasoning.push_str(delta);
                } else {
                    recorder.thinking = Some(delta.clone());
                }
            }
        }
        UnifiedStreamChunk::Text { delta } => {
            if !delta.trim().is_empty() {
                recorder.content.push_str(delta);
            }
        }
        UnifiedStreamChunk::ToolUseStart { id, name, .. } => {
            if recorder.tool_calls.is_none() {
                recorder.tool_calls = Some(Default::default());
            }
            if let Some(tool_calls) = &mut recorder.tool_calls {
                tool_calls.insert(
                    id.clone(),
                    UnifiedFunctionCallPart {
                        name: name.clone(),
                        args: "".to_string(),
                    },
                );
            }
        }
        UnifiedStreamChunk::ToolUseDelta { id, delta } => {
            if let Some(tool_calls) = &mut recorder.tool_calls {
                if let Some(tool) = tool_calls.get_mut(id) {
                    tool.args.push_str(delta);
                }
            }
        }
        UnifiedStreamChunk::MessageStop { usage, .. } => {
            recorder.input_tokens = Some(usage.input_tokens);
            recorder.output_tokens = Some(usage.output_tokens);

            if log_to_file {
                log::info!(target: "ccproxy_logger", "[Proxy] {} Stream Response: \n{}\n================\n\n", chat_protocol, serde_json::to_string_pretty(&recorder).unwrap_or_default());
            }
        }
        _ => {}
    }
}
