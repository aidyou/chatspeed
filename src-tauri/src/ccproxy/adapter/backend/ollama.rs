use crate::ccproxy::adapter::unified::{
    SseStatus, UnifiedContentBlock, UnifiedRequest, UnifiedResponse, UnifiedRole,
    UnifiedStreamChunk, UnifiedUsage,
};
use crate::ccproxy::types::ollama::{
    OllamaChatCompletionRequest, OllamaChatCompletionResponse, OllamaFunctionCall, OllamaMessage,
    OllamaOptions, OllamaStreamResponse, OllamaTool, OllamaToolCall,
};
use async_trait::async_trait;
use reqwest::{Client, RequestBuilder};
use std::sync::{Arc, RwLock};

use super::{BackendAdapter, BackendResponse};

pub struct OllamaBackendAdapter;

#[async_trait]
impl BackendAdapter for OllamaBackendAdapter {
    async fn adapt_request(
        &self,
        client: &Client,
        unified_request: &UnifiedRequest,
        _api_key: &str, // Ollama doesn't use API keys
        provider_full_url: &str,
        model: &str,
    ) -> Result<RequestBuilder, anyhow::Error> {
        let mut ollama_messages = Vec::new();

        if let Some(system_prompt) = &unified_request.system_prompt {
            ollama_messages.push(OllamaMessage {
                role: "system".to_string(),
                content: system_prompt.clone(),
                ..Default::default()
            });
        }

        for msg in &unified_request.messages {
            let mut content_text = String::new();
            let mut images = Vec::new();
            let mut tool_calls = Vec::new();

            for block in &msg.content {
                match block {
                    UnifiedContentBlock::Text { text } => {
                        content_text.push_str(text);
                    }
                    UnifiedContentBlock::Image { data, .. } => {
                        images.push(data.clone());
                    }
                    UnifiedContentBlock::ToolUse { name, input, .. } => {
                        tool_calls.push(OllamaToolCall {
                            function: OllamaFunctionCall {
                                name: name.clone(),
                                arguments: input.clone(),
                            },
                        });
                    }
                    UnifiedContentBlock::ToolResult { content, .. } => {
                        content_text.push_str(content);
                    }
                    _ => {}
                }
            }

            ollama_messages.push(OllamaMessage {
                role: match msg.role {
                    UnifiedRole::User => "user".to_string(),
                    UnifiedRole::Assistant => "assistant".to_string(),
                    UnifiedRole::Tool => "tool".to_string(),
                    _ => "user".to_string(),
                },
                content: content_text,
                images: if images.is_empty() {
                    None
                } else {
                    Some(images)
                },
                tool_calls: if tool_calls.is_empty() {
                    None
                } else {
                    Some(tool_calls)
                },
                tool_name: None, // This is set in the response from the model
            });
        }

        let ollama_request = OllamaChatCompletionRequest {
            model: model.to_string(),
            messages: ollama_messages,
            stream: Some(unified_request.stream),
            format: unified_request
                .response_mime_type
                .as_ref()
                .and_then(|mime| {
                    if mime == "application/json" {
                        Some("json".to_string())
                    } else {
                        None
                    }
                }),
            options: Some(OllamaOptions {
                temperature: unified_request.temperature,
                num_predict: unified_request.max_tokens,
                top_p: unified_request.top_p,
                top_k: unified_request.top_k.map(|k| k as i32),
                stop: unified_request.stop_sequences.clone(),
                presence_penalty: unified_request.presence_penalty,
                frequency_penalty: unified_request.frequency_penalty,
                seed: unified_request.seed,
                ..Default::default()
            }),
            keep_alive: Some("5m".to_string()),
            tools: unified_request.tools.as_ref().map(|tools| {
                tools
                    .iter()
                    .map(|tool| OllamaTool {
                        r#type: "function".to_string(),
                        function: crate::ccproxy::types::ollama::OllamaFunctionDefinition {
                            name: tool.name.clone(),
                            description: tool.description.clone().unwrap_or_default(),
                            parameters: tool.input_schema.clone(),
                        },
                    })
                    .collect()
            }),
        };

        let mut request_builder = client.post(provider_full_url);
        request_builder = request_builder.header("Content-Type", "application/json");
        request_builder = request_builder.json(&ollama_request);

        #[cfg(debug_assertions)]
        {
            match serde_json::to_string_pretty(&ollama_request) {
                Ok(request_json) => {
                    log::debug!("Ollama request: {}", request_json);
                }
                Err(e) => {
                    log::error!("Failed to serialize Ollama request: {}", e);
                    // Try to serialize individual parts to identify the issue
                    if let Some(tools) = &ollama_request.tools {
                        for (i, tool) in tools.iter().enumerate() {
                            if let Err(tool_err) = serde_json::to_string(&tool) {
                                log::error!("Failed to serialize tool {}: {}", i, tool_err);
                                log::error!(
                                    "Tool details - name: {}, type: {}",
                                    tool.function.name,
                                    tool.r#type
                                );
                            }
                        }
                    }
                    return Err(anyhow::anyhow!("Failed to serialize Ollama request: {}", e));
                }
            }
        }

        Ok(request_builder)
    }

