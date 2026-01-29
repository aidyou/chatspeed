use bytes::Bytes;
use serde_json::Value;

use super::OpenAIStreamResponse;
use crate::ai::traits::chat::{MessageType, ToolCallDeclaration};

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
    /// The role of the chunk.
    #[allow(unused)]
    pub role: Option<String>,
    /// The reasoning content of the chunk.
    pub reasoning_content: Option<String>,
    /// The content of the chunk.
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
    /// Parse OpenAI compatible format
    pub fn parse_openai(chunk: Bytes) -> Result<Vec<StreamChunk>, String> {
        // Use from_utf8_lossy instead of from_utf8 to continue processing when encountering invalid UTF-8 sequences
        // This will replace invalid UTF-8 sequences with replacement characters (U+FFFD) instead of returning an error
        let chunk_str = String::from_utf8_lossy(&chunk).into_owned();
        let mut chunks = Vec::new();

        // #[cfg(debug_assertions)]
        // log::debug!("openai stream chunk- {}", chunk_str);

        for line in chunk_str.lines() {
            // log::debug!("openai stream line- {}", line);
            if line.starts_with("data:") {
                let data = line["data:".len()..].trim();
                if data == "[DONE]" {
                    chunks.push(StreamChunk {
                        role: None,
                        reasoning_content: None,
                        content: None,
                        usage: None,
                        msg_type: Some(MessageType::Finished),
                        tool_calls: None,
                        finish_reason: None, // [DONE] is a stream protocol signal, not a message finish_reason.
                                             // The MessageType::Finished itself indicates the end.
                    });
                    continue;
                }
                if data.starts_with("{\"error\"") && data.contains("message") {
                    let error = if let Ok(json) = serde_json::from_str::<Value>(data) {
                        if let Some(error) = json.get("error") {
                            serde_json::to_string_pretty(error).unwrap_or(error.to_string())
                        } else {
                            serde_json::to_string_pretty(&json).unwrap_or(json.to_string())
                        }
                    } else {
                        data.to_string()
                    };
                    chunks.push(StreamChunk {
                        role: None,
                        reasoning_content: None,
                        content: Some(error),
                        usage: None,
                        msg_type: Some(MessageType::Error),
                        tool_calls: None,
                        finish_reason: Some("stop".to_string()),
                    });
                    break;
                } else {
                    match serde_json::from_str::<OpenAIStreamResponse>(data) {
                        Ok(response) => {
                            for choice in response.choices {
                                let usage = response.usage.as_ref().map(|usage| TokenUsage {
                                    total_tokens: usage.total_tokens,
                                    prompt_tokens: usage.prompt_tokens,
                                    completion_tokens: usage.completion_tokens,
                                    tokens_per_second: 0.0,
                                });

                                // `choice.delta` is OpenAIStreamDelta
                                let delta = choice.delta;

                                let tool_calls = delta.tool_calls.as_ref().map(|calls| {
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

                                let chunk_content = delta
                                    .reference
                                    .as_ref()
                                    .filter(|r| !r.is_empty())
                                    .or(delta.content.as_ref())
                                    .cloned();

                                let mut chunk_msg_type: Option<MessageType> = None;

                                // Check for role from the delta
                                if let Some(internal_msg_type_str) = &delta.msg_type {
                                    // Try to parse the internal_msg_type_str as MessageType
                                    // There may be extended types like "reference", "think", etc.,
                                    // so the message type must be parsed first,
                                    chunk_msg_type = MessageType::from_str(internal_msg_type_str);
                                } else if chunk_content.is_some() {
                                    // If no role, but content is present, it's a Text chunk.
                                    chunk_msg_type = Some(MessageType::Text);
                                }

                                chunks.push(StreamChunk {
                                    role: delta.role.clone(),
                                    reasoning_content: delta.reasoning_content,
                                    content: chunk_content,
                                    usage,
                                    // If tool_calls are present, they take precedence for msg_type
                                    msg_type: chunk_msg_type,
                                    tool_calls,
                                    finish_reason: choice.finish_reason.clone(),
                                });
                            }
                        }
                        Err(e) => {
                            let error = if let Ok(json) = serde_json::from_str::<Value>(data) {
                                if let Some(error) = json.get("error") {
                                    serde_json::to_string_pretty(error).unwrap_or(error.to_string())
                                } else {
                                    serde_json::to_string_pretty(&json).unwrap_or(json.to_string())
                                }
                            } else {
                                data.to_string()
                            };
                            chunks.push(StreamChunk {
                                role: None,
                                reasoning_content: None,
                                content: Some(error),
                                usage: None,
                                msg_type: Some(MessageType::Error),
                                tool_calls: None,
                                finish_reason: Some("stop".to_string()),
                            });
                            log::error!("Failed to parse OpenAI response: {}, error:{}", data, e);
                            break;
                        }
                    }
                }
            }
        }
        Ok(chunks)
    }
}
