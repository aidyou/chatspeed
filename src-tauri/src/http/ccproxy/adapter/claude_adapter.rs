use async_trait::async_trait;
use bytes::Bytes;
use reqwest::RequestBuilder;
use warp::http::HeaderMap;

use crate::http::ccproxy::{
    claude_types::{
        ClaudeNativeContentBlock, ClaudeNativeMessage, ClaudeNativeRequest, ClaudeNativeResponse,
        ClaudeNativeUsage, ClaudeToolChoice, StreamState,
    },
    errors::{ProxyAuthError, ProxyResult},
    openai_types::{
        OpenAIChatCompletionRequest, OpenAIChatCompletionResponse,
        OpenAIChatCompletionStreamResponse, OpenAIFunctionDefinition, OpenAIImageUrl,
        OpenAIMessageContent, OpenAIMessageContentPart, OpenAITool, UnifiedChatMessage,
    },
};

use crate::ai::network::types::{
    GeminiContent,
    GeminiFunctionResponse, // Added
    GeminiGenerationConfig,
    GeminiInlineData,
    GeminiPart,
    GeminiRequest,
    GeminiResponse as GeminiNetworkResponse,
};
use serde::Serialize;
use std::convert::Infallible;

/// Represents the raw response from a backend before adaptation.
pub struct RawBackendResponse {
    pub status_code: reqwest::StatusCode,
    pub headers: HeaderMap,
    pub body_bytes: Bytes,
}

/// Represents a single, complete chunk from a backend's streaming response.
pub struct RawBackendStreamChunk {
    pub data: Bytes,
}

/// Defines the structure of an SSE event after being parsed and adapted.
#[derive(Debug, Default, Clone)]
pub struct SseEvent {
    pub id: Option<String>,
    pub event_type: Option<String>,
    pub data: Option<String>,
    pub retry: Option<String>,
    pub usage: Option<ClaudeNativeUsage>, // Added usage field
}

/// Defines the necessary parts of a request after adaptation, ready to be built.
#[derive(Debug)]
pub struct AdaptedRequest {
    pub url: String,
    pub headers_to_add: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[async_trait]
pub trait ClaudeProtocolAdapter: Send + Sync {
    /// Adapts the incoming Claude-native request to a format suitable for the target backend.
    fn adapt_request(
        &self,
        base_url: &str,
        actual_model_name: &str,
        api_key: &str,
        client_request: &ClaudeNativeRequest,
    ) -> ProxyResult<AdaptedRequest>;

    /// Modifies the `reqwest::RequestBuilder` before it's sent.
    /// This is useful for adding headers or making other changes.
    fn modify_request_builder(
        &self,
        builder: RequestBuilder,
        adapted_request: &AdaptedRequest,
    ) -> ProxyResult<RequestBuilder> {
        let mut new_builder = builder;
        for (key, value) in &adapted_request.headers_to_add {
            new_builder = new_builder.header(key, value);
        }
        new_builder = new_builder.body(adapted_request.body.clone());
        Ok(new_builder)
    }

    /// Adapts a single chunk from the backend's stream into an SSE event for the client.
    fn adapt_stream_chunk(
        &self,
        chunk: RawBackendStreamChunk,
        stream_id: &str,
        model_name: &str,
        next_tool_call_stream_index: &mut u32,
        stream_state: &mut StreamState,
    ) -> ProxyResult<Vec<SseEvent>>;

    /// Provides the content for the final SSE event (e.g., "[DONE]").
    fn adapt_stream_end(&self) -> Option<String> {
        Some("[DONE]".to_string())
    }

