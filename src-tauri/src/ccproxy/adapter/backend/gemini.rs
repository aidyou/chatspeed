use crate::ccproxy::adapter::backend::update_message_block;
use crate::ccproxy::adapter::unified::SseStatus;
use async_trait::async_trait;
use reqwest::{Client, RequestBuilder};
use serde_json::json;
use std::sync::{Arc, RwLock};

use super::{BackendAdapter, BackendResponse};
use crate::ccproxy::adapter::{
    range_adapter::{adapt_temperature, Protocol},
    unified::{
        UnifiedContentBlock, UnifiedRequest, UnifiedResponse, UnifiedRole, UnifiedStreamChunk,
        UnifiedToolChoice, UnifiedUsage,
    },
};
use crate::ccproxy::gemini::{
    GeminiContent, GeminiFunctionCall, GeminiFunctionCallingConfig, GeminiFunctionDeclaration,
    GeminiFunctionResponse, GeminiGenerationConfig, GeminiInlineData, GeminiPart, GeminiRequest,
    GeminiResponse as GeminiNetworkResponse, GeminiTool as GeminiApiTool, GeminiToolConfig,
};

pub struct GeminiBackendAdapter;

impl GeminiBackendAdapter {
    /// Extract only Gemini-supported JSON Schema fields
    ///
    /// @link https://ai.google.dev/api/caching#Schema
    /// @link https://ai.google.dev/api/caching?hl=zh-cn#Schema
    fn extract_gemini_schema(schema: &serde_json::Value) -> serde_json::Value {
        match schema {
            serde_json::Value::Object(obj) => {
                let mut gemini_schema = serde_json::Map::new();

                // Gemini API 支持的 JSON Schema 字段
                let supported_fields = [
                    "type",
                    "description",
                    "enum",
                    "properties",
                    "required",
                    "items",
                    "format",
                    "minimum",
                    "maximum",
                    "minLength",
                    "maxLength",
                    "pattern",
                    "minItems",
                    "maxItems",
                ];

                for field in supported_fields {
                    if let Some(value) = obj.get(field) {
                        match field {
                            "properties" => {
                                // Recursively process each property in properties
                                if let serde_json::Value::Object(props) = value {
                                    let mut cleaned_props = serde_json::Map::new();
                                    for (prop_name, prop_schema) in props {
                                        cleaned_props.insert(
                                            prop_name.clone(),
                                            Self::extract_gemini_schema(prop_schema),
                                        );
                                    }
                                    gemini_schema.insert(
                                        field.to_string(),
                                        serde_json::Value::Object(cleaned_props),
                                    );
                                }
                            }
                            "items" => {
                                // Recursively process array items schema
                                gemini_schema
                                    .insert(field.to_string(), Self::extract_gemini_schema(value));
                            }
                            "format" => {
                                // According to Gemini official documentation, supported formats:
                                // NUMBER: float, double
                                // INTEGER: int32, int64
                                // STRING: enum, date-time
                                if let serde_json::Value::String(format_str) = value {
                                    match format_str.as_str() {
                                        "float" | "double" | "int32" | "int64" | "enum"
                                        | "date-time" => {
                                            gemini_schema.insert(field.to_string(), value.clone());
                                        }
                                        "uint" | "uint32" | "uint64" => {
                                            gemini_schema.insert(field.to_string(), json!("int64"));
                                        }
                                        _ => {
                                            // Skip unsupported format values like "uri", "email", etc.
                                            log::debug!(
                                                "Skipping unsupported format '{}' for Gemini API",
                                                format_str
                                            );
                                        }
                                    }
                                }
                            }
                            _ => {
                                // Directly copy other supported fields
                                gemini_schema.insert(field.to_string(), value.clone());
                            }
                        }
                    }
                }

                serde_json::Value::Object(gemini_schema)
            }
            serde_json::Value::Array(arr) => serde_json::Value::Array(
                arr.iter()
                    .map(|item| Self::extract_gemini_schema(item))
                    .collect(),
            ),
            _ => schema.clone(),
        }
    }
}

