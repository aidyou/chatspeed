//! A simple token estimator.

use serde_json::Value;

use crate::ccproxy::adapter::unified::{UnifiedContentBlock, UnifiedRequest};

/// A rough heuristic for token estimation.
///
/// This function provides a rough estimation of the number of tokens in a given text.
/// The estimation is based on the character type:
/// - For ASCII characters (common English, punctuation, numbers), it assumes approximately
///   3-4 characters per token, so each character contributes 0.3 to the count.
/// - For non-ASCII characters (assuming CJK and other languages), it uses a heuristic
///   of 1.5 tokens per character.
///
/// The final count is rounded up to the nearest whole number.
///
/// # Arguments
///
/// * `text` - A string slice to estimate the token count for.
///
/// # Returns
///
/// An estimated token count as a `f64`.
pub fn estimate_tokens(text: &str) -> f64 {
    let mut token_count: f64 = 0.0;
    for c in text.chars() {
        if c.is_ascii() {
            // Rough approximation for English text, punctuation, and numbers
            // 1 token ~ 3.3 chars, so 1 char ~ 0.3 tokens
            token_count += 0.3;
        } else {
            // For non-ASCII characters, assume they are mostly CJK.
            // 1.5 tokens per character is a reasonable estimate.
            token_count += 1.5;
        }
    }
    token_count
}

const IMAGE_BLOCK_PLACEHOLDER_TOKENS: f64 = 256.0;

fn estimate_json_value_tokens(value: &Value) -> f64 {
    match value {
        Value::Null => 0.0,
        Value::Bool(boolean) => estimate_tokens(if *boolean { "true" } else { "false" }),
        Value::Number(number) => estimate_tokens(&number.to_string()),
        Value::String(text) => estimate_tokens(text),
        Value::Array(items) => items.iter().map(estimate_json_value_tokens).sum(),
        Value::Object(map) => map
            .iter()
            .map(|(key, value)| estimate_tokens(key) + estimate_json_value_tokens(value))
            .sum(),
    }
}

fn estimate_data_url_tokens(url: &str) -> f64 {
    if url.starts_with("data:") {
        IMAGE_BLOCK_PLACEHOLDER_TOKENS
    } else {
        estimate_tokens(url)
    }
}

pub fn estimate_unified_request_tokens(request: &UnifiedRequest) -> f64 {
    let mut total = 0.0;

    if let Some(system_prompt) = request.system_prompt.as_deref() {
        total += estimate_tokens(system_prompt);
    }

    if let Some(combined_prompt) = request.combined_prompt.as_deref() {
        total += estimate_tokens(combined_prompt);
    }

    if let Some(prompt_injection) = request.prompt_injection.as_deref() {
        total += estimate_tokens(prompt_injection);
    }

    if let Some(prompt_enhance_text) = request.prompt_enhance_text.as_deref() {
        total += estimate_tokens(prompt_enhance_text);
    }

    if let Some(response_mime_type) = request.response_mime_type.as_deref() {
        total += estimate_tokens(response_mime_type);
    }

    if let Some(cached_content) = request.cached_content.as_deref() {
        total += estimate_tokens(cached_content);
    }

    if let Some(reasoning_effort) = request.reasoning_effort.as_deref() {
        total += estimate_tokens(reasoning_effort);
    }

    if let Some(response_schema) = request.response_schema.as_ref() {
        total += estimate_json_value_tokens(response_schema);
    }

    if let Some(custom_params) = request.custom_params.as_ref() {
        total += estimate_json_value_tokens(custom_params);
    }

    if let Some(tool_choice) = request.tool_choice.as_ref() {
        total += estimate_tokens(&format!("{tool_choice:?}"));
    }

    if let Some(metadata) = request
        .metadata
        .as_ref()
        .and_then(|meta| meta.user_id.as_deref())
    {
        total += estimate_tokens(metadata);
    }

    if let Some(thinking) = request.thinking.as_ref() {
        if let Some(budget_tokens) = thinking.budget_tokens {
            total += estimate_tokens(&budget_tokens.to_string());
        }
    }

    if let Some(tools) = request.tools.as_ref() {
        for tool in tools {
            total += estimate_tokens(&tool.name);
            if let Some(description) = tool.description.as_deref() {
                total += estimate_tokens(description);
            }
            total += estimate_json_value_tokens(&tool.input_schema);
        }
    }

    for message in &request.messages {
        total += estimate_tokens(&format!("{:?}", message.role));

        if let Some(reasoning_content) = message.reasoning_content.as_deref() {
            total += estimate_tokens(reasoning_content);
        }

        for block in &message.content {
            total += match block {
                UnifiedContentBlock::Text { text } => estimate_tokens(text),
                UnifiedContentBlock::Image { media_type, data } => {
                    estimate_tokens(media_type) + estimate_data_url_tokens(data)
                }
                UnifiedContentBlock::ToolUse { id, name, input } => {
                    estimate_tokens(id) + estimate_tokens(name) + estimate_json_value_tokens(input)
                }
                UnifiedContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } => {
                    estimate_tokens(tool_use_id)
                        + estimate_tokens(content)
                        + estimate_tokens(if *is_error { "true" } else { "false" })
                }
                UnifiedContentBlock::Thinking { thinking } => estimate_tokens(thinking),
            };
        }
    }

    total
}