    /// Adapts the full response from the backend into a Claude-native response for the client.
    fn adapt_response_body(
        &self,
        response: RawBackendResponse,
        model_name: &str,
    ) -> ProxyResult<ClaudeNativeResponse>;
}

// --- Concrete Adapters ---

pub struct ClaudeToClaudeAdapter;

#[async_trait]
impl ClaudeProtocolAdapter for ClaudeToClaudeAdapter {
    fn adapt_request(
        &self,
        base_url: &str,
        actual_model_name: &str,
        api_key: &str,
        client_request: &ClaudeNativeRequest,
    ) -> ProxyResult<AdaptedRequest> {
        #[cfg(debug_assertions)]
        log::debug!(
            "ClaudeToClaudeAdapter: Received request: {}\n\n\n\n",
            serde_json::to_string_pretty(client_request).unwrap_or_default()
        );
        let mut claude_request = client_request.clone();
        claude_request.model = actual_model_name.to_string();

        let body_bytes = serde_json::to_vec(&claude_request).map_err(|e| {
            ProxyAuthError::InternalError(format!(
                "Failed to serialize Claude native request: {}",
                e
            ))
        })?;

        Ok(AdaptedRequest {
            url: format!("{}/messages", base_url),
            headers_to_add: vec![
                (
                    reqwest::header::CONTENT_TYPE.as_str().to_string(),
                    "application/json".to_string(),
                ),
                ("x-api-key".to_string(), api_key.to_string()),
            ],
            body: body_bytes,
        })
    }

    fn adapt_stream_chunk(
        &self,
        chunk: RawBackendStreamChunk,
        _stream_id: &str,
        _model_name: &str,
        _next_tool_call_stream_index: &mut u32,
        _stream_state: &mut StreamState,
    ) -> ProxyResult<Vec<SseEvent>> {
        #[cfg(debug_assertions)]
        log::debug!(
            "ClaudeToClaudeAdapter: Received stream chunk: {:?}",
            chunk.data
        );
        let chunk_str = String::from_utf8_lossy(&chunk.data);
        let mut events = Vec::new();
        let mut event = SseEvent::default();
        for line in chunk_str.lines() {
            if let Some(event_type) = line.strip_prefix("event: ") {
                event.event_type = Some(event_type.trim().to_string());
            } else if let Some(data) = line.strip_prefix("data: ") {
                event.data = Some(data.trim().to_string());
            }
        }
        if event.data.is_some() {
            events.push(event);
        }
        Ok(events)
    }

    fn adapt_response_body(
        &self,
        response: RawBackendResponse,
        _model_name: &str,
    ) -> ProxyResult<ClaudeNativeResponse> {
        #[cfg(debug_assertions)]
        log::debug!(
            "ClaudeToClaudeAdapter: Received response body: {:?}",
            response.body_bytes
        );
        Ok(
            serde_json::from_slice::<ClaudeNativeResponse>(&response.body_bytes).map_err(|e| {
                ProxyAuthError::InternalError(format!(
                    "Failed to parse Claude native response: {}",
                    e
                ))
            })?,
        )
    }
}

pub struct ClaudeToOpenAIAdapter;

#[async_trait]
impl ClaudeProtocolAdapter for ClaudeToOpenAIAdapter {
    fn adapt_request(
        &self,
        base_url: &str,
        actual_model_name: &str,
        api_key: &str,
        client_request: &ClaudeNativeRequest,
    ) -> ProxyResult<AdaptedRequest> {
        #[cfg(debug_assertions)]
        log::debug!(
            "ClaudeToOpenAIAdapter: Received request: {}\n\n\n\n",
            serde_json::to_string_pretty(client_request).unwrap_or_default()
        );
        let openai_request = convert_claude_to_openai(client_request, actual_model_name)?;
        let body_bytes = serde_json::to_vec(&openai_request).map_err(|e| {
            log::error!(
                "ClaudeToOpenAIAdapter: Failed to serialize OpenAI request: {}",
                e
            );
            ProxyAuthError::InternalError(format!("Failed to serialize OpenAI request: {}", e))
        })?;

        let url = if base_url.contains("router.huggingface.co") {
            base_url
                .split_once("/hf-inference/models")
                .map(|(base, _)| format!("{base}/hf-inference/models/{actual_model_name}/v1"))
                .unwrap_or_else(|| {
                    format!(
                        "https://router.huggingface.co/hf-inference/models/{actual_model_name}/v1"
                    )
                })
        } else {
            format!("{base_url}/chat/completions")
        };

        Ok(AdaptedRequest {
            url,
            headers_to_add: {
                let mut headers = vec![(
                    reqwest::header::CONTENT_TYPE.as_str().to_string(),
                    "application/json".to_string(),
                )];
                if !api_key.is_empty() {
                    headers.push(("Authorization".to_string(), format!("Bearer {}", api_key)));
                }
                headers
            },
            body: body_bytes,
        })
    }

