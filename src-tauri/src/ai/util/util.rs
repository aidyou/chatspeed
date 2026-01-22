use serde_json::{json, Value};
use std::collections::HashMap;
use uuid::Uuid;
use rand::{distr::Alphanumeric, Rng};

use crate::ai::network::ProxyType;

/// Process custom headers with dynamic placeholders.
///
/// # Arguments
/// * `metadata`: The metadata containing `customHeaders`.
/// * `chat_id`: The current chat ID.
///
/// # Returns
/// Returns a map of processed headers.
pub fn process_custom_headers(metadata: &Option<Value>, chat_id: &str) -> HashMap<String, String> {
    let mut processed = HashMap::new();

    if let Some(custom_headers) = metadata.as_ref().and_then(|m| m.get("customHeaders")).and_then(Value::as_array) {
        for header in custom_headers {
            if let (Some(key), Some(value)) = (header.get("key").and_then(Value::as_str), header.get("value").and_then(Value::as_str)) {
                if key.trim().is_empty() {
                    continue;
                }

                let mut processed_value = value.to_string();

                // Replace placeholders
                if processed_value.contains("{UUID}") {
                    processed_value = processed_value.replace("{UUID}", &Uuid::new_v4().to_string());
                }

                if processed_value.contains("{RANDOM}") {
                    let random_str: String = rand::rng()
                        .sample_iter(&Alphanumeric)
                        .take(8)
                        .map(char::from)
                        .collect();
                    processed_value = processed_value.replace("{RANDOM}", &random_str);
                }

                if processed_value.contains("{CONV_ID}") {
                    let conv_id = if chat_id.parse::<u64>().is_ok() {
                        // If it's a numeric ID, convert to a deterministic UUID
                        Uuid::new_v5(&Uuid::NAMESPACE_DNS, chat_id.as_bytes()).to_string()
                    } else if chat_id.is_empty() {
                        Uuid::new_v4().to_string()
                    } else {
                        chat_id.to_string()
                    };
                    processed_value = processed_value.replace("{CONV_ID}", &conv_id);
                }

                // Add cs- prefix to the key
                let final_key = if key.to_lowercase().starts_with("cs-") {
                    key.to_string()
                } else {
                    format!("cs-{}", key)
                };

                processed.insert(final_key, processed_value);
            }
        }
    }

    processed
}
/// Process custom body parameters from metadata.
///
/// # Arguments
/// * `metadata`: The metadata containing `customParams`.
///
/// # Returns
/// Returns a map of processed parameters.
pub fn process_custom_params(metadata: &Option<Value>) -> HashMap<String, Value> {
    let mut processed = HashMap::new();

    if let Some(metadata_val) = metadata {
        // Find the actual KV array
        let params_array = if metadata_val.is_array() {
            metadata_val.as_array()
        } else if let Some(arr) = metadata_val.get("customParams").and_then(|v| v.as_array()) {
            Some(arr)
        } else {
            None
        };

        if let Some(custom_params) = params_array {
            for param in custom_params {
                if let (Some(key), Some(value_val)) = (
                    param.get("key").and_then(Value::as_str),
                    param.get("value"),
                ) {
                    if key.trim().is_empty() {
                        continue;
                    }

                    // Smart type conversion for values (consistent with proxy layer)
                    let final_value = if let Some(s) = value_val.as_str() {
                        match s.to_lowercase().as_str() {
                            "true" => json!(true),
                            "false" => json!(false),
                            "" | "null" => Value::Null,
                            _ => {
                                if let Ok(n) = s.parse::<i64>() {
                                    json!(n)
                                } else if let Ok(f) = s.parse::<f64>() {
                                    json!(f)
                                } else {
                                    json!(s)
                                }
                            }
                        }
                    } else {
                        value_val.clone()
                    };

                    processed.insert(key.to_string(), final_value);
                }
            }
        }
    }

    processed
}