pub fn estimate_known_request_json_tokens(body: &Value) -> f64 {
    let mut total = 0.0;

    if let Some(system) = body.get("system").and_then(|value| value.as_str()) {
        total += estimate_tokens(system);
    }

    if let Some(prompt) = body.get("prompt").and_then(|value| value.as_str()) {
        total += estimate_tokens(prompt);
    }

    if let Some(input) = body.get("input") {
        total += estimate_json_value_tokens(input);
    }

    if let Some(messages) = body.get("messages").and_then(|value| value.as_array()) {
        for message in messages {
            if let Some(role) = message.get("role").and_then(|value| value.as_str()) {
                total += estimate_tokens(role);
            }

            if let Some(reasoning) = message
                .get("reasoning_content")
                .or_else(|| message.get("reasoning"))
                .and_then(|value| value.as_str())
            {
                total += estimate_tokens(reasoning);
            }

            if let Some(content) = message.get("content") {
                total += estimate_openai_like_content_tokens(content);
            }

            if let Some(tool_calls) = message.get("tool_calls") {
                total += estimate_json_value_tokens(tool_calls);
            }

            if let Some(tool_call_id) = message.get("tool_call_id").and_then(|value| value.as_str())
            {
                total += estimate_tokens(tool_call_id);
            }

            if let Some(name) = message.get("name").and_then(|value| value.as_str()) {
                total += estimate_tokens(name);
            }
        }
    }

    if let Some(tools) = body.get("tools") {
        total += estimate_json_value_tokens(tools);
    }

    if let Some(tool_choice) = body.get("tool_choice") {
        total += estimate_json_value_tokens(tool_choice);
    }

    if let Some(response_format) = body.get("response_format") {
        total += estimate_json_value_tokens(response_format);
    }

    total
}

fn estimate_openai_like_content_tokens(content: &Value) -> f64 {
    match content {
        Value::String(text) => estimate_tokens(text),
        Value::Array(parts) => parts
            .iter()
            .map(|part| match part {
                Value::Object(map) => {
                    let part_type = map
                        .get("type")
                        .and_then(|value| value.as_str())
                        .unwrap_or("");
                    match part_type {
                        "text" | "input_text" => map
                            .get("text")
                            .and_then(|value| value.as_str())
                            .map(estimate_tokens)
                            .unwrap_or(0.0),
                        "image_url" | "input_image" => map
                            .get("image_url")
                            .and_then(|value| match value {
                                Value::String(url) => Some(estimate_data_url_tokens(url)),
                                Value::Object(obj) => obj
                                    .get("url")
                                    .and_then(|value| value.as_str())
                                    .map(estimate_data_url_tokens),
                                _ => None,
                            })
                            .unwrap_or(IMAGE_BLOCK_PLACEHOLDER_TOKENS),
                        _ => estimate_json_value_tokens(part),
                    }
                }
                other => estimate_json_value_tokens(other),
            })
            .sum(),
        other => estimate_json_value_tokens(other),
    }
}

pub fn resolve_usage_with_estimate(
    protocol: &str,
    usage_input_tokens: u64,
    usage_output_tokens: u64,
    estimated_input_tokens: f64,
    estimated_output_tokens: f64,
    context: &str,
) -> (u64, u64) {
    let input_tokens = if usage_input_tokens > 0 {
        usage_input_tokens
    } else {
        estimated_input_tokens.ceil() as u64
    };
    let output_tokens = if usage_output_tokens > 0 {
        usage_output_tokens
    } else {
        estimated_output_tokens.ceil() as u64
    };

    if usage_input_tokens == 0 || usage_output_tokens == 0 {
        log::debug!(
            "[ccproxy][token_estimate][protocol={}][context={}] upstream usage incomplete, using estimated tokens input={} output={} (upstream input={} output={})",
            protocol,
            context,
            input_tokens,
            output_tokens,
            usage_input_tokens,
            usage_output_tokens
        );
    }

    (input_tokens, output_tokens)
}

#[cfg(test)]
mod tests {
    use super::{
        estimate_known_request_json_tokens, estimate_tokens, estimate_unified_request_tokens,
    };
    use crate::ccproxy::adapter::unified::{
        UnifiedContentBlock, UnifiedMessage, UnifiedRequest, UnifiedRole, UnifiedTool,
    };
    use serde_json::json;

    #[test]
    fn unified_request_estimate_includes_reasoning_and_tools() {
        let request = UnifiedRequest {
            system_prompt: Some("system prompt".to_string()),
            tools: Some(vec![UnifiedTool {
                name: "read_file".to_string(),
                description: Some("Read file content".to_string()),
                input_schema: json!({
                    "type": "object",
                    "properties": { "path": { "type": "string" } }
                }),
            }]),
            messages: vec![UnifiedMessage {
                role: UnifiedRole::Assistant,
                content: vec![UnifiedContentBlock::ToolUse {
                    id: "tool_1".to_string(),
                    name: "read_file".to_string(),
                    input: json!({ "path": "/tmp/demo.txt" }),
                }],
                reasoning_content: Some("reasoning trace".to_string()),
            }],
            ..Default::default()
        };

        let estimated = estimate_unified_request_tokens(&request);
        let baseline = estimate_tokens("system prompt");

        assert!(estimated > baseline);
    }

    #[test]
    fn known_json_estimate_includes_tool_calls() {
        let request = json!({
            "system": "system prompt",
            "messages": [
                {
                    "role": "assistant",
                    "content": [{"type": "text", "text": "hello"}],
                    "tool_calls": [
                        {
                            "id": "call_1",
                            "function": {
                                "name": "write_file",
                                "arguments": "{\"path\":\"/tmp/a.txt\",\"content\":\"demo\"}"
                            }
                        }
                    ]
                }
            ]
        });

        let estimated = estimate_known_request_json_tokens(&request);
        let baseline = estimate_tokens("system prompt") + estimate_tokens("hello");

        assert!(estimated > baseline);
    }
}
