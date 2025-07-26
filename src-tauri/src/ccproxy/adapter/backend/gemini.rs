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
use crate::ccproxy::gemini::{
    GeminiContent, GeminiFunctionCall, GeminiFunctionCallingConfig, GeminiFunctionDeclaration,
    GeminiFunctionResponse, GeminiGenerationConfig, GeminiInlineData, GeminiPart, GeminiRequest,
    GeminiResponse as GeminiNetworkResponse, GeminiTool as GeminiApiTool, GeminiToolConfig,
};

pub struct GeminiBackendAdapter;

#[async_trait]
impl BackendAdapter for GeminiBackendAdapter {
    async fn adapt_request(
        &self,
        client: &Client,
        unified_request: &UnifiedRequest,
        api_key: &str,
        base_url: &str,
        model: &str,
    ) -> Result<RequestBuilder, anyhow::Error> {
        let mut gemini_contents = Vec::new();
        let mut system_instruction_parts = Vec::new();

        for msg in &unified_request.messages {
            let role = match msg.role {
                UnifiedRole::System => {
                    // System instructions are handled separately in Gemini
                    for block in &msg.content {
                        if let UnifiedContentBlock::Text { text } = block {
                            system_instruction_parts.push(GeminiPart {
                                text: Some(text.clone()),
                                ..Default::default()
                            });
                        }
                    }
                    continue;
                }
                UnifiedRole::User => "user",
                UnifiedRole::Assistant => "model",
                UnifiedRole::Tool => "function", // Gemini tool results are from the 'function' role
            };

            let mut parts = Vec::new();
            for block in &msg.content {
                match block {
                    UnifiedContentBlock::Text { text } => {
                        parts.push(GeminiPart {
                            text: Some(text.clone()),
                            ..Default::default()
                        });
                    }
                    UnifiedContentBlock::Image { media_type, data } => {
                        parts.push(GeminiPart {
                            inline_data: Some(GeminiInlineData {
                                mime_type: media_type.clone(),
                                data: data.clone(),
                            }),
                            ..Default::default()
                        });
                    }
                    UnifiedContentBlock::ToolUse { id: _, name, input } => {
                        parts.push(GeminiPart {
                            function_call: Some(GeminiFunctionCall {
                                name: name.clone(),
                                args: input.clone(),
                            }),
                            ..Default::default()
                        });
                    }
                    UnifiedContentBlock::ToolResult {
                        tool_use_id: _,
                        content,
                        is_error: _,
                    } => {
                        // Gemini expects tool results as a FunctionResponse part
                        parts.push(GeminiPart {
                            function_response: Some(GeminiFunctionResponse {
                                name: "tool_code".to_string(), // Generic name, actual tool name might be needed if available
                                response: json!({ "result": content.clone() }), // Wrap content in a JSON object
                            }),
                            ..Default::default()
                        });
                    }
                    UnifiedContentBlock::Thinking { .. } => {
                        // ignore the thinking and reasoning field, the input hasn't there field
                    }
                }
            }
            gemini_contents.push(GeminiContent {
                role: role.to_string(),
                parts,
            });
        }

        let system_instruction = if !system_instruction_parts.is_empty() {
            Some(GeminiContent {
                role: "system".to_string(),
                parts: system_instruction_parts,
            })
        } else {
            None
        };

        let gemini_tools = unified_request.tools.as_ref().map(|tools| {
            vec![GeminiApiTool {
                function_declarations: tools
                    .iter()
                    .map(|tool| GeminiFunctionDeclaration {
                        name: tool.name.clone(),
                        description: tool.description.clone().unwrap_or_default(),
                        parameters: tool.input_schema.clone(),
                    })
                    .collect(),
            }]
        });

        let gemini_tool_config = unified_request.tool_choice.as_ref().map(|choice| {
            let mode = match choice {
                UnifiedToolChoice::None => "NONE".to_string(),
                UnifiedToolChoice::Auto => "AUTO".to_string(),
                UnifiedToolChoice::Required => "ANY".to_string(),
                UnifiedToolChoice::Tool { name: _ } => "ANY".to_string(), // Gemini doesn't have specific tool choice by name in config
            };
            GeminiToolConfig {
                function_calling_config: GeminiFunctionCallingConfig { mode },
            }
        });

        let gemini_request = GeminiRequest {
            contents: gemini_contents,
            generation_config: Some(GeminiGenerationConfig {
                temperature: unified_request.temperature,
                top_p: unified_request.top_p,
                top_k: unified_request.top_k.map(|v| v as i32),
                max_output_tokens: unified_request.max_tokens.map(|v| v as i32),
                stop_sequences: unified_request.stop_sequences.clone(),
                response_mime_type: unified_request.response_mime_type.clone(),
                response_schema: unified_request.response_schema.clone(),
            }),
            tools: gemini_tools,
            tool_config: gemini_tool_config,
            system_instruction,
            safety_settings: unified_request.safety_settings.clone(),
        };

        let url = if unified_request.stream {
            format!(
                "{}/models/{}:streamGenerateContent?alt=sse&key={}",
                base_url, model, api_key
            )
        } else {
            format!(
                "{}/models/{}:generateContent?key={}",
                base_url, model, api_key
            )
        };

        let mut request_builder = client.post(url);
        request_builder = request_builder.header("Content-Type", "application/json");
        request_builder = request_builder.json(&gemini_request);

        Ok(request_builder)
    }