/// Merge custom body parameters from metadata into a JSON object.
///
/// # Arguments
/// * `body`: The target JSON object to merge into.
/// * `custom_params`: The metadata containing `customParams` (array of KV).
pub fn merge_custom_params(body: &mut Value, custom_params: &Option<Value>) {
    let processed_params = process_custom_params(custom_params);
    if let Some(obj) = body.as_object_mut() {
        // Check if the current request is a streaming request
        let is_stream = obj.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);

        for (k, v) in processed_params {
            let mut final_val = v;

            // Special fix for providers like ModelScope/Qwen:
            // "parameter.enable_thinking must be set to false for non-streaming calls"
            if k == "enable_thinking" && !is_stream && final_val.as_bool() == Some(true) {
                log::debug!("Forcing enable_thinking to false for non-streaming request to avoid API error");
                final_val = serde_json::json!(false);
            }

            obj.insert(k, final_val);
        }
    }
}

/// Get the metadata from the extra_params.
///
/// # Arguments
/// * `extra_params`: The extra parameters from the API request.
///
/// # Returns
/// Returns the metadata as a `Value` object. It is used to pass metadata back to the UI.
// pub fn get_meta_data(extra_params: Option<Value>) -> Option<Value> {
//     let excluded_keys = [
//         "presencePenalty",
//         "frequencyPenalty",
//         "responseFormat",
//         "stop",
//         "n",
//         "user",
//         "toolChoice",
//         "proxyType",
//         "proxyServer",
//         "proxyUsername",
//         "proxyPassword",
//     ];
//     let metadata = extra_params
//         .and_then(|v| v.as_object().cloned())
//         .unwrap_or_default()
//         .into_iter()
//         .filter(|(k, _)| !excluded_keys.contains(&k.as_str()))
//         .collect::<Map<_, _>>();
//     let metadata_option = if metadata.is_empty() {
//         None
//     } else {
//         Some(Value::Object(metadata))
//     };
//     metadata_option
// }

/// Initialize the extra parameters.
///
/// # Arguments
/// * `extra_params`: The extra parameters from the API request.
///
/// # Returns
/// Returns the initialized extra parameters as a `Value` object.
pub fn init_extra_params(extra_params: Option<Value>) -> Value {
    // OpenAI API: number, Optional, Defaults to 0.0
    // Number between -2.0 and 2.0.
    // Positive values penalize new tokens based on whether they appear in the text so far,
    // increasing the model's likelihood to talk about new topics.
    let presence_penalty = extra_params
        .as_ref()
        .and_then(|params| params.get("presencePenalty").and_then(|v| v.as_f64()));
    // OpenAI API: number, Optional, Defaults to 0.0
    // Number between -2.0 and 2.0.
    // Positive values penalize new tokens based on their existing frequency in the text so far,
    // decreasing the model's likelihood to repeat the same line verbatim.
    let frequency_penalty = extra_params
        .as_ref()
        .and_then(|params| params.get("frequencyPenalty").and_then(|v| v.as_f64()));

    // OpenAI API: object, Optional, Defaults to {"type": "text"}
    // An object specifying the format that the model must output.
    // Setting to { "type": "json_object" } enables JSON mode.
    // Important: when using JSON mode, you must also instruct the model to produce JSON yourself via a system or user message.
    let response_format = extra_params
        .as_ref()
        .and_then(|params| params.get("responseFormat").and_then(|v| v.as_str()));

    let tool_choice = extra_params.as_ref().and_then(|params| {
        params
            .get("toolChoice")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    });

    // Stop sequences: can be a string or an array of strings
    let stop_sequences = extra_params.as_ref().and_then(|params| {
        params.get("stop").and_then(|v| {
            if v.is_string() {
                v.as_str().map(|s| {
                    s.split('\n')
                        .filter_map(|part| {
                            let trimmed = part.trim();
                            if trimmed.is_empty() {
                                None
                            } else {
                                Some(trimmed.to_string())
                            }
                        })
                        .collect::<Vec<String>>()
                })
            } else if v.is_array() {
                v.as_array().map(|arr| {
                    arr.iter()
                        .filter_map(|val| val.as_str().map(String::from))
                        .collect()
                })
            } else {
                None
            }
        })
    });

    let n_candidate_count = extra_params
        .as_ref()
        .and_then(|params| params.get("n").and_then(|v| v.as_u64()));

    let user_id = extra_params.as_ref().and_then(|params| {
        params
            .get("user")
            .and_then(|v| v.as_str())
            .map(String::from)
    });
    // Prepare response_format object for OpenAI compatibility
    // This parameter is only added if explicitly provided and valid, to avoid errors on models that don't support it.
    let response_format_obj = match response_format {
        Some("json_object") => Some(json!({"type": "json_object"})),
        Some("text") => Some(json!({"type": "text"})),
        Some(other) if !other.is_empty() => {
            // Log a warning for unrecognized response_format and ignore it.
            log::warn!(
                "Unrecognized response_format value '{}', ignoring it.",
                other
            );
            None
        }
        _ => None, // Do not add the parameter if it's not provided or is an empty string.
    };

    // Keep the Rust underscore style for the final JSON keys
    let mut body = json!({
        "presence_penalty": presence_penalty.unwrap_or(0.0),
        "frequency_penalty": frequency_penalty.unwrap_or(0.0),
        "stop_sequences": stop_sequences.map_or(Value::Null, |s| json!(s)),
        "candidate_count": n_candidate_count.map_or(Value::Null, |n| json!(n)),
        "user_id": user_id.map_or(Value::Null, |u| json!(u)),
    });

    if let Some(rf_obj) = response_format_obj {
        if let Some(body_obj) = body.as_object_mut() {
            body_obj.insert("response_format".to_string(), rf_obj);
        }
    }

    if let Some(choice) = tool_choice {
        if let Some(body_obj) = body.as_object_mut() {
            body_obj.insert("tool_choice".to_string(), serde_json::Value::String(choice));
        }
    }

    body
}

