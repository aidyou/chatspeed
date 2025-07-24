use crate::ccproxy::{
    adapter::unified::{
        UnifiedContentBlock, UnifiedMessage, UnifiedRequest, UnifiedRole, UnifiedTool,
        UnifiedToolChoice,
    },
    types::openai::{OpenAIChatCompletionRequest, OpenAIMessageContent, OpenAIMessageContentPart},
};
use serde_json::Value;

/// Converts an OpenAI-compatible chat completion request into the `UnifiedRequest`.
pub fn from_openai(req: OpenAIChatCompletionRequest) -> Result<UnifiedRequest, anyhow::Error> {
    let mut messages = Vec::new();
    let mut system_prompt = None;

    for msg in req.messages {
        let role = match msg.role.as_deref() {
            Some("system") => {
                // Extract system prompt text and continue, as it's a top-level field in UnifiedRequest
                if let Some(OpenAIMessageContent::Text(text)) = msg.content {
                    system_prompt =
                        Some(format!("{}{}\n", system_prompt.unwrap_or_default(), text));
                }
                continue;
            }
            Some("user") => UnifiedRole::User,
            Some("assistant") => UnifiedRole::Assistant,
            Some("tool") => UnifiedRole::Tool,
            _ => anyhow::bail!("Invalid or missing role in OpenAI message"),
        };

        let content = convert_openai_content(msg.content, msg.tool_calls)?;

        messages.push(UnifiedMessage { role, content, reasoning_content: None });
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

    let tool_choice = req.tool_choice.map(convert_openai_tool_choice);

    Ok(UnifiedRequest {
        model: req.model,
        messages,
        system_prompt: system_prompt.map(|s: String| s.trim().to_string()),
        tools,
        tool_choice,
        stream: req.stream.unwrap_or(false),
        temperature: req.temperature,
        max_tokens: req.max_tokens,
        top_p: req.top_p,
        top_k: req.top_k,
        stop_sequences: req.stop,
        response_format: req.response_format,
        safety_settings: None,
        response_mime_type: None,
        response_schema: None,
    })
}

/// Converts OpenAI message content into a vector of `UnifiedContentBlock`.
fn convert_openai_content(
    content: Option<OpenAIMessageContent>,
    tool_calls: Option<Vec<crate::ccproxy::types::openai::UnifiedToolCall>>,
) -> Result<Vec<UnifiedContentBlock>, anyhow::Error> {
    let mut blocks = Vec::new();

    if let Some(content) = content {
        match content {
            OpenAIMessageContent::Text(text) => {
                blocks.push(UnifiedContentBlock::Text { text });
            }
            OpenAIMessageContent::Parts(parts) => {
                for part in parts {
                    match part {
                        OpenAIMessageContentPart::Text { text } => {
                            blocks.push(UnifiedContentBlock::Text { text });
                        }
                        OpenAIMessageContentPart::ImageUrl { image_url } => {
                            if image_url.url.starts_with("data:") {
                                if let Some(comma_idx) = image_url.url.find(',') {
                                    let header = &image_url.url[5..comma_idx];
                                    let data = &image_url.url[comma_idx + 1..];
                                    let media_type = header
                                        .split(';')
                                        .next()
                                        .unwrap_or("application/octet-stream")
                                        .to_string();
                                    blocks.push(UnifiedContentBlock::Image {
                                        media_type,
                                        data: data.to_string(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if let Some(tool_calls) = tool_calls {
        for tc in tool_calls {
            blocks.push(UnifiedContentBlock::ToolUse {
                id: tc.id.unwrap_or_default(),
                name: tc.function.name.unwrap_or_default(),
                input: serde_json::from_str(
                    &tc.function.arguments.unwrap_or_else(|| "{}".to_string()),
                )?,
            });
        }
    }

    Ok(blocks)
}

/// Converts OpenAI's tool_choice format to the `UnifiedToolChoice`.
fn convert_openai_tool_choice(choice: Value) -> UnifiedToolChoice {
    match choice {
        Value::String(s) => match s.as_str() {
            "none" => UnifiedToolChoice::None,
            "auto" => UnifiedToolChoice::Auto,
            "required" => UnifiedToolChoice::Required,
            _ => UnifiedToolChoice::Auto,
        },
        Value::Object(obj) => {
            if obj.get("type").and_then(Value::as_str) == Some("function") {
                if let Some(name) = obj
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(Value::as_str)
                {
                    return UnifiedToolChoice::Tool {
                        name: name.to_string(),
                    };
                }
            }
            UnifiedToolChoice::Auto
        }
        _ => UnifiedToolChoice::Auto,
    }
}
