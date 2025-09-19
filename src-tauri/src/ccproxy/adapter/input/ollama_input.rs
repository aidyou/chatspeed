use crate::ccproxy::{
    adapter::{
        range_adapter::clamp_to_protocol_range,
        unified::{UnifiedContentBlock, UnifiedMessage, UnifiedRequest, UnifiedRole, UnifiedTool},
    },
    get_tool_id,
    types::{
        ollama::{OllamaChatCompletionRequest, OllamaMessage},
        ChatProtocol, TOOL_ARG_ERROR_REMINDER,
    },
};
use anyhow::Result;

/// Converts an Ollama chat completion request into the `UnifiedRequest`.
pub fn from_ollama(
    req: OllamaChatCompletionRequest,
    tool_compat_mode: bool,
) -> Result<UnifiedRequest> {
    let mut messages = Vec::new();
    let mut system_prompt: Option<String> = None;

    for msg in req.messages {
        let role = match msg.role.as_str() {
            "system" => {
                let current_prompt = system_prompt.unwrap_or_default();
                system_prompt = Some(format!("{}{}\n", current_prompt, msg.content));
                continue;
            }
            "user" => UnifiedRole::User,
            "assistant" => UnifiedRole::Assistant,
            "tool" => UnifiedRole::Tool,
            _ => anyhow::bail!("Invalid or missing role in Ollama message: {}", msg.role),
        };

        let content = if role == UnifiedRole::Tool {
            // Correctly handle the tool role by creating a ToolResult block.
            // Ollama does not provide a tool_call_id, so we generate a placeholder.
            // This is crucial for the backend adapter to identify this as a tool result.
            vec![UnifiedContentBlock::ToolResult {
                tool_use_id: get_tool_id(),
                content: msg.content.clone(),
                is_error: false,
            }]
        } else {
            // For user and assistant roles, use the helper function.
            convert_ollama_message_to_content_blocks(&msg)?
        };

        messages.push(UnifiedMessage {
            role,
            content,
            reasoning_content: None,
        });
    }

    let tools = req.tools.map(|tools| {
        tools
            .into_iter()
            .map(|tool| UnifiedTool {
                name: tool.function.name,
                description: tool.function.description,
                input_schema: tool.function.parameters,
            })
            .collect()
    });

    let options = req.options.unwrap_or_default();

    Ok(UnifiedRequest {
        model: req.model,
        messages,
        system_prompt: system_prompt.map(|s| s.trim().to_string()),
        tools,
        tool_choice: None, // Ollama doesn't support tool_choice
        stream: req.stream.unwrap_or(false),
        temperature: options.temperature.and_then(|t| {
            if t < 0.0 {
                None
            } else {
                Some(clamp_to_protocol_range(
                    t,
                    ChatProtocol::Ollama,
                    crate::ccproxy::adapter::range_adapter::Parameter::Temperature,
                ))
            }
        }),
        max_tokens: options.num_predict,
        top_p: options.top_p,
        top_k: options.top_k,
        stop_sequences: options.stop,
        presence_penalty: options.presence_penalty,
        frequency_penalty: options.frequency_penalty,
        response_format: None,
        seed: options.seed,
        user: None,
        logprobs: None,
        top_logprobs: None,
        metadata: None,
        thinking: None,
        cache_control: None,
        safety_settings: None,
        response_mime_type: req.format.and_then(|f| {
            if f == "json" {
                Some("application/json".to_string())
            } else {
                None
            }
        }),
        response_schema: None,
        cached_content: None,
        tool_compat_mode,
        keep_alive: req
            .keep_alive
            .as_ref()
            .and_then(|ka| ka.as_str())
            .and_then(|ka| {
                if ka.is_empty() || ka == "-1" {
                    None
                } else {
                    Some(ka.to_string())
                }
            }),
        ..Default::default()
    })
}

/// Converts an Ollama message (for user or assistant) into a vector of `UnifiedContentBlock`.
/// Note: This function should not be called for 'tool' role messages.
fn convert_ollama_message_to_content_blocks(
    msg: &OllamaMessage,
) -> Result<Vec<UnifiedContentBlock>> {
    let mut blocks = Vec::new();

    // Add text content.
    if !msg.content.is_empty() {
        blocks.push(UnifiedContentBlock::Text {
            text: msg.content.clone(),
        });
    }

    // Add image content (for 'user' messages)
    if let Some(images) = &msg.images {
        for base64_data in images {
            let media_type = if base64_data.starts_with("iVBORw0KGgo") {
                "image/png".to_string()
            } else if base64_data.starts_with("/9j/") {
                "image/jpeg".to_string()
            } else {
                "application/octet-stream".to_string()
            };
            blocks.push(UnifiedContentBlock::Image {
                media_type,
                data: base64_data.clone(),
            });
        }
    }

    // Add tool calls (for 'assistant' messages)
    if let Some(tool_calls) = &msg.tool_calls {
        for tc in tool_calls {
            let tool_name = tc.function.name.clone();
            let arguments = tc.function.arguments.clone();

            // Check if the arguments are a string that needs to be parsed, or already a valid JSON value.
            match arguments {
                serde_json::Value::String(s) => {
                    // It's a string. Let's see if it's stringified JSON.
                    match serde_json::from_str(&s) {
                        Ok(parsed_json) => {
                            // It was stringified JSON, push a proper ToolUse block.
                            blocks.push(UnifiedContentBlock::ToolUse {
                                id: get_tool_id(),
                                name: tool_name,
                                input: parsed_json,
                            });
                        }
                        Err(_) => {
                            // It's a string, but not valid JSON. This is an error case.
                            log::warn!(
                                "Failed to parse Ollama tool arguments as JSON for tool '{}'. Original arguments: {}",
                                tool_name, s
                            );
                            blocks.extend(vec![
                                UnifiedContentBlock::Text {
                                    text: format!(
                                        "<cs:failed_tool_call>\n<name>{}</name>\n<input>{}</input>\n</cs:failed_tool_call>",
                                        tool_name, s
                                    ),
                                },
                                UnifiedContentBlock::Text {
                                    text: TOOL_ARG_ERROR_REMINDER.to_string(),
                                },
                            ]);
                        }
                    }
                }
                _ => {
                    // It's already a valid JSON Value (Object, Array, etc.), push a proper ToolUse block.
                    blocks.push(UnifiedContentBlock::ToolUse {
                        id: get_tool_id(),
                        name: tool_name,
                        input: arguments,
                    });
                }
            }
        }
    }

    Ok(blocks)
}
