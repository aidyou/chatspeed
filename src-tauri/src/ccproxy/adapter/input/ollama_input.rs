use crate::ccproxy::{
    adapter::unified::{
        UnifiedContentBlock, UnifiedMessage, UnifiedRequest, UnifiedRole, UnifiedTool,
    },
    types::ollama::{OllamaChatCompletionRequest, OllamaMessage},
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

        let content = convert_ollama_message_to_content_blocks(msg)?;

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
                description: Some(tool.function.description),
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
        temperature: options.temperature,
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
    })
}

/// Converts an Ollama message into a vector of `UnifiedContentBlock`.
/// This function follows the pattern of `openai_input.rs`, which may have limitations
/// in handling tool results correctly.
fn convert_ollama_message_to_content_blocks(
    msg: OllamaMessage,
) -> Result<Vec<UnifiedContentBlock>> {
    let mut blocks = Vec::new();

    // Add text content. For 'tool' role, this is the tool's output.
    if !msg.content.is_empty() {
        blocks.push(UnifiedContentBlock::Text { text: msg.content });
    }

    // Add image content (for 'user' messages)
    if let Some(images) = msg.images {
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
                data: base64_data,
            });
        }
    }

    // Add tool calls (for 'assistant' messages)
    if let Some(tool_calls) = msg.tool_calls {
        for tc in tool_calls {
            blocks.push(UnifiedContentBlock::ToolUse {
                // Ollama doesn't provide a tool_call_id. This is a known limitation.
                id: "".to_string(),
                name: tc.function.name,
                input: tc.function.arguments,
            });
        }
    }

    Ok(blocks)
}
