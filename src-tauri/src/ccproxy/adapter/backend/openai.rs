use crate::ccproxy::adapter::backend::update_message_block;
use crate::ccproxy::adapter::unified::{
    SseStatus, UnifiedContentBlock, UnifiedMessage, UnifiedRequest, UnifiedResponse, UnifiedRole,
};
use async_trait::async_trait;
use quick_xml::de::from_str;
use reqwest::{Client, RequestBuilder};
use serde::Deserialize;
use serde_json::json;
use std::sync::{Arc, RwLock};

use super::{BackendAdapter, BackendResponse};
use crate::ccproxy::adapter::{
    range_adapter::{adapt_temperature, Protocol},
    unified::{UnifiedStreamChunk, UnifiedToolChoice, UnifiedUsage},
};
use crate::ccproxy::openai::UnifiedToolCall;
use crate::ccproxy::types::openai::{
    OpenAIChatCompletionRequest, OpenAIChatCompletionResponse, OpenAIChatCompletionStreamResponse,
    OpenAIFunctionCall, OpenAIFunctionDefinition, OpenAIImageUrl, OpenAIMessageContent,
    OpenAIMessageContentPart, OpenAITool, OpenAIToolChoice, OpenAIToolChoiceFunction,
    OpenAIToolChoiceObject, UnifiedChatMessage,
};

#[derive(Deserialize, Debug)]
struct ToolUse {
    #[serde(rename = "name")]
    name: String,
    #[serde(rename = "params")]
    params: Params,
}

#[derive(Deserialize, Debug)]
struct Params {
    #[serde(rename = "param", default)]
    param: Vec<Param>,
}

#[derive(Deserialize, Debug)]
struct Param {
    #[serde(rename = "@name")]
    name: String,

    // parse the type attribute: <param name="location" type="string" value="北京" />
    #[serde(rename = "@value")]
    value: Option<String>,

    // parse the text content: <param name="location">北京</param>
    #[serde(rename = "$text", default)]
    text_content: Option<String>,
}

impl Param {
    fn get_value(&self) -> String {
        // use the @value attribate first
        self.value
            .as_ref()
            .map(|s| s.trim())
            .or_else(|| self.text_content.as_deref().map(|s| s.trim()))
            .unwrap_or_default()
            .to_string()
    }
}
const TOOL_TAG_START: &str = "<ccp:tool_use>";
const TOOL_TAG_END: &str = "</ccp:tool_use>";

pub struct OpenAIBackendAdapter;

fn generate_tools_xml(tools: &Vec<crate::ccproxy::adapter::unified::UnifiedTool>) -> String {
    let mut tools_xml = String::new();
    tools_xml.push_str("<cpp:tools description=\"You available tools\">\n");

    for tool in tools {
        tools_xml.push_str(&format!(
            "<cpp:tool_use>\n<n>{}</n>\n<description>{}</description>\n",
            tool.name,
            tool.description.as_deref().unwrap_or("")
        ));

        if let Some(schema) = tool.input_schema.as_object() {
            if let Some(properties) = schema.get("properties").and_then(|p| p.as_object()) {
                tools_xml.push_str("<params>\n");
                for (name, details) in properties {
                    let param_type = details
                        .get("type")
                        .and_then(|t| t.as_str())
                        .unwrap_or("any");
                    let description = details
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or("");
                    tools_xml.push_str(&format!(
                        "<param name=\"{}\" type=\"{}\">{}</param>\n",
                        name, param_type, description
                    ));
                }
                tools_xml.push_str("</params>\n");
            }
        }
        tools_xml.push_str("</cpp:tool_use>\n");
    }
    tools_xml.push_str("</cpp:tools>");
    tools_xml
}

