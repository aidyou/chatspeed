use serde_json::json;

use crate::ccproxy::{
    adapter::unified::{
        UnifiedContentBlock, UnifiedMessage, UnifiedRequest, UnifiedRole, UnifiedTool,
        UnifiedToolChoice,
    },
    gemini::{GeminiPart, GeminiRequest},
};

/// Converts a Gemini-compatible chat completion request into the `UnifiedRequest`.
pub fn from_gemini(
    req: GeminiRequest,
    tool_compat_mode: bool,
) -> Result<UnifiedRequest, anyhow::Error> {
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
        let role = match content.role.as_str() {
            "user" => UnifiedRole::User,
            "model" => UnifiedRole::Assistant,
            "function" => UnifiedRole::Tool,
            "system" => UnifiedRole::System, // Should be handled by system_instruction, but for safety
            _ => anyhow::bail!("Invalid or missing role in Gemini message"),
        };

        let content_blocks = content
            .parts
            .into_iter()
            .map(convert_gemini_part)
            .collect::<Result<Vec<_>, _>>()?;

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
                        description: Some(func_decl.description),
                        input_schema: func_decl.parameters,
                    })
            })
            .collect()
    });

    let tool_choice = req.tool_config.map(|config| {
        match config.function_calling_config.mode.to_uppercase().as_str() {
            "NONE" => UnifiedToolChoice::None,
            "AUTO" => UnifiedToolChoice::Auto,
            "ANY" => UnifiedToolChoice::Required,
            _ => UnifiedToolChoice::Auto, // Default to auto
        }
    });

    let stream = req
        .generation_config
        .as_ref()
        .and_then(|config| config.max_output_tokens)
        .is_none(); // Heuristic: If max_output_tokens is not set, assume streaming

    let temperature = req
        .generation_config
        .as_ref()
        .and_then(|config| config.temperature);
    let max_tokens = req
        .generation_config
        .as_ref()
        .and_then(|config| config.max_output_tokens);
    let top_p = req
        .generation_config
        .as_ref()
        .and_then(|config| config.top_p);
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
        response_format,
        response_mime_type,
        response_schema,
        safety_settings: req.safety_settings.clone(),
        tool_compat_mode,
    })
}

/// Converts a single Gemini content part to a `UnifiedContentBlock`.
fn convert_gemini_part(part: GeminiPart) -> Result<UnifiedContentBlock, anyhow::Error> {
    if let Some(text) = part.text {
        Ok(UnifiedContentBlock::Text { text })
    } else if let Some(function_call) = part.function_call {
        Ok(UnifiedContentBlock::ToolUse {
            id: uuid::Uuid::new_v4().to_string(), // Generate ID as Gemini might not provide one in request
            name: function_call.name,
            input: function_call.args,
        })
    } else if let Some(function_response) = part.function_response {
        // Gemini's function_response is a JSON object, we need to extract the relevant content
        let fc = function_response.response.to_string();
        let content_str = function_response.response["response"]
            .get("result")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| fc.as_str())
            .to_string();
        Ok(UnifiedContentBlock::ToolResult {
            tool_use_id: uuid::Uuid::new_v4().to_string(), // Generate ID
            content: content_str,
            is_error: false, // Gemini response doesn't directly indicate error here
        })
    } else if let Some(inline_data) = part.inline_data {
        Ok(UnifiedContentBlock::Image {
            media_type: inline_data.mime_type,
            data: inline_data.data,
        })
    } else if let Some(file_data) = part.file_data {
        // For file_data, we might need a new UnifiedContentBlock variant or convert to text/image if possible
        // For now, we'll convert it to a text block indicating the file URI.
        Ok(UnifiedContentBlock::Text {
            text: format!(
                "File data: {} ({})",
                file_data.file_uri, file_data.mime_type
            ),
        })
    } else {
        anyhow::bail!("Unsupported Gemini content part");
    }
}