    fn adapt_stream_chunk(
        &self,
        chunk: RawBackendStreamChunk,
        _stream_id: &str,
        _model_name: &str,
        _next_tool_call_stream_index: &mut u32,
        stream_state: &mut StreamState,
    ) -> ProxyResult<Vec<SseEvent>> {
        let chunk_str = String::from_utf8_lossy(&chunk.data);
        if chunk_str.contains("[DONE]") {
            return Ok(vec![]);
        }
        match convert_openai_stream_to_claude_stream(chunk_str, stream_state) {
            Ok(sse_events) => Ok(sse_events),
            Err(e) => Err(warp::reject::custom(ProxyAuthError::InternalError(
                format!("Stream conversion failed: {}", e),
            ))),
        }
    }

    fn adapt_response_body(
        &self,
        response: RawBackendResponse,
        model_name: &str,
    ) -> ProxyResult<ClaudeNativeResponse> {
        let openai_response: OpenAIChatCompletionResponse =
            serde_json::from_slice(&response.body_bytes).map_err(|e| {
                ProxyAuthError::InternalError(format!("Failed to parse OpenAI response: {}", e))
            })?;
        Ok(convert_openai_to_claude(&openai_response, model_name)?)
    }
}

pub struct ClaudeToGeminiAdapter;

#[async_trait]
impl ClaudeProtocolAdapter for ClaudeToGeminiAdapter {
    fn adapt_request(
        &self,
        base_url: &str,
        actual_model_name: &str,
        api_key: &str,
        client_request: &ClaudeNativeRequest,
    ) -> ProxyResult<AdaptedRequest> {
        #[cfg(debug_assertions)]
        log::debug!(
            "ClaudeToGeminiAdapter: Received request: {}\n\n\n\n",
            serde_json::to_string_pretty(client_request).unwrap_or_default()
        );
        let gemini_request = convert_claude_to_gemini(client_request, actual_model_name)?;
        let body_bytes = serde_json::to_vec(&gemini_request).map_err(|e| {
            ProxyAuthError::InternalError(format!("Failed to serialize Gemini request: {}", e))
        })?;

        Ok(AdaptedRequest {
            url: if client_request.stream.unwrap_or(false) {
                format!(
                    "{}/models/{}:streamGenerateContent?alt=sse&key={}",
                    base_url, actual_model_name, api_key
                )
            } else {
                format!(
                    "{}/models/{}:generateContent?key={}",
                    base_url, actual_model_name, api_key
                )
            },
            headers_to_add: vec![
                (
                    reqwest::header::CONTENT_TYPE.as_str().to_string(),
                    "application/json".to_string(),
                ),
                // ("x-goog-api-key".to_string(), api_key.to_string()),
            ],
            body: body_bytes,
        })
    }

    fn adapt_stream_chunk(
        &self,
        chunk: RawBackendStreamChunk,
        _stream_id: &str,
        _model_name: &str,
        _next_tool_call_stream_index: &mut u32,
    ) -> ProxyResult<Option<SseEvent>> {
        let chunk_str = String::from_utf8_lossy(&chunk.data);
        if chunk_str.contains("[DONE]") {
            return Ok(None);
        }
        match convert_gemini_stream_to_claude_stream(chunk_str) {
            Some(Ok(sse_event)) => Ok(Some(sse_event)),
            Some(Err(_)) => Err(warp::reject::custom(ProxyAuthError::InternalError(
                "Stream conversion failed".to_string(),
            ))),
            None => Ok(None),
        }
    }

