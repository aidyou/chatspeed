use anyhow::Result;

use crate::ccproxy::{
    adapter::{
        range_adapter::{clamp_to_protocol_range, Parameter},
        unified::{
            UnifiedCacheControl, UnifiedContentBlock, UnifiedMessage, UnifiedMetadata,
            UnifiedRequest, UnifiedRole, UnifiedThinking, UnifiedTool, UnifiedToolChoice,
        },
    },
    types::{
        claude::{ClaudeNativeContentBlock, ClaudeNativeRequest, ClaudeToolChoice},
        ChatProtocol, TOOL_ARG_ERROR_REMINDER,
    },
};

/// Converts a single Claude native content block into a unified content block.
/// This function handles the direct mapping between the source and unified formats.
fn convert_claude_content_block(
    block: ClaudeNativeContentBlock,
) -> Result<Vec<UnifiedContentBlock>> {
    match block {
        ClaudeNativeContentBlock::Text { text } => Ok(vec![UnifiedContentBlock::Text { text }]),
        ClaudeNativeContentBlock::Image { source } => Ok(vec![UnifiedContentBlock::Image {
            media_type: source.media_type,
            data: source.data,
        }]),
        ClaudeNativeContentBlock::ToolUse { id, name, input } => {
            let cleaned_input_str = input.to_string();
            match serde_json::from_str(&cleaned_input_str) {
                Ok(cleaned_input) => Ok(vec![UnifiedContentBlock::ToolUse {
                    id,
                    name,
                    input: cleaned_input,
                }]),
                Err(e) => {
                    log::warn!(
                        "Failed to parse Claude tool input as JSON after cleaning: {}. Original input: {}",
                        e,
                        input
                    );
                    Ok(vec![
                        // UnifiedContentBlock::ToolUse {
                        //     id,
                        //     name: name.clone(),
                        //     input: json!({}),
                        // },
                        UnifiedContentBlock::Text {
                            text: format!(
                                "<ccp:failed_tool_call>\n<name>{}</name>\n<input>{}</input>\n</ccp:failed_tool_call>",
                                name, input
                            ),
                        },
                        UnifiedContentBlock::Text {
                            text: TOOL_ARG_ERROR_REMINDER.to_string(),
                        },
                    ])
                }
            }
        }
        ClaudeNativeContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => Ok(vec![UnifiedContentBlock::ToolResult {
            tool_use_id,
            content, // The source is already a String, which is correct for the unified format.
            is_error: is_error.unwrap_or(false),
        }]),
        ClaudeNativeContentBlock::Thinking { thinking } => {
            Ok(vec![UnifiedContentBlock::Thinking { thinking }])
        }
    }
}