/// Update or create metadata options.
///
/// This function updates the existing metadata if provided; if not, it creates a new metadata object.
///
/// # Arguments
///
/// * `metadata_option` - The optional existing metadata
/// * `key` - The key to update or add
/// * `value` - The value associated with the key
///
/// # Returns
///
/// The updated metadata as a `Value` object
pub fn update_or_create_metadata(metadata_option: Option<Value>, key: &str, value: Value) -> Value {
    match metadata_option {
        Some(mut metadata) => {
            metadata[key] = value;
            metadata
        }
        None => json!({ key: value }),
    }
}

/// Gets proxy type from metadata
///
/// # Arguments
/// * `metadata` - Optional metadata containing proxy configuration
///
/// # Returns
/// ProxyType based on metadata configuration:
/// - "system" -> ProxyType::System
/// - "http" with non-empty proxy_server -> ProxyType::Http(server)
/// - others -> ProxyType::None
pub fn get_proxy_type(metadata: Option<Value>) -> ProxyType {
    let Some(metadata) = metadata else {
        return ProxyType::None;
    };

    match metadata.get("proxyType").and_then(Value::as_str) {
        Some("system") => ProxyType::System,
        Some("http") => metadata
            .get("proxyServer")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map_or(ProxyType::None, |server| {
                ProxyType::Http(
                    server.to_string(),
                    metadata
                        .get("proxyUsername")
                        .and_then(Value::as_str)
                        .map(|s| s.to_string()),
                    metadata
                        .get("proxyPassword")
                        .and_then(Value::as_str)
                        .map(|s| s.to_string()),
                )
            }),
        _ => ProxyType::None,
    }
}

