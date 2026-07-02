use bytes::Bytes;
use serde_json::Value;

use crate::ccproxy::adapter::unified::{UnifiedRequest, UnifiedToolChoice};
use crate::ccproxy::errors::CCProxyError;
use crate::ccproxy::types::ProxyModel;
use crate::ccproxy::ChatProtocol;

fn should_relax_required_tool_choice(base_url: &str) -> bool {
    reqwest::Url::parse(base_url)
        .ok()
        .and_then(|url| url.host_str().map(|host| host.to_lowercase()))
        .is_some_and(|host| host == "api.deepseek.com" || host == "api.deepseek.cn")
}

fn deepseek_reasoning_enabled(body_json: &Value) -> bool {
    if let Some(thinking_enabled) = body_json
        .get("thinking")
        .and_then(|thinking| thinking.get("type"))
        .and_then(|value| value.as_str())
        .map(|value| value.eq_ignore_ascii_case("enabled"))
    {
        return thinking_enabled;
    }

    if let Some(enable_thinking) = body_json
        .get("enable_thinking")
        .and_then(|value| value.as_bool())
    {
        return enable_thinking;
    }

    if body_json
        .get("reasoning_effort")
        .and_then(|value| value.as_str())
        .is_some_and(|value| !value.trim().is_empty())
    {
        return true;
    }

    body_json
        .get("thinking_budget")
        .and_then(|value| value.as_i64())
        .is_some_and(|value| value > 0)
}

fn normalize_deepseek_reasoning_replay(body_json: &mut Value) {
    if !deepseek_reasoning_enabled(body_json) {
        return;
    }

    let has_tool_round = body_json
        .get("messages")
        .and_then(|messages| messages.as_array())
        .is_some_and(|messages| {
            messages.iter().any(|message| {
                message
                    .get("role")
                    .and_then(|role| role.as_str())
                    .is_some_and(|role| role == "tool")
                    || message
                        .get("tool_calls")
                        .and_then(|tool_calls| tool_calls.as_array())
                        .is_some_and(|tool_calls| !tool_calls.is_empty())
            })
        });

    if !has_tool_round {
        return;
    }

    let Some(messages) = body_json
        .get_mut("messages")
        .and_then(|messages| messages.as_array_mut())
    else {
        return;
    };

    for message in messages.iter_mut() {
        let Some(role) = message.get("role").and_then(|role| role.as_str()) else {
            continue;
        };
        if role != "assistant" {
            continue;
        }

        let Some(message_obj) = message.as_object_mut() else {
            continue;
        };

        if message_obj.contains_key("reasoning_content") {
            continue;
        }

        if let Some(thinking) = message_obj.get("thinking").cloned() {
            message_obj.insert("reasoning_content".to_string(), thinking);
        } else {
            message_obj.insert(
                "reasoning_content".to_string(),
                Value::String(String::new()),
            );
        }
    }
}

pub fn preprocess_client_request_body(
    client_request_body: Bytes,
    chat_protocol: &ChatProtocol,
    proxy_model: &ProxyModel,
) -> Result<Bytes, CCProxyError> {
    let mut body_json: Value = serde_json::from_slice(&client_request_body).map_err(|e| {
        CCProxyError::InternalError(format!("Failed to deserialize request body: {}", e))
    })?;

    if should_relax_required_tool_choice(&proxy_model.base_url)
        && matches!(
            chat_protocol,
            ChatProtocol::OpenAI | ChatProtocol::HuggingFace
        )
    {
        let has_tools = body_json
            .get("tools")
            .and_then(|tools| tools.as_array())
            .is_some_and(|tools| !tools.is_empty());

        if has_tools {
            if let Some(tool_choice) = body_json.get_mut("tool_choice") {
                if tool_choice.as_str() == Some("required") {
                    *tool_choice = Value::String("auto".to_string());
                }
            }
        }

        normalize_deepseek_reasoning_replay(&mut body_json);
    }

    serde_json::to_vec(&body_json)
        .map(Bytes::from)
        .map_err(|e| {
            CCProxyError::InternalError(format!("Failed to serialize preprocessed body: {}", e))
        })
}

pub fn preprocess_unified_request(unified_request: &mut UnifiedRequest, proxy_model: &ProxyModel) {
    if matches!(
        unified_request.tool_choice,
        Some(UnifiedToolChoice::Required)
    ) && should_relax_required_tool_choice(&proxy_model.base_url)
    {
        unified_request.tool_choice = Some(UnifiedToolChoice::Auto);
    }
}

#[cfg(test)]
mod tests {
    use super::preprocess_client_request_body;
    use crate::ccproxy::{types::ProxyModel, ChatProtocol};
    use bytes::Bytes;
    use serde_json::json;

