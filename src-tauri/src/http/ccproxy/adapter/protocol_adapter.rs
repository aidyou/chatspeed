use async_trait::async_trait;
use bytes::Bytes;
use reqwest::RequestBuilder;

use crate::http::ccproxy::{
    errors::ProxyAuthError,
    openai_types::{OpenAIChatCompletionRequest, OpenAIChatCompletionResponse, SseEvent},
};

/// Represents the adapted request body and potentially modified headers
#[derive(Debug)]
pub struct AdaptedRequest {
    pub url: String,
    pub headers_to_add: Vec<(String, String)>,
    pub body: Bytes,
}

/// Represents the raw response received from the backend, which needs to be adapted
#[derive(Debug)]
pub struct RawBackendResponse {
    pub status_code: reqwest::StatusCode,
    pub headers: reqwest::header::HeaderMap,
    pub body_bytes: Bytes,
}

/// Represents a raw stream chunk received from the backend
#[derive(Debug)]
pub struct RawBackendStreamChunk {
    pub data: Bytes,
    // Potentially other stream-specific metadata if needed
}

/// Trait for protocol adapters
#[async_trait]
pub trait ProtocolAdapter: Send + Sync {
    /// Adapts an OpenAI-formatted request to the target protocol's format
    fn adapt_request(
        &self,
        base_url: &str,
        target_model_name: &str,
        target_api_key: &str, // Added API key
        openai_request: &OpenAIChatCompletionRequest,
    ) -> Result<AdaptedRequest, ProxyAuthError>;

    /// Adapts a non-streaming response body from the target protocol to OpenAI format
    fn adapt_response_body(
        &self,
        raw_response: RawBackendResponse,
        target_model_name: &str,
    ) -> Result<OpenAIChatCompletionResponse, ProxyAuthError>;

    /// Adapts a streaming response chunk from the target protocol to OpenAI SSE event format
    fn adapt_stream_chunk(
        &self,
        raw_chunk: RawBackendStreamChunk,
        stream_id: &str,
        target_model_name: &str,
        next_tool_call_stream_index: &mut u32,
    ) -> Result<Option<SseEvent>, ProxyAuthError>;

    /// (Optional) Some protocols may require a special termination signal at the end of a streaming response
    fn adapt_stream_end(&self) -> Option<String> {
        Some("[DONE]".to_string())
    }

    /// (Optional) Modifies the reqwest::RequestBuilder before sending it to the target backend.
    fn modify_request_builder(
        &self,
        mut builder: RequestBuilder,
        adapted_request_parts: &AdaptedRequest,
    ) -> Result<RequestBuilder, ProxyAuthError> {
        builder = builder.body(adapted_request_parts.body.clone());
        for (name, value) in &adapted_request_parts.headers_to_add {
            #[cfg(debug_assertions)]
            {
                let lower_name = name.to_lowercase();
                let v = if lower_name == "authorization" || lower_name.contains("-key") {
                    "<hidden>"
                } else {
                    value
                };
                log::debug!("Adding header to proxy request: {}={}", name, v);
            }
            builder = builder.header(name, value);
        }
        Ok(builder)
    }
}