/// Adapts a native Claude request to the unified request format.
///
/// This function is crucial for translating Claude-specific structures, especially
/// the tool-calling message sequence, into a standardized format that backend
/// adapters can understand.
pub fn from_claude(req: ClaudeNativeRequest, tool_compat_mode: bool) -> Result<UnifiedRequest> {
    // Validate Claude request parameters before proceeding.
    if let Err(e) = req.validate() {
        anyhow::bail!("Claude request validation failed: {}", e);
    }

    // Use `flat_map` to handle the one-to-many transformation required for tool results.
    // A single Claude `user` message containing `tool_result` blocks needs to be
    // transformed into one or more `UnifiedMessage`s, each with the `Tool` role.
    let messages = req
        .messages
        .into_iter()
        .flat_map(|msg| -> Vec<anyhow::Result<UnifiedMessage>> {
            match msg.role.as_str() {
                "assistant" => {
                    // Assistant messages are converted directly.
                    let result = msg
                        .content
                        .into_iter()
                        .map(convert_claude_content_block)
                        .collect::<Result<Vec<Vec<_>>>>()
                        .map(|vecs| vecs.into_iter().flatten().collect())
                        .map(|content| UnifiedMessage {
                            role: UnifiedRole::Assistant,
                            content,
                            reasoning_content: None,
                        });
                    vec![result]
                }
                "user" => {
                    // For user messages, we need to check if they contain tool results.
                    let has_tool_result = msg
                        .content
                        .iter()
                        .any(|c| matches!(c, ClaudeNativeContentBlock::ToolResult { .. }));

                    if has_tool_result {
                        // This message contains tool results. We need to separate tool results from other content.
                        let mut messages = Vec::new();
                        let mut tool_result_blocks = Vec::new();
                        let mut other_blocks = Vec::new();

                        // Separate tool result blocks from other content blocks
                        for block in msg.content {
                            if matches!(block, ClaudeNativeContentBlock::ToolResult { .. }) {
                                tool_result_blocks.push(block);
                            } else {
                                other_blocks.push(block);
                            }
                        }

                        // Create Tool messages for each tool result block
                        for tool_block in tool_result_blocks {
                            messages.push(
                                convert_claude_content_block(tool_block).map(|unified_blocks| {
                                    UnifiedMessage {
                                        role: UnifiedRole::Tool,
                                        content: unified_blocks,
                                        reasoning_content: None,
                                    }
                                }),
                            );
                        }

                        // Create a User message for remaining content blocks (if any)
                        if !other_blocks.is_empty() {
                            let user_message = other_blocks
                                .into_iter()
                                .map(convert_claude_content_block)
                                .collect::<Result<Vec<Vec<_>>>>()
                                .map(|vecs| vecs.into_iter().flatten().collect())
                                .map(|content| UnifiedMessage {
                                    role: UnifiedRole::User,
                                    content,
                                    reasoning_content: None,
                                });
                            messages.push(user_message);
                        }

                        messages
                    } else {
                        // It's a regular user message without tool results.
                        let result = msg
                            .content
                            .into_iter()
                            .map(convert_claude_content_block)
                            .collect::<Result<Vec<Vec<_>>>>()
                            .map(|vecs| vecs.into_iter().flatten().collect())
                            .map(|content| UnifiedMessage {
                                role: UnifiedRole::User,
                                content,
                                reasoning_content: None,
                            });
                        vec![result]
                    }
                }
                _ => vec![Err(anyhow::anyhow!(
                    "Invalid role '{}' in Claude message. Only 'user' and 'assistant' are supported in messages array",
                    msg.role
                ))],
            }
        })
        .collect::<Result<Vec<_>>>()?;

    let tools = req.tools.map(|tools| {
        tools
            .into_iter()
            .map(|tool| UnifiedTool {
                name: tool.name,
                description: tool.description,
                input_schema: tool.input_schema,
            })
            .collect()
    });

    let tool_choice = req.tool_choice.map(|choice| match choice {
        ClaudeToolChoice::Auto => UnifiedToolChoice::Auto,
        ClaudeToolChoice::Any => UnifiedToolChoice::Required, // Claude's 'any' is closer to OpenAI's 'required'.
        ClaudeToolChoice::Tool { name } => UnifiedToolChoice::Tool { name },
    });

    Ok(UnifiedRequest {
        model: req.model,
        messages,
        system_prompt: req.system.map(|x| x.trim().to_string()),
        tools,
        tool_choice,
        // stream: false, // Stream handling is managed by the handler, not in the request body itself.
        stream: req.stream.unwrap_or(false),
        temperature: req
            .temperature
            .map(|t| clamp_to_protocol_range(t, ChatProtocol::Claude, Parameter::Temperature)),
        max_tokens: if req.max_tokens <= 0 {
            None
        } else {
            Some(req.max_tokens)
        },
        top_p: req
            .top_p
            .map(|p| clamp_to_protocol_range(p, ChatProtocol::Claude, Parameter::TopP)),
        top_k: req.top_k,
        stop_sequences: req.stop_sequences,
        // OpenAI-specific parameters
        presence_penalty: None,
        frequency_penalty: None,
        response_format: None,
        seed: None,
        user: req.metadata.as_ref().and_then(|m| m.user_id.clone()),
        logprobs: None,
        top_logprobs: None,
        // Claude-specific parameters
        metadata: req.metadata.map(|m| UnifiedMetadata { user_id: m.user_id }),
        thinking: req.thinking.map(|t| UnifiedThinking {
            budget_tokens: t.budget_tokens,
        }),
        cache_control: req.cache_control.map(|c| UnifiedCacheControl {
            cache_type: c.cache_type,
            ttl: c.ttl,
        }),
        // Gemini-specific parameters
        safety_settings: None,
        response_mime_type: None,
        response_schema: None,
        cached_content: None,
        tool_compat_mode,
        ..Default::default()
    })
}
