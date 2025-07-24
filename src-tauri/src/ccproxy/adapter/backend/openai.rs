use crate::ccproxy::adapter::backend::update_message_block;
use crate::ccproxy::adapter::unified::SseStatus;
use async_trait::async_trait;
use reqwest::{Client, RequestBuilder};
use serde_json::json;
use std::sync::{Arc, RwLock};

use super::{BackendAdapter, BackendResponse};
use crate::ccproxy::adapter::unified::{
    UnifiedContentBlock, UnifiedRequest, UnifiedResponse, UnifiedRole, UnifiedStreamChunk,
    UnifiedToolChoice, UnifiedUsage,
};
use crate::ccproxy::openai::UnifiedToolCall;
use crate::ccproxy::types::openai::{
    OpenAIChatCompletionRequest, OpenAIChatCompletionResponse, OpenAIChatCompletionStreamResponse,
    OpenAIFunctionCall, OpenAIFunctionDefinition, OpenAIImageUrl, OpenAIMessageContent,
    OpenAIMessageContentPart, OpenAITool, UnifiedChatMessage,
};

pub struct OpenAIBackendAdapter;

#[async_trait]
impl BackendAdapter for OpenAIBackendAdapter {
    async fn adapt_request(
        &self,
        client: &Client,
        unified_request: &UnifiedRequest,
        api_key: &str,
        base_url: &str,
        model: &str,
    ) -> Result<RequestBuilder, anyhow::Error> {
        let mut openai_messages = Vec::new();

        for msg in &unified_request.messages {
            let role = match msg.role {
                UnifiedRole::System => "system",
                UnifiedRole::User => "user",
                UnifiedRole::Assistant => "assistant",
                UnifiedRole::Tool => "tool",
            };

            let mut content_parts = Vec::new();
            let mut tool_calls = Vec::new();
            let mut tool_call_id = None;

            for block in &msg.content {
                match block {
                    UnifiedContentBlock::Text { text } => {
                        content_parts.push(OpenAIMessageContentPart::Text { text: text.clone() });
                    }
                    UnifiedContentBlock::Image { media_type, data } => {
                        content_parts.push(OpenAIMessageContentPart::ImageUrl {
                            image_url: OpenAIImageUrl {
                                url: format!("data:{};base64,{}", media_type, data),
                                detail: None,
                            },
                        });
                    }
                    UnifiedContentBlock::ToolUse { id, name, input } => {
                        tool_calls.push(UnifiedToolCall {
                            id: Some(id.clone()),
                            r#type: Some("function".to_string()),
                            function: OpenAIFunctionCall {
                                name: Some(name.clone()),
                                arguments: Some(input.to_string()),
                            },
                            index: None,
                        });
                    }
                    UnifiedContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error: _,
                    } => {
                        tool_call_id = Some(tool_use_id.clone());
                        content_parts.push(OpenAIMessageContentPart::Text {
                            text: content.clone(),
                        });
                    }
                    UnifiedContentBlock::Thinking { .. } => {
                        // ignore the thinking field, the input hasn't there field
                    }
                }
            }

            let openai_content = if content_parts.is_empty() {
                None
            } else if content_parts.len() == 1
                && matches!(content_parts[0], OpenAIMessageContentPart::Text { .. })
            {
                if let OpenAIMessageContentPart::Text { text } = content_parts.remove(0) {
                    Some(OpenAIMessageContent::Text(text))
                } else {
                    unreachable!()
                }
            } else {
                Some(OpenAIMessageContent::Parts(content_parts))
            };

            openai_messages.push(UnifiedChatMessage {
                role: Some(role.to_string()),
                content: openai_content,
                tool_calls: if tool_calls.is_empty() {
                    None
                } else {
                    Some(tool_calls)
                },
                tool_call_id,
                reasoning_content: None, // OpenAI does not have a direct equivalent for thinking content in requests
            });
        }

        let openai_tools = unified_request.tools.as_ref().map(|tools| {
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
        });

        let openai_tool_choice = unified_request
            .tool_choice
            .as_ref()
            .map(|choice| match choice {
                UnifiedToolChoice::None => json!("none"),
                UnifiedToolChoice::Auto => json!("auto"),
                UnifiedToolChoice::Required => json!("required"),
                UnifiedToolChoice::Tool { name } => json!({
                    "type": "function",
                    "function": { "name": name }
                }),
            });

        let openai_request = OpenAIChatCompletionRequest {
            model: model.to_string(),
            messages: openai_messages,
            stream: Some(unified_request.stream),
            max_tokens: unified_request.max_tokens,
            temperature: unified_request.temperature,
            top_p: unified_request.top_p,
            top_k: unified_request.top_k, // OpenAI API does not directly support top_k, but some compatible APIs might
            presence_penalty: None,
            frequency_penalty: None,
            response_format: unified_request.response_format.clone(),
            stop: unified_request.stop_sequences.clone(),
            n: None,
            user: None,
            tools: openai_tools,
            tool_choice: openai_tool_choice,
        };