    async fn adapt_response(
        &self,
        backend_response: BackendResponse,
    ) -> Result<UnifiedResponse, anyhow::Error> {
        let ollama_response: OllamaChatCompletionResponse =
            serde_json::from_slice(&backend_response.body)?;

        let mut content_blocks = Vec::new();
        if !ollama_response.message.content.is_empty() {
            content_blocks.push(UnifiedContentBlock::Text {
                text: ollama_response.message.content,
            });
        }

        if let Some(tool_calls) = ollama_response.message.tool_calls {
            for tc in tool_calls {
                content_blocks.push(UnifiedContentBlock::ToolUse {
                    id: format!("tool_{}", uuid::Uuid::new_v4()),
                    name: tc.function.name,
                    input: tc.function.arguments,
                });
            }
        }

        Ok(UnifiedResponse {
            id: format!("chatcmpl-{}", uuid::Uuid::new_v4()),
            model: ollama_response.model,
            content: content_blocks,
            stop_reason: Some("stop".to_string()),
            usage: UnifiedUsage {
                input_tokens: ollama_response.prompt_eval_count.unwrap_or(0) as u64,
                output_tokens: ollama_response.eval_count.unwrap_or(0) as u64,
                ..Default::default()
            },
        })
    }

    async fn adapt_stream_chunk(
        &self,
        chunk: bytes::Bytes,
        _sse_status: Arc<RwLock<SseStatus>>,
    ) -> Result<Vec<UnifiedStreamChunk>, anyhow::Error> {
        let chunk_str = String::from_utf8_lossy(&chunk);
        let mut unified_chunks = Vec::new();

        for line in chunk_str.lines() {
            if line.trim().is_empty() {
                continue;
            }

            let ollama_chunk: OllamaStreamResponse = match serde_json::from_str(line) {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("Failed to parse Ollama stream chunk: {}, line: {}", e, line);
                    continue;
                }
            };

            if ollama_chunk.done {
                unified_chunks.push(UnifiedStreamChunk::MessageStop {
                    stop_reason: "stop".to_string(),
                    usage: UnifiedUsage {
                        input_tokens: ollama_chunk.prompt_eval_count.unwrap_or(0) as u64,
                        output_tokens: ollama_chunk.eval_count.unwrap_or(0) as u64,
                        ..Default::default()
                    },
                });
            } else {
                if !ollama_chunk.message.content.is_empty() {
                    unified_chunks.push(UnifiedStreamChunk::Text {
                        delta: ollama_chunk.message.content,
                    });
                }

                if let Some(tool_calls) = ollama_chunk.message.tool_calls {
                    for tc in tool_calls {
                        unified_chunks.push(UnifiedStreamChunk::ToolUseStart {
                            tool_type: "function".to_string(),
                            id: format!("tool_{}", uuid::Uuid::new_v4()),
                            name: tc.function.name.clone(),
                        });
                        unified_chunks.push(UnifiedStreamChunk::ToolUseDelta {
                            id: "".to_string(), // Ollama does not provide tool ID in stream
                            delta: tc.function.arguments.to_string(),
                        });
                        unified_chunks.push(UnifiedStreamChunk::ToolUseEnd {
                            id: "".to_string(), // Ollama does not provide tool ID in stream
                        });
                    }
                }
            }
        }

        Ok(unified_chunks)
    }
}
