use serde_json::{json, Value};

use crate::ccproxy::{
    adapter::{
        range_adapter::{clamp_to_protocol_range, Parameter},
        unified::{
            UnifiedContentBlock, UnifiedMessage, UnifiedRequest, UnifiedRole, UnifiedTool,
            UnifiedToolChoice,
        },
    },
    gemini::{GeminiPart, GeminiRequest},
    types::{ChatProtocol, TOOL_ARG_ERROR_REMINDER},
};

/// Converts a Gemini-compatible chat completion request into the `UnifiedRequest`.
pub fn from_gemini(
    req: GeminiRequest,
    tool_compat_mode: bool,
    generate_action: String,
) -> Result<UnifiedRequest, anyhow::Error> {
    // Validate Gemini request parameters
    if let Err(e) = req.validate() {
        anyhow::bail!("Gemini request validation failed: {}", e);
    }
    let mut messages = Vec::new();
    let mut system_prompt = None;

    // Process system instruction if present
    if let Some(sys_inst) = req.system_instruction {
        if let Some(text) = sys_inst
            .parts
            .into_iter()
            .filter_map(|p| p.text)
            .collect::<String>()
            .into()
        {
            system_prompt = Some(text);
        }
    }

    for content in req.contents {
        let role = match content.role.as_deref() {
            Some("user") => UnifiedRole::User,
            Some("model") => UnifiedRole::Assistant,
            Some("function") => UnifiedRole::Tool,
            Some("system") => UnifiedRole::System, // Should be handled by system_instruction, but for safety
            _ => anyhow::bail!("Invalid or missing role in Gemini message"),
        };

        let content_blocks: Vec<UnifiedContentBlock> = content
            .parts
            .into_iter()
            .map(convert_gemini_part)
            .collect::<Result<Vec<Vec<_>>, _>>()?
            .into_iter()
            .flatten()
            .collect();

        messages.push(UnifiedMessage {
            role,
            content: content_blocks,
            reasoning_content: None,
        });
    }

    let tools = req.tools.map(|tools| {
        tools
            .into_iter()
            .flat_map(|tool_list| {
                tool_list
                    .function_declarations
                    .into_iter()
                    .map(|func_decl| UnifiedTool {
                        name: func_decl.name,
                        description: func_decl.description,
                        input_schema: func_decl.parameters.unwrap_or(Value::Null),
                    })
            })
            .collect()
    });

    let tool_choice = req
        .tool_config
        .map(|config| match config.function_calling_config {
            Some(config) => match config.mode.as_str() {
                "NONE" => UnifiedToolChoice::None,
                "AUTO" => UnifiedToolChoice::Auto,
                "ANY" => UnifiedToolChoice::Required,
                _ => UnifiedToolChoice::Auto,
            },
            None => UnifiedToolChoice::Auto,
        });

    let stream = if generate_action == "streamGenerateContent".to_string() {
        true
    } else {
        false
    };

    let temperature = req
        .generation_config
        .as_ref()
        .and_then(|config| config.temperature)
        .map(|t| clamp_to_protocol_range(t, ChatProtocol::Gemini, Parameter::Temperature));
    let max_tokens = req.generation_config.as_ref().and_then(|config| {
        config
            .max_output_tokens
            .and_then(|m| if m <= 0 { None } else { Some(m) })
    });
    let top_p = req
        .generation_config
        .as_ref()
        .and_then(|config| config.top_p)
        .map(|p| clamp_to_protocol_range(p, ChatProtocol::Gemini, Parameter::TopP));
    let top_k = req
        .generation_config
        .as_ref()
        .and_then(|config| config.top_k);
    let stop_sequences = req
        .generation_config
        .as_ref()
        .and_then(|config| config.stop_sequences.clone());
    let response_mime_type = req
        .generation_config
        .as_ref()
        .and_then(|config| config.response_mime_type.clone());

    let response_schema = req
        .generation_config
        .as_ref()
        .map(|config| json!(&config.response_schema));

    let response_format = req.generation_config.as_ref().and_then(|config| {
        let mt = config.response_mime_type.as_ref().map(|mime| mime.as_str());
        match mt {
            Some("application/json") => Some(json!({"type": "json"})),
            Some(_) => Some(json!({"type": "text"})),
            None => None,
        }
    });

    Ok(UnifiedRequest {
        model: "gemini".to_string(), // Model name is often in the URL for Gemini
        messages,
        system_prompt,
        tools,
        tool_choice,
        stream,
        temperature,
        max_tokens,
        top_p,
        top_k,
        stop_sequences,
        // OpenAI-specific parameters - map Gemini response format to OpenAI
        presence_penalty: None,  // Gemini doesn't support presence penalty
        frequency_penalty: None, // Gemini doesn't support frequency penalty
        response_format,
        seed: None,         // Gemini doesn't support deterministic seeding
        user: None,         // Gemini doesn't have user field
        logprobs: None,     // Gemini doesn't support log probabilities
        top_logprobs: None, // Gemini doesn't support log probabilities
        // Claude-specific parameters - map Gemini fields to Claude equivalents
        metadata: None, // Gemini doesn't have user metadata
        thinking: req
            .generation_config
            .as_ref()
            .and_then(|config| config.thinking_config.as_ref())
            .map(
                |thinking| crate::ccproxy::adapter::unified::UnifiedThinking {
                    budget_tokens: thinking.thinking_budget,
                    include_thoughts: thinking.include_thoughts,
                },
            ),
        cache_control: None, // Gemini uses cached_content instead
        // Gemini-specific parameters
        safety_settings: req.safety_settings.clone(),
        response_mime_type,
        response_schema,
        cached_content: req.cached_content,
        tool_compat_mode,
        ..Default::default()
    })
}

