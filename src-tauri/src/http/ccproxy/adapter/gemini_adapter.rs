use bytes::Bytes;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::{json, Value};

use super::protocol_adapter::{
    AdaptedRequest, ProtocolAdapter, RawBackendResponse, RawBackendStreamChunk,
};
use crate::ai::network::{
    GeminiFunctionCall as GeminiFunctionCallData, GeminiResponse as GeminiResponsePayload,
};
use crate::http::ccproxy::{
    errors::ProxyAuthError,
    types::{
        OpenAIChatCompletionChoice,
        OpenAIChatCompletionRequest,
        OpenAIChatCompletionResponse,
        OpenAIChatCompletionStreamResponse,
        OpenAIFunctionCall,
        OpenAIMessageContent,
        OpenAIMessageContentPart,
        OpenAIStreamChoice,
        OpenAIUsage,
        SseEvent,
        UnifiedChatMessage,
        UnifiedToolCall, // Added SseEvent
    },
};

pub struct GeminiAdapter;

/// Helper structs for Gemini API (subset, add more as needed)
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct GeminiRequestPayload {
    contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<GeminiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_config: Option<GeminiToolConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiContent>, // System instruction is a single Content object
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GeminiGenerationConfig>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct GeminiContent {
    role: String, // "user" or "model"
    parts: Vec<GeminiPart>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
#[serde(untagged)] // Allows for different part types
enum GeminiPart {
    Text {
        text: String,
    },
    FunctionCall {
        function_call: GeminiFunctionCallData,
    },
    FunctionResponse {
        function_response: GeminiFunctionResponse,
    },
    InlineData {
        // Added for multimodal image input (base64)
        inline_data: GeminiInlineData,
    },
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct GeminiFunctionResponse {
    name: String,
    response: Value, // Gemini expects response as a JSON object, typically {"content": "..."}
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct GeminiInlineData {
    mime_type: String,
    data: String, // base64 encoded string
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct GeminiTool {
    function_declarations: Vec<GeminiFunctionDeclaration>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct GeminiFunctionDeclaration {
    name: String,
    description: String,
    parameters: Value, // JSON Schema
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct GeminiToolConfig {
    function_calling_config: GeminiFunctionCallingConfig,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct GeminiFunctionCallingConfig {
    mode: String, // "NONE", "AUTO", "ANY" (for one specific function, use ANY and provide only that function)
                  // allowed_function_names: Option<Vec<String>>, // If mode is ANY and you want to specify
}

#[derive(Serialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
struct GeminiGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_k: Option<i32>, // Gemini's topK is integer
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_sequences: Option<Vec<String>>,
    // candidate_count, etc.
}

// Note: GeminiErrorDetail was defined here. If errors are part of the GeminiResponsePayload (now GeminiResponse),
// the definition in `ai/network/types.rs` for `GeminiResponse` would need an `error` field.
// For now, we assume errors are handled by HTTP status codes or are not part of the successful stream payload structure.
// If Gemini stream can contain `{"error": ...}` objects, `ai/network/types.rs::GeminiResponse` needs an `Option<GeminiErrorDetail>` field.
// Let's assume `GeminiErrorDetail` is not needed for successful stream parsing for now.

#[async_trait]
impl ProtocolAdapter for GeminiAdapter {
    fn adapt_request(
        &self,
        base_url: &str,
        target_model_name: &str,
        target_api_key: &str,
        openai_request: &OpenAIChatCompletionRequest,
    ) -> Result<AdaptedRequest, ProxyAuthError> {
        let mut gemini_contents: Vec<GeminiContent> = Vec::new();
        let mut system_instruction_parts: Vec<GeminiPart> = Vec::new();

        for message in &openai_request.messages {
            let role = message.role.as_deref().unwrap_or_default();
            match role {
                "system" => {
                    if let Some(content) = &message.content {
                        match content {
                            OpenAIMessageContent::Text(text) => {
                                system_instruction_parts
                                    .push(GeminiPart::Text { text: text.clone() });
                            }
                            OpenAIMessageContent::Parts(parts) => {
                                for part in parts {
                                    match part {
                                        OpenAIMessageContentPart::Text { text } => {
                                            system_instruction_parts
                                                .push(GeminiPart::Text { text: text.clone() });
                                        }
                                        OpenAIMessageContentPart::ImageUrl { image_url } => {
                                            // Basic data URI parsing: "data:{mime_type};base64,{data}"
                                            if image_url.url.starts_with("data:") {
                                                if let Some(comma_idx) = image_url.url.find(',') {
                                                    let header = &image_url.url[5..comma_idx];
                                                    let base64_data =
                                                        &image_url.url[comma_idx + 1..];
                                                    let mime_type = header
                                                        .split(';')
                                                        .next()
                                                        .unwrap_or("application/octet-stream")
                                                        .to_string();

                                                    system_instruction_parts.push(
                                                        GeminiPart::InlineData {
                                                            inline_data: GeminiInlineData {
                                                                mime_type,
                                                                data: base64_data.to_string(),
                                                            },
                                                        },
                                                    );
                                                } else {
                                                    log::warn!("GeminiAdapter: Malformed data URI in system message image_url");
                                                }
                                            } else {
                                                log::warn!("GeminiAdapter: Non-data URI image_url in system message not supported for Gemini inline data, skipping.");
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                "user" => {
                    let mut gemini_user_parts = Vec::new();
                    if let Some(content) = &message.content {
                        match content {
                            OpenAIMessageContent::Text(text) => {
                                gemini_user_parts.push(GeminiPart::Text { text: text.clone() });
                            }
                            OpenAIMessageContent::Parts(parts_array) => {
                                for part in parts_array {
                                    match part {
                                        OpenAIMessageContentPart::Text { text } => {
                                            gemini_user_parts
                                                .push(GeminiPart::Text { text: text.clone() });
                                        }
                                        OpenAIMessageContentPart::ImageUrl { image_url } => {
                                            if image_url.url.starts_with("data:") {
                                                if let Some(comma_idx) = image_url.url.find(',') {
                                                    let header = &image_url.url[5..comma_idx];
                                                    let base64_data =
                                                        &image_url.url[comma_idx + 1..];
                                                    let mime_type = header
                                                        .split(';')
                                                        .next()
                                                        .unwrap_or("application/octet-stream")
                                                        .to_string();

                                                    gemini_user_parts.push(
                                                        GeminiPart::InlineData {
                                                            inline_data: GeminiInlineData {
                                                                mime_type,
                                                                data: base64_data.to_string(),
                                                            },
                                                        },
                                                    );
                                                } else {
                                                    log::warn!("GeminiAdapter: Malformed data URI in user message image_url");
                                                }
                                            } else {
                                                log::warn!("GeminiAdapter: Non-data URI image_url in user message not supported for Gemini inline data, skipping.");
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    gemini_contents.push(GeminiContent {
                        role: "user".to_string(),
                        parts: gemini_user_parts,
                    });
                }
                "assistant" => {
                    let mut parts = Vec::new();
                    if let Some(content) = &message.content {
                        // The check `!content.is_empty()` was removed as OpenAIMessageContent doesn't have `is_empty()`.
                        // The logic to handle empty text is covered by `!text_content.is_empty()` below.
                        // Assuming assistant content is primarily text for now, as per previous logic.
                        if let OpenAIMessageContent::Text(text_content) = content {
                            if !text_content.is_empty() {
                                // Avoids adding empty text part
                                parts.push(GeminiPart::Text {
                                    text: text_content.clone(),
                                });
                            }
                        }
                    }
                    if let Some(tool_calls) = &message.tool_calls {
                        for tc in tool_calls {
                            parts.push(GeminiPart::FunctionCall {
                                function_call: GeminiFunctionCallData {
                                    name: tc.function.name.clone().unwrap_or_default(),
                                    // OpenAI arguments is a string, Gemini expects a JSON object
                                    args: serde_json::from_str(
                                        tc.function.arguments.as_deref().unwrap_or("{}"),
                                    )
                                    .unwrap_or(Value::Object(Default::default())),
                                },
                            });
                        }
                    }
                    if !parts.is_empty() {
                        gemini_contents.push(GeminiContent {
                            role: "model".to_string(),
                            parts,
                        });
                    }
                }
                "tool" => {
                    // A "tool" role message in OpenAI corresponds to a function_response in Gemini,
                    // which should be part of a "user" role content.
                    // This means we need to attach it to the last "user" content or create a new one.
                    // For simplicity, let's assume it's a new user turn with just the function response.
                    if let Some(tool_call_id) = &message.tool_call_id {
                        // This is the function name for Gemini
                        if let Some(OpenAIMessageContent::Text(content_str)) = &message.content {
                            // Tool response content is expected to be a string.
                            if !content_str.is_empty() {
                                gemini_contents.push(GeminiContent {
                                    role: "user".to_string(), // Gemini function responses are from user role
                                    parts: vec![GeminiPart::FunctionResponse {
                                        function_response: GeminiFunctionResponse {
                                            name: tool_call_id.clone(),
                                            response: json!({ "content": content_str.clone() }), // Gemini expects a JSON object
                                        },
                                    }],
                                });
                            }
                        }
                    }
                }
                _ => log::warn!("GeminiAdapter: Unknown role '{}'", role),
            }
        }

        let system_instruction = if !system_instruction_parts.is_empty() {
            Some(GeminiContent {
                role: "user".to_string(),
                parts: system_instruction_parts,
            }) // Gemini system_instruction is a Content object
        } else {
            None
        };

        // Convert openai_request.tool_choice to gemini_payload.tool_config
        let mut chosen_function_name: Option<String> = None;
        let mut gemini_tool_config: Option<GeminiToolConfig> = None;
        if let Some(oa_tool_choice) = &openai_request.tool_choice {
            let mode = match oa_tool_choice {
                Value::String(s) => match s.as_str() {
                    "none" => "NONE".to_string(),
                    "auto" => "AUTO".to_string(),
                    "required" => "ANY".to_string(), // "required" in OpenAI implies Gemini should choose ANY available tool.
                    _ => "AUTO".to_string(),         // Default or unknown string
                },
                Value::Object(obj) => {
                    if obj.get("type").and_then(Value::as_str) == Some("function") {
                        if let Some(func_obj) = obj.get("function").and_then(Value::as_object) {
                            if let Some(name_val) = func_obj.get("name").and_then(Value::as_str) {
                                chosen_function_name = Some(name_val.to_string());
                                "ANY".to_string() // Gemini mode is ANY when a specific function is chosen
                            } else {
                                "AUTO".to_string() // Malformed specific function choice (missing name)
                            }
                        } else {
                            "AUTO".to_string() // Malformed specific choice
                        }
                    } else {
                        "AUTO".to_string() // Unknown object type
                    }
                }
                _ => "AUTO".to_string(), // Default for other Value types
            };
            gemini_tool_config = Some(GeminiToolConfig {
                function_calling_config: GeminiFunctionCallingConfig { mode },
            });
        }

        // Convert openai_request.tools to gemini_payload.tools
        // If a specific function was chosen, only include that one.
        let mut gemini_tools: Option<Vec<GeminiTool>> = None;
        if let Some(oa_tools) = &openai_request.tools {
            let mut declarations: Vec<GeminiFunctionDeclaration> = Vec::new();
            for tool in oa_tools {
                if tool.r#type == "function" {
                    // If a specific function is chosen, only add it if it matches.
                    // Otherwise, add all functions.
                    if let Some(chosen_name) = &chosen_function_name {
                        if tool.function.name == *chosen_name {
                            declarations.push(GeminiFunctionDeclaration {
                                name: tool.function.name.clone(),
                                description: tool.function.description.clone().unwrap_or_default(),
                                parameters: tool.function.parameters.clone(),
                            });
                        }
                    } else {
                        // No specific function chosen, add all
                        declarations.push(GeminiFunctionDeclaration {
                            name: tool.function.name.clone(),
                            description: tool.function.description.clone().unwrap_or_default(),
                            parameters: tool.function.parameters.clone(),
                        });
                    }
                }
            }
            if !declarations.is_empty() {
                gemini_tools = Some(vec![GeminiTool {
                    function_declarations: declarations,
                }]);
            }
        }

        let mut generation_config = GeminiGenerationConfig::default();
        generation_config.max_output_tokens = openai_request.max_tokens;
        generation_config.temperature = openai_request.temperature;
        generation_config.top_p = openai_request.top_p;
        generation_config.top_k = openai_request.top_k; // Assuming top_k is i32
        generation_config.stop_sequences = openai_request.stop.clone();

        let gemini_payload = GeminiRequestPayload {
            contents: gemini_contents,
            tools: gemini_tools,
            tool_config: gemini_tool_config,
            system_instruction,
            generation_config: Some(generation_config),
        };

        let body_bytes = serde_json::to_vec(&gemini_payload).map_err(|e| {
            log::error!("GeminiAdapter: Failed to serialize request: {}", e);
            ProxyAuthError::InternalError("Failed to serialize Gemini request".to_string())
        })?;

        let url = if openai_request.stream.unwrap_or(false) {
            format!(
                "{}/models/{}:streamGenerateContent?alt=sse&key={}",
                base_url, target_model_name, target_api_key
            )
        } else {
            format!(
                "{}/models/{}:generateContent?key={}",
                base_url, target_model_name, target_api_key
            )
        };

        Ok(AdaptedRequest {
            url,
            body: Bytes::from(body_bytes),
            headers_to_add: vec![
                (
                    reqwest::header::CONTENT_TYPE.as_str().to_string(),
                    "application/json".to_string(),
                ),
                // ("x-goog-api-key".to_string(), target_api_key.to_string()),
            ],
        })
    }

    fn adapt_response_body(
        &self,
        raw_response: RawBackendResponse,
        // Assuming ProtocolAdapter trait was updated to pass this:
        target_model_name: &str,
    ) -> Result<OpenAIChatCompletionResponse, ProxyAuthError> {
        let gemini_response: GeminiResponsePayload =
            serde_json::from_slice(&raw_response.body_bytes).map_err(|e| {
                log::error!(
                    "GeminiAdapter: Failed to deserialize Gemini response: {}, body: {:?}",
                    e,
                    String::from_utf8_lossy(&raw_response.body_bytes)
                );
                ProxyAuthError::InternalError(format!(
                    "Failed to deserialize Gemini response: {}",
                    e
                ))
            })?;

        // Error handling: The GeminiResponse struct from ai/network/types.rs does not currently have an 'error' field.
        // If Gemini API can return an error object within a 200 OK response (e.g. in non-streaming),
        // then the GeminiResponse struct in types.rs would need to be updated.
        // For now, we assume errors are primarily indicated by non-2xx HTTP status codes,
        // which should be handled before calling adapt_response_body, or this function
        // would receive a body that's an error structure, and deserialization into GeminiResponsePayload (now GeminiResponse) might fail.
        // If Gemini API can return `{"error": ...}` in a 200 OK, `ai::network::types::GeminiResponse` needs an `error` field.
        // For now, removing the check for `gemini_response.error`.
        // if let Some(gemini_error) = gemini_response.error {
        //     log::error!("Gemini API returned an error: code={}, message={}, status={}", gemini_error.code, gemini_error.message, gemini_error.status);
        //     return Err(ProxyAuthError::InternalError(format!("Gemini API Error ({} {}): {}", gemini_error.code, gemini_error.status, gemini_error.message)));
        // }

        let first_candidate = gemini_response
            .candidates
            .as_ref()
            .and_then(|c| c.first())
            .ok_or_else(|| {
                log::warn!("GeminiAdapter: No candidates found in Gemini response.");
                ProxyAuthError::InternalError("No candidates in Gemini response".to_string())
            })?;

        let mut assistant_message_content: Option<String> = None;
        let mut assistant_tool_calls: Option<Vec<UnifiedToolCall>> = None;

        // content_part is GeminiContentResponsePart (aliased to Content from types.rs)
        // content_part.parts is Vec<Part>. Part is a struct with Option<String> text and Option<GeminiFunctionCallData> function_call.
        for part_data in &first_candidate.content.parts {
            // part_data is &Part (aliased as &GeminiPartResponse)
            if let Some(text_content) = &part_data.text {
                assistant_message_content =
                    Some(assistant_message_content.unwrap_or_default() + text_content);
            }
            if let Some(function_call_data) = &part_data.function_call {
                // function_call_data is &GeminiFunctionCallData
                if assistant_tool_calls.is_none() {
                    assistant_tool_calls = Some(Vec::new());
                }
                if let Some(calls) = assistant_tool_calls.as_mut() {
                    let tool_call = UnifiedToolCall {
                        id: Some(format!("call_{}", uuid::Uuid::new_v4().to_string())), // Use UUID for unique ID
                        r#type: Some("function".to_string()),
                        function: OpenAIFunctionCall {
                            name: Some(function_call_data.name.clone()),
                            arguments: Some(function_call_data.args.to_string()), // Convert JSON Value back to string
                        },
                        index: None, // Not applicable for non-streaming response tool_calls
                    };
                    calls.push(tool_call);
                }
            }
        }

        let finish_reason_str = match first_candidate.finish_reason.as_deref() {
            Some("STOP") => Some("stop".to_string()),
            Some("MAX_TOKENS") => Some("length".to_string()),
            Some("TOOL_CODE") => Some("tool_calls".to_string()),
            Some("SAFETY") | Some("RECITATION") | Some("OTHER") => Some("stop".to_string()), // Or map to a custom reason if needed
            _ => None,
        };

        let choice = OpenAIChatCompletionChoice {
            index: 0,
            message: UnifiedChatMessage {
                role: Some("assistant".to_string()),
                content: assistant_message_content.map(OpenAIMessageContent::Text),
                tool_calls: assistant_tool_calls,
                tool_call_id: None,
            },
            finish_reason: finish_reason_str,
        };

        // Usage information is typically not directly in the candidate but in a separate `usageMetadata` field
        // which is not part of the provided GeminiCandidate struct.
        // For now, we'll omit usage or use candidate.token_count if available for completion tokens.
        // A more complete Gemini response structure would include top-level `usageMetadata`.
        // Let's assume for now `raw_response.body_bytes` might contain it at top level.
        let usage = gemini_response.usage_metadata.map(|um| {
            let completion_tokens = um
                .candidates_token_count // This is Option<u64> in types.rs from ai::network::types
                .unwrap_or(0); // Candidate struct in types.rs doesn't have token_count
            let prompt_tokens = um.prompt_token_count; // This is u64 in types.rs
            let total_tokens = um.total_token_count; // This is u64 in types.rs

            OpenAIUsage {
                prompt_tokens,
                completion_tokens,
                total_tokens,
            }
        });

        Ok(OpenAIChatCompletionResponse {
            id: format!("gemini-{}", uuid::Uuid::new_v4().to_string()), // Generate a unique ID
            object: "chat.completion".to_string(),
            created: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            model: target_model_name.to_string(),
            choices: vec![choice],
            usage,
        })
    }

    fn adapt_stream_chunk(
        &self,
        raw_chunk: RawBackendStreamChunk,
        // Assuming ProtocolAdapter trait was updated to pass these:
        stream_id: &str,
        target_model_name: &str,
        next_tool_call_stream_index: &mut u32,
    ) -> Result<Option<SseEvent>, ProxyAuthError> {
        let line = String::from_utf8_lossy(&raw_chunk.data);
        let trimmed_line = line.trim();
        #[cfg(debug_assertions)]
        log::debug!("GeminiAdapter Received a line: {}", trimmed_line);

        if trimmed_line.is_empty() {
            // This case should ideally not be hit if Gemini always sends non-empty JSON.
            log::trace!("GeminiAdapter: Received an empty JSON line after stripping delimiter.");
            return Ok(None);
        }

        // SSE data lines start with "data: ". We need to remove this prefix.
        let actual_json_payload = if trimmed_line.starts_with("data:") {
            trimmed_line["data:".len()..].trim_start()
        } else {
            trimmed_line // Should not happen if StreamProcessor sends full SSE lines
        };

        let gemini_response: GeminiResponsePayload = match serde_json::from_str(actual_json_payload)
        {
            Ok(resp) => resp,
            Err(e) => {
                log::error!(
                    "GeminiAdapter: Failed to deserialize stream chunk as GeminiResponsePayload: {}, line: '{}'",
                    e,
                    actual_json_payload // Log the actual string we tried to parse
                );
                return Err(ProxyAuthError::InternalError(format!(
                    "Failed to parse Gemini stream chunk: {}",
                    e
                )));
            }
        };

        #[cfg(debug_assertions)]
        log::debug!(
            "GeminiAdapter: Parsed GeminiResponsePayload: {:?}",
            gemini_response
        );
        // Error handling for stream chunks:
        // Similar to non-streaming, if `GeminiResponse` from `types.rs` doesn't have an `error` field,
        // this check will cause a compile error.
        // Removing the check for `gemini_response.error` for now.
        // if let Some(gemini_error) = gemini_response.error {
        //     log::error!("GeminiAdapter: Stream chunk contained an error: code={}, message={}, status={}", gemini_error.code, gemini_error.message, gemini_error.status);
        //     return Err(ProxyAuthError::InternalError(format!(
        //         "Gemini Stream Error ({} {}): {}",
        //         gemini_error.code, gemini_error.status, gemini_error.message
        //     )));
        // }

        let first_candidate = match gemini_response.candidates.as_ref().and_then(|c| c.first()) {
            Some(candidate) => candidate,
            None => {
                log::trace!("GeminiAdapter: No candidate in Gemini stream chunk.");
                return Ok(None); // No candidate in this chunk part
            }
        };

        let mut delta_content: Option<String> = None;
        let mut delta_tool_calls: Option<Vec<UnifiedToolCall>> = None;

        // content_part is GeminiContentResponsePart (aliased to Content from types.rs)
        // content_part.parts is Vec<Part>. Part is a struct.
        for part_data in &first_candidate.content.parts {
            // part_data is &Part (aliased as &GeminiPartResponse)
            if let Some(text_content) = &part_data.text {
                #[cfg(debug_assertions)]
                log::debug!("GeminiAdapter: Found text part: '{}'", text_content);
                delta_content = Some(delta_content.unwrap_or_default() + text_content);
            }
            if let Some(function_call_data) = &part_data.function_call {
                // function_call_data is &GeminiFunctionCallData
                if delta_tool_calls.is_none() {
                    delta_tool_calls = Some(Vec::new());
                }
                #[cfg(debug_assertions)]
                log::debug!(
                    "GeminiAdapter: Found functionCall part: Name: {}, Args: {:?}",
                    function_call_data.name,
                    function_call_data.args
                );

                if let Some(calls) = delta_tool_calls.as_mut() {
                    let tool_call = UnifiedToolCall {
                        index: Some(*next_tool_call_stream_index),
                        id: Some(format!("call_{}", uuid::Uuid::new_v4())), // Ensure unique ID
                        r#type: Some("function".to_string()),
                        function: OpenAIFunctionCall {
                            name: Some(function_call_data.name.clone()),
                            arguments: Some(function_call_data.args.to_string()),
                        },
                    };
                    #[cfg(debug_assertions)]
                    log::debug!("GeminiAdapter: Created UnifiedToolCall: {:?}", tool_call);
                    calls.push(tool_call);
                    *next_tool_call_stream_index += 1; // Increment for the next tool call
                }
            }
        }

        let mut finish_reason_str = match first_candidate.finish_reason.as_deref() {
            Some("STOP") => Some("stop".to_string()), // Default to "stop"
            Some("MAX_TOKENS") => Some("length".to_string()),
            Some("TOOL_CODE") => Some("tool_calls".to_string()),
            Some("SAFETY") | Some("RECITATION") | Some("OTHER") => Some("stop".to_string()),
            _ => None, // No finish reason from Gemini in this chunk
        };

        // If there are tool calls in this chunk and Gemini's finish_reason is STOP,
        // it implies the stop is due to tool invocation. Override to "tool_calls".
        if delta_tool_calls.is_some() && finish_reason_str.as_deref() == Some("stop") {
            finish_reason_str = Some("tool_calls".to_string());
        };

        #[cfg(debug_assertions)]
        log::debug!(
            "GeminiAdapter: Accumulated delta_content: {:?}, delta_tool_calls: {:?}",
            delta_content,
            delta_tool_calls
        );

        let stream_choice = OpenAIStreamChoice {
            index: 0,
            delta: UnifiedChatMessage {
                role: if delta_content.is_some() || delta_tool_calls.is_some() {
                    Some("assistant".to_string())
                } else {
                    None
                },
                content: delta_content.map(OpenAIMessageContent::Text),
                tool_calls: delta_tool_calls,
                tool_call_id: None,
            },
            finish_reason: finish_reason_str.clone(),
        };

        let usage_for_stream_chunk = if finish_reason_str.is_some() {
            gemini_response.usage_metadata.map(|um| OpenAIUsage {
                prompt_tokens: um.prompt_token_count,                      // u64
                completion_tokens: um.candidates_token_count.unwrap_or(0), // Sum for all candidates, usually one in stream
                total_tokens: um.total_token_count,                        // u64
            })
        } else {
            None
        };
        #[cfg(debug_assertions)]
        log::debug!(
            "GeminiAdapter: Prepared OpenAIStreamChoice: {:?}",
            stream_choice
        );

        if stream_choice.delta.content.is_some()
            || stream_choice.delta.tool_calls.is_some()
            || stream_choice.finish_reason.is_some()
            || usage_for_stream_chunk.is_some()
        // Send even if only usage is present (e.g. final chunk)
        {
            #[cfg(debug_assertions)]
            log::debug!(
                "GeminiAdapter: Condition to send event met. HasContent: {}, HasToolCalls: {}, HasFinishReason: {}, HasUsage: {}",
                stream_choice.delta.content.is_some(), stream_choice.delta.tool_calls.is_some(), stream_choice.finish_reason.is_some(), usage_for_stream_chunk.is_some()
            );
            let openai_stream_response = OpenAIChatCompletionStreamResponse {
                id: stream_id.to_string(),
                object: "chat.completion.chunk".to_string(),
                created: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                model: target_model_name.to_string(),
                choices: vec![stream_choice],
                usage: usage_for_stream_chunk,
            };

            match serde_json::to_string(&openai_stream_response) {
                Ok(ref json_payload_debug) => {
                    // Borrow here for debug logging
                    #[cfg(debug_assertions)]
                    log::debug!(
                        "GeminiAdapter: Serialized OpenAIStreamResponse JSON: {}",
                        json_payload_debug
                    );
                }
                Err(_) => {} // Error will be handled by the original match arm
            }
            match serde_json::to_string(&openai_stream_response) {
                // Original match
                Ok(json_payload) => Ok(Some(SseEvent {
                    data: Some(json_payload),
                    ..Default::default()
                })),
                Err(e) => {
                    log::error!(
                        "GeminiAdapter: Failed to serialize OpenAIStreamResponse: {}",
                        e
                    );
                    Err(ProxyAuthError::InternalError(
                        "Failed to serialize OpenAI stream response".to_string(),
                    ))
                }
            }
        } else {
            // No meaningful data to send in this chunk
            #[cfg(debug_assertions)]
            log::debug!(
                "GeminiAdapter: No meaningful data to send in this chunk. Skipping SseEvent."
            );
            Ok(None)
        }
    }

    fn adapt_stream_end(&self) -> Option<String> {
        Some("[DONE]".to_string())
    }
}