        let mut request_builder = client.post(format!("{}/chat/completions", base_url));
        request_builder = request_builder.header("Content-Type", "application/json");
        if !api_key.is_empty() {
            request_builder =
                request_builder.header("Authorization", format!("Bearer {}", api_key));
        }
        request_builder = request_builder.json(&openai_request);

        Ok(request_builder)
    }

    async fn adapt_response(
        &self,
        backend_response: BackendResponse,
    ) -> Result<UnifiedResponse, anyhow::Error> {
        let openai_response: OpenAIChatCompletionResponse =
            serde_json::from_slice(&backend_response.body)?;

        let first_choice = openai_response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No choices in OpenAI response"))?;

        let mut content_blocks = Vec::new();
        if let Some(content) = first_choice.message.content {
            match content {
                OpenAIMessageContent::Text(text) => {
                    content_blocks.push(UnifiedContentBlock::Text { text })
                }
                OpenAIMessageContent::Parts(parts) => {
                    for part in parts {
                        match part {
                            OpenAIMessageContentPart::Text { text } => {
                                content_blocks.push(UnifiedContentBlock::Text { text })
                            }
                            OpenAIMessageContentPart::ImageUrl { image_url: _ } => {
                                // This case should ideally not happen for assistant responses, but handle defensively
                                anyhow::bail!("Image URL in assistant response not supported for UnifiedResponse");
                            }
                        }
                    }
                }
            }
        }
        if let Some(reasoning_content) = first_choice.message.reasoning_content {
            if !reasoning_content.is_empty() {
                content_blocks.push(UnifiedContentBlock::Thinking {
                    thinking: reasoning_content,
                });
            }
        }

        if let Some(tool_calls) = first_choice.message.tool_calls {
            for tc in tool_calls {
                content_blocks.push(UnifiedContentBlock::ToolUse {
                    id: tc.id.clone().unwrap_or_default(),
                    name: tc.function.name.unwrap_or_default(),
                    input: serde_json::from_str(&tc.function.arguments.unwrap_or_default())?,
                });
            }
        }

        let usage = openai_response
            .usage
            .map(|u| UnifiedUsage {
                input_tokens: u.prompt_tokens,
                output_tokens: u.completion_tokens,
            })
            .unwrap_or_default();

        Ok(UnifiedResponse {
            id: openai_response.id,
            model: openai_response.model,
            content: content_blocks,
            stop_reason: first_choice.finish_reason,
            usage,
        })
    }

    async fn adapt_stream_chunk(
        &self,
        chunk: bytes::Bytes,
        sse_status: Arc<RwLock<SseStatus>>,
    ) -> Result<Vec<UnifiedStreamChunk>, anyhow::Error> {
        let chunk_str = String::from_utf8_lossy(&chunk);
        let mut unified_chunks = Vec::new();

        for event_block_str in chunk_str.split("\n\n") {
            if event_block_str.trim().is_empty() {
                continue;
            }

            let mut data_str: Option<String> = None;
            for line in event_block_str.lines() {
                if line.starts_with("data: ") {
                    data_str = Some(line["data:".len()..].trim().to_string());
                    break;
                }
            }

            if let Some(data) = data_str {
                if data == "[DONE]" {
                    // Handled by MessageStop or stream end signal
                    continue;
                }

                let openai_chunk: OpenAIChatCompletionStreamResponse = serde_json::from_str(&data)?;
                if let Ok(mut status) = sse_status.write() {
                    if !status.message_start {
                        status.message_start = true;
                        if !openai_chunk.id.is_empty() {
                            status.message_id = openai_chunk.id.clone();
                        }
                        unified_chunks.push(UnifiedStreamChunk::MessageStart {
                            id: status.message_id.clone(),
                            model: status.model_id.clone(),
                            usage: UnifiedUsage {
                                input_tokens: 0, // OpenAI stream doesn't provide input tokens in the first chunk
                                output_tokens: 0,
                            },
                        });
                    }
                }

                for choice in openai_chunk.choices {
                    let delta = choice.delta;

                    if let Some(content) = delta.reasoning_content {
                        if !content.is_empty() {
                            if let Ok(mut status) = sse_status.write() {
                                if status.thinking_delta_count == 0 {
                                    unified_chunks.push(UnifiedStreamChunk::ContentBlockStart {
                                        index: 0,
                                        block: json!({
                                            "type":"thinking",
                                            "thinking":"",
                                        }),
                                    })
                                }
                                status.thinking_delta_count += 1;
                                update_message_block(status, "thinking".to_string());
                            }
                            unified_chunks.push(UnifiedStreamChunk::Thinking { delta: content });
                        }
                    }

                    if let Some(content) = delta.content {
                        let has_text = match &content {
                            OpenAIMessageContent::Text(text) => !text.is_empty(),
                            OpenAIMessageContent::Parts(parts) => {
                                parts.iter().any(|part| {
                                    matches!(part, OpenAIMessageContentPart::Text { text } if !text.is_empty())
                                })
                            }
                        };
                        if has_text {
                            if let Ok(status) = sse_status.write() {
                                update_message_block(status, "text".to_string());
                            }
                        }

                        if let Ok(mut status) = sse_status.write() {
                            if status.text_delta_count == 0 && has_text {
                                // The thinking block is usually before the content block,
                                // so we need to output a block end flag first
                                if status.thinking_delta_count > 0 {
                                    unified_chunks.push(UnifiedStreamChunk::ContentBlockStop {
                                        index: (status.message_index - 1).max(0),
                                    });
                                }

                                if status.tool_delta_count > 0 {
                                    unified_chunks.push(UnifiedStreamChunk::ToolUseEnd {
                                        id: status.tool_id.clone(),
                                    });
                                    // reset tool delta count
                                    status.tool_delta_count = 0;
                                }

                                // start the new content block
                                unified_chunks.push(UnifiedStreamChunk::ContentBlockStart {
                                    index: status.message_index,
                                    block: json!({
                                         "type": "text",
                                         "text": ""
                                    }),
                                });
                            }
                        };

                        match content {
                            OpenAIMessageContent::Text(text) => {
                                if !text.is_empty() {
                                    if let Ok(mut status) = sse_status.write() {
                                        status.text_delta_count += 1;
                                    }
                                    unified_chunks.push(UnifiedStreamChunk::Text { delta: text });
                                }
                            }
                            OpenAIMessageContent::Parts(parts) => {
                                if let Ok(mut status) = sse_status.write() {
                                    status.text_delta_count += parts.len() as u32;
                                }
                                for part in parts {
                                    if let OpenAIMessageContentPart::Text { text } = part {
                                        if !text.is_empty() {
                                            unified_chunks
                                                .push(UnifiedStreamChunk::Text { delta: text });
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if let Some(tool_calls) = delta.tool_calls {
                        if let Ok(mut status) = sse_status.write() {
                            if status.tool_delta_count == 0 {
                                if status.text_delta_count > 0 || status.thinking_delta_count > 0 {
                                    unified_chunks.push(UnifiedStreamChunk::ContentBlockStop {
                                        index: status.message_index,
                                    });
                                }
                            }
                            status.tool_delta_count += 1;
                            update_message_block(status, "tool_use".to_string());
                        }

                        for tc in tool_calls {
                            if let Some(name) = tc.function.name.clone() {
                                if !name.is_empty() {
                                    let tool_id = tc.id.clone().unwrap_or_else(|| {
                                        format!("tool_{}", uuid::Uuid::new_v4())
                                    });
                                    let mut message_index = 0;
                                    if let Ok(mut status) = sse_status.write() {
                                        status.tool_id = tool_id.clone();
                                        message_index = status.message_index;
                                    }

                                    // for claude only
                                    unified_chunks.push(UnifiedStreamChunk::ContentBlockStart {
                                        index: message_index,
                                        block: json!({
                                            "type":"tool_use",
                                            "id": tool_id.clone(),
                                            "name": name.clone(),
                                            "input":{}
                                        }),
                                    });
                                    // for gemini and openai
                                    unified_chunks.push(UnifiedStreamChunk::ToolUseStart {
                                        tool_type: "tool_use".to_string(),
                                        id: tool_id.clone(),
                                        name,
                                    });
                                }
                            }

                            if let Some(args) = tc.function.arguments {
                                if !args.is_empty() {
                                    let mut tool_id = String::new();
                                    if let Ok(status) = sse_status.read() {
                                        tool_id = status.tool_id.clone();
                                    };
                                    unified_chunks.push(UnifiedStreamChunk::ToolUseDelta {
                                        id: tool_id,
                                        delta: args,
                                    });
                                }
                            }
                        }
                    }

                    if let Some(finish_reason) = choice.finish_reason {
                        let stop_reason = match finish_reason.to_lowercase().as_str() {
                            "stop" => "stop".to_string(),
                            "length" => "max_tokens".to_string(),
                            "tool_calls" => "tool_use".to_string(),
                            _ => "unknown".to_string(),
                        };
                        let usage = openai_chunk
                            .usage
                            .clone()
                            .map(|u| UnifiedUsage {
                                input_tokens: u.prompt_tokens,
                                output_tokens: u.completion_tokens,
                            })
                            .unwrap_or_default();
                        unified_chunks.push(UnifiedStreamChunk::MessageStop { stop_reason, usage });
                    }
                }
            }
        }

        Ok(unified_chunks)
    }
}