    fn adapt_response_body(
        &self,
        response: RawBackendResponse,
        model_name: &str,
    ) -> ProxyResult<ClaudeNativeResponse> {
        let gemini_response: GeminiNetworkResponse = serde_json::from_slice(&response.body_bytes)
            .map_err(|e| {
            ProxyAuthError::InternalError(format!("Failed to parse Gemini response: {}", e))
        })?;
        Ok(convert_gemini_to_claude(&gemini_response, model_name)?)
    }
}

// ==========================================================================================
// Conversion functions
// ==========================================================================================

/// Convert Claude native request to OpenAI format
pub fn convert_claude_to_openai(
    claude_request: &ClaudeNativeRequest,
    actual_model_name: &str,
) -> Result<OpenAIChatCompletionRequest, ProxyAuthError> {
    let mut openai_messages = Vec::new();

    if let Some(system_prompt) = &claude_request.system {
        openai_messages.push(UnifiedChatMessage {
            role: Some("system".to_string()),
            content: Some(OpenAIMessageContent::Text(system_prompt.clone())),
            ..Default::default()
        });
    }

    openai_messages.extend(convert_claude_messages_to_openai(&claude_request.messages)?);

    let openai_request = OpenAIChatCompletionRequest {
        model: actual_model_name.to_string(),
        messages: openai_messages,
        max_tokens: Some(claude_request.max_tokens),
        stream: claude_request.stream,
        temperature: claude_request.temperature,
        top_p: claude_request.top_p,
        top_k: None, // OpenAI API does not support top_k
        tools: claude_request.tools.as_ref().map(|tools| {
            tools
                .iter()
                .map(|tool| OpenAITool {
                    r#type: "function".to_string(),
                    function: OpenAIFunctionDefinition {
                        name: tool.name.clone(),
                        description: tool.description.clone(),
                        parameters: tool.input_schema.clone(),
                    },
                })
                .collect()
        }),
        tool_choice: claude_request
            .tool_choice
            .as_ref()
            .map(|choice| match choice {
                ClaudeToolChoice::Auto => serde_json::json!("auto"),
                ClaudeToolChoice::Any => serde_json::json!("any"),
                ClaudeToolChoice::Tool { name } => serde_json::json!({
                    "type": "function",
                    "function": { "name": name }
                }),
            }),
        ..Default::default()
    };

    Ok(openai_request)
}

/// Convert Claude messages to OpenAI format
pub fn convert_claude_messages_to_openai(
    claude_messages: &[ClaudeNativeMessage],
) -> Result<Vec<UnifiedChatMessage>, ProxyAuthError> {
    let mut openai_messages = Vec::new();

    for claude_msg in claude_messages {
        let mut openai_content_parts = Vec::new();
        let mut reasoning_content: Option<String> = None;

        for content_block in &claude_msg.content {
            match content_block {
                ClaudeNativeContentBlock::Text { text } => {
                    openai_content_parts
                        .push(OpenAIMessageContentPart::Text { text: text.clone() });
                }
                ClaudeNativeContentBlock::Image { source } => {
                    openai_content_parts.push(OpenAIMessageContentPart::ImageUrl {
                        image_url: OpenAIImageUrl {
                            url: format!("data:{};base64, {}", source.media_type, source.data),
                            detail: None,
                        },
                    });
                }
                ClaudeNativeContentBlock::ToolUse { id: _, name, input } => {
                    openai_content_parts.push(OpenAIMessageContentPart::Text {
                        text: format!(
                            "Tool call: {}({})",
                            name,
                            serde_json::to_string(input).unwrap_or_default()
                        ),
                    });
                }
                ClaudeNativeContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } => {
                    openai_content_parts.push(OpenAIMessageContentPart::Text {
                        text: format!("Tool result for {}: {}", tool_use_id, content),
                    });
                }
                ClaudeNativeContentBlock::Thinking { thinking } => {
                    // Store thinking content to be used in the reasoning_content field.
                    reasoning_content = Some(thinking.clone());
                }
            }
        }

        let content = if openai_content_parts.is_empty() {
            OpenAIMessageContent::Text(String::new())
        } else if openai_content_parts.len() == 1
            && matches!(
                openai_content_parts[0],
                OpenAIMessageContentPart::Text { .. }
            )
        {
            if let OpenAIMessageContentPart::Text { text } = openai_content_parts.remove(0) {
                OpenAIMessageContent::Text(text)
            } else {
                unreachable!()
            }
        } else {
            OpenAIMessageContent::Parts(openai_content_parts)
        };

        openai_messages.push(UnifiedChatMessage {
            role: Some(claude_msg.role.clone()),
            content: Some(content),
            tool_calls: None,
            tool_call_id: None,
            reasoning_content,
        });
    }

    Ok(openai_messages)
}

