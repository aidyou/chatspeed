use crate::ccproxy::{
    adapter::{
        range_adapter::{clamp_to_protocol_range, Parameter},
        unified::{
            UnifiedContentBlock, UnifiedMessage, UnifiedRequest, UnifiedRole, UnifiedTool,
            UnifiedToolChoice,
        },
    },
    types::{
        openai_responses::{
            OpenAIResponsesContent, OpenAIResponsesInput, OpenAIResponsesInputItem,
            OpenAIResponsesInstructions, OpenAIResponsesRequest, OpenAIResponsesTool,
        },
        ChatProtocol,
    },
};
use serde_json::Value;
use std::collections::HashSet;

/// Converts an OpenAI Responses API request into the canonical chat adaptation request.
pub fn from_openai_responses(
    req: OpenAIResponsesRequest,
    tool_compat_mode: bool,
) -> Result<UnifiedRequest, anyhow::Error> {
    let OpenAIResponsesRequest {
        model,
        input,
        instructions,
        max_output_tokens,
        max_tokens,
        temperature,
        top_p,
        tools,
        tool_choice,
        text,
        reasoning,
        stream,
        user,
        ..
    } = req;

    let mut response_tools = tools.unwrap_or_default();
    if let Some(OpenAIResponsesInput::Items(items)) = input.as_ref() {
        response_tools.extend(
            items
                .iter()
                .filter_map(|item| item.tools.as_ref())
                .flatten()
                .cloned(),
        );
    }

    let (mut messages, input_system_prompt) = match input {
        Some(input) => responses_input_to_unified_messages(input)?,
        None => (Vec::new(), None),
    };
    let instruction_prompt = instructions.and_then(responses_instructions_to_text);
    let system_prompt = match (instruction_prompt, input_system_prompt) {
        (Some(instructions), Some(input_system_prompt)) if !input_system_prompt.is_empty() => Some(
            format!("{}\n{}", instructions.trim_end(), input_system_prompt),
        ),
        (Some(instructions), _) => Some(instructions),
        (None, Some(input_system_prompt)) => Some(input_system_prompt),
        (None, None) => None,
    };

    if messages.is_empty() {
        messages.push(UnifiedMessage {
            role: UnifiedRole::User,
            content: vec![UnifiedContentBlock::Text {
                text: String::new(),
            }],
            reasoning_content: None,
        });
    }

    let (tools, responses_custom_tool_names) = responses_tools_to_unified(response_tools);
    let tool_choice = if tools.is_some() {
        tool_choice.map(convert_openai_tool_choice)
    } else {
        None
    };

    let response_format = text
        .as_ref()
        .and_then(|text| text.format.as_ref())
        .map(|format| serde_json::to_value(format).unwrap_or(serde_json::Value::Null));

    Ok(UnifiedRequest {
        model,
        messages,
        system_prompt,
        tools,
        responses_custom_tool_names,
        tool_choice,
        stream: stream.unwrap_or(false),
        temperature: temperature.and_then(|t| {
            if t < 0.0 {
                None
            } else {
                Some(clamp_to_protocol_range(
                    t,
                    ChatProtocol::OpenAI,
                    Parameter::Temperature,
                ))
            }
        }),
        max_tokens: max_output_tokens.or(max_tokens).and_then(
            |t| {
                if t <= 0 {
                    None
                } else {
                    Some(t)
                }
            },
        ),
        top_p: top_p.map(|p| clamp_to_protocol_range(p, ChatProtocol::OpenAI, Parameter::TopP)),
        response_format,
        reasoning_effort: reasoning.and_then(|r| r.effort),
        user: user.clone(),
        metadata: user.map(
            |user_id| crate::ccproxy::adapter::unified::UnifiedMetadata {
                user_id: Some(user_id),
            },
        ),
        response_mime_type: text
            .as_ref()
            .and_then(|text| text.format.as_ref())
            .map(|format| {
                if format.format_type == "json_object" || format.format_type == "json_schema" {
                    "application/json".to_string()
                } else {
                    "text/plain".to_string()
                }
            }),
        response_schema: text
            .as_ref()
            .and_then(|text| text.format.as_ref())
            .and_then(|format| format.json_schema.clone()),
        tool_compat_mode,
        ..Default::default()
    })
}