/// Converts a single Gemini content part to a `UnifiedContentBlock`.
fn convert_gemini_part(part: GeminiPart) -> Result<Vec<UnifiedContentBlock>, anyhow::Error> {
    if let Some(text) = part.text {
        Ok(vec![UnifiedContentBlock::Text { text }])
    } else if let Some(function_call) = part.function_call {
        let tool_name = function_call.name;
        let arguments = function_call.args;
        let mut blocks = Vec::new();

        match arguments {
            Value::String(s) => match serde_json::from_str(&s) {
                Ok(parsed_json) => {
                    blocks.push(UnifiedContentBlock::ToolUse {
                        id: uuid::Uuid::new_v4().to_string(),
                        name: tool_name,
                        input: parsed_json,
                    });
                }
                Err(_) => {
                    log::warn!(
                        "Failed to parse Gemini tool arguments as JSON for tool '{}'. Original arguments: {}",
                        tool_name, s
                    );
                    blocks.extend(vec![
                        UnifiedContentBlock::Text {
                            text: format!(
                                "<ccp:failed_tool_call>\n<name>{}</name>\n<input>{}</input>\n</ccp:failed_tool_call>",
                                tool_name, s
                            ),
                        },
                        UnifiedContentBlock::Text {
                            text: TOOL_ARG_ERROR_REMINDER.to_string(),
                        },
                    ]);
                }
            },
            _ => {
                blocks.push(UnifiedContentBlock::ToolUse {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: tool_name,
                    input: arguments,
                });
            }
        }
        Ok(blocks)
    } else if let Some(function_response) = part.function_response {
        let fc = function_response.response.to_string();
        let content_str = function_response.response["response"]
            .get("result")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| fc.as_str())
            .to_string();
        Ok(vec![UnifiedContentBlock::ToolResult {
            tool_use_id: uuid::Uuid::new_v4().to_string(), // Generate ID
            content: content_str,
            is_error: false, // Gemini response doesn't directly indicate error here
        }])
    } else if let Some(inline_data) = part.inline_data {
        Ok(vec![UnifiedContentBlock::Image {
            media_type: inline_data.mime_type,
            data: inline_data.data,
        }])
    } else if let Some(file_data) = part.file_data {
        Ok(vec![UnifiedContentBlock::Text {
            text: format!(
                "File data: {} ({})",
                file_data.file_uri, file_data.mime_type
            ),
        }])
    } else {
        anyhow::bail!("Unsupported Gemini content part")
    }
}
