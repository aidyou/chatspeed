use bytes::Bytes;
use serde_json::Value;
use std::fmt;

use crate::ai::traits::chat::MessageType;

use super::{AnthropicEventType, AnthropicStreamEvent, GeminiResponse, OpenAIStreamResponse};

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
    /// Anthropic format
    /// {"completion":"Hello","usage":{"input_tokens":10,"output_tokens":10}}
    Anthropic,
}

impl fmt::Debug for StreamFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OpenAI => write!(f, "StreamFormat::OpenAI"),
            Self::Gemini => write!(f, "StreamFormat::Gemini"),
            Self::StandardSSE => write!(f, "StreamFormat::StandardSSE"),
            Self::Custom(_) => write!(f, "StreamFormat::Custom(<custom_parser>)"),
            // Self::HuggingFace => write!(f, "StreamFormat::HuggingFace"),
            Self::Anthropic => write!(f, "StreamFormat::Anthropic"),
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
            StreamFormat::Anthropic => Self::parse_anthropic(chunk),
            StreamFormat::Custom(parser) => parser(chunk).map(|content| {
                vec![StreamChunk {
                    reasoning_content: None,
                    content,
                    usage: None,
                    msg_type: None,
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

        for line in chunk_str.lines() {
            if line.starts_with("data:") {
                let data = line["data:".len()..].trim();
                if data == "[DONE]" {
                    chunks.push(StreamChunk {
                        reasoning_content: None,
                        content: None,
                        usage: None,
                        msg_type: Some(MessageType::Finished),
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

                            chunks.push(StreamChunk {
                                reasoning_content: choice.delta.reasoning_content,
                                content: choice.delta.content,
                                usage,
                                msg_type: choice
                                    .delta
                                    .msg_type
                                    .and_then(|t| MessageType::from_str(&t)),
                            });
                        }
                    }
                    Err(e) => {
                        if let Ok(json) = serde_json::from_str::<Value>(data) {
                            if let Some(error) = json.get("error") {
                                let emsg = error["message"]
                                    .as_str()
                                    .map(String::from)
                                    .unwrap_or_else(|| "Unknown Error".to_string());
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
        let mut chunks = Vec::new();

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
                            for candidate in candidates {
                                for part in &candidate.content.parts {
                                    chunks.push(StreamChunk {
                                        reasoning_content: None,
                                        content: Some(part.text.clone()),
                                        usage: usage.clone(),
                                        msg_type: Some(MessageType::Text),
                                    });
                                }
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to parse Gemini response: {}, error:{}", data, e);
                    }
                }
            } else if !line.is_empty() {
                // Non-stream response
                match serde_json::from_str::<Value>(&chunk_str) {
                    Ok(json) => {
                        // Extract content
                        let content = json["candidates"][0]["content"]["parts"][0]["text"]
                            .as_str()
                            .map(String::from);

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
                            msg_type: Some(MessageType::Finished),
                        });
                    }
                    Err(e) => {
                        log::error!(
                            "Failed to parse Gemini non-stream response: {}, error: {}",
                            chunk_str,
                            e
                        );
                        return Err(format!("Failed to parse Gemini response: {}", e));
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
                });
            }
        }
        Ok(chunks)
    }

    /// Parse Anthropic format with full event streaming support
    fn parse_anthropic(chunk: Bytes) -> Result<Vec<StreamChunk>, String> {
        let chunk_str = String::from_utf8_lossy(&chunk).into_owned();
        let mut chunks = Vec::new();
        let mut current_content = String::new();
        let mut current_index = 0;

        for line in chunk_str.lines() {
            if !line.starts_with("event:") || !line.contains("data:") {
                continue;
            }

            let data = match line.find("data:") {
                Some(pos) => line[pos + "data:".len()..].trim(),
                None => {
                    log::error!("Failed to find 'data: ' in line: {}", line);
                    continue;
                }
            };

            match serde_json::from_str::<AnthropicStreamEvent>(data) {
                Ok(event) => {
                    match event.event_type {
                        AnthropicEventType::MessageStart => {
                            // Initialize new message
                            current_content.clear();
                        }
                        AnthropicEventType::ContentBlockStart => {
                            // Start new content block
                            if let Some(index) = event.index {
                                current_index = index;
                            }
                        }
                        AnthropicEventType::ContentBlockDelta => {
                            // handle content block delta
                            if let (Some(index), Some(delta)) = (event.index, event.delta) {
                                match delta["type"].as_str() {
                                    Some("text_delta") => {
                                        if let Some(text) = delta["text"].as_str() {
                                            if index == current_index {
                                                current_content.push_str(text);
                                                chunks.push(StreamChunk {
                                                    reasoning_content: None,
                                                    content: Some(text.to_string()),
                                                    usage: None,
                                                    msg_type: Some(MessageType::Text),
                                                });
                                            }
                                        }
                                    }
                                    Some("thinking_delta") => {
                                        // handle thinking delta
                                        if let Some(thinking) = delta["thinking"].as_str() {
                                            chunks.push(StreamChunk {
                                                reasoning_content: Some(thinking.to_string()),
                                                content: None,
                                                usage: None,
                                                msg_type: Some(MessageType::Reasoning),
                                            });
                                        }
                                    }
                                    _ => {} // 忽略其他delta类型
                                }
                            }
                        }
                        AnthropicEventType::ContentBlockStop => {
                            // Finalize current content block
                            if !current_content.is_empty() {
                                chunks.push(StreamChunk {
                                    reasoning_content: None,
                                    content: Some(current_content.clone()),
                                    usage: None,
                                    msg_type: Some(MessageType::Text),
                                });
                                current_content.clear();
                            }
                        }
                        AnthropicEventType::MessageDelta => {
                            // Handle final message updates
                            if let Some(usage) = event.usage {
                                let token_usage = TokenUsage {
                                    total_tokens: usage["input_tokens"].as_u64().unwrap_or(0)
                                        + usage["output_tokens"].as_u64().unwrap_or(0),
                                    prompt_tokens: usage["input_tokens"].as_u64().unwrap_or(0),
                                    completion_tokens: usage["output_tokens"].as_u64().unwrap_or(0),
                                    tokens_per_second: 0.0,
                                };
                                chunks.push(StreamChunk {
                                    reasoning_content: None,
                                    content: None,
                                    usage: Some(token_usage),
                                    msg_type: Some(MessageType::Finished),
                                });
                            }
                        }
                        AnthropicEventType::MessageStop => {
                            // Finalize message
                            chunks.push(StreamChunk {
                                reasoning_content: None,
                                content: None,
                                usage: None,
                                msg_type: Some(MessageType::Finished),
                            });
                        }
                        AnthropicEventType::Error => {
                            if let Some(error) = event.error {
                                let emsg = error["message"]
                                    .as_str()
                                    .map(String::from)
                                    .unwrap_or_else(|| "Unknown Error".to_string());
                                return Err(emsg);
                            }
                        }
                        _ => {} // Ignore ping and other unknown events
                    }
                }
                Err(e) => {
                    log::error!("Failed to parse Anthropic event: {}, error: {}", data, e);
                }
            }
        }
        Ok(chunks)
    }
}
