use async_trait::async_trait;
use bytes::Bytes;
use serde::{Deserialize, Serialize}; // Added Deserialize
use serde_json::Value; // For tool parameters and choices

use crate::http::ccproxy::{
    errors::ProxyAuthError,
    types::{
        OpenAIChatCompletionChoice, OpenAIChatCompletionRequest, OpenAIChatCompletionResponse,
        OpenAIChatCompletionStreamResponse, OpenAIFunctionCall, OpenAIMessageContent,
        OpenAIMessageContentPart, OpenAIStreamChoice, OpenAITool, OpenAIUsage, SseEvent,
        UnifiedChatMessage, UnifiedToolCall,
    },
};

use super::protocol_adapter::{
    AdaptedRequest, ProtocolAdapter, RawBackendResponse, RawBackendStreamChunk,
};

pub struct ClaudeAdapter;

// --- Claude API Request Structures ---
#[derive(Serialize, Debug)]
struct ClaudeRequestPayload<'a> {
    model: &'a str, // The actual model name for Claude, e.g., "claude-3-opus-20240229"
    messages: Vec<ClaudeMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    max_tokens: i32, // Claude's max_tokens is required.
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_k: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ClaudeTool<'a>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<ClaudeToolChoice>,
}

#[derive(Serialize, Debug)]
struct ClaudeMessage {
    role: String, // "user" or "assistant"
    content: Vec<ClaudeContentBlockRequest>,
}

#[derive(Serialize, Debug)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
enum ClaudeContentBlockRequest {
    Text {
        text: String,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String, // Can also be Vec<ClaudeContentBlockRequest> for complex tool results, but string is common for JSON.
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
    Image {
        source: ClaudeImageSource,
    },
    // Added for assistant messages requesting a tool use.
    // Claude's 'tool_use' type is for when the assistant wants to use a tool.
    #[serde(rename = "tool_use")] // Ensure correct serialization for Claude
    ToolUse {
        id: String,   // The ID for this tool use request
        name: String, // The name of the tool to be used
        input: Value, // The input to the tool, as a JSON object
    },
}

#[derive(Serialize, Debug)]
struct ClaudeTool<'a> {
    name: &'a str,
    description: Option<&'a str>,
    input_schema: &'a Value, // JSON Schema
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "type")]
enum ClaudeToolChoice {
    Auto,
    Any, // Forces the model to use a tool
    Tool { name: String },
}

#[derive(Serialize, Debug)]
struct ClaudeImageSource {
    #[serde(rename = "type")]
    source_type: String, // e.g., "base64"
    media_type: String, // e.g., "image/jpeg", "image/png"
    data: String,       // base64 encoded image data
}

// --- End Claude API Request Structures ---

// --- Claude API Response Structures ---
#[derive(Deserialize, Debug)]
struct ClaudeResponsePayload {
    id: String,
    #[serde(rename = "type")]
    response_type: String, // e.g., "message" or "error"
    role: Option<String>, // "assistant", present on successful message
    content: Option<Vec<ClaudeContentBlockResponse>>,
    model: Option<String>,       // Model that generated the response
    stop_reason: Option<String>, // e.g., "end_turn", "max_tokens", "tool_use"
    // stop_sequence: Option<String>, // Not directly mapped for now
    usage: Option<ClaudeUsage>,
    // For error responses
    error: Option<ClaudeErrorResponse>,
}

#[derive(Deserialize, Debug)]
struct ClaudeContentBlockResponse {
    #[serde(rename = "type")]
    block_type: String, // e.g., "text", "tool_use"
    text: Option<String>,
    // For tool_use type
    id: Option<String>,               // Tool use ID
    name: Option<String>,             // Tool name
    input: Option<serde_json::Value>, // Tool input
}

#[derive(Deserialize, Debug)]
struct ClaudeUsage {
    input_tokens: u64,
    output_tokens: u64,
}

#[derive(Deserialize, Debug)]
struct ClaudeErrorResponse {
    #[serde(rename = "type")]
    error_type: String,
    message: String,
}
// --- End Claude API Response Structures ---

