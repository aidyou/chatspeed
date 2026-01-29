use crate::ccproxy::adapter::unified::{
    SseStatus, UnifiedEmbeddingResponse, UnifiedResponse, UnifiedStreamChunk,
};
use crate::ccproxy::helper::Event;

use axum::response::Response;
use std::convert::Infallible;
use std::sync::{Arc, RwLock};

/// A trait for adapting a `UnifiedResponse` or `UnifiedStreamChunk` into a specific client-facing format.
pub trait OutputAdapter: Send + Sync {
    /// Converts a full `UnifiedResponse` into a `warp::Reply`.
    fn adapt_response(
        &self,
        response: UnifiedResponse,
        sse_status: Arc<RwLock<SseStatus>>,
    ) -> Result<Response, anyhow::Error>;

    /// Converts a `UnifiedStreamChunk` into a `sse::Event`.
    fn adapt_stream_chunk(
        &self,
        chunk: UnifiedStreamChunk,
        sse_status: Arc<RwLock<SseStatus>>,
    ) -> Result<Vec<Event>, Infallible>;

    /// Converts a `UnifiedEmbeddingResponse` into a `Response`.
    fn adapt_embedding_response(
        &self,
        response: UnifiedEmbeddingResponse,
    ) -> Result<Response, anyhow::Error>;
}
