use serde_json::{json, Map, Value};

use crate::ai::network::ProxyType;

/// Get the metadata from the extra_params.
///
/// # Parameters
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
/// # Parameters
/// * `extra_params`: The extra parameters from the API request.
///
/// # Returns
/// Returns the initialized extra parameters as a `Value` object and the metadata as a Option<Value> object.
pub fn init_extra_params(extra_params: Option<Value>) -> (Value, Option<Value>) {
    // print the value of extra_params
    dbg!(&extra_params);

    // The parameters are camelCase from the frontend
    let stream = extra_params
        .clone()
        .and_then(|params| params.get("stream").and_then(|v| v.as_bool()));
    let max_tokens = extra_params
        .clone()
        .and_then(|params| params.get("maxTokens").and_then(|v| v.as_u64()));
    let temperature = extra_params
        .clone()
        .and_then(|params| params.get("temperature").and_then(|v| v.as_f64()));
    let top_p = extra_params
        .clone()
        .and_then(|params| params.get("topP").and_then(|v| v.as_f64()));
    let top_k = extra_params
        .clone()
        .and_then(|params| params.get("topK").and_then(|v| v.as_u64()));
    let top_k = match top_k {
        Some(value) if value > 0 => value,
        _ => 40,
    };
    (
        // Keep the Rust underscore style
        json!({
            "stream": stream.unwrap_or(true),
            "max_tokens": max_tokens.unwrap_or(4096),
            "temperature": temperature.unwrap_or(1.0),
            "top_p": top_p.unwrap_or(1.0),
            "top_k": top_k,
        }),
        get_meta_data(extra_params),
    )
}

/// Update or create metadata options.
///
/// This function updates the existing metadata if provided; if not, it creates a new metadata object.
///
/// # Parameters
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
