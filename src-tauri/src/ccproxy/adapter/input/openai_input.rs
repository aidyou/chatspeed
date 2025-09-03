use crate::ccproxy::{
    adapter::{
        range_adapter::{clamp_to_protocol_range, Parameter},
        unified::{
            UnifiedContentBlock, UnifiedMessage, UnifiedRequest, UnifiedRole, UnifiedTool,
            UnifiedToolChoice,
        },
    },
    types::{
        openai::{
            OpenAIChatCompletionRequest, OpenAIMessageContent, OpenAIMessageContentPart,
            OpenAIToolChoice,
        },
        ChatProtocol,
    },
};

/// Converts an OpenAI-compatible chat completion request into the `UnifiedRequest`.
pub fn from_openai(
    req: OpenAIChatCompletionRequest,
    tool_compat_mode: bool,
) -> Result<UnifiedRequest, anyhow::Error> {
    // Validate OpenAI request parameters
    if let Err(e) = req.validate() {
        anyhow::bail!("OpenAI request validation failed: {}", e);
    }
    let mut messages = Vec::new();
    let mut system_prompt = None;

    for msg in req.messages {
        let role = match msg.role.as_deref() {
            Some("system") => {
                // Extract system prompt text and continue, as it's a top-level field in UnifiedRequest
                if let Some(OpenAIMessageContent::Text(text)) = msg.content {
                    system_prompt = Some(format!(
                        "{}{}\n",
                        system_prompt.unwrap_or_default(),
                        text.trim()
                    ));
                }
                continue;
            }
            Some("user") => UnifiedRole::User,
            Some("assistant") => UnifiedRole::Assistant,
            Some("tool") => UnifiedRole::Tool,
            _ => anyhow::bail!("Invalid or missing role in OpenAI message"),
        };

        let content = if role == UnifiedRole::Tool {
            let tool_call_id = msg.tool_call_id.clone().unwrap_or_default();
            let tool_content = if let Some(OpenAIMessageContent::Text(text)) = msg.content {
                text
            } else {
                // According to OpenAI spec, the content of a tool message is a string and required.
                // If it's missing or not text, we can treat it as an empty string,
                // though this indicates a malformed request from the client.
                log::warn!(
                    "Tool message received without text content for tool_call_id: {}",
                    tool_call_id
                );
                String::new()
            };
            vec![UnifiedContentBlock::ToolResult {
                tool_use_id: tool_call_id,
                content: tool_content,
                is_error: false, // The native OpenAI format does not have an `is_error` field.
            }]
        } else {
            convert_openai_content(msg.content, msg.tool_calls)?
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

    let tool_choice = req.tool_choice.map(convert_openai_tool_choice);

    Ok(UnifiedRequest {
        model: req.model,
        messages,
        system_prompt: system_prompt.map(|s: String| s.clone()),
        tools,
        tool_choice,
        stream: req.stream.unwrap_or(false),
        temperature: req.temperature.map(|t| {
            // Clamp to OpenAI range first, then it will be adapted in backend adapters
            clamp_to_protocol_range(t, ChatProtocol::OpenAI, Parameter::Temperature)
        }),
        max_tokens: req
            .max_tokens
            .and_then(|t| if t <= 0 { None } else { Some(t) }),
        top_p: req
            .top_p
            .map(|p| clamp_to_protocol_range(p, ChatProtocol::OpenAI, Parameter::TopP)),
        top_k: None, // OpenAI doesn't support top_k directly
        stop_sequences: req.stop,
        // OpenAI-specific parameters
        presence_penalty: req
            .presence_penalty
            .map(|p| clamp_to_protocol_range(p, ChatProtocol::OpenAI, Parameter::PresencePenalty)),
        frequency_penalty: req
            .frequency_penalty
            .map(|p| clamp_to_protocol_range(p, ChatProtocol::OpenAI, Parameter::FrequencyPenalty)),
        response_format: req
            .response_format
            .as_ref()
            .map(|rf| serde_json::to_value(rf).unwrap_or(serde_json::Value::Null)),
        seed: req.seed,
        user: req.user.clone(),
        logprobs: req.logprobs,
        top_logprobs: req.top_logprobs,
        // Claude-specific parameters - map OpenAI user to Claude metadata.user_id
        metadata: req.user.map(
            |user_id| crate::ccproxy::adapter::unified::UnifiedMetadata {
                user_id: Some(user_id),
            },
        ),
        thinking: None,      // OpenAI doesn't support thinking mode
        cache_control: None, // OpenAI doesn't support cache control
        // Gemini-specific parameters - map OpenAI response_format to Gemini fields
        safety_settings: None, // OpenAI doesn't have safety settings
        response_mime_type: req.response_format.as_ref().and_then(|rf| {
            if rf.format_type == "json_object" {
                Some("application/json".to_string())
            } else {
                Some("text/plain".to_string())
            }
        }),
        response_schema: req
            .response_format
            .as_ref()
            .and_then(|rf| rf.json_schema.clone()),
        cached_content: None,
        tool_compat_mode,
        ..Default::default()
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
fn convert_openai_tool_choice(choice: OpenAIToolChoice) -> UnifiedToolChoice {
    match choice {
        OpenAIToolChoice::String(s) => match s.as_str() {
            "none" => UnifiedToolChoice::None,
            "auto" => UnifiedToolChoice::Auto,
            "required" => UnifiedToolChoice::Required,
            _ => UnifiedToolChoice::Auto,
        },
        OpenAIToolChoice::Object(obj) => {
            if obj.choice_type == "function" {
                UnifiedToolChoice::Tool {
                    name: obj.function.name,
                }
            } else {
                UnifiedToolChoice::Auto
            }
        }
    }
}
