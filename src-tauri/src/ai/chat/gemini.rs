use async_trait::async_trait;
use reqwest::Response;
use rust_i18n::t;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

use crate::ai::error::AiError;
use crate::ai::interaction::chat_completion::ChatProtocol;
use crate::ai::interaction::constants::{
    TOKENS, TOKENS_COMPLETION, TOKENS_PER_SECOND, TOKENS_PROMPT, TOKENS_TOTAL,
};
use crate::ai::network::{
    ApiClient, ApiConfig, DefaultApiClient, ErrorFormat, StreamFormat, StreamProcessor, TokenUsage,
};
use crate::ai::traits::chat::{
    ChatResponse, FinishReason, MCPToolDeclaration, MessageType, ModelDetails, ToolCallDeclaration,
};
use crate::ai::traits::{chat::AiChatTrait, stoppable::Stoppable};
use crate::ai::util::{
    get_family_from_model_id, get_proxy_type, init_extra_params, is_function_call_supported,
    is_image_input_supported, is_reasoning_supported, update_or_create_metadata,
};
use crate::impl_stoppable;

const GEMINI_DEFAULT_API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta";

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct GeminiModel {
    name: String,         // e.g., "models/gemini-1.5-pro-latest"
    display_name: String, // e.g., "Gemini 1.5 Pro"
    description: String,
    #[serde(default)] // Make optional as not all models might have it explicitly
    input_token_limit: Option<u32>,
    #[serde(default)]
    output_token_limit: Option<u32>,
    supported_generation_methods: Vec<String>,
    #[serde(default)]
    version: Option<String>,
    // temperature, topP, topK could also be here if needed
}

#[derive(Deserialize, Debug)]
struct GeminiListModelsResponse {
    models: Vec<GeminiModel>,
}

/// Represents the Gemini chat implementation
#[derive(Clone)]
pub struct GeminiChat {
    stop_flag: Arc<Mutex<bool>>,
    client: DefaultApiClient,
}

impl GeminiChat {
    /// Creates a new instance of GeminiChat
    pub fn new() -> Self {
        Self {
            stop_flag: Arc::new(Mutex::new(false)),
            client: DefaultApiClient::new(ErrorFormat::Google),
        }
    }