fn responses_tools_to_unified(
    tools: Vec<OpenAIResponsesTool>,
) -> (Option<Vec<UnifiedTool>>, HashSet<String>) {
    let mut names = HashSet::new();
    let mut custom_tool_names = HashSet::new();
    let mut unified_tools = Vec::new();

    for tool in tools {
        let (tool, is_custom) = match tool {
            OpenAIResponsesTool::Chat(tool) => (
                UnifiedTool {
                    name: tool.function.name,
                    description: tool.function.description,
                    input_schema: tool.function.parameters,
                },
                false,
            ),
            OpenAIResponsesTool::Function(tool) if tool.tool_type == "function" => (
                UnifiedTool {
                    name: tool.name,
                    description: tool.description,
                    input_schema: tool.parameters,
                },
                false,
            ),
            OpenAIResponsesTool::Function(tool) if tool.tool_type == "custom" => (
                UnifiedTool {
                    name: tool.name,
                    description: tool.description,
                    input_schema: custom_tool_input_schema(tool.format.as_ref()),
                },
                true,
            ),
            OpenAIResponsesTool::Function(_) | OpenAIResponsesTool::Other(_) => continue,
        };

        if !names.insert(tool.name.clone()) {
            continue;
        }
        if is_custom {
            custom_tool_names.insert(tool.name.clone());
        }
        unified_tools.push(tool);
    }

    let tools = if unified_tools.is_empty() {
        None
    } else {
        Some(unified_tools)
    };
    (tools, custom_tool_names)
}

fn custom_tool_input_schema(format: Option<&Value>) -> Value {
    let input_description = format
        .map(|format| format!("Raw input for the custom tool. Format: {}", format))
        .unwrap_or_else(|| "Raw input for the custom tool.".to_string());
    serde_json::json!({
        "type": "object",
        "properties": {
            "input": {
                "type": "string",
                "description": input_description
            }
        },
        "required": ["input"],
        "additionalProperties": false
    })
}

fn responses_instructions_to_text(instructions: OpenAIResponsesInstructions) -> Option<String> {
    match instructions {
        OpenAIResponsesInstructions::Text(text) => Some(text),
        OpenAIResponsesInstructions::Items(items) => {
            let parts = items
                .into_iter()
                .filter_map(|item| responses_input_item_to_unified_message(item).ok().flatten())
                .flat_map(|message| message.content)
                .filter_map(|block| match block {
                    UnifiedContentBlock::Text { text } => Some(text),
                    _ => None,
                })
                .collect::<Vec<_>>();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n"))
            }
        }
    }
}

