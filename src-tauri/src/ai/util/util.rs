use serde_json::{json, Map, Value};

use crate::ai::network::ProxyType;

/// Get the metadata from the extra_params.
///
/// # Arguments
/// * `extra_params`: The extra parameters from the API request.
///
/// # Returns
/// Returns the metadata as a `Value` object. It is used to pass metadata back to the UI.
pub fn get_meta_data(extra_params: Option<Value>) -> Option<Value> {
    let excluded_keys = [
        "stream",
        "maxTokens",
        "temperature",
        "topP",
        "topK",
        "presencePenalty",
        "frequencyPenalty",
        "responseFormat",
        "stop",
        "n",
        "user",
        "toolChoice",
        "proxyType",
        "proxyServer",
        "proxyUsername",
        "proxyPassword",
    ];
    let metadata = extra_params
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter(|(k, _)| !excluded_keys.contains(&k.as_str()))
        .collect::<Map<_, _>>();
    let metadata_option = if metadata.is_empty() {
        None
    } else {
        Some(Value::Object(metadata))
    };
    metadata_option
}

/// Initialize the extra parameters.
///
/// # Arguments
/// * `extra_params`: The extra parameters from the API request.
///
/// # Returns
/// Returns the initialized extra parameters as a `Value` object and the metadata as a Option<Value> object.
pub fn init_extra_params(extra_params: Option<Value>) -> (Value, Option<Value>) {
    // The parameters are camelCase from the frontend
    let stream = extra_params
        .as_ref()
        .and_then(|params| params.get("stream").and_then(|v| v.as_bool()));
    let max_tokens = extra_params
        .as_ref()
        .and_then(|params| params.get("maxTokens").and_then(|v| v.as_u64()));
    let temperature = extra_params
        .as_ref()
        .and_then(|params| params.get("temperature").and_then(|v| v.as_f64()));
    let top_p = extra_params
        .as_ref()
        .and_then(|params| params.get("topP").and_then(|v| v.as_f64()));
    let top_k = extra_params
        .as_ref()
        .and_then(|params| params.get("topK").and_then(|v| v.as_u64()));
    let top_k = match top_k {
        Some(value) if value > 0 => value,
        _ => 0,
    };

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
    let response_format_obj = match response_format {
        Some("json_object") => json!({"type": "json_object"}),
        Some("text") => json!({"type": "text"}),
        Some(other) if !other.is_empty() => {
            // Log a warning for unrecognized response_format, and default to "text"
            log::warn!(
                "Unrecognized response_format value '{}', defaulting to 'text'.",
                other
            );
            json!({"type": "text"})
        }
        _ => json!({"type": "text"}), // Default for None or empty string from frontend
    };

    (
        // Keep the Rust underscore style
        json!({
            "stream": stream.unwrap_or(true),
            "max_tokens": max_tokens.unwrap_or(4096),
            "temperature": temperature.unwrap_or(1.0),
            "top_p": top_p.unwrap_or(0.0),
            "top_k": top_k,
            "tool_choice": tool_choice.unwrap_or("auto".to_string()),
            "presence_penalty": presence_penalty.unwrap_or(0.0),
            "frequency_penalty": frequency_penalty.unwrap_or(0.0),
            "response_format": response_format_obj,
            "stop_sequences": stop_sequences.map_or(Value::Null, |s| json!(s)),
            "candidate_count": n_candidate_count.map_or(Value::Null, |n| json!(n)),
            "user_id": user_id.map_or(Value::Null, |u| json!(u)),
        }),
        get_meta_data(extra_params),
    )
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
    // For Gemma, newer instruct models (e.g., based on Gemma 1 like 2B/7B instruct, or future Gemma 3+) support/will support it.
    // Gemma 2 instruct did not. This is a general catch for current and upcoming Gemma.
    if lower_model_id.contains("gemini") || lower_model_id.contains("gemma3") {
        return true;
    }

    // Qwen Qw2.5+ and Qwen QwQ
    if lower_model_id.contains("qwq")
        || lower_model_id.contains("qw2.5")
        || lower_model_id.contains("qwen3")
        || lower_model_id.contains("qwencoder")
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

    // Cohere Command R series
    if lower_model_id.contains("command-r") {
        return true;
    }

    // MistralAI models (Large, Small, Nemo, Codestral, Mixtral series)
    if lower_model_id.contains("mistral-large") // Covers mistral-large-latest, mistral-large-2402 etc.
        || lower_model_id.contains("mistral-small") // Covers mistral-small-latest etc.
        || lower_model_id.contains("mistral-nemo")
        || lower_model_id.contains("codestral")
        || lower_model_id.contains("mixtral")
    // Covers open-mixtral-8x7b, open-mixtral-8x22b etc.
    {
        return true;
    }

    // Meta Llama (Llama 3 series, Llama 3.1, Code Llama)
    if (lower_model_id.contains("llama-3") || lower_model_id.contains("llama3")) // Covers Llama 3 and Llama 3.1
        || lower_model_id.contains("code-llama")
    // Covers Code Llama instruct versions
    {
        return true;
    }

    // Specialized function calling models
    if lower_model_id.contains("firefunction-v1") // Specific model from Fireworks AI
        || lower_model_id.contains("toolcall-")
    // Generic identifier for tool-calling fine-tunes
    {
        return true;
    }

    // Microsoft Phi-3 models
    if lower_model_id.contains("phi-3") || lower_model_id.contains("phi3") {
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
    if lower_model_id.contains("qwq") || lower_model_id.contains("qwen3") {
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

    // Claude models with strong reasoning (Opus auto-shows CoT, others need prompting)
    if lower_model_id.contains("claude-3") || lower_model_id.contains("claude-4") {
        // Includes: claude-3-opus (auto CoT), claude-4-opus/sonnet (extended thinking mode)
        // Excludes: claude-instant (basic reasoning only)
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

    // Anthropic Claude
    // Claude 3 models support image input.
    if lower_model_id.contains("claude-3") || lower_model_id.contains("claude-4") {
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