    /// Builds the request payload for Gemini API
    fn build_request_body(
        &self,
        messages: Vec<Value>,
        tools: Option<Vec<MCPToolDeclaration>>,
        params: &Value,
    ) -> Value {
        let mut gemini_contents = Vec::new(); // Renamed to avoid confusion with original messages
        let mut system_instruction_val = None; // Renamed for clarity

        for message in messages {
            let role = message["role"].as_str().unwrap_or_default();
            let content_value = &message["content"]; // Keep as Value
            let tool_calls_value = message.get("tool_calls"); // OpenAI specific field

            // Handle different content types (string or array of parts) for the current message
            let parts = if content_value.is_string() {
                vec![json!({"text": content_value.as_str().unwrap_or_default()})]
            } else if content_value.is_array() {
                // Assuming the array structure is compatible or needs specific mapping
                // For now, let's try to pass it as is if it's an array of parts.
                // This might need adjustment based on actual content structure for Gemini.
                content_value
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|part| {
                        // A simple heuristic: if part is a string, wrap it. If object, assume it's a valid part.
                        if part.is_string() {
                            json!({"text": part.as_str().unwrap_or_default()})
                        } else {
                            part
                        }
                    })
                    .collect()
            } else {
                // Default or error handling for unexpected content type
                vec![json!({"text": ""})]
            };

            if role == "system" {
                // If multiple system messages exist, the last one will be used.
                // Gemini's systemInstruction is a single object.
                system_instruction_val = Some(json!({
                    "role": "user", // Gemini system prompts are passed as user role in systemInstruction
                    "parts": parts
                }));
            } else if role == "assistant" && tool_calls_value.is_some() {
                // This is an assistant message that previously requested tool calls (OpenAI format)
                // Convert it to Gemini's functionCall format
                if let Some(tool_calls_array) = tool_calls_value.and_then(Value::as_array) {
                    let mut gemini_function_call_parts = Vec::new();
                    if let Some(text_content) = content_value.as_str() {
                        if !text_content.is_empty() {
                            gemini_function_call_parts.push(json!({"text": text_content}));
                        }
                    }
                    for tool_call_obj in tool_calls_array {
                        if let (Some(name), Some(args_str), Some(_id)) = (
                            tool_call_obj
                                .get("function")
                                .and_then(|f| f.get("name"))
                                .and_then(Value::as_str)
                                .map(String::from), // Convert to String
                            tool_call_obj
                                .get("function")
                                .and_then(|f| f.get("arguments"))
                                .and_then(Value::as_str),
                            tool_call_obj.get("id").and_then(Value::as_str), // We might not need the ID when sending back to Gemini this way
                        ) {
                            // Gemini expects args as an object, not a string. Attempt to parse.
                            let args_obj: Value =
                                serde_json::from_str(args_str).unwrap_or(json!({}));
                            gemini_function_call_parts.push(json!({
                                "functionCall": {
                                    "name": name,
                                    "args": args_obj // Ensure args_obj is a JSON object
                                }
                            }));
                        }
                    }
                    if !gemini_function_call_parts.is_empty() {
                        gemini_contents.push(json!({
                            "role": "model", // Assistant requests map to "model" role
                            "parts": gemini_function_call_parts
                        }));
                    }
                }
            } else if role == "tool" {
                // This is a tool execution result (OpenAI format)
                // Convert it to Gemini's functionResponse format
                // Gemini's function response needs the name of the function that was called.
                // The content of the tool message is the result.
                if let (Some(name_val_str), Some(content_val)) = (
                    message.get("name").and_then(Value::as_str), // "name" is the function name called
                    message.get("content"),
                ) {
                    gemini_contents.push(json!({
                        "role": "function", // Gemini uses "function" for tool/function responses
                        "parts": [{
                            "functionResponse": {
                                "name": name_val_str, // This should be the name of the function that was called
                                "response": { // The actual result from your function
                                     // Gemini expects "content" inside "response" for the result.
                                     // The structure of `content_val` might need to be adapted.
                                     // For simplicity, assuming content_val is directly usable or a simple JSON.
                                     // Gemini's "response" usually contains a "content" field which then holds the actual data.
                                     // If content_val is already structured (e.g. json object), it might be okay.
                                     // If it's a simple string, it might need to be wrapped, e.g., {"text": content_val} or {"result": content_val}
                                    "content": content_val.clone() // Clone the value
                                }
                            }
                        }]
                    }));
                }
            } else {
                gemini_contents.push(json!({
                    "role": if role == "assistant" { "model" } else { "user" },
                    "parts": parts
                }));
            }
        }

        let response_format_type = params
            .get("response_format")
            .and_then(|rf| rf.get("type"))
            .and_then(|t| t.as_str())
            .unwrap_or("text");

        let response_mime_type = if response_format_type == "json_object" {
            "application/json"
        } else {
            "text/plain"
        };

        let mut payload = json!({
            "contents": gemini_contents, // Use the accumulated contents
            "generationConfig": {
                "responseMimeType": response_mime_type
            }
        });

        if let Some(obj) = payload.as_object_mut() {
            if let Some(instruction) = system_instruction_val {
                obj.insert("systemInstruction".to_string(), instruction);
            }

            if let Some(generation_config_value) = obj.get_mut("generationConfig") {
                if let Some(generation_config_map) = generation_config_value.as_object_mut() {
                    // Handle temperature if provided
                    if let Some(temperature_val) =
                        params.get("temperature").and_then(|v| v.as_f64())
                    {
                        // Gemini's temperature typically ranges from 0.0 to 2.0.
                        // We'll pass it if the user provides it.
                        // Add validation if necessary, e.g. if temperature_val >= 0.0 && temperature_val <= 2.0
                        if temperature_val >= 0.0 && temperature_val <= 2.0 {
                            generation_config_map
                                .insert("temperature".to_string(), json!(temperature_val));
                        }
                    }

                    // Handle maxOutputTokens (from params.max_tokens) if provided
                    if let Some(max_tokens_val) = params.get("max_tokens").and_then(|v| v.as_u64())
                    {
                        if max_tokens_val > 0 {
                            // Ensure max_tokens is positive
                            generation_config_map
                                .insert("maxOutputTokens".to_string(), json!(max_tokens_val));
                        }
                    }

                    if let Some(top_k) = params.get("top_k").and_then(|v| v.as_i64()) {
                        if top_k > 0 {
                            generation_config_map.insert("topK".to_string(), json!(top_k.clone()));
                        }
                    }
                    if let Some(top_p) = params.get("top_p").and_then(|v| v.as_f64()) {
                        if top_p > 0.0 && top_p <= 1.0 {
                            generation_config_map.insert("topP".to_string(), json!(top_p.clone()));
                        }
                    }

                    if let Some(stop_sequences) =
                        params.get("stop_sequences").and_then(|v| v.as_array())
                    {
                        if !stop_sequences.is_empty() {
                            generation_config_map
                                .insert("stopSequences".to_string(), json!(stop_sequences.clone()));
                        }
                    }

                    if let Some(candidate_count) =
                        params.get("candidate_count").and_then(|v| v.as_u64())
                    {
                        if candidate_count > 0 {
                            generation_config_map
                                .insert("candidateCount".to_string(), json!(candidate_count));
                        }
                    }
                }
            }

            if let Some(tools_vec) = tools {
                if params.get("tool_choice").and_then(|tc| tc.as_str()) != Some("none") {
                    let gemini_tools = tools_vec // Use the renamed variable
                        .into_iter()
                        .map(|tool| tool.to_gemini())
                        .collect::<Vec<Value>>();
                    if !gemini_tools.is_empty() {
                        obj.insert(
                            "tools".to_string(),
                            json!([{ "functionDeclarations": gemini_tools }]),
                        );
                    }
                }
            }
        }

