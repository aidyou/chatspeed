use serde_json::Value;

use super::common::host_matches;

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
