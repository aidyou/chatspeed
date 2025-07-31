use crate::ccproxy::adapter::unified::{
    SseStatus, UnifiedRequest, UnifiedResponse, UnifiedStreamChunk,
};
use async_trait::async_trait;
use reqwest::{Client, RequestBuilder};
use std::sync::{Arc, RwLock};

/// Represents a raw response from a backend, before being converted to a unified format.
pub struct BackendResponse {
    pub body: bytes::Bytes,
    pub tool_compat_mode: bool,
}

/// A trait for adapting between the `UnifiedRequest`/`UnifiedResponse` and a specific backend protocol.
#[async_trait]
pub trait BackendAdapter: Send + Sync {
    /// Adapts a `UnifiedRequest` into a `RequestBuilder` for the specific backend.
    async fn adapt_request(
        &self,
        client: &Client,
        unified_request: &UnifiedRequest,
        api_key: &str,
        provider_full_url: &str,
        model: &str,
    ) -> Result<RequestBuilder, anyhow::Error>;

    /// Adapts a full `BackendResponse` into a `UnifiedResponse`.
    async fn adapt_response(
        &self,
        backend_response: BackendResponse,
    ) -> Result<UnifiedResponse, anyhow::Error>;

    /// Adapts a raw stream chunk from the backend into a `UnifiedStreamChunk`.
    async fn adapt_stream_chunk(
        &self,
        chunk: bytes::Bytes,
        sse_status: Arc<RwLock<SseStatus>>,
    ) -> Result<Vec<UnifiedStreamChunk>, anyhow::Error>;
}
