use bytes::Bytes;
use rust_i18n::t;
use serde_json::Value;
use std::fmt;

use crate::ai::traits::chat::{MessageType, ToolCallDeclaration};

use super::{
    ClaudeEventType, ClaudeStreamEvent, GeminiFunctionCall, GeminiResponse, OpenAIStreamResponse,
};

/// Represents different types of stream response formats
pub enum StreamFormat {
    /// OpenAI compatible format
    /// data: {"choices":[{"delta":{"content":"Hello"},"index":0}]}
    OpenAI,

    /// Google AI (Gemini) format
    /// {"candidates":[{"content":{"parts":[{"text":"Hello"}],"role":"model"},"index":0}]}
    Gemini,

    /// Standard SSE format
    /// event: message
    /// data: Hello
    #[allow(dead_code)]
    StandardSSE,

    /// Custom format with user-provided parser
    #[allow(dead_code)]
    Custom(Box<dyn Fn(Bytes) -> Result<Option<String>, String> + Send + Sync>),
    /// Claude format
    /// {"completion":"Hello","usage":{"input_tokens":10,"output_tokens":10}}
    Claude,
}

impl fmt::Debug for StreamFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OpenAI => write!(f, "StreamFormat::OpenAI"),
            Self::Gemini => write!(f, "StreamFormat::Gemini"),
            Self::StandardSSE => write!(f, "StreamFormat::StandardSSE"),
            Self::Custom(_) => write!(f, "StreamFormat::Custom(<custom_parser>)"),
            // Self::HuggingFace => write!(f, "StreamFormat::HuggingFace"),
            Self::Claude => write!(f, "StreamFormat::Claude"),
        }
    }
}

/// Token usage information
#[derive(Debug, Default, Clone)]
pub struct TokenUsage {
    pub total_tokens: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// The speed of token generation in tokens per second (tokens/s)
    pub tokens_per_second: f64,
}

/// Stream chunk parsing result
#[derive(Debug)]
pub struct StreamChunk {
    pub reasoning_content: Option<String>,
    /// The content of the chunk
    pub content: Option<String>,
    /// Token usage information
    pub usage: Option<TokenUsage>,
    // Message type, text,step,reference
    pub msg_type: Option<MessageType>,
    /// Tool calls
    pub tool_calls: Option<Vec<ToolCallDeclaration>>,
    /// Finish reason (specific to OpenAI)
    pub finish_reason: Option<String>,
}

/// Stream response parser
pub struct StreamParser;

impl StreamParser {
    /// Parse a chunk of stream data according to the specified format
    pub async fn parse_chunk(
        chunk: Bytes,
        format: &StreamFormat,
    ) -> Result<Vec<StreamChunk>, String> {
        match format {
            StreamFormat::OpenAI => Self::parse_openai(chunk),
            StreamFormat::Gemini => Self::parse_gemini(chunk),
            StreamFormat::StandardSSE => Self::parse_standard_sse(chunk),
            StreamFormat::Claude => Self::parse_claude(chunk),
            StreamFormat::Custom(parser) => parser(chunk).map(|content| {
                vec![StreamChunk {
                    reasoning_content: None,
                    content,
                    usage: None,
                    msg_type: None,
                    tool_calls: None,
                    finish_reason: None,
                }]
            }),
        }
    }