fn responses_input_to_unified_messages(
    input: OpenAIResponsesInput,
) -> Result<(Vec<UnifiedMessage>, Option<String>), anyhow::Error> {
    match input {
        OpenAIResponsesInput::Text(text) => Ok((
            vec![UnifiedMessage {
                role: UnifiedRole::User,
                content: vec![UnifiedContentBlock::Text { text }],
                reasoning_content: None,
            }],
            None,
        )),
        OpenAIResponsesInput::Items(items) => {
            let mut messages = Vec::new();
            let mut system_parts = Vec::new();
            for item in items {
                if let Some(message) = responses_input_item_to_unified_message(item)? {
                    if message.role == UnifiedRole::System {
                        let text = message
                            .content
                            .into_iter()
                            .filter_map(|block| match block {
                                UnifiedContentBlock::Text { text } => Some(text),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        if !text.is_empty() {
                            system_parts.push(text);
                        }
                    } else {
                        messages.push(message);
                    }
                }
            }
            let system_prompt = if system_parts.is_empty() {
                None
            } else {
                Some(system_parts.join("\n"))
            };
            Ok((messages, system_prompt))
        }
    }
}

fn responses_input_item_to_unified_message(
    item: OpenAIResponsesInputItem,
) -> Result<Option<UnifiedMessage>, anyhow::Error> {
    let item_type = item.item_type.as_deref();
    let role = match item.role.as_deref().or(item_type) {
        Some("system") | Some("developer") => UnifiedRole::System,
        Some("assistant") | Some("message") => UnifiedRole::Assistant,
        Some("tool") | Some("function_call_output") | Some("custom_tool_call_output") => {
            UnifiedRole::Tool
        }
        Some("user") | Some("input_message") | None => UnifiedRole::User,
        Some(_) => UnifiedRole::User,
    };

    if matches!(
        item_type,
        Some("function_call_output") | Some("custom_tool_call_output")
    ) {
        return Ok(Some(UnifiedMessage {
            role: UnifiedRole::Tool,
            content: vec![UnifiedContentBlock::ToolResult {
                tool_use_id: item.call_id.or(item.id).unwrap_or_default(),
                content: responses_tool_output_to_text(item.output)?,
                is_error: false,
            }],
            reasoning_content: None,
        }));
    }

    if matches!(item_type, Some("function_call") | Some("custom_tool_call")) {
        let raw_input = item.arguments.or(item.input).unwrap_or_default();
        let input = if item_type == Some("custom_tool_call") {
            serde_json::json!({ "input": raw_input })
        } else {
            serde_json::from_str(&raw_input).unwrap_or(Value::String(raw_input))
        };
        return Ok(Some(UnifiedMessage {
            role: UnifiedRole::Assistant,
            content: vec![UnifiedContentBlock::ToolUse {
                id: item.call_id.or(item.id).unwrap_or_default(),
                name: item.name.unwrap_or_default(),
                input,
            }],
            reasoning_content: None,
        }));
    }

    let content = match item.content {
        Some(OpenAIResponsesContent::Text(text)) => vec![UnifiedContentBlock::Text { text }],
        Some(OpenAIResponsesContent::Parts(parts)) => {
            let mut blocks = Vec::new();
            for part in parts {
                match part.part_type.as_str() {
                    "input_text" | "output_text" | "text" => {
                        if let Some(text) = part.text {
                            blocks.push(UnifiedContentBlock::Text { text });
                        }
                    }
                    "input_image" | "image_url" => {
                        if let Some(url) = part.image_url.and_then(image_url_value_to_string) {
                            if url.starts_with("data:") {
                                if let Some(comma_idx) = url.find(',') {
                                    let header = &url[5..comma_idx];
                                    let data = &url[comma_idx + 1..];
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
                    _ => {}
                }
            }
            blocks
        }
        None => Vec::new(),
    };

    if content.is_empty() {
        Ok(None)
    } else {
        Ok(Some(UnifiedMessage {
            role,
            content,
            reasoning_content: None,
        }))
    }
}

fn responses_tool_output_to_text(output: Option<Value>) -> Result<String, anyhow::Error> {
    match output {
        None => Ok(String::new()),
        Some(Value::String(text)) => Ok(text),
        Some(Value::Array(parts)) => parts
            .into_iter()
            .map(|part| {
                part.get("text")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
                    .map(Ok)
                    .unwrap_or_else(|| serde_json::to_string(&part).map_err(Into::into))
            })
            .collect::<Result<Vec<_>, anyhow::Error>>()
            .map(|parts| parts.join("\n")),
        Some(output) => serde_json::to_string(&output).map_err(Into::into),
    }
}

fn image_url_value_to_string(value: Value) -> Option<String> {
    match value {
        Value::String(url) => Some(url),
        Value::Object(map) => map
            .get("url")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        _ => None,
    }
}

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
                if let Some(name) = obj.get("name").and_then(Value::as_str).or_else(|| {
                    obj.get("function")
                        .and_then(Value::as_object)
                        .and_then(|function| function.get("name"))
                        .and_then(Value::as_str)
                }) {
                    UnifiedToolChoice::Tool {
                        name: name.to_string(),
                    }
                } else {
                    UnifiedToolChoice::Auto
                }
            } else {
                UnifiedToolChoice::Auto
            }
        }
        _ => UnifiedToolChoice::Auto,
    }
}

#[cfg(test)]
mod tests {
    use super::from_openai_responses;
    use crate::ccproxy::adapter::unified::{UnifiedContentBlock, UnifiedRole, UnifiedToolChoice};
    use crate::ccproxy::types::openai_responses::OpenAIResponsesRequest;
    use serde_json::json;

    #[test]
    fn text_input_preserves_streaming_user_message() {
        let req: OpenAIResponsesRequest = serde_json::from_value(json!({
            "model": "gpt-5.2",
            "input": "Hello",
            "stream": true,
            "max_output_tokens": 128,
            "reasoning": { "effort": "low" }
        }))
        .expect("request should deserialize");

        let unified = from_openai_responses(req, false).expect("conversion should succeed");
        assert_eq!(unified.model, "gpt-5.2");
        assert!(unified.stream);
        assert_eq!(unified.max_tokens, Some(128));
        assert_eq!(unified.reasoning_effort.as_deref(), Some("low"));
        assert_eq!(unified.messages.len(), 1);
        assert_eq!(unified.messages[0].role, UnifiedRole::User);
        assert!(matches!(
            &unified.messages[0].content[0],
            UnifiedContentBlock::Text { text } if text == "Hello"
        ));
    }

    #[test]
    fn items_fold_system_prompt_and_map_function_tools() {
        let req: OpenAIResponsesRequest = serde_json::from_value(json!({
            "model": "alias",
            "instructions": "Base instruction.",
            "input": [
                {
                    "role": "developer",
                    "content": [{ "type": "input_text", "text": "Use concise answers." }]
                },
                {
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "Ping" }]
                }
            ],
            "tools": [
                {
                    "type": "function",
                    "name": "lookup",
                    "description": "Look up a value",
                    "parameters": { "type": "object", "properties": {} }
                }
            ],
            "tool_choice": "required"
        }))
        .expect("request should deserialize");

        let unified = from_openai_responses(req, true).expect("conversion should succeed");
        assert_eq!(
            unified.system_prompt.as_deref(),
            Some("Base instruction.\nUse concise answers.")
        );
        assert_eq!(unified.messages.len(), 1);
        assert_eq!(unified.messages[0].role, UnifiedRole::User);
        assert_eq!(unified.tools.as_ref().map(Vec::len), Some(1));
        assert_eq!(unified.tools.as_ref().unwrap()[0].name, "lookup");
        assert_eq!(unified.tool_choice, Some(UnifiedToolChoice::Required));
        assert!(unified.tool_compat_mode);
    }

    #[test]
    fn official_non_chat_fields_do_not_block_fallback_conversion() {
        let req: OpenAIResponsesRequest = serde_json::from_value(json!({
            "model": "alias",
            "background": true,
            "include": ["file_search_call.results"],
            "input": [
                {
                    "role": "user",
                    "content": [
                        { "type": "input_text", "text": "Read this." },
                        { "type": "input_file", "file_id": "file_123" },
                        {
                            "type": "input_image",
                            "image_url": {
                                "url": "data:image/png;base64,AAAA"
                            },
                            "detail": "low"
                        }
                    ]
                }
            ],
            "tools": [
                { "type": "web_search_preview" },
                {
                    "type": "function",
                    "name": "lookup",
                    "parameters": { "type": "object", "properties": {} }
                }
            ],
            "tool_choice": {
                "type": "function",
                "name": "lookup"
            },
            "parallel_tool_calls": true,
            "prompt": { "id": "pmpt_123" },
            "service_tier": "auto",
            "top_logprobs": 2,
            "truncation": "auto"
        }))
        .expect("request should deserialize");

        let unified = from_openai_responses(req, false).expect("conversion should succeed");
        assert_eq!(unified.messages.len(), 1);
        assert_eq!(unified.messages[0].content.len(), 2);
        assert!(matches!(
            &unified.messages[0].content[0],
            UnifiedContentBlock::Text { text } if text == "Read this."
        ));
        assert!(matches!(
            &unified.messages[0].content[1],
            UnifiedContentBlock::Image { media_type, data }
                if media_type == "image/png" && data == "AAAA"
        ));
        assert_eq!(unified.tools.as_ref().map(Vec::len), Some(1));
        assert_eq!(unified.tools.as_ref().unwrap()[0].name, "lookup");
        assert_eq!(
            unified.tool_choice,
            Some(UnifiedToolChoice::Tool {
                name: "lookup".to_string()
            })
        );
    }

    #[test]
    fn custom_tool_history_preserves_call_and_structured_output() {
        let custom_input =
            "const result = await tools.exec_command({ cmd: userCommand });\nreturn result.output;";
        let req: OpenAIResponsesRequest = serde_json::from_value(json!({
            "model": "gpt-5.6",
            "input": [
                {
                    "role": "developer",
                    "tools": [
                        {
                            "type": "custom",
                            "name": "functions.exec",
                            "description": "Run JavaScript",
                            "format": {
                                "type": "grammar",
                                "syntax": "lark",
                                "definition": "start: /[\\s\\S]+/"
                            }
                        }
                    ]
                },
                {
                    "type": "custom_tool_call",
                    "call_id": "call_123",
                    "name": "functions.exec",
                    "input": custom_input
                },
                {
                    "type": "custom_tool_call_output",
                    "call_id": "call_123",
                    "output": [
                        {
                            "type": "input_text",
                            "text": "Process exited with code 0"
                        },
                        {
                            "type": "input_file",
                            "file_id": "file_123"
                        }
                    ]
                }
            ]
        }))
        .expect("request should deserialize");

        let unified = from_openai_responses(req, false).expect("conversion should succeed");
        assert_eq!(unified.messages.len(), 2);
        assert!(unified
            .responses_custom_tool_names
            .contains("functions.exec"));
        let tools = unified
            .tools
            .as_ref()
            .expect("custom tool should be retained");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "functions.exec");
        assert_eq!(tools[0].input_schema["type"], "object");
        assert_eq!(
            tools[0].input_schema["properties"]["input"]["type"],
            "string"
        );
        assert_eq!(tools[0].input_schema["required"], json!(["input"]));
        assert_eq!(unified.messages[0].role, UnifiedRole::Assistant);
        assert!(matches!(
            &unified.messages[0].content[0],
            UnifiedContentBlock::ToolUse { id, name, input }
                if id == "call_123"
                    && name == "functions.exec"
                    && input == &json!({ "input": custom_input })
        ));

        assert_eq!(unified.messages[1].role, UnifiedRole::Tool);
        let UnifiedContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } = &unified.messages[1].content[0]
        else {
            panic!("expected custom tool result");
        };
        assert_eq!(tool_use_id, "call_123");
        assert!(!is_error);
        let (text, structured_part) = content
            .split_once('\n')
            .expect("structured output parts should remain present");
        assert_eq!(text, "Process exited with code 0");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(structured_part)
                .expect("non-text output part should remain valid JSON"),
            json!({ "type": "input_file", "file_id": "file_123" })
        );
    }

    #[test]
    fn function_tool_history_preserves_json_arguments_and_string_output() {
        let req: OpenAIResponsesRequest = serde_json::from_value(json!({
            "model": "alias",
            "input": [
                {
                    "type": "function_call",
                    "id": "item_123",
                    "call_id": "call_456",
                    "name": "lookup",
                    "arguments": "{\"query\":\"status\"}"
                },
                {
                    "type": "function_call_output",
                    "call_id": "call_456",
                    "output": "ready"
                }
            ]
        }))
        .expect("request should deserialize");

        let unified = from_openai_responses(req, false).expect("conversion should succeed");
        assert_eq!(unified.messages.len(), 2);
        assert!(matches!(
            &unified.messages[0].content[0],
            UnifiedContentBlock::ToolUse { id, name, input }
                if id == "call_456" && name == "lookup" && input == &json!({ "query": "status" })
        ));
        assert!(matches!(
            &unified.messages[1].content[0],
            UnifiedContentBlock::ToolResult { tool_use_id, content, is_error }
                if tool_use_id == "call_456" && content == "ready" && !is_error
        ));
    }

    #[test]
    fn missing_input_falls_back_to_empty_user_message() {
        let req: OpenAIResponsesRequest = serde_json::from_value(json!({
            "model": "alias",
            "instructions": [
                {
                    "role": "developer",
                    "content": [{ "type": "input_text", "text": "Stay brief." }]
                }
            ]
        }))
        .expect("request should deserialize");

        let unified = from_openai_responses(req, false).expect("conversion should succeed");
        assert_eq!(unified.system_prompt.as_deref(), Some("Stay brief."));
        assert_eq!(unified.messages.len(), 1);
        assert_eq!(unified.messages[0].role, UnifiedRole::User);
        assert!(matches!(
            &unified.messages[0].content[0],
            UnifiedContentBlock::Text { text } if text.is_empty()
        ));
    }
}