fn generate_tool_prompt(tools: &Vec<crate::ccproxy::adapter::unified::UnifiedTool>) -> String {
    let tools_xml = generate_tools_xml(tools);

    let template = r#"You have access to the following tools:

{TOOLS_LIST}

Each tool in the above list is defined within <cpp:tool_use>...</cpp:tool_use> tags, containing:
- <name>tool_name</name>: The name of the tool
- <description>tool_description</description>: What the tool does
- <params>: The parameters the tool accepts

To use a tool, respond with an XML block like this:
<ccp:tool_use>
    <name>TOOL_NAME</name>
    <params>
        <param name="PARAM_NAME" type="PARAM_TYPE">PARAM_VALUE</param>
    </params>
</ccp:tool_use>

IMPORTANT: When you need to perform the same operation multiple times with different parameters (e.g., checking weather for multiple cities, or querying different dates), you MUST make separate tool calls for each one. Use multiple <ccp:tool_use> blocks in your response.

Example: If asked to read two files, make two separate tool calls:
<ccp:tool_use>
    <name>Read</name>
    <params>
        <param name="file_path" type="string">/path/to/a.txt</param>
    </params>
</ccp:tool_use>
<ccp:tool_use>
    <name>Read</name>
    <params>
        <param name="file_path" type="string">/path/to/b.txt</param>
    </params>
</ccp:tool_use>

"#;

    template.replace("{TOOLS_LIST}", &tools_xml)
}

#[async_trait]
impl BackendAdapter for OpenAIBackendAdapter {
    async fn adapt_request(
        &self,
        client: &Client,
        unified_request: &UnifiedRequest,
        api_key: &str,
        base_url: &str,
        model: &str,
    ) -> Result<RequestBuilder, anyhow::Error> {
        let mut openai_messages: Vec<UnifiedChatMessage> = Vec::new();
        let mut unified_request = unified_request.clone();

        if unified_request.tool_compat_mode {
            if let Some(tools) = &unified_request.tools {
                let tool_prompt = generate_tool_prompt(tools);
                // Prepend to the last user message
                if let Some(last_message) = unified_request.messages.iter_mut().last() {
                    if last_message.role == UnifiedRole::User {
                        if let Some(UnifiedContentBlock::Text { text }) =
                            last_message.content.get_mut(0)
                        {
                            *text = format!("{}\n\n{}", tool_prompt, text);
                        } else {
                            last_message
                                .content
                                .insert(0, UnifiedContentBlock::Text { text: tool_prompt });
                        }
                    }
                } else {
                    // if last message is not from user, add a new user message
                    unified_request.messages.push(UnifiedMessage {
                        role: UnifiedRole::User,
                        content: vec![UnifiedContentBlock::Text { text: tool_prompt }],
                        reasoning_content: None,
                    });
                }
                // Remove tools from the request to avoid sending them to the backend
                unified_request.tools = None;
            }
        }

        let mut last_role: Option<UnifiedRole> = None;

        for msg in &unified_request.messages {
            let current_role = msg.role.clone();

            // Convert current message's content blocks to OpenAI content parts.
            let mut current_content_parts: Vec<OpenAIMessageContentPart> = Vec::new();
            let mut current_tool_calls = Vec::new();
            let mut current_tool_call_id = None;

            for block in &msg.content {
                match block {
                    UnifiedContentBlock::Text { text } => {
                        current_content_parts
                            .push(OpenAIMessageContentPart::Text { text: text.clone() });
                    }
                    UnifiedContentBlock::Image { media_type, data } => {
                        current_content_parts.push(OpenAIMessageContentPart::ImageUrl {
                            image_url: OpenAIImageUrl {
                                url: format!("data:{};base64,{}", media_type, data),
                                detail: None,
                            },
                        });
                    }
                    UnifiedContentBlock::ToolUse { id, name, input } => {
                        current_tool_calls.push(UnifiedToolCall {
                            id: Some(id.clone()),
                            r#type: Some("function".to_string()),
                            function: OpenAIFunctionCall {
                                name: Some(name.clone()),
                                arguments: Some(input.to_string()),
                            },
                            index: None,
                        });
                    }
                    UnifiedContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error: _,
                    } => {
                        current_tool_call_id = Some(tool_use_id.clone());
                        current_content_parts.push(OpenAIMessageContentPart::Text {
                            text: content.clone(),
                        });
                    }
                    UnifiedContentBlock::Thinking { .. } => { /* ignore */ }
                }
            }

            // Decide whether to merge with the previous message or create a new one.
            if current_role == UnifiedRole::User && last_role == Some(UnifiedRole::User) {
                if let Some(last_message) = openai_messages.last_mut() {
                    if let Some(existing_content) = &mut last_message.content {
                        match existing_content {
                            OpenAIMessageContent::Text(text) => {
                                let mut new_parts =
                                    vec![OpenAIMessageContentPart::Text { text: text.clone() }];
                                new_parts.extend(current_content_parts);
                                *existing_content = OpenAIMessageContent::Parts(new_parts);
                            }
                            OpenAIMessageContent::Parts(parts) => {
                                parts.extend(current_content_parts);
                            }
                        }
                    } else if !current_content_parts.is_empty() {
                        last_message.content =
                            Some(OpenAIMessageContent::Parts(current_content_parts));
                    }
                }
            } else {
                // Not merging, create a new message.
                let role_str = match msg.role {
                    UnifiedRole::System => "system",
                    UnifiedRole::User => "user",
                    UnifiedRole::Assistant => "assistant",
                    UnifiedRole::Tool => "tool",
                };

                let openai_content = if current_content_parts.is_empty() {
                    None
                } else if current_content_parts.len() == 1
                    && matches!(
                        &current_content_parts[0],
                        OpenAIMessageContentPart::Text { .. }
                    )
                {
                    if let OpenAIMessageContentPart::Text { text } = current_content_parts.remove(0)
                    {
                        Some(OpenAIMessageContent::Text(text))
                    } else {
                        unreachable!();
                    }
                } else {
                    Some(OpenAIMessageContent::Parts(current_content_parts))
                };

                openai_messages.push(UnifiedChatMessage {
                    role: Some(role_str.to_string()),
                    content: openai_content,
                    tool_calls: if current_tool_calls.is_empty() {
                        None
                    } else {
                        Some(current_tool_calls)
                    },
                    tool_call_id: if role_str == "tool" {
                        current_tool_call_id
                    } else {
                        None
                    },
                    reasoning_content: None,
                });
            }

            last_role = Some(current_role);
        }

