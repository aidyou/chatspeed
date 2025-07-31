use std::convert::Infallible;
use std::sync::{Arc, RwLock};
use warp::Reply;

use super::traits::OutputAdapter;
use super::{
    claude_output::ClaudeOutputAdapter, gemini_output::GeminiOutputAdapter,
    ollama_output::OllamaOutputAdapter, openai_output::OpenAIOutputAdapter,
};
use crate::ccproxy::adapter::unified::{SseStatus, UnifiedResponse, UnifiedStreamChunk};
use crate::ccproxy::helper::sse::Event;

pub enum OutputAdapterEnum {
    OpenAI(OpenAIOutputAdapter),
    Claude(ClaudeOutputAdapter),
    Gemini(GeminiOutputAdapter),
    Ollama(OllamaOutputAdapter),
}

#[allow(refining_impl_trait)]
impl OutputAdapter for OutputAdapterEnum {
    fn adapt_response(
        &self,
        response: UnifiedResponse,
        sse_status: Arc<RwLock<SseStatus>>,
    ) -> Result<warp::http::Response<warp::hyper::Body>, anyhow::Error> {
        let reply = match self {
            Self::OpenAI(adapter) => adapter
                .adapt_response(response, sse_status)?
                .into_response(),
            Self::Claude(adapter) => adapter
                .adapt_response(response, sse_status)?
                .into_response(),
            Self::Gemini(adapter) => adapter
                .adapt_response(response, sse_status)?
                .into_response(),
            Self::Ollama(adapter) => adapter
                .adapt_response(response, sse_status)?
                .into_response(),
        };
        Ok(reply)
    }

    fn adapt_stream_chunk(
        &self,
        chunk: UnifiedStreamChunk,
        sse_status: Arc<RwLock<SseStatus>>,
    ) -> Result<Vec<Event>, Infallible> {
        match self {
            Self::OpenAI(adapter) => adapter.adapt_stream_chunk(chunk, sse_status),
            Self::Claude(adapter) => adapter.adapt_stream_chunk(chunk, sse_status),
            Self::Gemini(adapter) => adapter.adapt_stream_chunk(chunk, sse_status),
            Self::Ollama(adapter) => adapter.adapt_stream_chunk(chunk, sse_status),
        }
    }
}