/// Determines the model family based on the model ID string
///
/// # Arguments
/// * `id` - The model ID string to analyze
///
/// # Returns
/// * `Option<&str>` - Some containing the model family name if matched, None otherwise
pub fn get_family_from_model_id(lower_model_id: &str) -> Option<String> {
    let family = if lower_model_id.contains("qwen")
        || lower_model_id.contains("qwq")
        || lower_model_id.contains("qw")
    {
        Some("Qwen".to_string())
    } else if lower_model_id.contains("deepseek") {
        Some("Deepseek".to_string())
    } else if lower_model_id.contains("mistralai") || lower_model_id.contains("mistral") {
        Some("Mistralai".to_string())
    } else if lower_model_id.contains("llama") {
        Some("Llama".to_string())
    } else if lower_model_id.starts_with("gpt-4")
        || lower_model_id.starts_with("gpt-3.5")
        || lower_model_id.contains("davinci") // GPT-3
        || lower_model_id.contains("curie")// GPT-3
        || lower_model_id.contains("babbage")// GPT-3
        || lower_model_id.contains("ada")// GPT-3
        || lower_model_id.starts_with("text-embedding")
        || lower_model_id.starts_with("whisper")
        || lower_model_id.starts_with("dall-e")
    {
        Some("ChatGPT".to_string())
    } else if lower_model_id.contains("claude") {
        Some("Claude".to_string())
    } else if lower_model_id.contains("gemini") {
        Some("Gemini".to_string())
    } else if lower_model_id.contains("gemma") {
        Some("Gemma".to_string())
    } else if lower_model_id.contains("phi") {
        Some("Phi".to_string())
    } else if lower_model_id.contains("/") {
        lower_model_id.split('/').next().map(|s| s.to_string())
    } else {
        None
    };

    family
}

// Checks if a model likely supports function calling (tool use).
/// This is based on common model names and families known to support this feature.
/// The matching is case-insensitive.
///
/// # Arguments
/// * `model_id` - The model ID string to check.
///
/// # Returns
/// * `bool` - True if the model likely supports function calling, false otherwise.
pub fn is_function_call_supported(lower_model_id: &str) -> bool {
    // OpenAI models (GPT-3.5 Turbo onwards)
    if lower_model_id.contains("gpt-3.5") // Covers gpt-3.5-turbo and its variants
        || lower_model_id.contains("gpt-4") // Covers gpt-4, gpt-4-turbo, gpt-4o etc.
        || lower_model_id.starts_with("o1-") // Specific provider prefixes for OpenAI models
        || lower_model_id.starts_with("o3-")
    // Specific provider prefixes for OpenAI models
    {
        return true;
    }

    // Anthropic Claude
    if lower_model_id.contains("claude") {
        // Primarily Claude 3 series. Claude 2.1 had some support.
        return true;
    }

    // Most Gemini models support it.
    if lower_model_id.contains("gemini") {
        return true;
    }

    // Qwen Qw2.5+ and Qwen QwQ
    if lower_model_id.contains("qwq")
        || lower_model_id.contains("qw2.5")
        || lower_model_id.contains("qwen3")
    {
        return true;
    }

    // Qwen series (e.g., Qwen1.5-Chat, Qwen2-Instruct, qwen-turbo, qwen-plus, qwen-max)
    if lower_model_id.contains("qwen")
        && (lower_model_id.contains("chat")
            || lower_model_id.contains("instruct")
            || lower_model_id.contains("turbo")
            || lower_model_id.contains("plus")
            || lower_model_id.contains("max"))
        && !lower_model_id.contains("audio")
    // Exclude vision/audio models that might not support standard chat tool calls
    {
        return true;
    }

    // DeepSeek (Coder/Chat series 2.0+, v3)
    if lower_model_id.contains("deepseek")
        && (lower_model_id.contains("chat")
            || lower_model_id.contains("coder")
            || lower_model_id.contains("v3"))
    {
        return true;
    }

    // Yi large models (e.g., yi-large)
    if lower_model_id.contains("yi-large") {
        return true;
    }

    // Baichuan models
    if lower_model_id.contains("baichuan")
        && (lower_model_id.contains("turbo")
            || lower_model_id.contains("chat")
            || lower_model_id.contains("instruct")
            || lower_model_id.contains("pro") // e.g. baichuan-turbo-pro
            || lower_model_id.contains("4"))
    // For Baichuan4 if it follows similar naming
    {
        return true;
    }

    false
}

