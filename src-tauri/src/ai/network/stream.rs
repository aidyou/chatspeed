use bytes::Bytes;
use rust_i18n::t;
use serde_json::Value;

use super::{ClaudeEventType, ClaudeStreamEvent, GeminiResponse, OpenAIStreamResponse};
use crate::ai::traits::chat::{MessageType, ToolCallDeclaration};
use crate::ccproxy::StreamFormat;

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
    /// Parse a chunk of stream data according to the specified format
    pub async fn parse_chunk(
        chunk: Bytes,
        format: &StreamFormat,
    ) -> Result<Vec<StreamChunk>, String> {
        match format {
            StreamFormat::OpenAI => Self::parse_openai(chunk),
            StreamFormat::Gemini => Self::parse_gemini(chunk),
            StreamFormat::Claude => Self::parse_claude(chunk),
        }
    }

    /// Parse OpenAI compatible format
    fn parse_openai(chunk: Bytes) -> Result<Vec<StreamChunk>, String> {
        // 使用 from_utf8_lossy 替代 from_utf8，以便在遇到无效的 UTF-8 序列时能够继续处理
        // 这将用替换字符 (U+FFFD) 替代无效的 UTF-8 序列，而不是返回错误
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

                                let chunk_content: Option<String> = delta.content.clone();
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
                                    msg_type: if tool_calls.is_some() {
                                        Some(MessageType::ToolCall)
                                    } else {
                                        chunk_msg_type
                                    },
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

    /// Parse Google AI (Gemini) format
    fn parse_gemini(chunk: Bytes) -> Result<Vec<StreamChunk>, String> {
        let chunk_str = String::from_utf8_lossy(&chunk).into_owned();
        let mut stream_chunks = Vec::new();

        #[cfg(debug_assertions)]
        log::debug!("gemini stream chunk- {}", chunk_str);

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
                                let mut text_content_parts: Vec<String> = Vec::new(); // Collect text parts
                                let mut candidate_role: Option<String> = None;

                                // Gemini's `candidate.content.role` field.
                                if let Some(role_str) = candidate.content.role {
                                    candidate_role = Some(if role_str == "model" {
                                        "assistant".to_string()
                                    } else {
                                        role_str
                                    });
                                }

                                if let Some(parts) = candidate.content.parts.as_ref() {
                                    for (part_index, part) in parts.into_iter().enumerate() {
                                        // Check if the part is a function call
                                        // Note: Gemini's Part struct needs to be updated to handle functionCall
                                        // Assuming Part now has an optional functionCall field (it does)
                                        if let Some(func_call) = &part.function_call {
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
                                                name: func_call.name.clone(),
                                                arguments: Some(args_string),
                                                results: None,
                                            });
                                        } else if let Some(text_val) = &part.text {
                                            if !text_val.is_empty() {
                                                text_content_parts.push(text_val.clone());
                                            }
                                        }
                                    }
                                }

                                let combined_text_content = if text_content_parts.is_empty() {
                                    None
                                } else {
                                    Some(text_content_parts.join(""))
                                };

                                // Emit text content chunk if present
                                if let Some(text_str) = combined_text_content {
                                    stream_chunks.push(StreamChunk {
                                        role: candidate_role.clone(),
                                        reasoning_content: None,
                                        content: Some(text_str),
                                        usage: usage.clone(),
                                        msg_type: Some(MessageType::Text),
                                        tool_calls: None,
                                        finish_reason: None, // Text itself isn't a finish reason for the stream part
                                    });
                                }

                                // Emit tool calls chunk if present
                                if !tool_calls_in_chunk.is_empty() {
                                    stream_chunks.push(StreamChunk {
                                        role: candidate_role.clone(),
                                        reasoning_content: None,
                                        content: None, // Tool calls are primary here
                                        usage: usage.clone(),
                                        msg_type: Some(MessageType::ToolCall),
                                        tool_calls: Some(tool_calls_in_chunk),
                                        finish_reason: None, // Tool call itself isn't a finish reason for the stream part
                                    });
                                }

                                // If there's a finish_reason for the candidate, emit a separate chunk for it
                                // This is important for signaling the end of the candidate's contribution.
                                if candidate.finish_reason.is_some() {
                                    stream_chunks.push(StreamChunk {
                                        role: candidate_role.clone(),
                                        reasoning_content: None,
                                        content: None,
                                        usage: usage.clone(), // Usage can be associated with the finish
                                        msg_type: Some(MessageType::Finished), // Or determine based on finish_reason
                                        tool_calls: None,
                                        finish_reason: candidate.finish_reason.clone(),
                                    });
                                }
                            }
                        } else if gemini_response.prompt_feedback.is_some() {
                            log::warn!("Gemini response without candidates, but with prompt_feedback: {:?}", gemini_response.prompt_feedback);
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to parse Gemini response: {}, error:{}", data, e);
                    }
                }
            } else if !line.is_empty() {
                // Handle non-stream response (full JSON object)
                match serde_json::from_str::<GeminiResponse>(&chunk_str) {
                    Ok(gemini_response) => {
                        if let Some(candidates) = gemini_response.candidates {
                            for candidate in candidates.into_iter() {
                                let mut current_content: Option<String> = None;
                                let mut current_role: Option<String> = None;
                                if let Some(role_str) = candidate.content.role {
                                    current_role = Some(if role_str == "model" {
                                        "assistant".to_string()
                                    } else {
                                        role_str
                                    });
                                }

                                let mut tool_calls_list: Vec<ToolCallDeclaration> = Vec::new();
                                let mut text_parts_content = String::new();

                                if let Some(parts) = candidate.content.parts {
                                    for (part_idx, part) in parts.into_iter().enumerate() {
                                        if let Some(func_call) = part.function_call {
                                            let args_string =
                                                serde_json::to_string(&func_call.args)
                                                    .unwrap_or_default();
                                            tool_calls_list.push(ToolCallDeclaration {
                                                index: part_idx as u32,
                                                id: String::new(),
                                                name: func_call.name,
                                                arguments: Some(args_string),
                                                results: None,
                                            });
                                        } else if let Some(text_val) = part.text {
                                            text_parts_content.push_str(&text_val);
                                        }
                                    }
                                }
                                if !text_parts_content.is_empty() {
                                    current_content = Some(text_parts_content);
                                }

                                let usage_data =
                                    gemini_response.usage_metadata.clone().map(|um| TokenUsage {
                                        total_tokens: um.total_token_count,
                                        prompt_tokens: um.prompt_token_count,
                                        completion_tokens: um.candidates_token_count.unwrap_or(
                                            um.total_token_count - um.prompt_token_count,
                                        ),
                                        tokens_per_second: 0.0,
                                    });

                                if let Some(content_str) = current_content {
                                    stream_chunks.push(StreamChunk {
                                        role: current_role.clone(),
                                        reasoning_content: None,
                                        content: Some(content_str),
                                        usage: usage_data.clone(),
                                        msg_type: Some(MessageType::Text),
                                        tool_calls: None,
                                        finish_reason: None,
                                    });
                                }
                                if !tool_calls_list.is_empty() {
                                    stream_chunks.push(StreamChunk {
                                        role: current_role.clone(),
                                        reasoning_content: None,
                                        content: None,
                                        usage: usage_data.clone(),
                                        msg_type: Some(MessageType::ToolCall),
                                        tool_calls: Some(tool_calls_list),
                                        finish_reason: None,
                                    });
                                }
                                if candidate.finish_reason.is_some() {
                                    stream_chunks.push(StreamChunk {
                                        role: current_role.clone(),
                                        reasoning_content: None,
                                        content: None,
                                        usage: usage_data,
                                        msg_type: Some(MessageType::Finished),
                                        tool_calls: None,
                                        finish_reason: candidate.finish_reason,
                                    });
                                }
                            }
                        }
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
                break;
            }
        }
        Ok(stream_chunks)
    }

    /// Parse Claude format with full event streaming support
    fn parse_claude(chunk: Bytes) -> Result<Vec<StreamChunk>, String> {
        let chunk_str = String::from_utf8_lossy(&chunk).into_owned();
        let mut stream_chunks = Vec::new();
        let mut data_json_str: Option<String> = None;

        log::debug!("claude chunk: {}", chunk_str);

        for line in chunk_str.lines() {
            if line.starts_with("data:") {
                data_json_str = Some(line["data:".len()..].trim().to_string());
                break;
            }
        }

        if data_json_str.is_none() {
            return Ok(stream_chunks);
        }

        let data_str = data_json_str.unwrap_or_default();
        if data_str.is_empty() {
            return Ok(stream_chunks);
        }

        match serde_json::from_str::<ClaudeStreamEvent>(&data_str) {
            Ok(event) => {
                match event.event_type {
                    ClaudeEventType::MessageStart => {
                        if let Some(message_val) = event.message {
                            let role_from_event = message_val
                                .get("role")
                                .and_then(Value::as_str)
                                .map(|role_str| String::from(role_str));
                            if let Some(usage_val) = message_val.get("usage") {
                                if let Some(input_tokens) =
                                    usage_val.get("input_tokens").and_then(Value::as_u64)
                                {
                                    stream_chunks.push(StreamChunk {
                                        role: role_from_event,
                                        reasoning_content: None,
                                        content: None,
                                        usage: Some(TokenUsage {
                                            prompt_tokens: input_tokens,
                                            completion_tokens: 0,
                                            total_tokens: input_tokens,
                                            tokens_per_second: 0.0,
                                        }),
                                        msg_type: None,
                                        tool_calls: None,
                                        finish_reason: None,
                                    });
                                }
                            }
                        }
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
                                    arguments: Some(String::new()),
                                    results: None,
                                };
                                stream_chunks.push(StreamChunk {
                                    role: Some("assistant".to_string()), // Assistant is initiating a tool use
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
                        if let (Some(_index), Some(delta_value)) = (event.index, event.delta) {
                            // index might be useful for tool arg aggregation
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
                                                    role: Some("assistant".to_string()), // Assistant is providing text
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
                                                // Create a ToolCallDeclaration part just for the arguments delta
                                                // The `index` comes from the event.index
                                                let tool_arg_delta = ToolCallDeclaration {
                                                    index: event.index.unwrap_or(0), // Use the index from the event
                                                    id: String::new(), // ID is not in delta, was in ContentBlockStart
                                                    name: String::new(), // Name is not in delta, was in ContentBlockStart
                                                    arguments: Some(partial_json.to_string()),
                                                    results: None,
                                                };
                                                stream_chunks.push(StreamChunk {
                                                    role: Some("assistant".to_string()), // Assistant is providing tool arguments
                                                    reasoning_content: None,
                                                    content: None, // Do not put partial_json in content
                                                    usage: None,
                                                    msg_type: None, // Not a standalone message type for UI
                                                    tool_calls: Some(vec![tool_arg_delta]), // Pass the delta via tool_calls
                                                    finish_reason: None,
                                                });
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    ClaudeEventType::ContentBlockStop => {}
                    ClaudeEventType::MessageDelta => {
                        let mut usage_chunk_opt: Option<TokenUsage> = None;
                        let mut finish_reason_chunk: Option<String> = None;

                        if let Some(usage_val) = event.usage {
                            let output_tokens = usage_val
                                .get("output_tokens")
                                .and_then(Value::as_u64)
                                .unwrap_or(0);
                            if output_tokens > 0 {
                                usage_chunk_opt = Some(TokenUsage {
                                    total_tokens: output_tokens,
                                    prompt_tokens: 0,
                                    completion_tokens: output_tokens,
                                    tokens_per_second: 0.0,
                                });
                            }
                        }

                        if let Some(delta_value) = event.delta {
                            if let Some(stop_reason) =
                                delta_value.get("stop_reason").and_then(Value::as_str)
                            {
                                let stop_sequence_val =
                                    delta_value.get("stop_sequence").and_then(Value::as_str);
                                // Check if this stop_reason indicates a tool_use via stop_sequence
                                if stop_reason == "stop_sequence"
                                    && stop_sequence_val.map_or(false, |s| {
                                        s.contains("tool_calls") || s.contains("function_calls")
                                    })
                                {
                                    // This indicates a tool call is expected or in progress.
                                    // We will set the finish_reason, but the msg_type should not be Finished.
                                    // The upstream logic will use this finish_reason to decide if it's a tool_use scenario.
                                    log::debug!("Claude MessageDelta stop_sequence indicates tool_use. Reason: {}, Sequence: {:?}", stop_reason, stop_sequence_val);
                                    finish_reason_chunk =
                                        Some("tool_use_via_stop_sequence".to_string());
                                // Special marker
                                } else {
                                    finish_reason_chunk = Some(stop_reason.to_string());
                                }
                            }
                        }

                        if usage_chunk_opt.is_some() || finish_reason_chunk.is_some() {
                            stream_chunks.push(StreamChunk {
                                role: Some("assistant".to_string()), // Updates related to the assistant's message
                                reasoning_content: None,
                                content: None,
                                usage: usage_chunk_opt,
                                msg_type: if finish_reason_chunk.as_deref()
                                    == Some("tool_use_via_stop_sequence")
                                {
                                    // If it's a tool_use indicator, don't mark as Finished.
                                    // The actual tool_use content block will determine the MessageType::ToolCall.
                                    None
                                } else if finish_reason_chunk.is_some() {
                                    Some(MessageType::Finished) // For other stop reasons like "end_turn"
                                } else {
                                    None
                                },
                                tool_calls: None,
                                finish_reason: finish_reason_chunk,
                            });
                        }
                    }
                    ClaudeEventType::MessageStop => {
                        let mut final_usage: Option<TokenUsage> = None;
                        let mut final_finish_reason: Option<String> = None;
                        let mut role_from_event: Option<String> = None;

                        if let Some(message_val) = event.message {
                            role_from_event = message_val
                                .get("role")
                                .and_then(Value::as_str)
                                .map(String::from);

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
                                    tokens_per_second: 0.0,
                                });
                            }
                            if let Some(stop_reason) =
                                message_val.get("stop_reason").and_then(Value::as_str)
                            {
                                let stop_sequence_val =
                                    message_val.get("stop_sequence").and_then(Value::as_str);
                                if stop_reason == "stop_sequence"
                                    && stop_sequence_val.map_or(false, |s| {
                                        s.contains("tool_calls") || s.contains("function_calls")
                                    })
                                {
                                    log::debug!("Claude MessageStop stop_sequence indicates tool_use. Reason: {}, Sequence: {:?}", stop_reason, stop_sequence_val);
                                    final_finish_reason =
                                        Some("tool_use_via_stop_sequence".to_string());
                                // Special marker
                                } else {
                                    final_finish_reason = Some(stop_reason.to_string());
                                }
                            }
                        }

                        // If role_from_event is None (e.g. event.message was not present or lacked role),
                        // default to "assistant" as MessageStop concludes the assistant's turn.
                        let role_for_chunk =
                            role_from_event.or_else(|| Some("assistant".to_string()));

                        stream_chunks.push(StreamChunk {
                            role: role_for_chunk,
                            reasoning_content: None,
                            content: None,
                            usage: final_usage,
                            msg_type: if final_finish_reason.as_deref()
                                == Some("tool_use_via_stop_sequence")
                                || final_finish_reason.as_deref() == Some("tool_use")
                            {
                                None // Let the tool_call event itself define the message type, or upstream logic handles it.
                            } else {
                                Some(MessageType::Finished) // For "end_turn", etc.
                            },
                            tool_calls: None,
                            finish_reason: final_finish_reason,
                        });
                    }
                    ClaudeEventType::Ping => {}
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
                                "Claude API Error - type: {}, message: {}",
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
                                "Claude Error event with no error details in stream: {}",
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
