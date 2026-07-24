use axum::response::{IntoResponse, Response};
use std::convert::Infallible;
use std::sync::{Arc, RwLock};

use super::traits::OutputAdapter;
use super::{
    claude_output::ClaudeOutputAdapter, gemini_output::GeminiOutputAdapter,
    ollama_output::OllamaOutputAdapter, openai_output::OpenAIOutputAdapter,
    openai_responses_output::OpenAIResponsesOutputAdapter,
};
use crate::ccproxy::adapter::unified::{
    SseStatus, UnifiedEmbeddingResponse, UnifiedErrorResponse, UnifiedResponse, UnifiedStreamChunk,
};
use crate::ccproxy::helper::sse::Event;

pub enum OutputAdapterEnum {
    OpenAI(OpenAIOutputAdapter),
    OpenAIResponses(OpenAIResponsesOutputAdapter),
    Claude(ClaudeOutputAdapter),
    Gemini(GeminiOutputAdapter),
    Ollama(OllamaOutputAdapter),
}

impl OutputAdapterEnum {
    pub fn adapt_error_response(&self, error: UnifiedErrorResponse) -> Response {
        match self {
            Self::OpenAI(_) | Self::OpenAIResponses(_) => {
                super::error_response::openai_error_response(error)
            }
            Self::Claude(_) => super::error_response::claude_error_response(error),
            Self::Gemini(_) => super::error_response::gemini_error_response(error),
            Self::Ollama(_) => super::error_response::ollama_error_response(error),
        }
    }
}

#[allow(refining_impl_trait)]
impl OutputAdapter for OutputAdapterEnum {
    fn adapt_response(
        &self,
        response: UnifiedResponse,
        sse_status: Arc<RwLock<SseStatus>>,
    ) -> Result<Response, anyhow::Error> {
        let reply = match self {
            Self::OpenAI(adapter) => adapter
                .adapt_response(response, sse_status)?
                .into_response(),
            Self::OpenAIResponses(adapter) => adapter
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
            Self::OpenAIResponses(adapter) => adapter.adapt_stream_chunk(chunk, sse_status),
            Self::Claude(adapter) => adapter.adapt_stream_chunk(chunk, sse_status),
            Self::Gemini(adapter) => adapter.adapt_stream_chunk(chunk, sse_status),
            Self::Ollama(adapter) => adapter.adapt_stream_chunk(chunk, sse_status),
        }
    }

    fn adapt_embedding_response(
        &self,
        response: UnifiedEmbeddingResponse,
    ) -> Result<Response, anyhow::Error> {
        let reply = match self {
            Self::OpenAI(adapter) => adapter.adapt_embedding_response(response)?.into_response(),
            Self::OpenAIResponses(adapter) => {
                adapter.adapt_embedding_response(response)?.into_response()
            }
            Self::Claude(adapter) => adapter.adapt_embedding_response(response)?.into_response(),
            Self::Gemini(adapter) => adapter.adapt_embedding_response(response)?.into_response(),
            Self::Ollama(adapter) => adapter.adapt_embedding_response(response)?.into_response(),
        };
        Ok(reply)
    }
}
