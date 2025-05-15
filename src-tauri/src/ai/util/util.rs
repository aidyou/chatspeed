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
        _ => 40,
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
            "top_p": top_p.unwrap_or(1.0),
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