// --- Claude API Stream Event Structures ---
#[derive(Deserialize, Debug)]
struct ClaudeStreamEvent {
    #[serde(rename = "type")]
    event_type: String,
    message: Option<ClaudeStreamMessageStart>,
    index: Option<u32>, // For content_block_start, content_block_delta
    content_block: Option<ClaudeStreamContentBlock>, // For content_block_start
    delta: Option<ClaudeStreamDelta>, // For content_block_delta, message_delta
    usage: Option<ClaudeStreamMessageUsage>, // For message_delta
    error: Option<ClaudeErrorResponse>, // For error event
}

#[derive(Deserialize, Debug)]
struct ClaudeStreamMessageStart {
    id: String,
    #[serde(rename = "type")]
    _message_type: String, // "message"
    role: String, // "assistant"
    model: String,
    // stop_reason: Option<String>, // Not typically in message_start
    // stop_sequence: Option<String>, // Not typically in message_start
    usage: ClaudeStreamMessageUsage, // input_tokens here
}

#[derive(Deserialize, Debug)]
struct ClaudeStreamContentBlock {
    #[serde(rename = "type")]
    block_type: String, // "text" or "tool_use"
    text: Option<String>, // For text block_type
    // For tool_use block_type
    id: Option<String>,
    name: Option<String>,
    input: Option<serde_json::Value>,
}

#[derive(Deserialize, Debug)]
struct ClaudeStreamDelta {
    #[serde(rename = "type")]
    delta_type: Option<String>, // "text_delta", "input_json_delta", "tool_use_delta" (not standard, Claude uses "text_delta" for text, and tool details in content_block_start/delta)
    text: Option<String>, // For text_delta
    // For message_delta
    #[serde(skip_serializing_if = "Option::is_none")]
    partial_json: Option<String>, // For input_json_delta
    stop_reason: Option<String>,
    // stop_sequence: Option<String>,
    // No usage here, it's in a separate field in message_delta event
}

#[derive(Deserialize, Debug)]
struct ClaudeStreamMessageUsage {
    input_tokens: Option<u64>,  // In message_start
    output_tokens: Option<u64>, // In message_delta
}
// --- End Claude API Stream Event Structures ---

