use bytes::Bytes;
use serde_json::Value;
use std::fmt;

use crate::ai::traits::chat::MessageType;

use super::GeminiResponse;

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
            StreamFormat::StandardSSE => Self::parse_sse(chunk),
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
                match serde_json::from_str::<Value>(data) {
                    Ok(json) => {
                        // Handle errors first
                        if let Some(error) = json.get("error") {
                            let emsg = error["message"]
                                .as_str()
                                .map(String::from)
                                .unwrap_or("Unknown Error".to_string());
                            return Err(emsg);
                        }

                        // chat content
                        let content = json["choices"][0]["delta"]["content"]
                            .as_str()
                            .map(String::from);

                        // reasoning content
                        let reasoning_content = json["choices"][0]["delta"]["reasoning_content"]
                            .as_str()
                            .map(String::from);

                        // message type
                        let msg_type = json["choices"][0]["delta"]["type"]
                            .as_str()
                            .and_then(|s| MessageType::from_str(s));

                        let usage = if let Some(usage) = json.get("usage") {
                            let total_tokens = usage["total_tokens"].as_u64().unwrap_or_default();
                            let prompt_tokens = usage["prompt_tokens"].as_u64().unwrap_or_default();
                            let completion_tokens =
                                usage["completion_tokens"].as_u64().unwrap_or_default();

                            Some(TokenUsage {
                                total_tokens,
                                prompt_tokens,
                                completion_tokens,
                                tokens_per_second: 0.0,
                            })
                        } else {
                            None
                        };

                        chunks.push(StreamChunk {
                            reasoning_content,
                            content,
                            usage,
                            msg_type,
                        });
                    }
                    Err(e) => {
                        log::error!("Failed to parse OpenAI response: {}, error:{}", data, e);
                        // return Err(e.to_string());
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
                        let usage = TokenUsage {
                            total_tokens: gemini_response.usage_metadata.total_token_count,
                            prompt_tokens: gemini_response.usage_metadata.prompt_token_count,
                            completion_tokens: gemini_response.usage_metadata.total_token_count
                                - gemini_response.usage_metadata.prompt_token_count,
                            tokens_per_second: 0.0,
                        };
                        for candidate in &gemini_response.candidates {
                            for part in &candidate.content.parts {
                                chunks.push(StreamChunk {
                                    reasoning_content: None,
                                    content: Some(part.text.clone()),
                                    usage: Some(usage.clone()),
                                    msg_type: Some(MessageType::Text),
                                });
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
    fn parse_sse(chunk: Bytes) -> Result<Vec<StreamChunk>, String> {
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
                match serde_json::from_str::<Value>(data) {
                    Ok(json) => {
                        let content = json
                            .get("content")
                            .and_then(Value::as_str)
                            .map(String::from);
                        let reasoning_content = json
                            .get("reasoning_content")
                            .and_then(Value::as_str)
                            .map(String::from);
                        let r#type = json
                            .get("type")
                            .and_then(Value::as_str)
                            .and_then(|s| MessageType::from_str(s));

                        let usage = json.get("usage").and_then(|usage| {
                            Some(TokenUsage {
                                total_tokens: usage["total_tokens"].as_u64().unwrap_or_default(),
                                prompt_tokens: usage["prompt_tokens"].as_u64().unwrap_or_default(),
                                completion_tokens: usage["completion_tokens"]
                                    .as_u64()
                                    .unwrap_or_default(),
                                tokens_per_second: 0.0,
                            })
                        });
                        chunks.push(StreamChunk {
                            reasoning_content,
                            content,
                            usage,
                            msg_type: r#type,
                        });
                    }
                    Err(e) => {
                        log::error!("Failed to parse OpenAI response: {}, error:{}", data, e);
                    }
                }
            }
        }
        Ok(chunks)
    }

    // /// Parse HuggingFace format
    // fn parse_huggingface(chunk: Bytes) -> Result<StreamChunk, String> {
    //     let chunk_str = String::from_utf8_lossy(&chunk).into_owned();

    //     if let Ok(json) = serde_json::from_str::<Value>(&chunk_str) {
    //         let content = json["completion"].as_str().map(String::from);
    //         return Ok(StreamChunk {
    //             content,
    //             usage: None, // HuggingFace 不提供 token 统计
    //         });
    //     }
    //     Ok(StreamChunk {
    //         content: None,
    //         usage: None,
    //     })
    // }

    /// Parse Anthropic format
    fn parse_anthropic(chunk: Bytes) -> Result<Vec<StreamChunk>, String> {
        let chunk_str = String::from_utf8_lossy(&chunk).into_owned();
        let mut chunks = Vec::new();

        for line in chunk_str.lines() {
            if line.starts_with("data: ") {
                let data = &line["data: ".len()..];
                if data == "[DONE]" {
                    chunks.push(StreamChunk {
                        reasoning_content: None,
                        content: None,
                        usage: None,
                        msg_type: Some(MessageType::Finished),
                    });
                    continue;
                }
                match serde_json::from_str::<Value>(data) {
                    Ok(json) => {
                        let content = json["completion"].as_str().map(String::from);
                        let usage = json.get("usage").map(|usage| TokenUsage {
                            total_tokens: usage["input_tokens"].as_u64().unwrap_or(0)
                                + usage["output_tokens"].as_u64().unwrap_or(0),
                            prompt_tokens: usage["input_tokens"].as_u64().unwrap_or(0),
                            completion_tokens: usage["output_tokens"].as_u64().unwrap_or(0),
                            tokens_per_second: 0.0,
                        });

                        chunks.push(StreamChunk {
                            reasoning_content: None,
                            content,
                            usage,
                            msg_type: Some(MessageType::Text),
                        });
                    }
                    Err(e) => {
                        log::error!("Failed to parse Anthropic response: {}, error:{}", data, e);
                    }
                }
            }
        }
        Ok(chunks)
    }
}
