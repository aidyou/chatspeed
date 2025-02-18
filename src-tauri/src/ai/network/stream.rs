use bytes::Bytes;
use rust_i18n::t;
use serde_json::Value;
use std::fmt;

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
    // / HuggingFace format
    // HuggingFace,
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
    pub msg_type: Option<String>,
}

/// Stream response parser
pub struct StreamParser;

impl StreamParser {
    /// Parse a chunk of stream data according to the specified format
    pub async fn parse_chunk(chunk: Bytes, format: &StreamFormat) -> Result<StreamChunk, String> {
        match format {
            StreamFormat::OpenAI => Self::parse_openai(chunk),
            StreamFormat::Gemini => Self::parse_gemini(chunk),
            StreamFormat::StandardSSE => Self::parse_sse(chunk),
            StreamFormat::Custom(parser) => parser(chunk).map(|content| StreamChunk {
                reasoning_content: None,
                content,
                usage: None,
                msg_type: None,
            }),
            // StreamFormat::HuggingFace => Self::parse_huggingface(chunk),
            StreamFormat::Anthropic => Self::parse_anthropic(chunk),
        }
    }

    /// Parse OpenAI compatible format
    fn parse_openai(chunk: Bytes) -> Result<StreamChunk, String> {
        let chunk_str = String::from_utf8(chunk.to_vec())
            .map_err(|e| t!("network.stream_decode_error", error = e.to_string()))?;

        for line in chunk_str.lines() {
            if line.starts_with("data: ") {
                let data = &line["data: ".len()..];
                if data == "[DONE]" {
                    return Ok(StreamChunk {
                        reasoning_content: None,
                        content: None,
                        usage: None,
                        msg_type: None,
                    });
                }
                if let Ok(json) = serde_json::from_str::<Value>(data) {
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
                        .map(String::from);

                    let usage = if let Some(usage) = json.get("usage") {
                        Some(TokenUsage {
                            total_tokens: usage["total_tokens"].as_u64().unwrap_or_default(),
                            prompt_tokens: usage["prompt_tokens"].as_u64().unwrap_or_default(),
                            completion_tokens: usage["completion_tokens"]
                                .as_u64()
                                .unwrap_or_default(),
                        })
                    } else {
                        None
                    };

                    return Ok(StreamChunk {
                        reasoning_content,
                        content,
                        usage,
                        msg_type,
                    });
                }
            }
        }
        Ok(StreamChunk {
            reasoning_content: None,
            content: None,
            usage: None,
            msg_type: None,
        })
    }

    /// Parse Google AI (Gemini) format
    fn parse_gemini(chunk: Bytes) -> Result<StreamChunk, String> {
        let chunk_str = String::from_utf8(chunk.to_vec())
            .map_err(|e| t!("network.stream_decode_error", error = e.to_string()))?;

        if let Ok(json) = serde_json::from_str::<Value>(&chunk_str) {
            let content = json["candidates"][0]["content"]["parts"][0]["text"]
                .as_str()
                .map(String::from);

            let usage = json.get("usageMetadata").and_then(|usage| {
                Some(TokenUsage {
                    total_tokens: usage["totalTokenCount"].as_u64()?,
                    prompt_tokens: usage["promptTokenCount"].as_u64()?,
                    completion_tokens: usage["candidatesTokenCount"].as_u64()?,
                })
            });

            return Ok(StreamChunk {
                reasoning_content: None,
                content,
                usage,
                msg_type: None,
            });
        }
        Ok(StreamChunk {
            reasoning_content: None,
            content: None,
            usage: None,
            msg_type: None,
        })
    }

    /// Parse standard SSE format
    fn parse_sse(chunk: Bytes) -> Result<StreamChunk, String> {
        let chunk_str = String::from_utf8(chunk.to_vec())
            .map_err(|e| t!("network.stream_decode_error", error = e.to_string()))?;

        for line in chunk_str.lines() {
            if line.starts_with("event: done") {
                return Ok(StreamChunk {
                    reasoning_content: None,
                    content: None,
                    usage: None,
                    msg_type: None,
                });
            }
            if line.starts_with("data: ") {
                let data = &line["data: ".len()..];
                if let Ok(json) = serde_json::from_str::<Value>(data) {
                    let content = json
                        .get("content")
                        .and_then(Value::as_str)
                        .map(String::from);
                    let reasoning_content = json
                        .get("reasoning_content")
                        .and_then(Value::as_str)
                        .map(String::from);
                    let r#type = json.get("type").and_then(Value::as_str).map(String::from);

                    let usage = json.get("usage").and_then(|usage| {
                        Some(TokenUsage {
                            total_tokens: usage["total_tokens"].as_u64().unwrap_or_default(),
                            prompt_tokens: usage["prompt_tokens"].as_u64().unwrap_or_default(),
                            completion_tokens: usage["completion_tokens"]
                                .as_u64()
                                .unwrap_or_default(),
                        })
                    });
                    return Ok(StreamChunk {
                        reasoning_content,
                        content,
                        usage,
                        msg_type: r#type,
                    });
                }
                return Ok(StreamChunk {
                    reasoning_content: None,
                    content: Some(data.to_string()),
                    usage: None,
                    msg_type: None,
                });
            }
        }
        Ok(StreamChunk {
            reasoning_content: None,
            content: None,
            usage: None,
            msg_type: None,
        })
    }

    // /// Parse HuggingFace format
    // fn parse_huggingface(chunk: Bytes) -> Result<StreamChunk, String> {
    //     let chunk_str = String::from_utf8(chunk.to_vec())
    //         .map_err(|e| t!("network.stream_decode_error", error = e.to_string()))?;

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
    fn parse_anthropic(chunk: Bytes) -> Result<StreamChunk, String> {
        let chunk_str = String::from_utf8(chunk.to_vec())
            .map_err(|e| t!("network.stream_decode_error", error = e.to_string()))?;

        for line in chunk_str.lines() {
            if line.starts_with("data: ") {
                let data = &line["data: ".len()..];
                if data == "[DONE]" {
                    return Ok(StreamChunk {
                        reasoning_content: None,
                        content: None,
                        usage: None,
                        msg_type: None,
                    });
                }
                if let Ok(json) = serde_json::from_str::<Value>(data) {
                    let content = json["completion"].as_str().map(String::from);
                    let usage = json.get("usage").map(|usage| TokenUsage {
                        total_tokens: usage["input_tokens"].as_u64().unwrap_or(0)
                            + usage["output_tokens"].as_u64().unwrap_or(0),
                        prompt_tokens: usage["input_tokens"].as_u64().unwrap_or(0),
                        completion_tokens: usage["output_tokens"].as_u64().unwrap_or(0),
                    });

                    return Ok(StreamChunk {
                        reasoning_content: None,
                        content,
                        usage,
                        msg_type: None,
                    });
                }
            }
        }
        Ok(StreamChunk {
            reasoning_content: None,
            content: None,
            usage: None,
            msg_type: None,
        })
    }
}
