use async_trait::async_trait;
use bytes::Bytes;
use reqwest::RequestBuilder;

use crate::http::ccproxy::{
    errors::ProxyAuthError,
    types::{OpenAIChatCompletionRequest, OpenAIChatCompletionResponse, SseEvent},
};

/// 表示转换后的请求体和可能需要修改的请求头
#[derive(Debug)]
pub struct AdaptedRequest {
    pub url: String,
    pub headers_to_add: Vec<(String, String)>,
    pub body: Bytes,
}

/// 表示从后端接收到的原始响应，需要被转换
#[derive(Debug)]
pub struct RawBackendResponse {
    pub status_code: reqwest::StatusCode,
    pub headers: reqwest::header::HeaderMap,
    pub body_bytes: Bytes,
}

/// 表示从后端接收到的原始流式响应块
#[derive(Debug)]
pub struct RawBackendStreamChunk {
    pub data: Bytes,
    // Potentially other stream-specific metadata if needed
}

/// 协议转换器的 Trait
#[async_trait]
pub trait ProtocolAdapter: Send + Sync {
    /// 将 OpenAI 格式的请求转换为目标协议格式的请求
    fn adapt_request(
        &self,
        base_url: &str,
        target_model_name: &str,
        target_api_key: &str, // Added API key
        openai_request: &OpenAIChatCompletionRequest,
    ) -> Result<AdaptedRequest, ProxyAuthError>;

    /// 将目标协议的非流式响应体转换为 OpenAI 格式的响应体
    fn adapt_response_body(
        &self,
        raw_response: RawBackendResponse,
        target_model_name: &str,
    ) -> Result<OpenAIChatCompletionResponse, ProxyAuthError>;

    /// 将目标协议的流式响应块转换为 OpenAI SSE 事件格式的字符串
    fn adapt_stream_chunk(
        &self,
        raw_chunk: RawBackendStreamChunk,
        stream_id: &str,
        target_model_name: &str,
        next_tool_call_stream_index: &mut u32,
    ) -> Result<Option<SseEvent>, ProxyAuthError>;

    /// (可选) 有些协议的流式响应结束时可能需要发送一个特殊的终止信号
    fn adapt_stream_end(&self) -> Option<String> {
        Some("[DONE]".to_string())
    }

    /// (可选) 修改即将发送到目标后端的 reqwest::RequestBuilder。
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