        let openai_tools = unified_request.tools.as_ref().map(|tools| {
            tools
                .iter()
                .map(|tool| OpenAITool {
                    r#type: "function".to_string(),
                    function: OpenAIFunctionDefinition {
                        name: tool.name.clone(),
                        description: tool.description.clone(),
                        parameters: tool.input_schema.clone(),
                    },
                })
                .collect()
        });

        let openai_tool_choice = unified_request
            .tool_choice
            .as_ref()
            .map(|choice| match choice {
                UnifiedToolChoice::None => OpenAIToolChoice::String("none".to_string()),
                UnifiedToolChoice::Auto => OpenAIToolChoice::String("auto".to_string()),
                UnifiedToolChoice::Required => OpenAIToolChoice::String("required".to_string()),
                UnifiedToolChoice::Tool { name } => {
                    OpenAIToolChoice::Object(OpenAIToolChoiceObject {
                        choice_type: "function".to_string(),
                        function: OpenAIToolChoiceFunction { name: name.clone() },
                    })
                }
            });

        let openai_request = OpenAIChatCompletionRequest {
            model: model.to_string(),
            messages: openai_messages,
            stream: Some(unified_request.stream),
            max_tokens: unified_request.max_tokens,
            temperature: unified_request.temperature.map(|t| {
                // Adapt temperature from source protocol to OpenAI range
                adapt_temperature(t, Protocol::Claude, Protocol::OpenAI)
            }),
            top_p: unified_request.top_p,
            presence_penalty: unified_request.presence_penalty,
            frequency_penalty: unified_request.frequency_penalty,
            response_format: unified_request
                .response_format
                .as_ref()
                .and_then(|rf| serde_json::from_value(rf.clone()).ok()),
            stop: unified_request.stop_sequences.clone(),
            seed: unified_request.seed,
            user: unified_request.user.clone(),
            tools: openai_tools,
            tool_choice: openai_tool_choice,
            logprobs: unified_request.logprobs,
            top_logprobs: unified_request.top_logprobs,
            stream_options: None,
            logit_bias: None, // Not supported in unified request yet
        };

        let mut request_builder = client.post(format!("{}/chat/completions", base_url));
        request_builder = request_builder.header("Content-Type", "application/json");
        if !api_key.is_empty() {
            request_builder =
                request_builder.header("Authorization", format!("Bearer {}", api_key));
        }
        request_builder = request_builder.json(&openai_request);
        log::debug!(
            "openai request: {}",
            serde_json::to_string_pretty(&openai_request).unwrap_or_default()
        );

