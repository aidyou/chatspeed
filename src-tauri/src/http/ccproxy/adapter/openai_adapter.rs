use async_trait::async_trait;
use bytes::Bytes;

use crate::http::ccproxy::{
    errors::ProxyAuthError,
    types::{OpenAIChatCompletionRequest, OpenAIChatCompletionResponse, SseEvent},
};

use super::protocol_adapter::{
    AdaptedRequest, ProtocolAdapter, RawBackendResponse, RawBackendStreamChunk,
};

pub struct OpenAIAdapter;

#[async_trait]
impl ProtocolAdapter for OpenAIAdapter {
    fn adapt_request(
        &self,
        base_url: &str,
        target_model_name: &str,
        target_api_key: &str,
        openai_request: &OpenAIChatCompletionRequest,
    ) -> Result<AdaptedRequest, ProxyAuthError> {
        let mut request = openai_request.clone();
        request.top_k = None;
        let body_bytes = serde_json::to_vec(&request).map_err(|e| {
            log::error!("OpenAIAdapter: Failed to serialize request: {}", e);
            ProxyAuthError::InternalError("Failed to serialize OpenAI request".to_string())
        })?;
        let url = if base_url.contains("router.huggingface.co") {
            base_url
                .split_once("/hf-inference/models")
                .map(|(base, _)| format!("{base}/hf-inference/models/{target_model_name}/v1"))
                .unwrap_or_else(|| {
                    format!(
                        "https://router.huggingface.co/hf-inference/models/{target_model_name}/v1"
                    )
                })
        } else {
            format!("{base_url}/chat/completions")
        };
        if base_url.contains("router.huggingface.co") && request.top_p.unwrap_or_default() >= 1.0 {
            request.top_p = Some(0.99);
        }

        Ok(AdaptedRequest {
            url,
            headers_to_add: {
                let mut headers = vec![(
                    reqwest::header::CONTENT_TYPE.as_str().to_string(),
                    "application/json".to_string(),
                )];
                if !target_api_key.is_empty() {
                    headers.push((
                        "Authorization".to_string(),
                        format!("Bearer {}", target_api_key),
                    ));
                }
                headers
            },
            body: Bytes::from(body_bytes),
        })
    }

    fn adapt_response_body(
        &self,
        raw_response: RawBackendResponse,
        _target_model_name: &str,
    ) -> Result<OpenAIChatCompletionResponse, ProxyAuthError> {
        serde_json::from_slice(&raw_response.body_bytes).map_err(|e| {
            log::error!("OpenAIAdapter: Failed to deserialize response body: {}", e);
            ProxyAuthError::InternalError(format!(
                "Failed to deserialize OpenAI response body: {}",
                e
            ))
        })
    }

    fn adapt_stream_chunk(
        &self,
        raw_chunk: RawBackendStreamChunk,
        _stream_id: &str,
        _target_model_name: &str,
        _next_tool_call_stream_index: &mut u32,
    ) -> Result<Option<SseEvent>, ProxyAuthError> {
        let chunk_str = String::from_utf8_lossy(&raw_chunk.data);
        // #[cfg(debug_assertions)]
        // {
        //     log::debug!("OpenAIAdapter: raw_chunk: {}", chunk_str);
        // }
        if chunk_str.is_empty() {
            return Ok(None);
        }

        // For OpenAIAdapter, the backend is already OpenAI compatible.
        // The StreamProcessor in handlers.rs should ensure complete SSE events are formed
        // (e.g., "data: {...}\n\n" or "event: foo\ndata: bar\n\n").
        // This adapter needs to parse this block and construct an SseEvent.

        let mut sse_event = SseEvent::default();
        let mut has_any_field = false;

        for line in chunk_str.lines() {
            if line.starts_with("id:") {
                sse_event.id = Some(line["id:".len()..].trim().to_string());
                has_any_field = true;
            } else if line.starts_with("event:") {
                sse_event.event_type = Some(line["event:".len()..].trim().to_string());
                has_any_field = true;
            } else if line.starts_with("data:") {
                // For multi-line data, SSE spec says they are concatenated with '\n'.
                // Here, we are simplifying: if multiple "data:" lines appear in one chunk_str from StreamProcessor,
                // we'll take the content of the last one or concatenate.
                // However, StreamProcessor should ideally give one logical event, which might have been pre-concatenated.
                // For now, let's assume a single 'data' line or that StreamProcessor handles multi-line data correctly into one chunk.
                let current_data = line["data:".len()..].trim();
                sse_event.data = Some(current_data.to_string()); // Overwrites if multiple, or use Some(sse_event.data.unwrap_or_default() + "\n" + current_data)
                has_any_field = true;
            } else if line.starts_with("retry:") {
                sse_event.retry = Some(line["retry:".len()..].trim().to_string());
                has_any_field = true;
            }
        }

        if has_any_field {
            Ok(Some(sse_event))
        } else {
            // If the chunk_str was just empty lines or comments, it's not a valid event to forward.
            Ok(None)
        }
    }

    fn adapt_stream_end(&self) -> Option<String> {
        // OpenAI compatible streams send [DONE] as part of the data stream.
        None
    }
}
