use crate::{
    ai::interaction::ChatProtocol,
    ccproxy::{
        adapter::{
            backend::BackendAdapter,
            output::OutputAdapter,
            unified::{SseStatus, UnifiedStreamChunk},
        },
        errors::{CCProxyError, ProxyResult},
        StreamFormat, StreamProcessor,
    },
};

use futures_util::{stream::iter, StreamExt};
use std::sync::{Arc, RwLock};
use tokio_stream::wrappers::ReceiverStream;
use warp::http::Response;
use warp::{hyper::Body, reply, Reply};

pub async fn handle_streamed_response(
    chat_protocol: &ChatProtocol,
    target_response: reqwest::Response,
    backend_adapter: Arc<dyn BackendAdapter>,
    output_adapter: impl OutputAdapter + Send + Sync + 'static,
    sse_status: Arc<RwLock<SseStatus>>,
    log_to_file: bool,
) -> ProxyResult<reply::Response> {
    let stream_format = match chat_protocol {
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

    if log_to_file {
        log::info!(target: "ccproxy_logger", "{} Stream Response Chunk Start: \n-----\n", chat_protocol.to_string());
    }

    let sse_status_clone = sse_status.clone();
    let unified_stream = reassembled_stream
        .then(move |item| {
            let adapter = backend_adapter.clone();
            let status = sse_status_clone.clone();
            async move {
                match item {
                    Ok(chunk) => {
                        #[cfg(debug_assertions)]
                        {
                            log::debug!("recv: {}", String::from_utf8_lossy(&chunk));
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

    let byte_stream = unified_stream.flat_map(move |unified_chunk| {
        let events = output_adapter
            .adapt_stream_chunk(unified_chunk, sse_status.clone())
            .unwrap_or_else(|_| {
                log::error!(
                    "adapt_stream_chunk failed unexpectedly despite Infallible error type"
                );
                Vec::new()
            });

        if log_to_file && !events.is_empty() {
            log::info!(target:"ccproxy_logger","{}", events.iter().map(|event| event.to_string()).collect::<Vec<String>>().join(""));
        }
        let bytes: Vec<u8> = events
            .iter()
            .flat_map(|event| event.to_string().into_bytes())
            .collect();
        futures_util::stream::iter(vec![Ok::<_, std::io::Error>(bytes)])
    });

    let body = Body::wrap_stream(byte_stream);
    let response_builder = Response::builder()
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache");

    let mut response = match response_builder.body(body) {
        Ok(res) => res.into_response(),
        Err(e) => {
            return Err(warp::reject::custom(CCProxyError::InternalError(
                e.to_string(),
            )));
        }
    };

    let sse_status_code = if status_code.is_client_error() || status_code.is_server_error() {
        warp::http::StatusCode::from_u16(status_code.as_u16())
            .unwrap_or(warp::http::StatusCode::INTERNAL_SERVER_ERROR)
    } else {
        warp::http::StatusCode::OK
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
            if let (Ok(warp_name), Ok(warp_value)) = (
                warp::http::header::HeaderName::from_bytes(reqwest_name.as_ref()),
                warp::http::header::HeaderValue::from_bytes(reqwest_value.as_ref()),
            ) {
                final_headers.insert(warp_name, warp_value);
            }
        }
    }
    if !final_headers.contains_key(warp::http::header::CONTENT_TYPE) {
        final_headers.insert(
            warp::http::header::CONTENT_TYPE,
            warp::http::header::HeaderValue::from_static("text/event-stream"),
        );
    }
    Ok(response)
}