    async fn adapt_response(
        &self,
        backend_response: BackendResponse,
    ) -> Result<UnifiedResponse, anyhow::Error> {
        let gemini_response: GeminiNetworkResponse =
            serde_json::from_slice(&backend_response.body)?;

        let mut content_blocks = Vec::new();
        let mut stop_reason = None;
        let mut usage = UnifiedUsage::default();

        if let Some(candidates) = gemini_response.candidates {
            if let Some(candidate) = candidates.into_iter().next() {
                for part in candidate.content.parts {
                    if let Some(text) = part.text {
                        content_blocks.push(UnifiedContentBlock::Text { text });
                    }
                    if let Some(function_call) = part.function_call {
                        content_blocks.push(UnifiedContentBlock::ToolUse {
                            id: "".to_string(), // Gemini doesn't provide tool_use_id in non-streaming
                            name: function_call.name,
                            input: function_call.args,
                        });
                    }
                }
                stop_reason = candidate.finish_reason.map(|r| r.to_string());
            }
        }

        if let Some(usage_meta) = gemini_response.usage_metadata {
            usage.input_tokens = usage_meta.prompt_token_count;
            usage.output_tokens = usage_meta.candidates_token_count.unwrap_or(0);
        }

        Ok(UnifiedResponse {
            id: uuid::Uuid::new_v4().to_string(), // Generate a new ID as Gemini doesn't provide one
            model: "gemini".to_string(),          // Model name might need to be passed through
            content: content_blocks,
            stop_reason,
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

        for line in chunk_str.lines() {
            if line.starts_with("data:") {
                let data_str = line["data:".len()..].trim();
                let gemini_response: GeminiNetworkResponse = serde_json::from_str(data_str)?;

                if let Ok(mut status) = sse_status.write() {
                    if !status.message_start {
                        status.message_start = true;
                        // Gemini does not provide a message ID in the stream, so we use the one from the initial status.
                        unified_chunks.push(UnifiedStreamChunk::MessageStart {
                            id: status.message_id.clone(),
                            model: status.model_id.clone(),
                            usage: UnifiedUsage {
                                input_tokens: 0, // Gemini stream doesn't provide input tokens in the first chunk
                                output_tokens: 0,
                            },
                        });
                    }
                }

                if let Some(candidates) = gemini_response.candidates {
                    for candidate in candidates {
                        for part in candidate.content.parts {
                            if let Some(text) = part.text.clone() {
                                if !text.is_empty() {
                                    if let Ok(mut status) = sse_status.write() {
                                        if status.text_delta_count == 0 {
                                            // Server may output: message -> tool -> message
                                            // So when outputting a message, if there is tool content, it means the tool output has ended
                                            if status.tool_delta_count > 0 {
                                                unified_chunks.push(
                                                    UnifiedStreamChunk::ToolUseEnd {
                                                        id: status.tool_id.clone(),
                                                    },
                                                );
                                                // reset tool delta count
                                                status.tool_delta_count = 0;
                                            }

                                            // start the new content block
                                            unified_chunks.push(
                                                UnifiedStreamChunk::ContentBlockStart {
                                                    index: status.message_index,
                                                    block: json!({
                                                         "type": "text",
                                                         "text": ""
                                                    }),
                                                },
                                            );
                                        }

                                        status.text_delta_count += 1;
                                        update_message_block(status, "text".to_string());
                                    }
                                    unified_chunks.push(UnifiedStreamChunk::Text { delta: text });
                                }
                            }

                            if let Some(function_call) = part.function_call.clone() {
                                let tool_id = format!("tool_{}", function_call.name);
                                let mut tool_delta_count = 0;
                                if let Ok(mut status) = sse_status.write() {
                                    tool_delta_count = status.tool_delta_count;
                                    status.tool_delta_count += 1;
                                    status.tool_id = tool_id.clone();
                                    update_message_block(status, "tool_use".to_string());
                                }
                                if tool_delta_count == 0 {
                                    if let Ok(status) = sse_status.read() {
                                        if status.text_delta_count > 0 {
                                            unified_chunks.push(
                                                UnifiedStreamChunk::ContentBlockStop {
                                                    index: (status.message_index - 1).max(0),
                                                },
                                            );
                                        }
                                        // for claude only
                                        unified_chunks.push(
                                            UnifiedStreamChunk::ContentBlockStart {
                                                index: status.message_index,
                                                block: json!({
                                                    "type":"tool_use",
                                                    "id":tool_id.clone(),
                                                    "name":function_call.name.clone(),
                                                    "input":{}
                                                }),
                                            },
                                        );
                                        // for gemini and openai
                                        unified_chunks.push(UnifiedStreamChunk::ToolUseStart {
                                            tool_type: "tool_use".to_string(),
                                            id: tool_id.clone(),
                                            name: function_call.name.clone(),
                                        });
                                    }
                                }

                                unified_chunks.push(UnifiedStreamChunk::ToolUseDelta {
                                    id: tool_id.clone(),
                                    delta: function_call.args.to_string(),
                                });
                            }
                        }

                        if let Some(finish_reason) = candidate.finish_reason {
                            let stop_reason = finish_reason.to_string();
                            let usage = gemini_response
                                .usage_metadata
                                .clone()
                                .map(|u| UnifiedUsage {
                                    input_tokens: u.prompt_token_count,
                                    output_tokens: u.candidates_token_count.unwrap_or(0),
                                })
                                .unwrap_or_default();
                            unified_chunks
                                .push(UnifiedStreamChunk::MessageStop { stop_reason, usage });
                        }
                    }
                }
            }
        }

        Ok(unified_chunks)
    }
}