/// Convert Gemini streaming format to Claude streaming format
pub fn convert_gemini_stream_to_claude_stream(
    chunk_str: std::borrow::Cow<str>,
) -> Option<Result<SseEvent, Infallible>> {
    for event_block_str in chunk_str.split("\n\n") {
        if event_block_str.trim().is_empty() {
            continue;
        }

        let mut data: Option<String> = None;

        for line in event_block_str.lines() {
            if line.starts_with("data: ") {
                data = Some(line.trim_start_matches("data: ").trim().to_string());
            }
        }

        if let Some(data_str) = data {
            if data_str == "[DONE]" {
                return None;
            }

            match parse_and_convert_gemini_chunk(&data_str) {
                Ok(claude_events) => {
                    if !claude_events.is_empty() {
                        let event_data = claude_events[0].clone();
                        let event = SseEvent {
                            event_type: Some("content_block_delta".to_string()),
                            data: Some(event_data),
                            ..Default::default()
                        };
                        return Some(Ok(event));
                    }
                }
                Err(e) => {
                    log::error!("Failed to convert Gemini stream chunk: {}", e);
                    return None;
                }
            }
        }
    }

    None
}

/// Parse Gemini streaming chunk and convert to Claude SSE format
pub fn parse_and_convert_gemini_chunk(
    data_str: &str,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut events = Vec::new();

    let gemini_chunk: GeminiNetworkResponse = serde_json::from_str(data_str)?;

    if let Some(candidates) = &gemini_chunk.candidates {
        if let Some(candidate) = candidates.first() {
            let content_parts = &candidate.content.parts;
            let text_content: String = content_parts
                .iter()
                .filter_map(|part| part.text.clone())
                .collect::<Vec<_>>()
                .join("");

            if !text_content.is_empty() {
                let claude_delta = ClaudeStreamDeltaResponse {
                    block_type: "content_block_delta".to_string(),
                    index: 0,
                    delta: ClaudeStreamTextDelta {
                        delta_type: "text_delta".to_string(),
                        text: text_content,
                    },
                    usage: gemini_chunk
                        .usage_metadata
                        .as_ref()
                        .map(|u| ClaudeNativeUsage {
                            input_tokens: u.prompt_token_count as u64,
                            output_tokens: u.candidates_token_count.unwrap_or(0) as u64,
                        }),
                };

                let claude_data = serde_json::to_string(&claude_delta)?;
                events.push(claude_data);
            }

            if let Some(_) = &candidate.finish_reason {
                let claude_stop = serde_json::json!({
                    "type": "message_stop"
                });

                let stop_data = serde_json::to_string(&claude_stop)?;
                events.push(stop_data);
            }
        }
    }

    Ok(events)
}

/// Convert OpenAI streaming format to Claude streaming format
pub fn convert_openai_stream_to_claude_stream(
    chunk_str: std::borrow::Cow<str>,
) -> Option<Result<SseEvent, Infallible>> {
    for event_block_str in chunk_str.split("\n\n") {
        if event_block_str.trim().is_empty() {
            continue;
        }

        let mut data: Option<String> = None;

        for line in event_block_str.lines() {
            if line.starts_with("data: ") {
                data = Some(line.trim_start_matches("data: ").trim().to_string());
            }
        }

        if let Some(data_str) = data {
            if data_str == "[DONE]" {
                return None;
            }

            match parse_and_convert_openai_chunk(&data_str) {
                Ok(claude_events) => {
                    if !claude_events.is_empty() {
                        let event_data = claude_events[0].clone();
                        let event = SseEvent {
                            event_type: Some("content_block_delta".to_string()),
                            data: Some(event_data),
                            ..Default::default()
                        };
                        return Some(Ok(event));
                    }
                }
                Err(e) => {
                    log::error!("Failed to convert OpenAI stream chunk: {}", e);
                    return None;
                }
            }
        }
    }

    None
}