#[async_trait]
impl BackendAdapter for GeminiBackendAdapter {
    async fn adapt_request(
        &self,
        client: &Client,
        unified_request: &UnifiedRequest,
        _api_key: &str,
        full_provider_url: &str,
        _model: &str,
    ) -> Result<RequestBuilder, anyhow::Error> {
        let mut gemini_contents = Vec::new();
        let mut system_instruction_parts = Vec::new();

        for msg in &unified_request.messages {
            let role = match msg.role {
                UnifiedRole::System => continue,
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

        if let Some(sys_prompt_str) = unified_request.system_prompt.as_ref() {
            if !sys_prompt_str.trim().is_empty() {
                system_instruction_parts.push(GeminiPart {
                    text: Some(sys_prompt_str.to_string()),
                    ..Default::default()
                });
            }
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
                        parameters: Self::extract_gemini_schema(&tool.input_schema),
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
                temperature: unified_request.temperature.map(|t| {
                    // Adapt temperature from source protocol to Gemini range
                    adapt_temperature(t, Protocol::OpenAI, Protocol::Gemini)
                }),
                top_p: unified_request.top_p,
                top_k: unified_request.top_k.map(|v| v as i32),
                max_output_tokens: unified_request.max_tokens.map(|v| v as i32),
                stop_sequences: unified_request.stop_sequences.clone(),
                response_mime_type: unified_request.response_mime_type.clone(),
                response_schema: unified_request.response_schema.clone(),
                thinking_config: unified_request.thinking.as_ref().map(|t| {
                    crate::ccproxy::types::gemini::GeminiThinkingConfig {
                        thinking_budget: Some(t.budget_tokens),
                    }
                }),
            }),
            tools: gemini_tools,
            tool_config: gemini_tool_config,
            system_instruction,
            safety_settings: unified_request.safety_settings.clone(),
            cached_content: unified_request.cached_content.clone(),
        };

        let mut request_builder = client.post(full_provider_url);
        request_builder = request_builder.header("Content-Type", "application/json");
        request_builder = request_builder.json(&gemini_request);

        #[cfg(debug_assertions)]
        {
            match serde_json::to_string_pretty(&gemini_request) {
                Ok(request_json) => {
                    log::debug!("Gemini request: {}", request_json);
                }
                Err(e) => {
                    log::error!("Failed to serialize Gemini request: {}", e);
                    // Try to serialize individual parts to identify the issue
                    if let Some(tools) = &gemini_request.tools {
                        for (i, tool) in tools.iter().enumerate() {
                            if let Err(tool_err) = serde_json::to_string(&tool) {
                                log::error!("Failed to serialize tool {}: {}", i, tool_err);
                                log::error!("Tool details: {:?}", tool.function_declarations);
                            }
                        }
                    }
                    return Err(anyhow::anyhow!("Failed to serialize Gemini request: {}", e));
                }
            }
        }

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
            usage.tool_use_prompt_tokens = usage_meta.tool_use_prompt_token_count;
            usage.thoughts_tokens = usage_meta.thoughts_token_count;
            usage.cached_content_tokens = usage_meta.cached_content_token_count;
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
        #[cfg(debug_assertions)]
        {
            log::debug!("gemini sse chunk: {}", chunk_str);
        }

        let mut unified_chunks = Vec::new();
        for line in chunk_str.lines() {
            if line.starts_with("data:") {
                let data_str = line["data:".len()..].trim();
                let gemini_response: GeminiNetworkResponse = serde_json::from_str(data_str)
                    .map_err(|e| {
                        log::error!(
                            "Gemini delta deserialize failed, delta: {}, error:{}",
                            &data_str,
                            e.to_string()
                        );
                        e
                    })?;

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
                                ..Default::default()
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
                                        update_message_block(&mut status, "text".to_string());
                                    }
                                    unified_chunks.push(UnifiedStreamChunk::Text { delta: text });
                                }
                            }

                            if let Some(function_call) = part.function_call.clone() {
                                // Gemini's functionCall in a stream for parallel calls is a COMPLETE, ATOMIC event.
                                // It is NOT a delta of a larger call.
                                // Therefore, for each one, we must emit a full Start -> Delta -> End sequence.

                                // Generate a unique ID for this specific tool call.
                                let tool_id = format!("tool_{}", uuid::Uuid::new_v4());
                                let mut message_index = 0;
                                // We need a unique ID for each parallel call. Let's use an index from our status.
                                if let Ok(mut status) = sse_status.write() {
                                    // Increment the total count of tools we've seen in this turn.
                                    status.tool_delta_count += 1;

                                    if status.tool_id != "" {
                                        // send tool stop
                                        unified_chunks.push(UnifiedStreamChunk::ContentBlockStop {
                                            index: status.message_index,
                                        })
                                    }
                                    status.tool_id = tool_id.clone();
                                    update_message_block(
                                        &mut status,
                                        format!("{tool_id}").to_string(),
                                    );
                                    message_index = status.message_index;
                                    // Record tool_id to index mapping
                                    status
                                        .tool_id_to_index
                                        .insert(tool_id.clone(), message_index);
                                } else {
                                    // Handle error, maybe continue to next line
                                    log::warn!(
                                        "failed to get status write lock, block index: {}",
                                        message_index
                                    );
                                    continue;
                                }

                                let tool_name = function_call.name.clone();

                                // The `args` field contains the full JSON for the arguments.
                                // It might be a complex JSON object, not just a string.
                                let args_json_string = function_call.args.to_string();

                                // 1. Announce the start of this specific tool call.
                                // for claude only
                                unified_chunks.push(UnifiedStreamChunk::ContentBlockStart {
                                    index: message_index,
                                    block: json!({
                                        "type":"tool_use",
                                        "id": tool_id.clone(),
                                        "name": tool_name.clone(),
                                        "input":{}
                                    }),
                                });
                                // for openai and gemini
                                unified_chunks.push(UnifiedStreamChunk::ToolUseStart {
                                    tool_type: "tool_use".to_string(), // or whatever is appropriate
                                    id: tool_id.clone(),
                                    name: tool_name,
                                });

                                // 2. Send all its arguments in a single delta.
                                unified_chunks.push(UnifiedStreamChunk::ToolUseDelta {
                                    id: tool_id.clone(),
                                    delta: args_json_string,
                                });

                                // 3. Immediately announce the end of this specific tool call.
                                unified_chunks.push(UnifiedStreamChunk::ToolUseEnd { id: tool_id });
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
                                    cache_creation_input_tokens: None,
                                    cache_read_input_tokens: None,
                                    tool_use_prompt_tokens: u.tool_use_prompt_token_count,
                                    thoughts_tokens: u.thoughts_token_count,
                                    cached_content_tokens: u.cached_content_token_count,
                                    total_duration: None,
                                    load_duration: None,
                                    prompt_eval_duration: None,
                                    eval_duration: None,
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
