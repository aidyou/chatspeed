use crate::ccproxy::adapter::unified::{SseStatus, UnifiedResponse, UnifiedStreamChunk};
use std::convert::Infallible;
use std::sync::{Arc, RwLock};
use warp::sse::Event;

/// A trait for adapting a `UnifiedResponse` or `UnifiedStreamChunk` into a specific client-facing format.
pub trait OutputAdapter: Send + Sync {
    /// Converts a full `UnifiedResponse` into a `warp::Reply`.
    fn adapt_response(&self, response: UnifiedResponse) -> Result<impl warp::Reply, anyhow::Error>;

    /// Converts a `UnifiedStreamChunk` into a `warp::sse::Event`.
    fn adapt_stream_chunk(
        &self,
        chunk: UnifiedStreamChunk,
        sse_status: Arc<RwLock<SseStatus>>,
    ) -> Result<Vec<Event>, Infallible>;
}