        #[cfg(debug_assertions)]
        {
            log::debug!(
                "Gemini payload: {}",
                serde_json::to_string_pretty(&payload).unwrap_or_default()
            );
        }

        payload
    }

    /// Processes the response from Gemini API (streaming)
    async fn process_response(
        &self,
        chat_id: String,
        response: Response,
        callback: impl Fn(Arc<ChatResponse>) + Send + 'static,
        metadata_option: Option<Value>,
    ) -> Result<String, AiError> {
        let mut reasoning_content = String::new();
        let mut full_response = String::new();
        let mut token_usage = TokenUsage::default();
        let start_time = Instant::now();

        let processor = StreamProcessor::new();
        let mut event_receiver = processor
            .process_stream(response, &StreamFormat::Gemini)
            .await;

        let mut accumulated_tool_calls: HashMap<u32, ToolCallDeclaration> = HashMap::new();
        let mut finish_reason = FinishReason::Complete;

        while let Some(event) = event_receiver.recv().await {
            if self.should_stop().await {
                processor.stop();
                break;
            }

            match event {
                Ok(chunk_bytes) => {
                    // Renamed from chunk to chunk_bytes for clarity
                    let stream_chunks = self
                        .client
                        .process_stream_chunk(chunk_bytes, &StreamFormat::Gemini)
                        .await
                        .map_err(|e| {
                            let err = AiError::StreamProcessingFailed {
                                provider: "Gemini".to_string(),
                                details: e.to_string(),
                            };
                            log::error!("Gemini stream processing error: {}", err);
                            callback(ChatResponse::new_with_arc(
                                chat_id.clone(),
                                err.to_string(),
                                MessageType::Error,
                                metadata_option.clone(),
                                Some(FinishReason::Error),
                            ));
                            err
                        })?;

                    for chunk in stream_chunks {
                        // Iterate over processed chunks
                        if let Some(content) = chunk.content.clone() {
                            if !content.is_empty() {
                                full_response.push_str(&content);
                                callback(ChatResponse::new_with_arc(
                                    chat_id.clone(),
                                    content,
                                    MessageType::Text,
                                    metadata_option.clone(),
                                    None,
                                ));
                            }
                        }

                        if let Some(usage) = chunk.usage {
                            if usage.total_tokens > 0 {
                                // Gemini might send usage at the end or with specific events
                                token_usage = usage;
                                let completion_tokens = token_usage.completion_tokens as f64;
                                let duration = start_time.elapsed();
                                token_usage.tokens_per_second =
                                    if duration.as_secs_f64() > 0.0 && completion_tokens > 0.0 {
                                        completion_tokens / duration.as_secs_f64()
                                    } else {
                                        0.0
                                    };
                            }
                        }

                        if let Some(content) = chunk.reasoning_content {
                            if !content.is_empty() {
                                reasoning_content.push_str(&content);
                                callback(ChatResponse::new_with_arc(
                                    chat_id.clone(),
                                    content,
                                    MessageType::Reasoning,
                                    metadata_option.clone(),
                                    None,
                                ));
                            }
                        }

                        if let Some(tool_call_parts) = chunk.tool_calls {
                            for part in tool_call_parts {
                                let acc_call = accumulated_tool_calls
                                    .entry(part.index)
                                    .or_insert_with(|| ToolCallDeclaration {
                                        index: part.index, // part.index should be unique for each tool call instance
                                        // If Gemini's streaming part doesn't have a persistent ID for the call, generate one.
                                        // This ID needs to be consistent for this specific tool call across AssistantAction and ToolCall messages.
                                        // For now, we assume part.id might be empty initially and populated if a full ID is found,
                                        // or we might need to generate it based on index if Gemini doesn't provide one per tool_call instance.
                                        // A robust solution might be to generate a UUID here if part.id is empty.
                                        id: if part.id.is_empty() {
                                            // Use a consistent way to generate ID if not provided by Gemini stream part
                                            // Using chat_id and index makes it unique per chat and tool call part.
                                            format!(
                                                "gemtool_{}_{}",
                                                chat_id.chars().take(8).collect::<String>(),
                                                part.index
                                            )
                                        } else {
                                            part.id.clone()
                                        }, // Ensure id is always populated
                                        name: part.name.clone(),
                                        arguments: Some(String::new()),
                                        results: None,
                                    });
                                // If a more complete ID or name comes later in the stream for the same index, update it.
                                if !part.id.is_empty() && acc_call.id.is_empty() {
                                    acc_call.id = part.id.clone();
                                } // This condition might be redundant if id is always populated above
                                if !part.name.is_empty() && acc_call.name.is_empty() {
                                    acc_call.name = part.name.clone();
                                } // This condition might be redundant if name is always populated above

                                if let Some(args_chunk) = part.arguments {
                                    if !args_chunk.is_empty() {
                                        acc_call
                                            .arguments
                                            .get_or_insert_with(String::new)
                                            .push_str(&args_chunk);
                                    }
                                }
                            }
                        }

                        // Check Gemini's finish reason
                        let current_chunk_finish_reason = chunk.finish_reason.as_deref();
                        if let Some(reason_str) = current_chunk_finish_reason {
                            // If tools have been accumulated and we get any terminal finish reason,
                            // it means the model has finished its turn and expects tool calls.
                            if !accumulated_tool_calls.is_empty() {
                                finish_reason = FinishReason::ToolCalls;
                            } else {
                                // If no tools accumulated, map the finish reason normally
                                match reason_str {
                                    "STOP" => finish_reason = FinishReason::Complete,
                                    "MAX_TOKENS" => finish_reason = FinishReason::Complete, // As per your previous decision
                                    "TOOL_CODE" | "FUNCTION_CALL" => {
                                        // This case should ideally be caught by the `!accumulated_tool_calls.is_empty()` above
                                        // but if it somehow reaches here and tools *are* empty, it's an anomaly.
                                        log::warn!("Gemini finish_reason '{}' received but no tools were accumulated.", reason_str);
                                        finish_reason = FinishReason::Complete;
                                    }
                                    "SAFETY" | "RECITATION" | "OTHER" => {
                                        finish_reason = FinishReason::Error
                                    }
                                    unknown => {
                                        log::warn!("Unknown Gemini finish reason: {}", unknown);
                                        finish_reason = FinishReason::Complete;
                                    }
                                }
                            }
                        }

                        // If the determined finish_reason is ToolCalls (from this chunk or a previous one that set it)
                        // AND we have accumulated tools, then send AssistantAction and ToolCalls
                        if finish_reason == FinishReason::ToolCalls
                            && !accumulated_tool_calls.is_empty()
                        {
                            // This block should only execute once when all tool call parts are received and finish_reason is ToolCalls
                            if !accumulated_tool_calls.is_empty() {
                                // First, send the AssistantAction message with all requested tool calls
                                let assistant_tool_requests: Vec<Value> = accumulated_tool_calls
                                    .values()
                                    .map(|tcd| {
                                        // Convert ToolCallDeclaration to the format expected in the assistant's message
                                        // OpenAI's assistant message tool_calls usually look like:
                                        // { "id": "...", "type": "function", "function": { "name": "...", "arguments": "..." } }
                                        // Our ToolCallDeclaration is already very close to this.
                                        let arguments_str =
                                            tcd.arguments.as_deref().unwrap_or_default();
                                        json!({
                                            "id": tcd.id,
                                            "type": "function",
                                            "function": {
                                                "name": tcd.name,
                                                "arguments": arguments_str
                                            }
                                        })
                                    })
                                    .collect();

                                let assistant_action_message = json!({
                                    "role": "assistant",
                                    "content": if full_response.is_empty() { Value::Null } else { Value::String(full_response.clone()) },
                                    "tool_calls": assistant_tool_requests
                                });

                                match serde_json::to_string(&assistant_action_message) {
                                    Ok(serialized_assistant_action) => {
                                        callback(ChatResponse::new_with_arc(
                                            chat_id.clone(),
                                            serialized_assistant_action,
                                            MessageType::AssistantAction,
                                            metadata_option.clone(),
                                            None,
                                        ));
                                    }
                                    Err(e) => {
                                        log::error!(
                                            "Failed to serialize AssistantAction message: {}",
                                            e
                                        );
                                        // Optionally send an error message via callback here
                                    }
                                }

                                for tcd in accumulated_tool_calls.values() {
                                    match serde_json::to_string(tcd) {
                                        Ok(serialized_tcd) => {
                                            callback(ChatResponse::new_with_arc(
                                                chat_id.clone(),
                                                serialized_tcd,
                                                MessageType::ToolCall,
                                                metadata_option.clone(),
                                                None,
                                            ));
                                            #[cfg(debug_assertions)]
                                            {
                                                log::debug!(
                                                    "Gemini Tool call: {}",
                                                    serde_json::to_string_pretty(tcd)
                                                        .unwrap_or_default()
                                                );
                                            }
                                        }
                                        Err(e) => {
                                            let err = AiError::ToolCallSerializationFailed {
                                                details: e.to_string(),
                                            };
                                            log::error!("Gemini tool call serialization error for tool {:?}: {}", tcd.name, err);
                                            callback(ChatResponse::new_with_arc(
                                                chat_id.clone(),
                                                err.to_string(),
                                                MessageType::Error,
                                                metadata_option.clone(),
                                                None,
                                            ));
                                        }
                                    }
                                }
                                accumulated_tool_calls.clear();
                            }
                        }
                    }
                }
                Err(e) => {
                    // Error from SSE processor
                    let err = AiError::StreamProcessingFailed {
                        provider: "Gemini".to_string(),
                        details: e.to_string(), // e is likely String here from StreamProcessor
                    };
                    log::error!("Gemini stream event error: {}", err);
                    callback(ChatResponse::new_with_arc(
                        chat_id.clone(),
                        err.to_string(),
                        MessageType::Error,
                        metadata_option.clone(),
                        None,
                    ));
                }
            }
        }

        callback(ChatResponse::new_with_arc(
            chat_id.clone(),
            String::new(), // Empty content for Finished message
            MessageType::Finished,
            Some(update_or_create_metadata(
                metadata_option,
                TOKENS,
                json!({
                    TOKENS_TOTAL: token_usage.total_tokens,
                    TOKENS_PROMPT: token_usage.prompt_tokens,
                    TOKENS_COMPLETION: token_usage.completion_tokens,
                    TOKENS_PER_SECOND: token_usage.tokens_per_second
                }),
            )),
            Some(finish_reason),
        ));

        Ok(format!(
            "<think>{}</think>{}",
            reasoning_content, full_response
        ))
    }
}

