use serde_json::Value;

#[cfg(test)]
use super::common::host_matches;

#[cfg(test)]
pub(super) fn applies_to(base_url: &str) -> bool {
    host_matches(base_url, "dashscope.aliyuncs.com")
}

fn is_thinking_only_model(model: &str) -> bool {
    model.contains("qwq")
        || model.contains("-thinking")
        || model.contains("qwen3.7-max-preview")
        || model.contains("qwen3.7-max-2026-05-17")
        || model.contains("kimi-k3")
        || model.contains("kimi-k2.7-code")
}

pub(super) fn normalize_request(body: &mut Value, model: &str, _base_url: &str) {
    let model = model.trim().to_ascii_lowercase();
    if !model.contains("qwen") && !model.contains("qwq") && !model.contains("kimi-") {
        return;
    }

    let thinking_enabled = body
        .get("thinking")
        .and_then(|thinking| thinking.get("type"))
        .and_then(Value::as_str)
        .map(|thinking_type| !thinking_type.eq_ignore_ascii_case("disabled"));

    if let Some(body) = body.as_object_mut() {
        body.remove("thinking");
        body.remove("reasoning_effort");
        if !body.contains_key("enable_thinking") {
            if let Some(enabled) = thinking_enabled {
                body.insert("enable_thinking".to_string(), Value::Bool(enabled));
            }
        }
        if model.contains("kimi-k3") {
            body.remove("thinking_budget");
        }
    }

    if is_thinking_only_model(&model)
        && body.get("enable_thinking").and_then(Value::as_bool) == Some(false)
    {
        body["enable_thinking"] = Value::Bool(true);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::normalize_request_with_adapter;
    use crate::model_catalog_engine::resolve_transport;

    #[test]
    fn opencode_qwen_3_8_flash_keeps_budget_without_conflicting_effort() {
        let original = json!({
            "reasoning_effort": "high",
            "thinking_budget": 2048,
            "enable_thinking": true
        });

        for model in ["qwen3.8-flash", "qwen-3.8-flash"] {
            let url = "https://opencode.ai/zen/go/v1";
            let (adapter, transport) = resolve_transport(model, Some(url), Some("openai"), None)
                .expect("transport catalog")
                .expect("OpenCode Qwen transport");
            assert_eq!(transport, "qwen-opencode-3-8-flash");

            let mut body = original.clone();
            normalize_request_with_adapter(&mut body, model, url, Some(adapter));
            assert_eq!(
                body,
                json!({ "thinking_budget": 2048, "enable_thinking": true })
            );
        }

        for (model, url) in [
            ("qwen3.8-flash", "https://openrouter.ai/api/v1"),
            ("qwen3.8-max", "https://opencode.ai/zen/go/v1"),
        ] {
            assert!(resolve_transport(model, Some(url), Some("openai"), None)
                .expect("transport catalog")
                .is_none());
        }
    }
}