#[async_trait]
impl ProtocolAdapter for ClaudeAdapter {
    fn adapt_request(
        &self,
        base_url: &str,
        target_model_name: &str, // This is the actual Claude model ID
        target_api_key: &str,
        openai_request: &OpenAIChatCompletionRequest,
    ) -> Result<AdaptedRequest, ProxyAuthError> {
        log::debug!(
            "ClaudeAdapter: adapt_request for model alias '{}', target Claude model '{}'",
            openai_request.model, // This is the alias from client
            target_model_name     // This is the actual model for Claude
        );

        let mut claude_messages: Vec<ClaudeMessage> = Vec::new();
        let mut system_prompt: Option<String> = None;
        let mut has_tools = false; // Flag to add beta header

        for message in &openai_request.messages {
            let openai_role = message.role.as_deref().unwrap_or_default();
            let mut claude_content_blocks: Vec<ClaudeContentBlockRequest> = Vec::new();

            if let Some(content_parts_or_text) = &message.content {
                match content_parts_or_text {
                    OpenAIMessageContent::Text(text) => {
                        claude_content_blocks
                            .push(ClaudeContentBlockRequest::Text { text: text.clone() });
                    }
                    OpenAIMessageContent::Parts(parts_array) => {
                        for part in parts_array {
                            match part {
                                OpenAIMessageContentPart::Text { text } => {
                                    claude_content_blocks.push(ClaudeContentBlockRequest::Text {
                                        text: text.clone(),
                                    });
                                }
                                OpenAIMessageContentPart::ImageUrl { image_url } => {
                                    if image_url.url.starts_with("data:") {
                                        if let Some(comma_idx) = image_url.url.find(',') {
                                            let header = &image_url.url[5..comma_idx]; // "image/jpeg;base64" or "image/png;base64" etc.
                                            let base64_data = &image_url.url[comma_idx + 1..];
                                            let mime_type = header
                                                .split(';')
                                                .next()
                                                .unwrap_or("application/octet-stream")
                                                .to_string();

                                            claude_content_blocks.push(
                                                ClaudeContentBlockRequest::Image {
                                                    source: ClaudeImageSource {
                                                        source_type: "base64".to_string(),
                                                        media_type: mime_type,
                                                        data: base64_data.to_string(),
                                                    },
                                                },
                                            );
                                        } else {
                                            log::warn!(
                                                "ClaudeAdapter: Malformed data URI in ImageUrl: {}",
                                                image_url.url
                                            );
                                        }
                                    } else {
                                        log::warn!("ClaudeAdapter: Non-data URI ImageUrl not supported for Claude: {}", image_url.url);
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if !claude_content_blocks.is_empty() {
                match openai_role {
                    "system" => {
                        // System prompts in Claude are top-level, not part of messages with content blocks.
                        // We'll extract text from system messages and concatenate.
                        let system_text: String = claude_content_blocks
                            .iter()
                            .filter_map(|block| {
                                if let ClaudeContentBlockRequest::Text { text } = block {
                                    Some(text.as_str())
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<&str>>()
                            .join("\n");
                        system_prompt =
                            Some(system_prompt.unwrap_or_default() + &system_text + "\n");
                    }
                    "user" => {
                        claude_messages.push(ClaudeMessage {
                            role: "user".to_string(),
                            content: claude_content_blocks,
                        });
                    }
                    "assistant" => {
                        // If the assistant message from OpenAI history contains tool_calls,
                        // it means the model previously decided to use tools.
                        // These need to be mapped to Claude's 'tool_use' content blocks.
                        // An assistant message can have both text content and tool_use requests.
                        // claude_content_blocks already contains text/image if present from the logic above.
                        if let Some(tool_calls) = &message.tool_calls {
                            log::debug!(
                                "ClaudeAdapter: Processing {} tool_calls for assistant message.",
                                tool_calls.len()
                            );
                            for tool_call in tool_calls {
                                if let (Some(id), Some(name), Some(arguments_str)) = (
                                    &tool_call.id,
                                    &tool_call.function.name,
                                    &tool_call.function.arguments,
                                ) {
                                    // Claude's 'input' for tool_use must be a JSON object.
                                    // OpenAI's 'arguments' is a JSON string.
                                    match serde_json::from_str::<Value>(arguments_str) {
                                        Ok(parsed_args) if parsed_args.is_object() => {
                                            claude_content_blocks.push(
                                                ClaudeContentBlockRequest::ToolUse {
                                                    id: id.clone(),
                                                    name: name.clone(),
                                                    input: parsed_args,
                                                },
                                            );
                                        }
                                        Ok(_) => {
                                            log::error!("ClaudeAdapter: Tool call arguments for id '{}' are not a JSON object: {}", id, arguments_str);
                                            // Decide if to skip this tool_use or return an error for the whole request
                                        }
                                        Err(e) => {
                                            log::error!("ClaudeAdapter: Failed to parse tool call arguments for id '{}': {}. Args: {}", id, e, arguments_str);
                                            // Decide if to skip or error out
                                        }
                                    }
                                } else {
                                    log::warn!("ClaudeAdapter: Assistant tool_call in history is missing id, name, or arguments: {:?}", tool_call);
                                }
                            }
                        }
                        // Now, claude_content_blocks contains any text/image parts AND any tool_use parts.
                        claude_messages.push(ClaudeMessage {
                            role: "assistant".to_string(),
                            content: claude_content_blocks,
                        });
                    }
                    "tool" => {
                        if let Some(tool_call_id) = &message.tool_call_id {
                            // Assuming the content of a tool message is a single text block representing the tool's output string.
                            let tool_result_content_str = claude_content_blocks
                                .iter()
                                .filter_map(|block| {
                                    if let ClaudeContentBlockRequest::Text { text } = block {
                                        Some(text.as_str())
                                    } else {
                                        None
                                    }
                                })
                                .collect::<Vec<&str>>()
                                .join("\n");

                            claude_messages.push(ClaudeMessage {
                                role: "user".to_string(), // Tool results are provided by the user to Claude
                                content: vec![ClaudeContentBlockRequest::ToolResult {
                                    tool_use_id: tool_call_id.clone(),
                                    content: tool_result_content_str,
                                    is_error: None, // TODO: Potentially detect if the content indicates an error
                                }],
                            });
                        } else {
                            log::warn!("ClaudeAdapter: 'tool' role message without tool_call_id, skipping.");
                        }
                    }
                    _ => log::warn!("ClaudeAdapter: Unknown role '{}' encountered.", openai_role),
                }
            }
        }

        let claude_tools = openai_request.tools.as_ref().map(|oa_tools| {
            has_tools = true;
            oa_tools
                .iter()
                .filter(|tool| tool.r#type == "function") // Claude primarily supports function-like tools
                .map(|tool: &OpenAITool| ClaudeTool {
                    // Explicit type for tool
                    name: &tool.function.name,
                    description: tool.function.description.as_deref(),
                    input_schema: &tool.function.parameters,
                })
                .collect::<Vec<_>>()
        });

        let claude_tool_choice = openai_request.tool_choice.as_ref().map(|oa_choice| {
            has_tools = true; // Using tool_choice implies tools are relevant
            match oa_choice {
                Value::String(s) => match s.as_str() {
                    "none" => ClaudeToolChoice::Auto, // Claude doesn't have a direct "none". "auto" might be closest if no tools are provided. Or we omit `tool_choice`. For now, map to Auto.
                    "auto" => ClaudeToolChoice::Auto,
                    "required" => ClaudeToolChoice::Any, // "required" in OpenAI means model must use a tool.
                    _ => ClaudeToolChoice::Auto,         // Default
                },
                Value::Object(obj)
                    if obj.get("type").and_then(Value::as_str) == Some("function") =>
                {
                    obj.get("function")
                        .and_then(Value::as_object)
                        .and_then(|func_obj| func_obj.get("name").and_then(Value::as_str))
                        .map(|name| ClaudeToolChoice::Tool {
                            name: name.to_string(),
                        })
                        .unwrap_or(ClaudeToolChoice::Auto)
                }
                _ => ClaudeToolChoice::Auto,
            }
        });

        // Claude requires max_tokens. Use OpenAI's value or a default.
        let max_tokens = openai_request.max_tokens.unwrap_or(1024); // Default to 1024 if not provided

        let claude_payload = ClaudeRequestPayload {
            model: target_model_name,
            messages: claude_messages,
            system: system_prompt.map(|s| s.trim_end().to_string()),
            max_tokens,
            stream: openai_request.stream,
            temperature: openai_request.temperature,
            top_p: openai_request.top_p,
            top_k: openai_request.top_k,
            tools: claude_tools,
            tool_choice: claude_tool_choice,
        };

        let body_bytes = serde_json::to_vec(&claude_payload).map_err(|e| {
            log::error!(
                "ClaudeAdapter: Failed to serialize ClaudeRequestPayload: {}",
                e
            );
            ProxyAuthError::InternalError("Failed to serialize request for Claude".to_string())
        })?;

        let mut headers_to_add = vec![
            (
                reqwest::header::CONTENT_TYPE.as_str().to_string(),
                "application/json".to_string(),
            ),
            ("x-api-key".to_string(), target_api_key.to_string()),
            // Anthropic-Version header is crucial for Claude API
            ("anthropic-version".to_string(), "2023-06-01".to_string()),
        ];

        if has_tools || openai_request.tools.is_some() {
            // Add beta header if tools are defined or chosen
            // Note: The exact value "tools-2024-04-04" might change. Always refer to official Claude docs.
            headers_to_add.push(("anthropic-beta".to_string(), "tools-2024-04-04".to_string()));
        }

        Ok(AdaptedRequest {
            url: format!("{base_url}/messages"),
            body: Bytes::from(body_bytes),
            headers_to_add,
        })
    }

    fn adapt_response_body(
        &self,
        raw_response: RawBackendResponse,
        target_model_name: &str, // Used for OpenAI response's 'model' field
    ) -> Result<OpenAIChatCompletionResponse, ProxyAuthError> {
        log::debug!(
            "ClaudeAdapter: adapt_response_body. Raw status: {}",
            raw_response.status_code
        );

        let claude_response: ClaudeResponsePayload =
            serde_json::from_slice(&raw_response.body_bytes).map_err(|e| {
                log::error!(
                    "ClaudeAdapter: Failed to deserialize ClaudeResponsePayload: {}, body: {:?}",
                    e,
                    String::from_utf8_lossy(&raw_response.body_bytes)
                );
                ProxyAuthError::InternalError(format!(
                    "Failed to deserialize Claude response body: {}",
                    e
                ))
            })?;

        if let Some(error_details) = claude_response.error {
            log::error!(
                "Claude API returned an error: type={}, message={}",
                error_details.error_type,
                error_details.message
            );
            return Err(ProxyAuthError::InternalError(format!(
                "Claude API Error ({}): {}",
                error_details.error_type, error_details.message
            )));
        }

        let mut assistant_message_content: Option<String> = None;
        let mut assistant_tool_calls: Option<Vec<UnifiedToolCall>> = None;

        if let Some(content_blocks) = claude_response.content {
            for block in content_blocks {
                match block.block_type.as_str() {
                    "text" => {
                        if let Some(text) = block.text {
                            assistant_message_content =
                                Some(assistant_message_content.unwrap_or_default() + &text);
                        }
                    }
                    "tool_use" => {
                        if let (Some(id), Some(name), Some(input_val)) =
                            (block.id, block.name, block.input)
                        {
                            let tool_call = UnifiedToolCall {
                                id: Some(id),                         // Use Claude's tool_use_id as the tool_call.id
                                r#type: Some("function".to_string()), // Assuming all Claude tools are functions
                                function: OpenAIFunctionCall {
                                    name: Some(name),
                                    arguments: Some(input_val.to_string()), // Convert JSON value to string
                                },
                                index: None, // Not applicable for non-streaming response tool_calls array
                            };
                            assistant_tool_calls
                                .get_or_insert_with(Vec::new)
                                .push(tool_call);
                        } else {
                            log::warn!(
                                "ClaudeAdapter: Received 'tool_use' block with missing id, name, or input."
                            );
                        }
                    }
                    _ => log::warn!(
                        "ClaudeAdapter: Unknown content block type '{}'",
                        block.block_type
                    ),
                }
            }
        }

        let finish_reason_str = match claude_response.stop_reason.as_deref() {
            Some("end_turn") => Some("stop".to_string()),
            Some("max_tokens") => Some("length".to_string()),
            Some("tool_use") => Some("tool_calls".to_string()),
            Some(other_reason) => {
                log::info!(
                    "ClaudeAdapter: Received other stop_reason '{}', mapping to 'stop'.",
                    other_reason
                );
                Some("stop".to_string())
            }
            None => None,
        };

        let choice = OpenAIChatCompletionChoice {
            index: 0,
            message: UnifiedChatMessage {
                role: claude_response.role, // Should be "assistant"
                content: assistant_message_content.map(OpenAIMessageContent::Text),
                tool_calls: assistant_tool_calls,
                tool_call_id: None,
            },
            finish_reason: finish_reason_str,
        };

        let usage = claude_response.usage.map(|u| OpenAIUsage {
            prompt_tokens: u.input_tokens,
            completion_tokens: u.output_tokens,
            total_tokens: u.input_tokens + u.output_tokens,
        });

        Ok(OpenAIChatCompletionResponse {
            id: format!("claude-{}", claude_response.id), // Use Claude's message ID
            object: "chat.completion".to_string(),
            created: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            model: target_model_name.to_string(), // Use the target_model_name passed in
            choices: vec![choice],
            usage,
        })
    }

    fn adapt_stream_chunk(
        &self,
        raw_chunk: RawBackendStreamChunk,
        stream_id: &str,                       // For OpenAI chunk 'id' field
        target_model_name: &str,               // For OpenAI chunk 'model' field
        next_tool_call_stream_index: &mut u32, // For OpenAI tool call 'index'
    ) -> Result<Option<SseEvent>, ProxyAuthError> {
        let chunk_str = String::from_utf8_lossy(&raw_chunk.data);
        log::trace!("ClaudeAdapter: adapt_stream_chunk received: {}", chunk_str);

        // Claude stream sends events separated by double newlines.
        // Each event block usually starts with "event: <type>" followed by "data: <json>".
        for event_block_str in chunk_str.split("\n\n") {
            if event_block_str.trim().is_empty() {
                continue;
            }

            let mut event_type_from_line: Option<String> = None;
            let mut data_json_str: Option<String> = None;

            for line in event_block_str.lines() {
                if line.starts_with("event: ") {
                    event_type_from_line =
                        Some(line.trim_start_matches("event: ").trim().to_string());
                } else if line.starts_with("data: ") {
                    data_json_str = Some(line.trim_start_matches("data: ").trim().to_string());
                }
            }

            if event_type_from_line.is_none() || data_json_str.is_none() {
                log::warn!(
                    "ClaudeAdapter: Skipping malformed event block: {}",
                    event_block_str
                );
                continue;
            }

            let event_type = event_type_from_line.unwrap();
            let data_str = data_json_str.unwrap();

            log::trace!(
                "ClaudeAdapter: Processing event type '{}', data: {}",
                event_type,
                data_str
            );

            match event_type.as_str() {
                "message_start" => {
                    // The message_start event contains the overall message ID and input token usage.
                    // We don't typically send a separate SSE event for this in OpenAI format,
                    // but we use the ID for subsequent chunks.
                    // The model name from here can also be used if `target_model_name` isn't precise enough.
                    if let Ok(start_event_data) =
                        serde_json::from_str::<ClaudeStreamEvent>(&data_str)
                    {
                        if let Some(msg) = start_event_data.message {
                            log::debug!(
                                "ClaudeAdapter: Stream started. ID: {}, Model: {}",
                                msg.id,
                                msg.model
                            );
                            // The stream_id passed in is already unique for the OpenAI stream.
                            // msg.id is Claude's internal message ID.
                        }
                    }
                }
                "content_block_start" => {
                    // If it's a tool_use block, we might need to prepare a tool_call structure.
                    if let Ok(event_data) = serde_json::from_str::<ClaudeStreamEvent>(&data_str) {
                        if let Some(block) = event_data.content_block {
                            if block.block_type == "tool_use" {
                                // A tool use block is starting
                                if let (Some(tool_id), Some(tool_name), Some(tool_input_value)) =
                                    (block.id, block.name, block.input)
                                // `input` might be an empty object {} if args are streamed via input_json_delta
                                {
                                    // Send the initial part of the tool call, arguments might be empty or partial here.
                                    // If tool_input_value is not empty, it means Claude sent some/all arguments upfront.
                                    let tool_call = UnifiedToolCall {
                                        index: Some(*next_tool_call_stream_index),
                                        id: Some(tool_id),
                                        r#type: Some("function".to_string()),
                                        function: OpenAIFunctionCall {
                                            name: Some(tool_name),
                                            // Convert the JSON Value from Claude's input to a string for OpenAI
                                            // If input is an empty object, arguments will be "".
                                            // If input has content, it will be serialized.
                                            // If arguments are streamed, this initial part might be empty or just "{"
                                            arguments: Some(
                                                if tool_input_value.is_object()
                                                    && tool_input_value
                                                        .as_object()
                                                        .map_or(false, |m| m.is_empty())
                                                {
                                                    "".to_string()
                                                } else {
                                                    tool_input_value.to_string()
                                                },
                                            ),
                                        },
                                    };
                                    let choice = OpenAIStreamChoice {
                                        index: 0,
                                        delta: UnifiedChatMessage {
                                            role: Some("assistant".to_string()),
                                            content: None,
                                            tool_calls: Some(vec![tool_call]),
                                            tool_call_id: None,
                                        },
                                        finish_reason: None,
                                    };
                                    let sse_resp = OpenAIChatCompletionStreamResponse {
                                        id: stream_id.to_string(),
                                        object: "chat.completion.chunk".to_string(),
                                        created: std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_secs(),
                                        model: target_model_name.to_string(),
                                        choices: vec![choice],
                                        usage: None,
                                    };
                                    if let Ok(json_payload) = serde_json::to_string(&sse_resp) {
                                        return Ok(Some(SseEvent {
                                            data: Some(json_payload),
                                            ..Default::default()
                                        }));
                                    }
                                    // Increment the index for the next potential tool call,
                                    // as this `content_block_start` initiates a new tool_call sequence.
                                    *next_tool_call_stream_index += 1;
                                }
                            }
                        }
                    }
                }
                "content_block_delta" => {
                    if let Ok(event_data) = serde_json::from_str::<ClaudeStreamEvent>(&data_str) {
                        if let Some(delta) = event_data.delta {
                            if delta.delta_type.as_deref() == Some("input_json_delta") {
                                // Use the new `partial_json` field
                                if let Some(partial_json_args) = delta.partial_json {
                                    // This is a delta for tool call arguments.
                                    // The `next_tool_call_stream_index` would have been incremented by `content_block_start`.
                                    // We need to send this for the *current* tool call, so use `*next_tool_call_stream_index - 1`.
                                    // Ensure index is not 0 if it was never incremented (edge case).
                                    let current_tool_call_idx = if *next_tool_call_stream_index > 0
                                    {
                                        *next_tool_call_stream_index - 1
                                    } else {
                                        0 // Should not happen if content_block_start for tool_use was processed
                                    };
                                    let tool_call_delta = UnifiedToolCall {
                                        index: Some(current_tool_call_idx),
                                        id: None, // ID and name were sent in the initial part
                                        r#type: None,
                                        function: OpenAIFunctionCall {
                                            name: None,
                                            arguments: Some(partial_json_args),
                                        },
                                    };
                                    let choice = OpenAIStreamChoice {
                                        index: 0,
                                        delta: UnifiedChatMessage {
                                            role: Some("assistant".to_string()), // Role for tool call deltas
                                            content: None,
                                            tool_calls: Some(vec![tool_call_delta]),
                                            tool_call_id: None,
                                        },
                                        finish_reason: None,
                                    };
                                    let sse_resp = OpenAIChatCompletionStreamResponse {
                                        id: stream_id.to_string(), // Use the overall stream ID
                                        object: "chat.completion.chunk".to_string(),
                                        created: std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_secs(),
                                        model: target_model_name.to_string(),
                                        choices: vec![choice],
                                        usage: None,
                                    };
                                    if let Ok(json_payload) = serde_json::to_string(&sse_resp) {
                                        return Ok(Some(SseEvent {
                                            data: Some(json_payload),
                                            ..Default::default()
                                        }));
                                    }
                                }
                            } else if let Some(text_delta) = delta.text {
                                // Standard text_delta
                                let choice = OpenAIStreamChoice {
                                    index: 0,
                                    delta: UnifiedChatMessage {
                                        role: Some("assistant".to_string()),
                                        content: Some(OpenAIMessageContent::Text(text_delta)),
                                        tool_calls: None,
                                        tool_call_id: None,
                                    },
                                    finish_reason: None,
                                };
                                let sse_resp = OpenAIChatCompletionStreamResponse {
                                    id: stream_id.to_string(),
                                    object: "chat.completion.chunk".to_string(),
                                    created: std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_secs(),
                                    model: target_model_name.to_string(),
                                    choices: vec![choice],
                                    usage: None,
                                };
                                if let Ok(json_payload) = serde_json::to_string(&sse_resp) {
                                    return Ok(Some(SseEvent {
                                        data: Some(json_payload),
                                        ..Default::default()
                                    }));
                                }
                            }
                        }
                    }
                }
                "content_block_stop" => {
                    if let Ok(event_data) = serde_json::from_str::<ClaudeStreamEvent>(&data_str) {
                        if let Some(index) = event_data.index {
                            // Check if this index corresponds to a tool_call that was started.
                            log::debug!("ClaudeAdapter: content_block_stop for index {}", index);
                            // `next_tool_call_stream_index` is now incremented when a `tool_use` `content_block_start`
                            // is processed, as Claude typically sends the full tool input at once.
                            // This event mainly signals the end of that specific block (text or tool_use).
                            // No specific OpenAI SSE event is usually generated for content_block_stop itself.
                        }
                    }
                }
                "message_delta" => {
                    if let Ok(event_data) = serde_json::from_str::<ClaudeStreamEvent>(&data_str) {
                        let mut finish_reason_str: Option<String> = None;
                        let mut usage_data: Option<OpenAIUsage> = None;

                        if let Some(delta) = event_data.delta {
                            finish_reason_str = match delta.stop_reason.as_deref() {
                                Some("end_turn") => Some("stop".to_string()),
                                Some("max_tokens") => Some("length".to_string()),
                                Some("tool_use") => Some("tool_calls".to_string()), // This indicates the model wants to use tools.
                                Some(other) => {
                                    log::info!(
                                        "ClaudeAdapter: Stream message_delta stop_reason: {}",
                                        other
                                    );
                                    Some("stop".to_string())
                                }
                                None => None,
                            };
                        }
                        if let Some(usage) = event_data.usage {
                            if let Some(output_tokens) = usage.output_tokens {
                                // Input tokens are usually in message_start.
                                // OpenAI stream usage typically only includes completion_tokens and total_tokens if available.
                                usage_data = Some(OpenAIUsage {
                                    prompt_tokens: 0, // Or fetch from a stored message_start event
                                    completion_tokens: output_tokens,
                                    total_tokens: output_tokens, // Simplification
                                });
                            }
                        }

                        if finish_reason_str.is_some() || usage_data.is_some() {
                            let choice = OpenAIStreamChoice {
                                index: 0,
                                delta: UnifiedChatMessage {
                                    role: None,
                                    content: None,
                                    tool_calls: None,
                                    tool_call_id: None,
                                }, // Delta is empty here
                                finish_reason: finish_reason_str,
                            };
                            let sse_resp = OpenAIChatCompletionStreamResponse {
                                id: stream_id.to_string(),
                                object: "chat.completion.chunk".to_string(),
                                created: std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs(),
                                model: target_model_name.to_string(),
                                choices: vec![choice],
                                usage: usage_data,
                            };
                            if let Ok(json_payload) = serde_json::to_string(&sse_resp) {
                                return Ok(Some(SseEvent {
                                    data: Some(json_payload),
                                    ..Default::default()
                                }));
                            }
                        }
                    }
                }
                "message_stop" => {
                    // This is the final event. OpenAI typically sends [DONE] via adapt_stream_end.
                    log::debug!("ClaudeAdapter: Stream ended (message_stop event).");
                    // No specific SSE event for this, [DONE] is handled by adapt_stream_end.
                }
                "ping" => {
                    // No OpenAI equivalent, just acknowledge.
                    log::trace!("ClaudeAdapter: Ping event received.");
                }
                "error" => {
                    if let Ok(error_event_data) =
                        serde_json::from_str::<ClaudeStreamEvent>(&data_str)
                    {
                        if let Some(err_details) = error_event_data.error {
                            log::error!(
                                "ClaudeAdapter: Error event in stream: type={}, message={}",
                                err_details.error_type,
                                err_details.message
                            );
                            //  Optionally, could try to format this as an OpenAI error in the stream,
                            //  but for now, we'll just log and the stream might be cut by the handler.
                            //  Returning an error from adapt_stream_chunk would stop processing.
                            return Err(ProxyAuthError::InternalError(format!(
                                "Claude Stream Error ({}): {}",
                                err_details.error_type, err_details.message
                            )));
                        }
                    }
                }
                _ => {
                    log::warn!(
                        "ClaudeAdapter: Unknown stream event type received: {}",
                        event_type
                    );
                }
            }
        }

        // If the loop completes, it means no event in this chunk was adapted.
        Ok(None)
    }

    fn adapt_stream_end(&self) -> Option<String> {
        // Claude stream ends with a `message_stop` event.
        // The standard OpenAI stream end is `data: [DONE]\n\n`.
        Some("[DONE]".to_string())
    }
}