    /// Parse OpenAI compatible format
    fn parse_openai(chunk: Bytes) -> Result<Vec<StreamChunk>, String> {
        // 使用 from_utf8_lossy 替代 from_utf8，以便在遇到无效的 UTF-8 序列时能够继续处理
        // 这将用替换字符 (U+FFFD) 替代无效的 UTF-8 序列，而不是返回错误
        let chunk_str = String::from_utf8_lossy(&chunk).into_owned();
        let mut chunks = Vec::new();
        // log::debug!("openai - {}", chunk_str);

        for line in chunk_str.lines() {
            if line.starts_with("data:") {
                let data = line["data:".len()..].trim();
                if data == "[DONE]" {
                    chunks.push(StreamChunk {
                        reasoning_content: None,
                        content: None,
                        usage: None,
                        msg_type: Some(MessageType::Finished),
                        tool_calls: None,
                        finish_reason: Some("[DONE]".to_string()),
                    });
                    continue;
                }
                match serde_json::from_str::<OpenAIStreamResponse>(data) {
                    Ok(response) => {
                        // handle all choices
                        for choice in response.choices {
                            let usage = response.usage.as_ref().map(|usage| TokenUsage {
                                total_tokens: usage.total_tokens,
                                prompt_tokens: usage.prompt_tokens,
                                completion_tokens: usage.completion_tokens,
                                tokens_per_second: 0.0,
                            });

                            let tool_calls = choice.delta.tool_calls.as_ref().map(|calls| {
                                calls
                                    .iter()
                                    .map(|call| ToolCallDeclaration {
                                        index: call.index,
                                        id: call.id.clone().unwrap_or_default(),
                                        name: call.function.name.clone().unwrap_or_default(),
                                        arguments: call.function.arguments.clone(),
                                        results: None,
                                    })
                                    .collect::<Vec<ToolCallDeclaration>>()
                            });

                            // Push a chunk for the delta content and usage
                            chunks.push(StreamChunk {
                                reasoning_content: choice.delta.reasoning_content,
                                content: choice.delta.content,
                                usage,
                                msg_type: choice
                                    .delta
                                    .msg_type
                                    .and_then(|t| MessageType::from_str(&t)),
                                tool_calls: tool_calls,
                                finish_reason: choice.finish_reason.clone(),
                            });
                        }
                    }
                    Err(e) => {
                        if let Ok(json) = serde_json::from_str::<Value>(data) {
                            if let Some(error) = json.get("error") {
                                let emsg =
                                    error["message"].as_str().map(String::from).unwrap_or_else(
                                        || t!("network.stream.unknown_openai_error").to_string(),
                                    );
                                return Err(emsg);
                            }
                        }
                        log::error!("Failed to parse OpenAI response: {}, error:{}", data, e);
                    }
                }
            }
        }
        Ok(chunks)
    }