/// Parse OpenAI streaming chunk and convert to Claude SSE format
pub fn parse_and_convert_openai_chunk(
    data_str: &str,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut events = Vec::new();
    log::debug!("event data: {}", &data_str);
    let openai_chunk: OpenAIChatCompletionStreamResponse = serde_json::from_str(data_str)?;

    let choices = &openai_chunk.choices;
    for choice in choices {
        let delta = &choice.delta;

        // Handle reasoning content as thinking blocks
        if let Some(reasoning_content) = &delta.reasoning_content {
            if !reasoning_content.is_empty() {
                // Map reasoning content to Claude's thinking format
                let thinking_start = serde_json::json!({
                    "type": "content_block_start",
                    "index": 0,
                    "content_block": {
                        "type": "text",
                        "text": reasoning_content
                    }
                });
                events.push(serde_json::to_string(&thinking_start)?);

                let thinking_delta = serde_json::json!({
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {
                        "type": "text_delta",
                        "text": reasoning_content
                    }
                });
                events.push(serde_json::to_string(&thinking_delta)?);
            }
        }

        if let Some(content) = &delta.content {
            let text = match content {
                OpenAIMessageContent::Text(text) => text.clone(),
                OpenAIMessageContent::Parts(parts) => parts
                    .iter()
                    .filter_map(|part| match part {
                        OpenAIMessageContentPart::Text { text } => Some(text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(""),
            };

            if !text.is_empty() {
                let claude_delta = ClaudeStreamDeltaResponse {
                    block_type: "content_block_delta".to_string(),
                    index: 1,
                    delta: ClaudeStreamTextDelta {
                        delta_type: "text_delta".to_string(),
                        text: text,
                    },
                    usage: openai_chunk.usage.as_ref().map(|u| ClaudeNativeUsage {
                        input_tokens: u.prompt_tokens,
                        output_tokens: u.completion_tokens,
                    }),
                };

                let claude_data = serde_json::to_string(&claude_delta)?;
                events.push(claude_data);
            }

            if let Some(_finish_reason) = &choice.finish_reason {
                let claude_stop = serde_json::json!({
                    "type": "message_stop"
                });

                let stop_data = serde_json::to_string(&claude_stop)?;
                events.push(stop_data);
            }
        }
    }

    Ok(events)
}

/// Claude streaming response structures
#[derive(Serialize)]
struct ClaudeStreamDeltaResponse {
    #[serde(rename = "type")]
    block_type: String,
    index: u32,
    delta: ClaudeStreamTextDelta,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<ClaudeNativeUsage>, // Added usage field
}

#[derive(Serialize)]
struct ClaudeStreamTextDelta {
    #[serde(rename = "type")]
    delta_type: String,
    text: String,
}

/// Convert Claude request to Gemini format
pub fn convert_claude_to_gemini(
    claude_request: &ClaudeNativeRequest,
    _actual_model_name: &str,
) -> Result<GeminiRequest, ProxyAuthError> {
    let mut gemini_contents: Vec<GeminiContent> = Vec::new();

    for claude_msg in &claude_request.messages {
        let role = match claude_msg.role.as_str() {
            "user" => "user",
            "assistant" => "model",
            _ => "user",
        };

        let mut parts = Vec::new();
        for content_block in &claude_msg.content {
            match content_block {
                ClaudeNativeContentBlock::Text { text } => {
                    parts.push(GeminiPart {
                        text: Some(text.clone()),
                        inline_data: None,
                        function_call: None,
                        function_response: None,
                    });
                }
                ClaudeNativeContentBlock::Image { source } => {
                    parts.push(GeminiPart {
                        text: None,
                        inline_data: Some(GeminiInlineData {
                            mime_type: source.media_type.clone(),
                            data: source.data.clone(),
                        }),
                        function_call: None,
                        function_response: None,
                    });
                }
                ClaudeNativeContentBlock::ToolUse { id: _, name, input } => {
                    parts.push(crate::ai::network::types::GeminiPart {
                        text: None,
                        inline_data: None,
                        function_call: Some(crate::ai::network::types::GeminiFunctionCall {
                            name: name.clone(),
                            args: input.clone(),
                        }),
                        function_response: None,
                    });
                }
                ClaudeNativeContentBlock::ToolResult {
                    tool_use_id: _,
                    content,
                    ..
                } => {
                    parts.push(GeminiPart {
                        text: None,
                        inline_data: None,
                        function_call: None,
                        function_response: Some(GeminiFunctionResponse {
                            name: "tool_result".to_string(), // Or map to actual tool name if available
                            response: serde_json::Value::String(content.clone()),
                        }),
                    });
                }
                ClaudeNativeContentBlock::Thinking { thinking } => {
                    // Gemini does not have a direct equivalent for thinking content in user messages.
                    // We can represent it as text for now, or ignore it.
                    parts.push(GeminiPart {
                        text: Some(thinking.clone()),
                        inline_data: None,
                        function_call: None,
                        function_response: None,
                    });
                }
            }
        }

        gemini_contents.push(GeminiContent {
            role: role.to_string(),
            parts,
        });
    }

    let generation_config = GeminiGenerationConfig {
        max_output_tokens: Some(claude_request.max_tokens),
        temperature: claude_request.temperature,
        top_p: claude_request.top_p,
        top_k: None, // OpenAI API does not support top_k
        stop_sequences: None,
    };

    let gemini_request = GeminiRequest {
        contents: gemini_contents,
        generation_config: Some(generation_config),
        tools: None,
        tool_config: None,
        system_instruction: claude_request.system.as_ref().map(|s| GeminiContent {
            role: "system".to_string(),
            parts: vec![GeminiPart {
                text: Some(s.clone()),
                ..Default::default()
            }],
        }),
    };

    Ok(gemini_request)
}

/// Convert Gemini response to Claude format
pub fn convert_gemini_to_claude(
    gemini_response: &GeminiNetworkResponse,
    actual_model_name: &str,
) -> Result<ClaudeNativeResponse, ProxyAuthError> {
    let mut content = Vec::new();
    let mut usage = None;
    let mut stop_reason = None;

    if let Some(candidates) = &gemini_response.candidates {
        if let Some(candidate) = candidates.first() {
            let content_parts = &candidate.content.parts;
            let text_content: String = content_parts
                .iter()
                .filter_map(|part| part.text.clone())
                .collect::<Vec<_>>()
                .join("");

            content.push(ClaudeNativeContentBlock::Text { text: text_content });

            stop_reason = candidate.finish_reason.as_ref().map(|reason| {
                match reason.as_str() {
                    "STOP" => "end_turn",
                    "MAX_TOKENS" => "max_tokens",
                    _ => "end_turn",
                }
                .to_string()
            });
        }
    }

    if let Some(usage_metadata) = &gemini_response.usage_metadata {
        usage = Some(ClaudeNativeUsage {
            input_tokens: usage_metadata.prompt_token_count as u64,
            output_tokens: usage_metadata.candidates_token_count.unwrap_or(0) as u64,
        });
    }

    let claude_response = ClaudeNativeResponse {
        id: format!("gemini-claude-{}-converted", uuid::Uuid::new_v4()),
        response_type: "message".to_string(),
        role: Some("assistant".to_string()),
        content,
        model: Some(actual_model_name.to_string()),
        stop_reason,
        usage,
        error: None,
    };

    Ok(claude_response)
}

/// Convert OpenAI response to Claude format
pub fn convert_openai_to_claude(
    openai_response: &OpenAIChatCompletionResponse,
    actual_model_name: &str,
) -> Result<ClaudeNativeResponse, ProxyAuthError> {
    let content = openai_response
        .choices
        .iter()
        .filter_map(|choice| {
            if let Some(ref content) = choice.message.content {
                match content {
                    OpenAIMessageContent::Text(text) => Some(text.clone()),
                    OpenAIMessageContent::Parts(parts) => Some(
                        parts
                            .iter()
                            .filter_map(|part| match part {
                                OpenAIMessageContentPart::Text { text } => Some(text.clone()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join(""),
                    ),
                }
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("")
        .trim()
        .to_string();

    let mut claude_content = Vec::new();

    // Add reasoning content if present
    if let Some(reasoning) = openai_response
        .choices
        .first()
        .and_then(|choice| choice.message.reasoning_content.clone())
    {
        claude_content.push(ClaudeNativeContentBlock::Thinking {
            thinking: reasoning,
        });
    }

    // Add text content
    claude_content.push(ClaudeNativeContentBlock::Text { text: content });

    let stop_reason = openai_response
        .choices
        .first()
        .and_then(|choice| choice.finish_reason.as_ref())
        .map(|reason| {
            match reason.as_str() {
                "stop" => "end_turn",
                "length" => "max_tokens",
                _ => "end_turn",
            }
            .to_string()
        });

    let usage = openai_response
        .usage
        .as_ref()
        .map(|usage| ClaudeNativeUsage {
            input_tokens: usage.prompt_tokens,
            output_tokens: usage.completion_tokens,
        });

    Ok(ClaudeNativeResponse {
        id: format!("openai-claude-{}-converted", openai_response.id),
        response_type: "message".to_string(),
        role: Some("assistant".to_string()),
        content: claude_content,
        model: Some(actual_model_name.to_string()),
        stop_reason,
        usage,
        error: None,
    })
}