        Ok(request_builder)
    }

    async fn adapt_response(
        &self,
        backend_response: BackendResponse,
    ) -> Result<UnifiedResponse, anyhow::Error> {
        log::debug!(
            "openai response: {}",
            String::from_utf8_lossy(&backend_response.body)
        );

        let openai_response: OpenAIChatCompletionResponse =
            serde_json::from_slice(&backend_response.body)?;

        let first_choice = openai_response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No choices in OpenAI response"))?;

        let mut content_blocks = Vec::new();

        // Handle tool compatibility mode parsing
        if backend_response.tool_compat_mode {
            if let Some(content) = first_choice.message.content {
                let text = match content {
                    OpenAIMessageContent::Text(text) => text,
                    OpenAIMessageContent::Parts(parts) => parts
                        .into_iter()
                        .filter_map(|part| match part {
                            OpenAIMessageContentPart::Text { text } => Some(text),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join(""),
                };

                let mut processed_text = text.clone();
                let open_tags = processed_text.matches(TOOL_TAG_START).count();
                let close_tags = processed_text.matches(TOOL_TAG_END).count();
                let end_tag_len = TOOL_TAG_END.len();
                if open_tags > 0 && open_tags != close_tags {
                    log::warn!(
                        "Mismatched tool tags in response: {} open, {} close",
                        open_tags,
                        close_tags
                    );
                }

                while let Some(start_pos) = processed_text.find(TOOL_TAG_START) {
                    // Add text before the tool call as a separate text block
                    if start_pos > 0 {
                        content_blocks.push(UnifiedContentBlock::Text {
                            text: processed_text[..start_pos].to_string(),
                        });
                    }

                    // Search for the end tag *after* the start tag to handle multiple tools correctly
                    // and prevent panics from invalid slicing.
                    if let Some(relative_end_pos) = processed_text[start_pos..].find(TOOL_TAG_END) {
                        let end_pos = start_pos + relative_end_pos;
                        let tool_xml = &processed_text[start_pos..end_pos + end_tag_len];
                        if let Ok(parsed_tool) = from_str::<ToolUse>(tool_xml) {
                            let mut arguments = serde_json::Map::new();
                            for param in parsed_tool.params.param {
                                arguments.insert(
                                    param.name.clone(),
                                    serde_json::Value::String(param.get_value()),
                                );
                            }

                            log::debug!(
                                "tool_use parse result: name: {}, param: {:?}",
                                &parsed_tool.name,
                                &arguments
                            );

                            content_blocks.push(UnifiedContentBlock::ToolUse {
                                id: format!("tool_{}", uuid::Uuid::new_v4()),
                                name: parsed_tool.name,
                                input: serde_json::Value::Object(arguments),
                            });
                        } else {
                            let tool_xml = &processed_text[start_pos..end_pos + end_tag_len];

                            log::warn!("parse tool xml failed, xml: {}", tool_xml);
                            // If parsing fails, treat the text as a regular text block
                            content_blocks.push(UnifiedContentBlock::Text {
                                text: tool_xml.to_string(),
                            });
                        }
                        // Remove the parsed tool XML from the text
                        processed_text = processed_text[(end_pos + end_tag_len)..].to_string();
                    } else {
                        // Incomplete tool call, treat remaining as text
                        if !processed_text.is_empty() {
                            content_blocks.push(UnifiedContentBlock::Text {
                                text: processed_text.clone(),
                            });
                        }
                        break;
                    }
                }
                // Add any remaining text after the last tool call
                if !processed_text.is_empty() {
                    content_blocks.push(UnifiedContentBlock::Text {
                        text: processed_text,
                    });
                }
            }
        } else {
            // Original parsing logic for non-tool compatibility mode
            if let Some(content) = first_choice.message.content {
                match content {
                    OpenAIMessageContent::Text(text) => {
                        content_blocks.push(UnifiedContentBlock::Text { text })
                    }
                    OpenAIMessageContent::Parts(parts) => {
                        for part in parts {
                            match part {
                                OpenAIMessageContentPart::Text { text } => {
                                    content_blocks.push(UnifiedContentBlock::Text { text })
                                }
                                OpenAIMessageContentPart::ImageUrl { image_url: _ } => {
                                    anyhow::bail!("Image URL in assistant response not supported for UnifiedResponse");
                                }
                            }
                        }
                    }
                }
            }
        }

        if let Some(reasoning_content) = first_choice.message.reasoning_content {
            if !reasoning_content.is_empty() {
                content_blocks.push(UnifiedContentBlock::Thinking {
                    thinking: reasoning_content,
                });
            }
        }

        // Handle native tool calls (non-tool compatibility mode)
        if !backend_response.tool_compat_mode {
            if let Some(tool_calls) = first_choice.message.tool_calls {
                for tc in tool_calls {
                    content_blocks.push(UnifiedContentBlock::ToolUse {
                        id: tc.id.clone().unwrap_or_default(),
                        name: tc.function.name.unwrap_or_default(),
                        input: serde_json::from_str(&tc.function.arguments.unwrap_or_default())?,
                    });
                }
            }
        }

        let usage = openai_response
            .usage
            .map(|u| UnifiedUsage {
                input_tokens: u.prompt_tokens,
                output_tokens: u.completion_tokens,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
                tool_use_prompt_tokens: None,
                thoughts_tokens: None,
                cached_content_tokens: None,
            })
            .unwrap_or_default();

        Ok(UnifiedResponse {
            id: openai_response.id,
            model: openai_response.model,
            content: content_blocks,
            stop_reason: first_choice.finish_reason,
            usage,
        })
    }

    /// Adapt a openai stream chunk into a unified stream chunk.
    /// @link https://platform.openai.com/docs/api-reference/chat-streaming
    async fn adapt_stream_chunk(
        &self,
        chunk: bytes::Bytes,
        sse_status: Arc<RwLock<SseStatus>>,
    ) -> Result<Vec<UnifiedStreamChunk>, anyhow::Error> {
        let chunk_str = String::from_utf8_lossy(&chunk);
        let mut unified_chunks = Vec::new();

        for event_block_str in chunk_str.split("\n\n") {
            if event_block_str.trim().is_empty() {
                continue;
            }

            let mut data_str: Option<String> = None;
            for line in event_block_str.lines() {
                if line.starts_with("data:") {
                    data_str = Some(line["data:".len()..].trim().to_string());
                    break;
                }
            }

            if let Some(data) = data_str {
                if data == "[DONE]" {
                    // Handled by MessageStop or stream end signal
                    continue;
                }

                let openai_chunk: OpenAIChatCompletionStreamResponse = serde_json::from_str(&data)?;
                if let Ok(mut status) = sse_status.write() {
                    if !status.message_start {
                        status.message_start = true;
                        if !openai_chunk.id.is_empty() {
                            status.message_id = openai_chunk.id.clone();
                        }
                        unified_chunks.push(UnifiedStreamChunk::MessageStart {
                            id: status.message_id.clone(),
                            model: status.model_id.clone(),
                            usage: UnifiedUsage {
                                input_tokens: 0, // OpenAI stream doesn't provide input tokens in the first chunk
                                output_tokens: 0,
                                cache_creation_input_tokens: None,
                                cache_read_input_tokens: None,
                                tool_use_prompt_tokens: None,
                                thoughts_tokens: None,
                                cached_content_tokens: None,
                            },
                        });
                    }
                }

                for choice in openai_chunk.choices {
                    let delta = choice.delta;

                    if let Some(content) = delta.reasoning_content {
                        if !content.is_empty() {
                            if let Ok(mut status) = sse_status.write() {
                                // Send the thinking start flag
                                if status.thinking_delta_count == 0 {
                                    unified_chunks.push(UnifiedStreamChunk::ContentBlockStart {
                                        index: 0,
                                        block: json!({
                                            "type":"thinking",
                                            "thinking":"",
                                        }),
                                    })
                                }
                                status.thinking_delta_count += 1;
                                update_message_block(status, "thinking".to_string());
                            }
                            unified_chunks.push(UnifiedStreamChunk::Thinking { delta: content });
                        }
                    }

                    if let Some(content) = delta.content {
                        let has_text = match &content {
                            OpenAIMessageContent::Text(text) => !text.is_empty(),
                            OpenAIMessageContent::Parts(parts) => {
                                parts.iter().any(|part| {
                                    matches!(part, OpenAIMessageContentPart::Text { text } if !text.is_empty())
                                })
                            }
                        };

                        if let Ok(mut status) = sse_status.write() {
                            // Handle tool compatibility mode for streaming
                            if status.tool_compat_mode {
                                // Check buffer size before adding new content
                                if status.tool_compat_fragment_buffer.len() > 1024 * 1024 {
                                    log::warn!("Fragment buffer size limit exceeded");
                                    let fragment = status.tool_compat_fragment_buffer.clone();
                                    status.tool_compat_buffer.push_str(&fragment);

                                    status.tool_compat_fragment_buffer.clear();
                                    status.tool_compat_fragment_count = 0;
                                    status.tool_compat_last_flush_time = std::time::Instant::now();
                                }

                                status
                                    .tool_compat_fragment_buffer
                                    .push_str(&match &content {
                                        OpenAIMessageContent::Text(text) => text.as_str(),
                                        OpenAIMessageContent::Parts(parts) => parts
                                            .iter()
                                            .find_map(|part| match part {
                                                OpenAIMessageContentPart::Text { text } => {
                                                    Some(text.as_str())
                                                }
                                                _ => None,
                                            })
                                            .unwrap_or(""),
                                    });
                                status.tool_compat_fragment_count += 1;

                                let now = std::time::Instant::now();
                                let time_since_flush = now
                                    .duration_since(status.tool_compat_last_flush_time)
                                    .as_millis();

                                // Force flush if conditions are met
                                let should_flush = status.tool_compat_fragment_count >= 25
                                    || time_since_flush >= 100
                                    || status.tool_compat_fragment_buffer.len() > 500
                                    || status.tool_compat_fragment_buffer.contains(TOOL_TAG_START)
                                    || status.tool_compat_fragment_buffer.contains(TOOL_TAG_END);

                                if should_flush {
                                    let fragment = status.tool_compat_fragment_buffer.clone();
                                    status.tool_compat_buffer.push_str(&fragment);
                                    status.tool_compat_fragment_buffer.clear();
                                    status.tool_compat_fragment_count = 0;
                                    status.tool_compat_last_flush_time = now;

                                    // Process tool calls in the buffer
                                    loop {
                                        if !status.in_tool_call_block {
                                            let buffer = &status.tool_compat_buffer;
                                            let tool_start = TOOL_TAG_START;

                                            if let Some(start_pos) = buffer.find(tool_start) {
                                                log::debug!("buffer has tool start tag `<cpp:tool_use>` at pos: {}", start_pos);
                                                // Send text before the tool tag
                                                let text_before = &buffer[..start_pos];
                                                if !text_before.is_empty() {
                                                    unified_chunks.push(UnifiedStreamChunk::Text {
                                                        delta: text_before.to_string(),
                                                    });
                                                }

                                                status.tool_compat_buffer =
                                                    buffer[start_pos..].to_string();
                                                status.in_tool_call_block = true;
                                            } else {
                                                break;
                                            }
                                        }

                                        if status.in_tool_call_block {
                                            let buffer = &status.tool_compat_buffer;
                                            let tool_end = TOOL_TAG_END;

                                            if let Some(end_pos) = buffer.find(tool_end) {
                                                let tool_xml = &buffer[..end_pos + tool_end.len()];
                                                log::debug!("find tool end tag `{}` at pos: {}, tool xml: {}",TOOL_TAG_END, end_pos, tool_xml);

                                                if let Ok(parsed_tool) =
                                                    from_str::<ToolUse>(tool_xml)
                                                {
                                                    let tool_id =
                                                        format!("ccp_{}", uuid::Uuid::new_v4());
                                                    let mut arguments = serde_json::Map::new();
                                                    for param in parsed_tool.params.param {
                                                        arguments.insert(
                                                            param.name.clone(),
                                                            serde_json::Value::String(
                                                                param.get_value(),
                                                            ),
                                                        );
                                                    }

                                                    // Send tool call start
                                                    unified_chunks.push(
                                                        UnifiedStreamChunk::ToolUseStart {
                                                            tool_type: "function".to_string(),
                                                            id: tool_id.clone(),
                                                            name: parsed_tool.name.clone(),
                                                        },
                                                    );

                                                    // Send tool call parameters
                                                    let args_json =
                                                        serde_json::to_string(&arguments)
                                                            .unwrap_or_default();
                                                    unified_chunks.push(
                                                        UnifiedStreamChunk::ToolUseDelta {
                                                            id: tool_id,
                                                            delta: args_json.clone(),
                                                        },
                                                    );
                                                    log::info!(
                                                        "tool parse success, name: {}, param: {}",
                                                        parsed_tool.name.clone(),
                                                        args_json
                                                    )
                                                } else {
                                                    unified_chunks.push(
                                                        UnifiedStreamChunk::Error {
                                                            message: format!(
                                                                "tool xml parse failed, xml: {}",
                                                                tool_xml
                                                            ),
                                                        },
                                                    );
                                                    log::warn!(
                                                        "tool xml parse failed, xml: {}",
                                                        tool_xml
                                                    );
                                                }

                                                status.tool_compat_buffer =
                                                    buffer[end_pos + tool_end.len()..].to_string();
                                                status.in_tool_call_block = false;
                                            } else {
                                                break;
                                            }
                                        }
                                    }

                                    // After processing complete tool calls, handle the remaining buffer.
                                    // This part handles text that is not part of a tool call, but might
                                    // end with a partial tool tag (e.g., "<ccp:tool_u").
                                    if !status.in_tool_call_block
                                        && !status.tool_compat_buffer.is_empty()
                                    {
                                        let buffer = &status.tool_compat_buffer;
                                        let tool_start = TOOL_TAG_START;
                                        let tool_end = TOOL_TAG_END;

                                        let mut partial_tag_len = 0;

                                        // Check for a partial start tag at the end of the buffer to avoid flushing it as text.
                                        // Iterate from the minimum of buffer length and tag length down to 1.
                                        for i in (1..=std::cmp::min(buffer.len(), tool_start.len()))
                                            .rev()
                                        {
                                            if buffer.ends_with(&tool_start[..i]) {
                                                partial_tag_len = i;
                                                break;
                                            }
                                        }

                                        // Also check for a partial end tag if no partial start tag was found.
                                        // This prevents flushing text that might be part of a closing tool tag.
                                        if partial_tag_len == 0 {
                                            for i in
                                                (1..=std::cmp::min(buffer.len(), tool_end.len()))
                                                    .rev()
                                            {
                                                if buffer.ends_with(&tool_end[..i]) {
                                                    partial_tag_len = i;
                                                    break;
                                                }
                                            }
                                        }

                                        let text_to_flush_len = buffer.len() - partial_tag_len;
                                        if text_to_flush_len > 0 {
                                            // Flush the text part that is definitely not part of a tag.
                                            let text_to_flush = &buffer[..text_to_flush_len];
                                            unified_chunks.push(UnifiedStreamChunk::Text {
                                                delta: text_to_flush.to_string(),
                                            });
                                            // Update the buffer to only contain the partial tag (or be empty).
                                            status.tool_compat_buffer =
                                                buffer[text_to_flush_len..].to_string();
                                        }
                                        // If text_to_flush_len is 0, the entire buffer is a partial tag.
                                        // In this case, we do nothing and wait for the next chunk to complete the tag.
                                    }
                                }
                            } else {
                                // Original streaming logic for non-tool compatibility mode
                                if has_text {
                                    if let Ok(status) = sse_status.write() {
                                        update_message_block(status, "text".to_string());
                                    }
                                }

                                if let Ok(mut status) = sse_status.write() {
                                    if status.text_delta_count == 0 && has_text {
                                        if status.thinking_delta_count > 0 {
                                            unified_chunks.push(
                                                UnifiedStreamChunk::ContentBlockStop {
                                                    index: (status.message_index - 1).max(0),
                                                },
                                            );
                                        }

                                        if status.tool_delta_count > 0 {
                                            unified_chunks.push(UnifiedStreamChunk::ToolUseEnd {
                                                id: status.tool_id.clone(),
                                            });
                                            status.tool_delta_count = 0;
                                        }

                                        unified_chunks.push(
                                            UnifiedStreamChunk::ContentBlockStart {
                                                index: status.message_index,
                                                block: json!({
                                                     "type": "text",
                                                     "text": ""
                                                }),
                                            },
                                        );
                                    }
                                };

                                match content {
                                    OpenAIMessageContent::Text(text) => {
                                        if !text.is_empty() {
                                            if let Ok(mut status) = sse_status.write() {
                                                status.text_delta_count += 1;
                                            }
                                            unified_chunks
                                                .push(UnifiedStreamChunk::Text { delta: text });
                                        }
                                    }
                                    OpenAIMessageContent::Parts(parts) => {
                                        if let Ok(mut status) = sse_status.write() {
                                            status.text_delta_count += parts.len() as u32;
                                        }
                                        for part in parts {
                                            if let OpenAIMessageContentPart::Text { text } = part {
                                                if !text.is_empty() {
                                                    unified_chunks.push(UnifiedStreamChunk::Text {
                                                        delta: text,
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if let Some(tool_calls) = delta.tool_calls {
                        if let Ok(mut status) = sse_status.write() {
                            if status.tool_delta_count == 0 {
                                if status.text_delta_count > 0 || status.thinking_delta_count > 0 {
                                    unified_chunks.push(UnifiedStreamChunk::ContentBlockStop {
                                        index: status.message_index,
                                    });
                                }
                            }
                            status.tool_delta_count += 1;
                            update_message_block(status, "tool_use".to_string());
                        }

                        for tc in tool_calls {
                            if let Some(name) = tc.function.name.clone() {
                                if !name.is_empty() {
                                    let tool_id = tc.id.clone().unwrap_or_else(|| {
                                        format!("tool_{}", uuid::Uuid::new_v4())
                                    });
                                    let mut message_index = 0;
                                    if let Ok(mut status) = sse_status.write() {
                                        status.tool_id = tool_id.clone();
                                        message_index = status.message_index;
                                    }

                                    // for claude only
                                    unified_chunks.push(UnifiedStreamChunk::ContentBlockStart {
                                        index: message_index,
                                        block: json!({
                                            "type":"tool_use",
                                            "id": tool_id.clone(),
                                            "name": name.clone(),
                                            "input":{}
                                        }),
                                    });
                                    // for gemini and openai
                                    unified_chunks.push(UnifiedStreamChunk::ToolUseStart {
                                        tool_type: "tool_use".to_string(),
                                        id: tool_id.clone(),
                                        name,
                                    });
                                }
                            }

                            if let Some(args) = tc.function.arguments {
                                if !args.is_empty() {
                                    let mut tool_id = String::new();
                                    if let Ok(status) = sse_status.read() {
                                        tool_id = status.tool_id.clone();
                                    };
                                    unified_chunks.push(UnifiedStreamChunk::ToolUseDelta {
                                        id: tool_id,
                                        delta: args,
                                    });
                                }
                            }
                        }
                    }

                    if let Some(finish_reason) = choice.finish_reason {
                        if let Ok(mut status) = sse_status.write() {
                            if !status.tool_compat_buffer.is_empty()
                                || !status.tool_compat_fragment_buffer.is_empty()
                            {
                                log::debug!("Flushing remaining buffers at stream end - buffer: {} chars, fragment: {} chars, in_tool_block: {}",
                                    status.tool_compat_buffer.len(),
                                    status.tool_compat_fragment_buffer.len(),
                                    status.in_tool_call_block
                                );
                                unified_chunks.push(UnifiedStreamChunk::Text {
                                    delta: format!(
                                        "{}{}",
                                        status.tool_compat_buffer,
                                        status.tool_compat_fragment_buffer
                                    ),
                                });

                                status.tool_compat_buffer.clear();
                                status.tool_compat_fragment_buffer.clear();
                                status.in_tool_call_block = false;
                            }
                        }

                        let stop_reason = match finish_reason.to_lowercase().as_str() {
                            "stop" => "stop".to_string(),
                            "length" => "max_tokens".to_string(),
                            "tool_calls" => "tool_use".to_string(),
                            _ => "unknown".to_string(),
                        };
                        let usage = openai_chunk
                            .usage
                            .clone()
                            .map(|u| UnifiedUsage {
                                input_tokens: u.prompt_tokens,
                                output_tokens: u.completion_tokens,
                                cache_creation_input_tokens: None,
                                cache_read_input_tokens: None,
                                tool_use_prompt_tokens: None,
                                thoughts_tokens: None,
                                cached_content_tokens: None,
                            })
                            .unwrap_or_default();
                        unified_chunks.push(UnifiedStreamChunk::MessageStop { stop_reason, usage });
                    }
                }
            }
        }

        Ok(unified_chunks)
    }
}
