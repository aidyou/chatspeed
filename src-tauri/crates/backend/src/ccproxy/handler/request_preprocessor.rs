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

/// Keep the experimental Go body field out of other upstreams and avoid
/// promoting a request-local or explicitly overridden header to a cache key.
pub(super) fn inject_opencode_go_prompt_cache_key(
    body: &mut Value,
    upstream_url: &str,
    proxy_model: &ProxyModel,
    client_headers: &http::HeaderMap,
    upstream_headers: &reqwest::header::HeaderMap,
) {
    if !matches!(proxy_model.chat_protocol, ChatProtocol::OpenAI)
        || proxy_model
            .model_metadata
            .as_ref()
            .and_then(|metadata| metadata.get("opencodeGoPromptCacheKey"))
            .and_then(Value::as_bool)
            != Some(true)
        || !reqwest::Url::parse(upstream_url).is_ok_and(|url| {
            url.scheme() == "https"
                && url.host_str() == Some("opencode.ai")
                && url.port().is_none()
                && url.path() == "/inference/go/openai/v1/chat/completions"
        })
    {
        return;
    }

    let Some(session) = client_headers
        .get("x-cs-conversation-id")
        .and_then(|value| value.to_str().ok())
    else {
        return;
    };
    if !upstream_headers
        .get("x-opencode-session")
        .is_some_and(|value| value == session)
        || !uuid::Uuid::parse_str(session).is_ok_and(|id| id.get_version_num() == 5)
        || !crate::ccproxy::auth::is_trusted_internal_request(client_headers)
    {
        return;
    }

    if let Some(body) = body.as_object_mut() {
        body.entry("prompt_cache_key")
            .or_insert_with(|| Value::String(session.to_owned()));
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
    use super::{inject_opencode_go_prompt_cache_key, preprocess_client_request_body};
    use crate::ai::model_catalog::ThinkingAdapter;
    use crate::ccproxy::{types::ProxyModel, ChatProtocol};
    use bytes::Bytes;
    use serde_json::json;

    const GO_CHAT_URL: &str = "https://opencode.ai/inference/go/openai/v1/chat/completions";

    #[test]
    fn opencode_go_prompt_cache_key_reuses_the_trusted_conversation_uuid() {
        use crate::ai::chat::openai::build_prompt_cache_key;
        use crate::ccproxy::helper::ModelResolver;

        let mut model = deepseek_proxy_model();
        model.base_url = "https://opencode.ai/inference/go/openai/v1".into();
        model.model_metadata = Some(json!({"opencodeGoPromptCacheKey": true}));
        let mut client = http::HeaderMap::new();
        client.insert("x-cs-internal-request", "true".parse().unwrap());
        client.insert(
            "authorization",
            format!(
                "Bearer {}",
                crate::constants::INTERNAL_CCPROXY_API_KEY.read()
            )
            .parse()
            .unwrap(),
        );

        let mut keys = Vec::new();
        for conversation in [42, 42, 43] {
            let extra = json!({"conversationId": conversation});
            let key = build_prompt_cache_key(None, extra.as_object(), "request-id").unwrap();
            client.insert("x-cs-conversation-id", key.parse().unwrap());
            let mut upstream = reqwest::header::HeaderMap::new();
            ModelResolver::inject_proxy_headers(&mut upstream, &client, &model, "request-id");
            let mut body = json!({"stream": true});
            inject_opencode_go_prompt_cache_key(&mut body, GO_CHAT_URL, &model, &client, &upstream);
            assert_eq!(upstream["x-opencode-session"], key.as_str());
            assert_eq!(body, json!({"stream": true, "prompt_cache_key": key}));
            keys.push(key);
        }
        assert_eq!(keys[0], keys[1]);
        assert_ne!(keys[1], keys[2]);
    }

    #[test]
    fn opencode_go_prompt_cache_key_preserves_unrelated_requests_and_overrides() {
        let mut model = deepseek_proxy_model();
        let mut client = http::HeaderMap::new();
        let mut upstream = reqwest::header::HeaderMap::new();
        let session = "d856d57d-27d9-52ac-9226-c93192fb3924";
        client.insert("x-cs-conversation-id", session.parse().unwrap());
        client.insert("x-cs-internal-request", "true".parse().unwrap());
        client.insert(
            "authorization",
            format!(
                "Bearer {}",
                crate::constants::INTERNAL_CCPROXY_API_KEY.read()
            )
            .parse()
            .unwrap(),
        );
        upstream.insert("x-opencode-session", session.parse().unwrap());
        let mut body = json!({});

        inject_opencode_go_prompt_cache_key(&mut body, GO_CHAT_URL, &model, &client, &upstream);
        assert_eq!(body, json!({}));
        model.model_metadata = Some(json!({"opencodeGoPromptCacheKey": true}));
        for url in [
            "https://api.deepseek.com/chat/completions",
            "https://opencode.ai.example/inference/go/openai/v1/chat/completions",
            "https://opencode.ai/zen/go/v1/chat/completions",
            "https://opencode.ai/inference/go/openai/v1/responses",
        ] {
            inject_opencode_go_prompt_cache_key(&mut body, url, &model, &client, &upstream);
            assert_eq!(body, json!({}));
        }

        let mut explicit = json!({"prompt_cache_key": "caller-key"});
        inject_opencode_go_prompt_cache_key(&mut explicit, GO_CHAT_URL, &model, &client, &upstream);
        assert_eq!(explicit, json!({"prompt_cache_key": "caller-key"}));

        upstream.insert("x-opencode-session", "override".parse().unwrap());
        inject_opencode_go_prompt_cache_key(&mut body, GO_CHAT_URL, &model, &client, &upstream);
        assert_eq!(body, json!({}));

        upstream.insert("x-opencode-session", session.parse().unwrap());
        client.remove("authorization");
        inject_opencode_go_prompt_cache_key(&mut body, GO_CHAT_URL, &model, &client, &upstream);
        assert_eq!(body, json!({}));
    }

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
