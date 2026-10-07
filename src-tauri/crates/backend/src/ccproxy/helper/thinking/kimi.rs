use serde_json::Value;

#[cfg(test)]
use super::common::host_matches;
use super::common::normalize_effort;

#[cfg(test)]
pub(super) fn applies_to(base_url: &str) -> bool {
    host_matches(base_url, "api.moonshot.cn")
}

pub fn is_k3_model(model: &str) -> bool {
    model.trim().to_ascii_lowercase().contains("kimi-k3")
}

pub fn is_k2_7_code_model(model: &str) -> bool {
    model.trim().to_ascii_lowercase().contains("kimi-k2.7-code")
}

pub(super) fn normalize_request(body: &mut Value, model: &str, _base_url: &str) {
    let model = model.trim().to_ascii_lowercase();

    if is_k3_model(&model) {
        if let Some(body) = body.as_object_mut() {
            body.remove("thinking");
            body.remove("thinking_budget");
        }
        normalize_effort(body, |effort| {
            match effort.trim().to_ascii_lowercase().as_str() {
                "none" | "minimal" | "low" => Some("low"),
                "medium" | "high" => Some("high"),
                "xhigh" | "max" => Some("max"),
                _ => None,
            }
        });
    } else if model.contains("kimi-k2.7-code") {
        if let Some(body) = body.as_object_mut() {
            body.remove("thinking");
            body.remove("thinking_budget");
            body.remove("reasoning_effort");
        }
    } else if model.contains("kimi-k2.5") {
        if let Some(thinking) = body.get_mut("thinking").and_then(Value::as_object_mut) {
            thinking.remove("keep");
        }
        if let Some(body) = body.as_object_mut() {
            body.remove("thinking_budget");
            body.remove("reasoning_effort");
        }
    } else if model.contains("kimi-k2.6") {
        if let Some(body) = body.as_object_mut() {
            body.remove("thinking_budget");
            body.remove("reasoning_effort");
        }
    }
}
