use crate::ccproxy::{
    adapter::{
        backend::BackendAdapter,
        output::OutputAdapter,
        unified::{SseStatus, StreamLogRecorder, UnifiedFunctionCallPart, UnifiedStreamChunk},
    },
    diagnostic::{self, RequestContext},
    errors::{CCProxyError, ProxyResult},
    helper::stat_guard::StreamStatGuard,
    ChatProtocol, StreamFormat, StreamProcessor,
};
use crate::error_diagnostics::FailureMessage;

use crate::db::MainStore;
use axum::body::Body;
use axum::response::Response;
use futures_util::{stream::iter, StreamExt};
use http::StatusCode;
use std::sync::{Arc, Mutex, RwLock};
use tokio_stream::wrappers::ReceiverStream;

#[derive(Clone)]
struct StreamLogContext {
    request_id: String,
    client_protocol: String,
    backend_protocol: String,
    client_model: String,
    backend_model: String,
    provider_id: i64,
    provider: String,
    compat: bool,
}

impl StreamLogContext {
    fn request_context(&self) -> RequestContext<'_> {
        RequestContext {
            request_id: &self.request_id,
            client_protocol: &self.client_protocol,
            backend_protocol: &self.backend_protocol,
            client_model: &self.client_model,
            backend_model: &self.backend_model,
            provider_id: self.provider_id,
            provider: &self.provider,
            group: None,
            path: "unified",
            compat: self.compat,
        }
    }
}

