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

pub fn preprocess_client_request_body(
    client_request_body: Bytes,
    chat_protocol: &ChatProtocol,
    proxy_model: &ProxyModel,
) -> Result<Bytes, CCProxyError> {
    let mut body_json: Value = serde_json::from_slice(&client_request_body).map_err(|e| {
        CCProxyError::InternalError(format!("Failed to deserialize request body: {}", e))
    })?;

    crate::ccproxy::helper::thinking::normalize_request_with_adapter(
        &mut body_json,
        &proxy_model.model,
        &proxy_model.base_url,
        proxy_model.thinking_adapter,
    );

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
    use crate::ai::model_catalog::ThinkingAdapter;
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
            key_index: None,
            model_metadata: None,
            custom_params: None,
            pricing: None,
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
            thinking_adapter: Some(ThinkingAdapter::DeepSeek),
            matched_transport_id: None,
        }
    }

    #[test]
    fn preprocess_mistral_normalizes_effort_without_rewriting_thinking_chunks() {
        let body = json!({
            "model": "mistral-small-latest",
            "reasoning_effort": "xhigh",
            "messages": [{
                "role": "assistant",
                "content": [{
                    "type": "thinking",
                    "thinking": [{ "type": "text", "text": "raw trace" }]
                }]
            }]
        });
        let mut proxy_model = deepseek_proxy_model();
        proxy_model.provider = "Mistral".to_string();
        proxy_model.base_url = "https://api.mistral.ai/v1".to_string();
        proxy_model.model = "mistral-small-latest".to_string();
        proxy_model.thinking_adapter = Some(ThinkingAdapter::Mistral);

        let processed = preprocess_client_request_body(
            Bytes::from(body.to_string()),
            &ChatProtocol::OpenAI,
            &proxy_model,
        )
        .expect("preprocess should succeed");
        let processed_json: serde_json::Value =
            serde_json::from_slice(&processed).expect("processed body should be valid json");

        assert_eq!(processed_json["reasoning_effort"], "high");
        assert_eq!(
            processed_json["messages"][0]["content"],
            body["messages"][0]["content"]
        );
    }

    #[test]
    fn preprocess_deepseek_maps_xhigh_reasoning_effort_to_high() {
        let body = json!({
            "model": "deepseek-v4-flash",
            "thinking": { "type": "enabled" },
            "reasoning_effort": "xhigh",
            "messages": [{ "role": "user", "content": "start" }]
        });

        let processed = preprocess_client_request_body(
            Bytes::from(body.to_string()),
            &ChatProtocol::OpenAI,
            &deepseek_proxy_model(),
        )
        .expect("preprocess should succeed");
        let processed_json: serde_json::Value =
            serde_json::from_slice(&processed).expect("processed body should be valid json");

        assert_eq!(processed_json["reasoning_effort"], "high");
    }

    #[test]
    fn preprocess_sensenova_maps_xhigh_reasoning_effort_to_high() {
        let body = json!({
            "model": "sensenova-6.8-flash-lite",
            "reasoning_effort": "xhigh",
            "messages": [{ "role": "user", "content": "start" }]
        });
        let mut proxy_model = deepseek_proxy_model();
        proxy_model.model = "sensenova-6.8-flash-lite".to_string();
        proxy_model.thinking_adapter = Some(ThinkingAdapter::SenseNova);

        let processed = preprocess_client_request_body(
            Bytes::from(body.to_string()),
            &ChatProtocol::OpenAI,
            &proxy_model,
        )
        .expect("preprocess should succeed");
        let processed_json: serde_json::Value =
            serde_json::from_slice(&processed).expect("processed body should be valid json");

        assert_eq!(processed_json["reasoning_effort"], "high");
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