    fn deepseek_proxy_model() -> ProxyModel {
        ProxyModel {
            client_alias: "deepseek-v4-flash".to_string(),
            provider_id: 1,
            provider: "Deepseek".to_string(),
            chat_protocol: ChatProtocol::OpenAI,
            base_url: "https://api.deepseek.com".to_string(),
            model: "deepseek-v4-flash".to_string(),
            api_key: String::new(),
            model_metadata: None,
            custom_params: None,
            prompt_injection: "off".to_string(),
            prompt_injection_position: None,
            prompt_text: String::new(),
            tool_filter: Default::default(),
            prompt_replace: Vec::new(),
            temp_ratio: 1.0,
            max_tokens: None,
            temperature: None,
            presence_penalty: None,
            frequency_penalty: None,
            top_p: None,
            top_k: None,
            stop: Vec::new(),
            tool_compat_mode: None,
        }
    }

    #[test]
    fn preprocess_deepseek_replays_missing_reasoning_as_empty_string() {
        let body = json!({
            "model": "deepseek-v4-flash",
            "reasoning_effort": "medium",
            "messages": [
                { "role": "user", "content": "start" },
                {
                    "role": "assistant",
                    "content": "planning",
                    "reasoning_content": "hidden plan",
                    "tool_calls": [{
                        "id": "tool_1",
                        "type": "function",
                        "function": { "name": "read_file", "arguments": "{}" }
                    }]
                },
                { "role": "tool", "tool_call_id": "tool_1", "content": "ok" },
                {
                    "role": "assistant",
                    "content": "final visible answer",
                    "tool_calls": [{
                        "id": "tool_2",
                        "type": "function",
                        "function": { "name": "todo_update", "arguments": "{}" }
                    }]
                }
            ]
        });

        let processed = preprocess_client_request_body(
            Bytes::from(body.to_string()),
            &ChatProtocol::OpenAI,
            &deepseek_proxy_model(),
        )
        .expect("preprocess should succeed");

        let processed_json: serde_json::Value =
            serde_json::from_slice(&processed).expect("processed body should be valid json");

        assert_eq!(
            processed_json["messages"][1]["reasoning_content"],
            "hidden plan"
        );
        assert_eq!(processed_json["messages"][3]["reasoning_content"], "");
    }

    #[test]
    fn preprocess_deepseek_promotes_thinking_field_into_reasoning_content() {
        let body = json!({
            "model": "deepseek-v4-flash",
            "thinking": { "type": "enabled" },
            "messages": [
                { "role": "user", "content": "start" },
                {
                    "role": "assistant",
                    "content": "visible answer",
                    "thinking": "hidden chain",
                    "tool_calls": [{
                        "id": "tool_1",
                        "type": "function",
                        "function": { "name": "todo_update", "arguments": "{}" }
                    }]
                }
            ]
        });

        let processed = preprocess_client_request_body(
            Bytes::from(body.to_string()),
            &ChatProtocol::OpenAI,
            &deepseek_proxy_model(),
        )
        .expect("preprocess should succeed");

        let processed_json: serde_json::Value =
            serde_json::from_slice(&processed).expect("processed body should be valid json");

        assert_eq!(
            processed_json["messages"][1]["reasoning_content"],
            "hidden chain"
        );
    }

    #[test]
    fn preprocess_deepseek_skips_reasoning_replay_when_thinking_is_not_enabled() {
        let body = json!({
            "model": "deepseek-v4-flash",
            "messages": [
                { "role": "user", "content": "start" },
                {
                    "role": "assistant",
                    "content": "final visible answer",
                    "tool_calls": [{
                        "id": "tool_2",
                        "type": "function",
                        "function": { "name": "todo_update", "arguments": "{}" }
                    }]
                }
            ]
        });

        let processed = preprocess_client_request_body(
            Bytes::from(body.to_string()),
            &ChatProtocol::OpenAI,
            &deepseek_proxy_model(),
        )
        .expect("preprocess should succeed");

        let processed_json: serde_json::Value =
            serde_json::from_slice(&processed).expect("processed body should be valid json");

        assert!(processed_json["messages"][1]
            .get("reasoning_content")
            .is_none());
    }

    #[test]
    fn preprocess_deepseek_respects_explicit_thinking_disable_over_reasoning_effort() {
        let body = json!({
            "model": "deepseek-v4-flash",
            "thinking": { "type": "disabled" },
            "reasoning_effort": "high",
            "messages": [
                { "role": "user", "content": "start" },
                {
                    "role": "assistant",
                    "content": "final visible answer",
                    "tool_calls": [{
                        "id": "tool_2",
                        "type": "function",
                        "function": { "name": "todo_update", "arguments": "{}" }
                    }]
                }
            ]
        });

        let processed = preprocess_client_request_body(
            Bytes::from(body.to_string()),
            &ChatProtocol::OpenAI,
            &deepseek_proxy_model(),
        )
        .expect("preprocess should succeed");

        let processed_json: serde_json::Value =
            serde_json::from_slice(&processed).expect("processed body should be valid json");

        assert!(processed_json["messages"][1]
            .get("reasoning_content")
            .is_none());
    }
}
