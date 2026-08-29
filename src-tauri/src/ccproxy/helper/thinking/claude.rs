use serde_json::Value;

#[cfg(test)]
use super::common::host_matches;

#[cfg(test)]
pub(super) fn applies_to(base_url: &str) -> bool {
    host_matches(base_url, "api.anthropic.com") || host_matches(base_url, "api.claude.com")
}

pub(super) fn normalize_request(body: &mut Value, model: &str, _base_url: &str) {
    let model = model.trim().to_ascii_lowercase();
    if !model.contains("claude-opus-5") {
        return;
    }

    let thinking_is_disabled = body
        .get("thinking")
        .and_then(|thinking| thinking.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|thinking_type| thinking_type.eq_ignore_ascii_case("disabled"));
    let effort_requires_thinking =
        body.get("effort")
            .and_then(Value::as_str)
            .is_some_and(|effort| {
                matches!(effort.trim().to_ascii_lowercase().as_str(), "xhigh" | "max")
            });

    if thinking_is_disabled && effort_requires_thinking {
        body["thinking"]["type"] = Value::String("adaptive".to_string());
    }
}