impl_stoppable!(GeminiChat);

#[async_trait]
impl AiChatTrait for GeminiChat {
    async fn chat(
        &self,
        api_url: Option<&str>,
        model: &str,
        api_key: Option<&str>,
        chat_id: String,
        messages: Vec<Value>,
        tools: Option<Vec<MCPToolDeclaration>>,
        extra_params: Option<Value>,
        callback: impl Fn(Arc<ChatResponse>) + Send + 'static,
    ) -> Result<String, AiError> {
        // Changed return type
        let (params, metadata_option) = init_extra_params(extra_params.clone());

        let default_url = format!("{GEMINI_DEFAULT_API_BASE}/models");
        let base_url = api_url.unwrap_or(&default_url);
        // Determine if streaming is requested from params, default to true if not specified
        let is_streaming = params
            .get("stream")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        let query_url = if is_streaming {
            format!(
                "{}/{}:streamGenerateContent?alt=sse&key={}",
                base_url,
                model,
                api_key.unwrap_or("")
            )
        } else {
            format!(
                "{}/{}:generateContent?key={}",
                base_url,
                model,
                api_key.unwrap_or("")
            )
        };

        let response = self
            .client
            .post_request(
                &ApiConfig::new(Some(query_url), None, get_proxy_type(extra_params), None),
                "", // Gemini endpoint path is part of query_url
                self.build_request_body(messages, tools, &params),
                true,
            )
            .await
            .map_err(|network_err| {
                let err = AiError::ApiRequestFailed {
                    provider: "Gemini".to_string(),
                    details: network_err.to_string(),
                };
                log::error!("Gemini API request failed: {}", err);
                callback(ChatResponse::new_with_arc(
                    chat_id.clone(),
                    err.to_string(),
                    MessageType::Error,
                    metadata_option.clone(),
                    Some(FinishReason::Error),
                ));
                err
            })?;

        if response.is_error {
            let err = AiError::ApiRequestFailed {
                provider: "Gemini".to_string(),
                details: response.content.clone(),
            };
            log::error!("Gemini API returned an error: {}", err);
            callback(ChatResponse::new_with_arc(
                chat_id.clone(),
                err.to_string(),
                MessageType::Error,
                metadata_option.clone(),
                Some(FinishReason::Error),
            ));
            return Err(err);
        }

        if let Some(raw_response) = response.raw_response {
            self.process_response(chat_id.clone(), raw_response, callback, metadata_option)
                .await
        } else {
            callback(ChatResponse::new_with_arc(
                chat_id.clone(),
                response.content.clone(),
                MessageType::Finished,
                metadata_option,
                Some(FinishReason::Error),
            ));
            Ok(response.content)
        }
    }

