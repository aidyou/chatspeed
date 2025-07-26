use crate::ccproxy::{
    adapter::unified::{
        UnifiedContentBlock, UnifiedMessage, UnifiedRequest, UnifiedRole, UnifiedTool,
        UnifiedToolChoice,
    },
    claude::ClaudeToolChoice,
    types::claude::{ClaudeNativeContentBlock, ClaudeNativeRequest},
};

/// Converts a Claude-native chat completion request into the `UnifiedRequest`.
pub fn from_claude(
    req: ClaudeNativeRequest,
    tool_compat_mode: bool,
) -> Result<UnifiedRequest, anyhow::Error> {
    let messages = req
        .messages
        .into_iter()
        .map(|msg| {
            let role = match msg.role.as_str() {
                "user" => UnifiedRole::User,
                "assistant" => UnifiedRole::Assistant,
                _ => anyhow::bail!("Invalid role '{}' in Claude message. Only 'user' and 'assistant' are supported in messages array", msg.role),
            };
            let content = msg
                .content
                .into_iter()
                .map(convert_claude_content_block)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(UnifiedMessage {
                role,
                content,
                reasoning_content: None,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

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
        ClaudeToolChoice::Any => UnifiedToolChoice::Required, // Claude's 'any' is closer to OpenAI's 'required'
        ClaudeToolChoice::Tool { name } => UnifiedToolChoice::Tool { name },
    });

    Ok(UnifiedRequest {
        model: req.model,
        messages,
        system_prompt: req.system,
        tools,
        tool_choice,
        stream: false,
        // stream: req.stream.unwrap_or(false),
        temperature: req.temperature,
        max_tokens: Some(req.max_tokens),
        top_p: req.top_p,
        top_k: req.top_k,
        stop_sequences: None, // Claude API doesn't have a direct stop sequence field in the request
        response_format: None,
        safety_settings: None,
        response_mime_type: None,
        response_schema: None,
        tool_compat_mode,
    })
}

/// Converts a single Claude content block to a `UnifiedContentBlock`.
fn convert_claude_content_block(
    block: ClaudeNativeContentBlock,
) -> Result<UnifiedContentBlock, anyhow::Error> {
    match block {
        ClaudeNativeContentBlock::Text { text } => Ok(UnifiedContentBlock::Text { text }),
        ClaudeNativeContentBlock::Image { source } => Ok(UnifiedContentBlock::Image {
            media_type: source.media_type,
            data: source.data,
        }),
        ClaudeNativeContentBlock::ToolUse { id, name, input } => {
            Ok(UnifiedContentBlock::ToolUse { id, name, input })
        }
        ClaudeNativeContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => Ok(UnifiedContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error: is_error.unwrap_or(false),
        }),
        ClaudeNativeContentBlock::Thinking { .. } => {
            // 'Thinking' blocks are part of responses, not requests. We can ignore them here.
            anyhow::bail!("'Thinking' content block is not supported in requests.")
        }
    }
}