pub async fn handle_streamed_response(
    client_headers: &http::HeaderMap,
    backend_protocol: Arc<ChatProtocol>,
    client_protocol: ChatProtocol,
    target_response: reqwest::Response,
    backend_adapter: Arc<dyn BackendAdapter>,
    output_adapter: impl OutputAdapter + Send + Sync + 'static,
    sse_status: Arc<RwLock<SseStatus>>,
    log_client_to_file: bool,
    log_upstream_to_file: bool,
    main_store_arc: Arc<MainStore>,
    client_model: String,
    backend_model: String,
    provider_id: i64,
    provider: String,
    tool_compat_mode: bool,
    pricing: Option<crate::db::PricingConfig>,
) -> ProxyResult<Response> {
    let stream_format = match backend_protocol.as_ref() {
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
    let backend_protocol_for_log = backend_protocol.clone();

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
    let log_context = Arc::new(StreamLogContext {
        request_id: message_id.clone(),
        client_protocol: client_protocol.to_string(),
        backend_protocol: backend_protocol.to_string(),
        client_model: client_model.clone(),
        backend_model: backend_model.clone(),
        provider_id,
        provider: provider.clone(),
        compat: tool_compat_mode,
    });
    let log_recorder = Arc::new(Mutex::new(StreamLogRecorder::new(message_id, model_id)));

    let unified_stream = reassembled_stream
        .then(move |item| {
            let adapter = backend_adapter.clone();
            let status = sse_status_clone.clone();
            let backend_protocol = backend_protocol_for_log.clone();
            let log_context = log_context.clone();
            async move {
                match item {
                    Ok(chunk) => {
                        if log_upstream_to_file {
                            log::info!(
                                target: "ccproxy_upstream_logger",
                                "[Upstream] {} Stream Event: \n{}\n----------------\n",
                                backend_protocol,
                                String::from_utf8_lossy(&chunk)
                            );
                        }
                        let result = adapter
                            .adapt_stream_chunk(chunk, status)
                            .await
                            .unwrap_or_else(|e| {
                                let diagnostic = diagnostic::from_error(
                                    "ccproxy",
                                    "stream_adapt",
                                    e.as_ref(),
                                    diagnostic::request_frames(log_context.request_context()),
                                );
                                diagnostic::emit(log::Level::Error, &diagnostic);
                                vec![
                                    crate::ccproxy::adapter::unified::UnifiedStreamChunk::Error {
                                        message: e.to_string(),
                                    },
                                ]
                            });
                        result
                    }
                    Err(e) => {
                        let failure = FailureMessage::from(e);
                        let diagnostic = diagnostic::from_failure(
                            "ccproxy",
                            "stream_read",
                            &failure,
                            diagnostic::request_frames(log_context.request_context()),
                        );
                        diagnostic::emit(log::Level::Error, &diagnostic);
                        vec![UnifiedStreamChunk::Error {
                            message: failure.as_str().to_string(),
                        }]
                    }
                }
            }
        })
        .flat_map(iter);

    let output_adapter = Arc::new(output_adapter);
    let log_recorder_clone = log_recorder.clone();

    // Create the stat guard from public module
    let stat_guard = Arc::new(
        StreamStatGuard {
            log_recorder: log_recorder.clone(),
            sse_status: sse_status.clone(),
            main_store: main_store_arc.clone(),
            client_model: client_model.clone(),
            backend_model: backend_model.clone(),
            provider_id,
            provider: provider.clone(),
            protocol: client_protocol.to_string(),
            tool_compat_mode,
            workflow_session_id: None,
            workflow_task_run_id: None,
            workflow_segment_id: None,
            root_session_id: None,
            root_task_run_id: None,
            request_kind: None,
            pricing,
        }
        .with_workflow_attribution(client_headers),
    );

    let byte_stream = unified_stream.then(move |unified_chunk| {
        let client_protocol_inner = client_protocol.clone();
        let output_adapter = output_adapter.clone();
        let sse_status = sse_status.clone();
        let inner_log_recorder = log_recorder_clone.clone();
        let _guard = stat_guard.clone();

        async move {
            if let Ok(mut log_recorder) = inner_log_recorder.lock() {
                adapt_stream_chunk_to_log(
                    client_protocol_inner,
                    &unified_chunk,
                    &mut log_recorder,
                    log_client_to_file,
                );
            }

            let events = output_adapter
                .adapt_stream_chunk(unified_chunk, sse_status)
                .unwrap_or_else(|_| {
                    let diagnostic = diagnostic::from_message(
                        "ccproxy",
                        "stream_emit",
                        "output adapter reported an infallible stream failure",
                        [("route", Some("path=unified".to_string()))],
                    );
                    diagnostic::emit(log::Level::Error, &diagnostic);
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
    let filtered_headers =
        crate::ccproxy::utils::http::filter_proxy_headers(&response_headers_from_target);

    for (name, value) in filtered_headers.iter() {
        let name_str = name.as_str().to_lowercase();
        if name_str == "content-type"
            || name_str == "cache-control"
            || name_str == "x-request-id"
            || name_str.starts_with("x-")
            || name_str == "retry-after"
        {
            final_headers.insert(name.clone(), value.clone());
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
    client_protocol: ChatProtocol,
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
                recorder.has_thinking = true;
                if let Some(reasoning) = &mut recorder.thinking {
                    reasoning.push_str(delta);
                } else {
                    recorder.thinking = Some(delta.clone());
                }
            }
        }
        UnifiedStreamChunk::Text { delta } => {
            if !delta.trim().is_empty() {
                recorder.has_content = true;
                recorder.content.push_str(delta);
            }
        }
        UnifiedStreamChunk::ToolUseStart { id, name, .. } => {
            recorder.has_tool_calls = true;
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
        UnifiedStreamChunk::ToolUseDelta { id, delta, .. } => {
            if !delta.is_empty() {
                recorder.has_tool_calls = true;
            }
            if let Some(tool_calls) = &mut recorder.tool_calls {
                if let Some(tool) = tool_calls.get_mut(id) {
                    tool.args.push_str(delta);
                }
            }
        }
        UnifiedStreamChunk::MessageStop { usage, .. } => {
            recorder.input_tokens = Some(usage.input_tokens);
            recorder.output_tokens = Some(usage.output_tokens);
            recorder.cache_tokens = usage
                .prompt_cached_tokens
                .or(usage.cache_read_input_tokens)
                .or(usage.cached_content_tokens);
            recorder.cache_creation_tokens = usage.cache_creation_input_tokens;
            recorder.reasoning_tokens = usage.thoughts_tokens;
            recorder.audio_input_tokens = usage.audio_input_tokens;
            recorder.audio_output_tokens = usage.audio_output_tokens;

            if log_to_file {
                log::info!(target: "ccproxy_client_logger", "[Proxy] {} Stream Response: \n{}\n================\n\n", client_protocol.to_string(), serde_json::to_string_pretty(&recorder).unwrap_or_default());
            }
        }
        UnifiedStreamChunk::Error { message } => {
            recorder.stream_failed = true;
            recorder.stream_status_code = 502;
            recorder.error_message = Some(message.clone());
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::handle_streamed_response;
    use crate::ccproxy::{
        adapter::{backend::OpenAIBackendAdapter, output::OpenAIOutputAdapter, unified::SseStatus},
        ChatProtocol,
    };
    use crate::db::MainStore;
    use std::sync::{Arc, RwLock};
    use tempfile::tempdir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn ccproxy_stream_helper_abort_emits_error_event_and_records_failure() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let address = listener.local_addr().expect("listener should have address");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("upstream should accept");
            let mut request = [0_u8; 8192];
            socket
                .read(&mut request)
                .await
                .expect("upstream should read request");
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nX-Upstream: kept\r\nTransfer-Encoding: chunked\r\n\r\n",
                )
                .await
                .expect("upstream should write headers");
            let event = b"data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"index\":0}]}\n\n";
            socket
                .write_all(format!("{:x}\r\n", event.len()).as_bytes())
                .await
                .expect("upstream should write chunk size");
            socket
                .write_all(event)
                .await
                .expect("upstream should write chunk");
            socket
                .write_all(b"\r\n")
                .await
                .expect("upstream should finish chunk");
            socket.flush().await.expect("upstream should flush");
            // Close without the terminating chunk so the helper observes a stream abort.
        });
        let target_response = reqwest::Client::new()
            .get(format!("http://{address}/stream"))
            .send()
            .await
            .expect("upstream response should be available");

        let directory = tempdir().expect("test directory should exist");
        let store = Arc::new(
            MainStore::new(directory.path().join("stream-helper-abort.db"))
                .expect("test store should initialize"),
        );
        let sse_status = Arc::new(RwLock::new(SseStatus::new(
            "message-id".to_string(),
            "backend-model".to_string(),
            false,
            0.0,
        )));
        let response = handle_streamed_response(
            &http::HeaderMap::new(),
            Arc::new(ChatProtocol::OpenAI),
            ChatProtocol::OpenAI,
            target_response,
            Arc::new(OpenAIBackendAdapter),
            OpenAIOutputAdapter,
            sse_status,
            false,
            false,
            store.clone(),
            "alias".to_string(),
            "backend-model".to_string(),
            1,
            "provider".to_string(),
            false,
            None,
        )
        .await
        .expect("stream helper should return a protocol-safe response");

        assert_eq!(response.status(), http::StatusCode::OK);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "text/event-stream"
        );
        assert_eq!(response.headers().get("x-upstream").unwrap(), "kept");
        assert!(!response.headers().contains_key("content-length"));
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("protocol error event should terminate the body");
        let body = String::from_utf8_lossy(&body);
        assert!(
            body.contains("\"error\""),
            "aborted backend stream must surface a protocol-safe error event"
        );

        let runtime = store.db_runtime().expect("runtime should exist");
        let flush_runtime = runtime.clone();
        tokio::task::spawn_blocking(move || flush_runtime.drain_blocking())
            .await
            .expect("flush task should join")
            .expect("stream failure stat should flush");
        let stat = runtime
            .read(|conn| {
                Ok(conn.query_row(
                    "SELECT status_code, error_message FROM ccproxy_stats",
                    [],
                    |row| Ok((row.get::<_, i32>(0)?, row.get::<_, Option<String>>(1)?)),
                )?)
            })
            .await
            .expect("stream failure stat should persist");
        assert_eq!(stat.0, 502);
        assert!(stat.1.is_some_and(|message| !message.is_empty()));
        server.await.expect("upstream task should join");
    }
}
