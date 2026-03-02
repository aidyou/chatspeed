use rand::{distr::Alphanumeric, Rng};
use serde_json::{json, Value};
use std::collections::HashMap;
use uuid::Uuid;

use crate::ai::network::ProxyType;
use crate::ai::traits::chat::ChatMetadata;

/// Process custom headers with dynamic placeholders from type-safe metadata.
pub fn process_custom_headers(
    metadata: &Option<ChatMetadata>,
    chat_id: &str,
) -> HashMap<String, String> {
    // Convert ChatMetadata to Value for compatibility with legacy function
    let metadata_value = metadata.as_ref().and_then(|md| md.to_value());
    process_custom_headers_value(&metadata_value, chat_id)
}

/// Legacy wrapper for processing custom headers from raw JSON value.
pub fn process_custom_headers_value(
    metadata: &Option<Value>,
    chat_id: &str,
) -> HashMap<String, String> {
    let mut processed = HashMap::new();

    if let Some(custom_headers) = metadata
        .as_ref()
        .and_then(|m| m.get("customHeaders"))
        .and_then(Value::as_array)
    {
        for header in custom_headers {
            if let (Some(key), Some(value)) = (
                header.get("key").and_then(Value::as_str),
                header.get("value").and_then(Value::as_str),
            ) {
                if key.trim().is_empty() {
                    continue;
                }

                let mut processed_value = value.to_string();

                // Replace placeholders
                if processed_value.contains("{UUID}") {
                    processed_value =
                        processed_value.replace("{UUID}", &Uuid::new_v4().to_string());
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
                        Uuid::new_v5(&Uuid::NAMESPACE_DNS, chat_id.as_bytes()).to_string()
                    } else if chat_id.is_empty() {
                        Uuid::new_v4().to_string()
                    } else {
                        chat_id.to_string()
                    };
                    processed_value = processed_value.replace("{CONV_ID}", &conv_id);
                }

                // Headers from backend UI are prefixed with "cs-" to distinguish from standard ones
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

/// Process custom body parameters from raw JSON value.
pub fn process_custom_params_value(metadata: &Option<Value>) -> HashMap<String, Value> {
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
                if let (Some(key), Some(value_val)) =
                    (param.get("key").and_then(Value::as_str), param.get("value"))
                {
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

/// Merge custom body parameters into a JSON object from ChatMetadata.
pub fn merge_custom_params(body: &mut Value, metadata: &Option<ChatMetadata>) {
    // Convert ChatMetadata to Value for compatibility with legacy function
    let metadata_value = metadata.as_ref().and_then(|md| md.to_value());
    merge_custom_params_value(body, &metadata_value);
}

/// Merge custom body parameters from metadata value into a JSON object.
pub fn merge_custom_params_value(body: &mut Value, metadata: &Option<Value>) {
    let processed_params = process_custom_params_value(metadata);
    if let Some(obj) = body.as_object_mut() {
        let is_stream = obj.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);

        for (k, v) in processed_params {
            let mut final_val = v;

            // Provider-specific fixes (e.g., Qwen doesn't support reasoning in non-streaming mode)
            if k == "enable_thinking" && !is_stream && final_val.as_bool() == Some(true) {
                log::debug!(
                    "Forcing enable_thinking to false for non-streaming request to avoid API error"
                );
                final_val = serde_json::json!(false);
            }

            obj.insert(k, final_val);
        }
    }
}

/// Initialize standard request parameters from ChatMetadata.
pub fn init_request_params(metadata: &Option<ChatMetadata>) -> Value {
    // Convert ChatMetadata to Value for compatibility with legacy function
    let metadata_value = metadata.as_ref().and_then(|md| md.to_value());
    init_request_params_value(metadata_value)
}

/// Initialize the standard request parameters from metadata value.
pub fn init_request_params_value(metadata: Option<Value>) -> Value {
    // Extract parameters from raw JSON value
    let presence_penalty = metadata
        .as_ref()
        .and_then(|params| params.get("presencePenalty").and_then(|v| v.as_f64()))
        .unwrap_or(0.0);

    let frequency_penalty = metadata
        .as_ref()
        .and_then(|params| params.get("frequencyPenalty").and_then(|v| v.as_f64()))
        .unwrap_or(0.0);

    let response_format = metadata
        .as_ref()
        .and_then(|params| params.get("responseFormat").and_then(|v| v.as_str()));

    let stop_sequences = metadata.as_ref().and_then(|params| {
        params.get("stop").and_then(|v| {
            if v.is_string() {
                v.as_str()
                    .map(|s| {
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
                    .filter(|v| !v.is_empty())
            } else if v.is_array() {
                v.as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|val| val.as_str().map(String::from))
                            .collect::<Vec<String>>()
                    })
                    .filter(|v| !v.is_empty())
            } else {
                None
            }
        })
    });

    let n_candidate_count = metadata
        .as_ref()
        .and_then(|params| params.get("n").and_then(|v| v.as_u64()));

    let user_id = metadata.as_ref().and_then(|params| {
        params
            .get("user")
            .and_then(|v| v.as_str())
            .map(String::from)
    });

    let tool_choice = metadata.as_ref().and_then(|params| {
        params
            .get("toolChoice")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    });

    // Prepare response_format object for compatibility
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
        "presence_penalty": presence_penalty,
        "frequency_penalty": frequency_penalty,
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
        || lower_model_id.contains("davinci")
        || lower_model_id.contains("curie")
        || lower_model_id.contains("babbage")
        || lower_model_id.contains("ada")
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
    } else if lower_model_id.contains("kimi") {
        Some("Kimi".to_string())
    } else if lower_model_id.contains("glm") {
        Some("GLM".to_string())
    } else if lower_model_id.contains("minimax") {
        Some("MiniMax".to_string())
    } else if lower_model_id.contains("step") || lower_model_id.contains("stepfun") {
        Some("StepFun".to_string())
    } else if lower_model_id.contains("/") {
        lower_model_id.split('/').next().map(|s| s.to_string())
    } else {
        None
    };

    family
}

/// Checks if a model likely supports function calling (tool use).
pub fn is_function_call_supported(lower_model_id: &str) -> bool {
    if lower_model_id.contains("gpt-")
        || lower_model_id.starts_with("o1-")
        || lower_model_id.starts_with("o3-")
    {
        return true;
    }

    if lower_model_id.contains("claude") {
        return true;
    }

    if lower_model_id.contains("gemini") {
        return true;
    }

    if lower_model_id.contains("qwq")
        || lower_model_id.contains("qw2.5")
        || lower_model_id.contains("qwen3")
    {
        return true;
    }

    if lower_model_id.contains("qwen")
        && (lower_model_id.contains("chat")
            || lower_model_id.contains("instruct")
            || lower_model_id.contains("turbo")
            || lower_model_id.contains("plus")
            || lower_model_id.contains("max"))
        && !lower_model_id.contains("audio")
    {
        return true;
    }

    if lower_model_id.contains("deepseek")
        && (lower_model_id.contains("chat")
            || lower_model_id.contains("coder")
            || lower_model_id.contains("v3"))
    {
        return true;
    }

    if lower_model_id.contains("yi-large") {
        return true;
    }

    // Kimi (Moonshot)
    if lower_model_id.contains("kimi")
        && (lower_model_id.contains("k2")
            || lower_model_id.contains("k1.5")
            || lower_model_id.contains("k1.6")
            || lower_model_id.contains("latest"))
    {
        return true;
    }

    // GLM (Zhipu AI)
    if lower_model_id.contains("glm")
        && (lower_model_id.contains("4")
            || lower_model_id.contains("3")
            || lower_model_id.contains("turbo")
            || lower_model_id.contains("flash")
            || lower_model_id.contains("air"))
    {
        return true;
    }

    // MiniMax
    if lower_model_id.contains("minimax")
        && (lower_model_id.contains("2.")
            || lower_model_id.contains("text")
            || lower_model_id.contains("chat"))
    {
        return true;
    }

    // StepFun
    if lower_model_id.contains("step")
        && (lower_model_id.contains("1")
            || lower_model_id.contains("2")
            || lower_model_id.contains("chat"))
    {
        return true;
    }

    if lower_model_id.contains("baichuan")
        && (lower_model_id.contains("turbo")
            || lower_model_id.contains("chat")
            || lower_model_id.contains("instruct")
            || lower_model_id.contains("pro")
            || lower_model_id.contains("4"))
    {
        return true;
    }

    false
}

/// Checks if a model is known to explicitly support or output reasoning/thinking steps
pub fn is_reasoning_supported(lower_model_id: &str) -> bool {
    if lower_model_id.contains("qwq")
        || (lower_model_id.contains("qwen") && lower_model_id.contains("thinking"))
    {
        return true;
    }

    if lower_model_id.starts_with("o1-") || lower_model_id.starts_with("o3") {
        return true;
    }

    if lower_model_id.contains("deepseek") && lower_model_id.contains("r1") {
        return true;
    }

    if lower_model_id.contains("gemini-2.5-flash") {
        return true;
    }

    if lower_model_id.contains("claude-opus-4")
        || lower_model_id.contains("claude-sonnet-4")
        || lower_model_id.contains("claude-3-7-sonnet")
    {
        return true;
    }

    if lower_model_id.contains("glm4.5")
        || lower_model_id.contains("glm4.6")
        || lower_model_id.contains("glm5")
    {
        return true;
    }

    // Kimi (Moonshot) - k2 and above support reasoning
    if lower_model_id.contains("kimi") && lower_model_id.contains("k2") {
        return true;
    }

    // MiniMax - 2 and above support reasoning
    if lower_model_id.contains("minimax") && lower_model_id.contains("2") {
        return true;
    }

    // StepFun - Step-2 and above support reasoning
    if lower_model_id.contains("step") && lower_model_id.contains("2") {
        return true;
    }

    if lower_model_id.contains("thinking") {
        return true;
    }

    false
}

/// Checks if a model likely supports image input (multimodal capabilities).
pub fn is_image_input_supported(lower_model_id: &str) -> bool {
    // GPT-4 models with vision/multimodal support
    if lower_model_id.contains("gpt-4-vision")
        || lower_model_id.contains("gpt-4-turbo")
        || lower_model_id.starts_with("gpt-4o") // gpt-4o, gpt-4o-mini, etc.
        || lower_model_id.contains("gpt-5")
    {
        return true;
    }

    // All Gemini models support image input (multimodal by default)
    if lower_model_id.starts_with("gemini-") {
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
