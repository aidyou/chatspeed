use serde_json::Value;

use super::common::{host_matches, normalize_effort};

pub(super) fn applies_to(base_url: &str) -> bool {
    host_matches(base_url, "open.bigmodel.cn")
}

pub(super) fn normalize_request(body: &mut Value, model: &str, _base_url: &str) {
    let model = model.trim().to_ascii_lowercase();
    if !model.contains("glm-5.3") && !model.contains("glm5.3") {
        return;
    }

    if body
        .get("thinking")
        .and_then(|thinking| thinking.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|thinking_type| thinking_type.eq_ignore_ascii_case("disabled"))
    {
        body["thinking"]["type"] = Value::String("enabled".to_string());
    }

    normalize_effort(body, |effort| {
        match effort.trim().to_ascii_lowercase().as_str() {
            "none" | "minimal" | "low" => Some("low"),
            "medium" | "high" => Some("high"),
            "xhigh" | "max" => Some("max"),
            _ => None,
        }
    });
}