    /// Parse Google AI (Gemini) format
    fn parse_gemini(chunk: Bytes) -> Result<Vec<StreamChunk>, String> {
        let chunk_str = String::from_utf8_lossy(&chunk).into_owned();
        // Gemini stream format is "data: {json}\r\n"
        // Non-stream format is just "{json}"
        let mut chunks = Vec::new();
        // log::debug!("gemini - {}", chunk_str);

        for line in chunk_str.lines() {
            if line.starts_with("data:") {
                let data = line["data:".len()..].trim();
                match serde_json::from_str::<GeminiResponse>(data) {
                    Ok(gemini_response) => {
                        let usage =
                            gemini_response
                                .usage_metadata
                                .map(|usage_metadata| TokenUsage {
                                    total_tokens: usage_metadata.total_token_count,
                                    prompt_tokens: usage_metadata.prompt_token_count,
                                    completion_tokens: usage_metadata.total_token_count
                                        - usage_metadata.prompt_token_count,
                                    tokens_per_second: 0.0,
                                });

                        if let Some(candidates) = gemini_response.candidates {
                            for candidate in candidates.into_iter() {
                                let mut tool_calls_in_chunk: Vec<ToolCallDeclaration> = Vec::new();
                                let mut text_content: Option<String> = None;

                                for (part_index, part) in
                                    candidate.content.parts.into_iter().enumerate()
                                {
                                    // Check if the part is a function call
                                    // Note: Gemini's Part struct needs to be updated to handle functionCall
                                    // Assuming Part now has an optional functionCall field (it does)
                                    if let Some(func_call) = part.function_call {
                                        // Serialize args Value to String
                                        let args_string = serde_json::to_string(&func_call.args).map_err(|e| {
                                            t!("network.stream.gemini_tool_arg_serialization_error", error = e.to_string())
                                                .to_string()
                                        })?;

                                        tool_calls_in_chunk.push(ToolCallDeclaration {
                                            // Use a combination of candidate and part index for uniqueness if needed,
                                            // but simple part_index might suffice if only one candidate is streamed.
                                            // Let's use part_index for now, assuming single candidate stream.
                                            index: part_index as u32,
                                            id: String::new(), // Gemini stream delta doesn't provide ID
                                            name: func_call.name,
                                            arguments: Some(args_string),
                                            results: None,
                                        });
                                    } else if let Some(text_val) = part.text {
                                        if !text_val.is_empty() {
                                            // Accumulate text content from text parts
                                            text_content =
                                                Some(text_content.unwrap_or_default() + &text_val);
                                        }
                                    }
                                }

                                let msg_type = if text_content.is_some() {
                                    Some(MessageType::Text)
                                } else if !tool_calls_in_chunk.is_empty() {
                                    Some(MessageType::ToolCall)
                                } else {
                                    None
                                };
                                // Push a chunk for this candidate's content and tool calls
                                chunks.push(StreamChunk {
                                    reasoning_content: None, // Gemini doesn't have explicit reasoning_content in this format
                                    content: text_content,
                                    usage: usage.clone(),
                                    msg_type,
                                    tool_calls: if !tool_calls_in_chunk.is_empty() {
                                        Some(tool_calls_in_chunk)
                                    } else {
                                        None
                                    },
                                    finish_reason: candidate.finish_reason.clone(),
                                });
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to parse Gemini response: {}, error:{}", data, e);
                    }
                }
            } else if !line.is_empty() {
                // Handle non-stream response (full JSON object)
                // Non-stream response
                match serde_json::from_str::<Value>(&chunk_str) {
                    Ok(json) => {
                        // Extract content
                        let content = json["candidates"][0]["content"]["parts"][0]["text"]
                            .as_str()
                            .map(String::from);

                        // Extract tool calls from non-stream response
                        let tool_calls: Option<Vec<ToolCallDeclaration>> = json["candidates"][0]
                            ["content"]["parts"]
                            .as_array()
                            .map(|parts| {
                                parts
                                    .iter()
                                    .filter_map(|part| {
                                        part.get("functionCall")
                                            .and_then(|func_call_val| {
                                                serde_json::from_value::<GeminiFunctionCall>(
                                                    func_call_val.clone(),
                                                )
                                                .ok()
                                            })
                                            .map(|func_call| {
                                                // Serialize args Value to String
                                                let args_string =
                                                    serde_json::to_string(&func_call.args)
                                                        .unwrap_or_default();
                                                ToolCallDeclaration {
                                                    index: 0,          // Non-stream doesn't have index per part, use 0 or generate
                                                    id: String::new(), // Non-stream doesn't provide ID
                                                    name: func_call.name,
                                                    arguments: Some(args_string),
                                                    results: None,
                                                }
                                            })
                                    })
                                    .collect()
                            });

                        // Extract token usage
                        let usage = if let Some(usage) = json.get("usageMetadata") {
                            Some(TokenUsage {
                                total_tokens: usage["totalTokenCount"].as_u64().unwrap_or_default(),
                                prompt_tokens: usage["promptTokenCount"]
                                    .as_u64()
                                    .unwrap_or_default(),
                                completion_tokens: usage["candidateTokenCount"]
                                    .as_u64()
                                    .unwrap_or_default(),
                                tokens_per_second: 0.0,
                            })
                        } else {
                            None
                        };

                        chunks.push(StreamChunk {
                            reasoning_content: None,
                            content,
                            usage,
                            msg_type: if tool_calls.is_some() {
                                Some(MessageType::ToolCall)
                            } else {
                                Some(MessageType::Finished)
                            }, // Determine message type based on content or tool calls
                            tool_calls: None,
                            finish_reason: None,
                        });
                    }
                    Err(e) => {
                        log::error!(
                            "Failed to parse Gemini non-stream response: {}, error: {}",
                            chunk_str,
                            e
                        );
                        return Err(
                            t!("network.stream.gemini_parse_error", error = e.to_string())
                                .to_string(),
                        );
                    }
                }
            }
        }

        Ok(chunks)
    }

    /// Parse standard SSE format
    fn parse_standard_sse(chunk: Bytes) -> Result<Vec<StreamChunk>, String> {
        let chunk_str = String::from_utf8_lossy(&chunk).into_owned();
        let mut chunks = Vec::new();

        for line in chunk_str.lines() {
            if line.starts_with("event: done") {
                chunks.push(StreamChunk {
                    reasoning_content: None,
                    content: None,
                    usage: None,
                    msg_type: Some(MessageType::Finished),
                    tool_calls: None,
                    finish_reason: None,
                });
                continue;
            }
            if line.starts_with("data:") {
                let data = line["data:".len()..].trim();
                chunks.push(StreamChunk {
                    reasoning_content: None,
                    content: Some(data.to_string()),
                    usage: None,
                    msg_type: None,
                    tool_calls: None,
                    finish_reason: None,
                });
            }
        }
        Ok(chunks)
    }

    /// Parse Claude format with full event streaming support
    fn parse_claude(chunk: Bytes) -> Result<Vec<StreamChunk>, String> {
        let chunk_str = String::from_utf8_lossy(&chunk).into_owned();
        let mut stream_chunks = Vec::new();
        // log::debug!("claude raw event block: {}", chunk_str); // For debugging the whole event block

        // The StreamProcessor provides a full SSE message (event lines + data lines) as one `chunk`.
        // We need to parse the `data:` line which contains the JSON payload.
        // Claude's `data:` payload itself contains a `type` field that indicates the event type.
        let mut data_json_str: Option<String> = None;

        for line in chunk_str.lines() {
            if line.starts_with("data:") {
                data_json_str = Some(line["data:".len()..].trim().to_string());
                break; // Assuming only one data line per event block from StreamProcessor
            }
        }

        if data_json_str.is_none() {
            // This might happen for empty lines or malformed events, can be ignored or logged.
            // log::warn!("Claude stream: No data field found in event block: {}", chunk_str);
            return Ok(stream_chunks); // Return empty if no data
        }

        let data_str = data_json_str.unwrap();
        if data_str.is_empty() {
            return Ok(stream_chunks); // Return empty if data is empty
        }

        match serde_json::from_str::<ClaudeStreamEvent>(&data_str) {
            Ok(event) => {
                // log::debug!("Parsed Claude event: {:?}", event); // For detailed event logging
                match event.event_type {
                    ClaudeEventType::MessageStart => {
                        // Potentially extract input_tokens from event.message.usage if needed
                        // log::debug!("Claude MessageStart: {:?}", event.message);
                    }
                    ClaudeEventType::ContentBlockStart => {
                        if let (Some(index), Some(content_block)) =
                            (event.index, event.content_block)
                        {
                            if content_block.block_type == "tool_use" {
                                let tool_call = ToolCallDeclaration {
                                    index,
                                    id: content_block.id.unwrap_or_default(),
                                    name: content_block.name.unwrap_or_default(),
                                    arguments: Some(String::new()), // Initialize for accumulation
                                    results: None,
                                };
                                stream_chunks.push(StreamChunk {
                                    reasoning_content: None,
                                    content: None,
                                    usage: None,
                                    msg_type: Some(MessageType::ToolCall),
                                    tool_calls: Some(vec![tool_call]),
                                    finish_reason: None,
                                });
                            }
                        }
                    }
                    ClaudeEventType::ContentBlockDelta => {
                        if let (Some(index), Some(delta_value)) = (event.index, event.delta) {
                            if let Some(delta_type) =
                                delta_value.get("type").and_then(Value::as_str)
                            {
                                match delta_type {
                                    "text_delta" => {
                                        if let Some(text) =
                                            delta_value.get("text").and_then(Value::as_str)
                                        {
                                            if !text.is_empty() {
                                                stream_chunks.push(StreamChunk {
                                                    reasoning_content: None,
                                                    content: Some(text.to_string()),
                                                    usage: None,
                                                    msg_type: Some(MessageType::Text),
                                                    tool_calls: None,
                                                    finish_reason: None,
                                                });
                                            }
                                        }
                                    }
                                    "input_json_delta" => {
                                        if let Some(partial_json) =
                                            delta_value.get("partial_json").and_then(Value::as_str)
                                        {
                                            if !partial_json.is_empty() {
                                                let tool_call_delta = ToolCallDeclaration {
                                                    index,
                                                    id: String::new(), // ID/Name are in ContentBlockStart
                                                    name: String::new(),
                                                    arguments: Some(partial_json.to_string()),
                                                    results: None,
                                                };
                                                stream_chunks.push(StreamChunk {
                                                    reasoning_content: None,
                                                    content: None,
                                                    usage: None,
                                                    msg_type: Some(MessageType::ToolCall),
                                                    tool_calls: Some(vec![tool_call_delta]),
                                                    finish_reason: None,
                                                });
                                            }
                                        }
                                    }
                                    _ => {
                                        // log::debug!("Unhandled Claude content_block_delta type: {}", delta_type);
                                    }
                                }
                            }
                        }
                    }
                    ClaudeEventType::ContentBlockStop => {
                        // Signals end of a content block.
                        // Arguments/text already streamed.
                        // log::debug!("Claude ContentBlockStop for index: {:?}", event.index);
                    }
                    ClaudeEventType::MessageDelta => {
                        let mut usage_chunk: Option<TokenUsage> = None;
                        let mut finish_reason_chunk: Option<String> = None;

                        if let Some(usage_val) = event.usage {
                            // MessageDelta typically only contains output_tokens
                            let output_tokens = usage_val
                                .get("output_tokens")
                                .and_then(Value::as_u64)
                                .unwrap_or(0);
                            if output_tokens > 0 {
                                // Only create usage if there are tokens
                                usage_chunk = Some(TokenUsage {
                                    total_tokens: output_tokens, // Will be summed up later with prompt_tokens
                                    prompt_tokens: 0, // Prompt tokens are in message_start or message_stop
                                    completion_tokens: output_tokens,
                                    tokens_per_second: 0.0, // Calculated later
                                });
                            }
                        }

                        if let Some(delta_value) = event.delta {
                            if let Some(stop_reason) =
                                delta_value.get("stop_reason").and_then(Value::as_str)
                            {
                                finish_reason_chunk = Some(stop_reason.to_string());
                                // If stop_reason is "tool_use", this is the primary signal.
                            }
                        }

                        if usage_chunk.is_some() || finish_reason_chunk.is_some() {
                            stream_chunks.push(StreamChunk {
                                reasoning_content: None,
                                content: None,
                                usage: usage_chunk,
                                msg_type: None, // This chunk is for metadata/signal
                                tool_calls: None,
                                finish_reason: finish_reason_chunk,
                            });
                        }
                    }
                    ClaudeEventType::MessageStop => {
                        // Message stream finished.
                        // `event.message` might contain final `id`, `role`, `model`, `stop_reason`, `stop_sequence`, and `usage` (with both input_tokens and output_tokens).
                        // We can extract final usage here if needed.
                        let mut final_usage: Option<TokenUsage> = None;
                        if let Some(message_val) = event.message {
                            if let Some(usage_val) = message_val.get("usage") {
                                let input_tokens = usage_val
                                    .get("input_tokens")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(0);
                                let output_tokens = usage_val
                                    .get("output_tokens")
                                    .and_then(Value::as_u64)
                                    .unwrap_or(0);
                                final_usage = Some(TokenUsage {
                                    total_tokens: input_tokens + output_tokens,
                                    prompt_tokens: input_tokens,
                                    completion_tokens: output_tokens,
                                    tokens_per_second: 0.0, // Calculated later
                                });
                            }
                        }

                        stream_chunks.push(StreamChunk {
                            reasoning_content: None,
                            content: None,
                            usage: final_usage,
                            msg_type: Some(MessageType::Finished),
                            tool_calls: None,
                            finish_reason: None, // Actual finish_reason (like "tool_use" or "end_turn") is in message.stop_reason or earlier message_delta
                        });
                    }
                    ClaudeEventType::Ping => {
                        // Keep-alive, can be ignored.
                    }
                    ClaudeEventType::Error => {
                        if let Some(error_val) = event.error {
                            let error_type = error_val
                                .get("type")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown_error");
                            let error_message = error_val
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("Unknown error from Claude");
                            log::error!(
                                "Claude API Error - type: {}, message: {}", // Log in English
                                error_type,
                                error_message
                            );
                            return Err(t!(
                                "network.stream.claude_api_error_format",
                                message = error_message
                            )
                            .to_string());
                        } else {
                            log::error!(
                                "Claude Error event with no error details in stream: {}", // Log in English
                                data_str
                            );
                            return Err(
                                t!("network.stream.unknown_claude_error_no_details").to_string()
                            );
                        }
                    }
                }
            }
            Err(e) => {
                // It's possible to receive non-JSON data or malformed JSON.
                // Decide whether to error out or log and continue.
                // For Claude, data should always be JSON.
                log::error!(
                    "Failed to parse Claude event JSON: {}, error: {}",
                    data_str,
                    e
                );
            }
        }
        Ok(stream_chunks)
    }
}