/// Checks if a model is known to explicitly support or output reasoning/thinking steps
/// (e.g., DeepSeek's `<think>` tags or Claude's strong Chain-of-Thought capabilities).
/// The matching is case-insensitive.
///
/// # Arguments
/// * `model_id` - The model ID string to check.
///
/// # Returns
/// * `bool` - True if the model is known for reasoning/thinking step output, false otherwise.
pub fn is_reasoning_supported(lower_model_id: &str) -> bool {
    // qwq series models, especially qwq3
    if lower_model_id.contains("qwq")
        || (lower_model_id.contains("qwen") && lower_model_id.contains("thinking"))
    {
        return true;
    }

    // OpenAI o1 and o3 series, known for advanced reasoning/agentic capabilities
    if lower_model_id.starts_with("o1-") || lower_model_id.starts_with("o3") {
        return true;
    }

    // DeepSeek R1 series, known for explicit <think> tags / reasoning chains
    if lower_model_id.contains("deepseek") && lower_model_id.contains("r1") {
        return true;
    }

    // Gemini2.5 Flash support Thinking or Non-thinking
    if lower_model_id.contains("gemini-2.5-flash") {
        return true;
    }

    if lower_model_id.contains("claude-opus-4")
        || lower_model_id.contains("claude-sonnet-4")
        || lower_model_id.contains("claude-3-7-sonnet")
    {
        return true;
    }

    if lower_model_id.contains("glm4.5") || lower_model_id.contains("glm4.6") {
        return true;
    }

    // General check for models explicitly designated as "thinking" variants,
    // which includes "Gemini thinking series"， such as gemini-2.0-flash-thinking-exp, etc.
    if lower_model_id.contains("thinking") {
        return true;
    }

    false
}

/// Checks if a model likely supports image input (multimodal capabilities).
/// This is based on common model names and families known to support this feature.
/// The matching is case-insensitive.
///
/// # Arguments
/// * `lower_model_id` - The lowercased model ID string to check.
///
/// # Returns
/// * `bool` - True if the model likely supports image input, false otherwise.
pub fn is_image_input_supported(lower_model_id: &str) -> bool {
    // OpenAI
    if lower_model_id.contains("gpt-4-vision")
        || lower_model_id.contains("gpt-4-turbo") // gpt-4-turbo often includes vision
        || lower_model_id.contains("gpt-4o")
    {
        return true;
    }

    // Google Gemini
    if lower_model_id.contains("gemini-pro-vision")
        || lower_model_id.contains("gemini-1.0-pro-vision")
        || lower_model_id.contains("gemini-1.5-pro") // Gemini 1.5 Pro is multimodal
        || lower_model_id.contains("gemini-1.5-flash")
    // Gemini 1.5 Flash is multimodal
    {
        return true;
    }

    // All Claude models support image input, as version 3 and above natively support it,
    // and version 2 models have been deprecated.
    if lower_model_id.starts_with("claude-") {
        return true;
    }

    // Qwen (Alibaba), like Qwen2.5-VL-32B-Instruct
    if lower_model_id.contains("qwen")
        && (lower_model_id.contains("vl") || lower_model_id.contains("vision"))
    {
        return true;
    }

    // DeepSeek
    if lower_model_id.contains("deepseek-vl") {
        return true;
    }

    // Common pattern for LLaVA-based models or other vision models
    if lower_model_id.contains("llava") {
        return true;
    }

    // General check for models with common vision/VL suffixes
    if lower_model_id.contains("-vision") || lower_model_id.contains("-vl") {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_get_proxy_type() {
        // Test None metadata
        assert!(matches!(get_proxy_type(None), ProxyType::None));

        // Test system proxy
        let metadata = json!({
            "proxyType": "system"
        });
        assert!(matches!(get_proxy_type(Some(metadata)), ProxyType::System));

        // Test http proxy with valid server
        let metadata = json!({
            "proxyType": "http",
            "proxyServer": "http://127.0.0.1:7890"
        });
        if let ProxyType::Http(server, None, None) = get_proxy_type(Some(metadata)) {
            assert_eq!(server, "http://127.0.0.1:7890");
        } else {
            panic!("Expected ProxyType::Http");
        }

        // Test http proxy with empty server
        let metadata = json!({
            "proxyType": "http",
            "proxyServer": ""
        });
        assert!(matches!(get_proxy_type(Some(metadata)), ProxyType::None));

        // Test invalid proxy type
        let metadata = json!({
            "proxyType": "invalid"
        });
        assert!(matches!(get_proxy_type(Some(metadata)), ProxyType::None));
    }
}