    async fn list_models(
        &self,
        api_url: Option<&str>,
        api_key: Option<&str>, // This is the Gemini API Key
        extra_args: Option<Value>,
    ) -> Result<Vec<ModelDetails>, AiError> {
        let base_url = api_url.unwrap_or(GEMINI_DEFAULT_API_BASE);

        if api_key.is_none() || api_key.as_deref().unwrap_or("").is_empty() {
            return Err(AiError::InvalidInput(
                t!("api_key_is_require_for_list_models", provider = "Gemini").to_string(),
            ));
        }
        let endpoint_with_key = format!("models?key={}", api_key.unwrap());

        // For Gemini, API key is in query param, so ApiConfig.api_key should be None
        // to prevent DefaultApiClient from adding a "Bearer" token.
        let config = ApiConfig::new(
            Some(base_url.to_string()),
            None,
            get_proxy_type(extra_args.clone()),
            None, // No special headers needed if key is in URL for this request
        );

        let response = self
            .client
            .get_request(&config, &endpoint_with_key, None)
            .await
            .map_err(|e| AiError::ApiRequestFailed {
                provider: "Gemini".to_string(),
                details: e,
            })?;

        if response.is_error || response.content.is_empty() {
            return Err(AiError::ApiRequestFailed {
                provider: "Gemini".to_string(),
                details: response.content,
            });
        }

        let models_response: GeminiListModelsResponse = serde_json::from_str(&response.content)
            .map_err(|e| AiError::ResponseParseFailed {
                provider: "Gemini".to_string(),
                details: e.to_string(),
            })?;

        let model_details: Vec<ModelDetails> = models_response
            .models
            .into_iter()
            // Filter for models that support 'generateContent', common for chat
            .filter(|m| {
                m.supported_generation_methods
                    .contains(&"generateContent".to_string())
            })
            .map(|model| {
                let family = if model.display_name.contains("Gemini 1.5") {
                    Some("Gemini 1.5".to_string())
                } else if model.display_name.contains("Gemini 1.0 Pro")
                    || model.display_name.contains("Gemini Pro")
                {
                    Some("Gemini 1.0 Pro".to_string())
                } else if model.display_name.contains("Embedding")
                    || model.name.contains("embedding")
                {
                    Some("Embedding".to_string())
                } else if model.display_name.contains("AQA") {
                    // Attributed Question Answering
                    Some("AQA".to_string())
                } else {
                    None
                };

                let id = model.name.to_lowercase();

                ModelDetails {
                    id: model.name.clone(), // e.g. "models/gemini-1.5-pro-latest"
                    name: model.display_name.clone(),
                    protocol: ChatProtocol::Gemini,
                    max_input_tokens: model.input_token_limit,
                    max_output_tokens: model.output_token_limit,
                    description: Some(model.description),
                    last_updated: model.version.clone(), // Use version as a proxy for update info
                    family: get_family_from_model_id(&id),
                    function_call: Some(is_function_calling_supported(&id)),
                    reasoning: Some(is_reasoning_supported(&id)),
                    image_input: Some(is_image_input_supported(&id)),
                    metadata: Some(json!({
                        "version": model.version,
                        "supported_generation_methods": model.supported_generation_methods,
                    })),
                }
            })
            .collect();

        Ok(model_details)
    }
}
