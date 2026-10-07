use serde_json::Value;

#[cfg(test)]
pub(super) fn applies_to(model: &str) -> bool {
    let model = model.trim().to_ascii_lowercase();
    model.contains("mistral-small-latest") || model.contains("mistral-medium-3-5")
}

pub(super) fn normalize_request(body: &mut Value, _model: &str, _base_url: &str) {
    let Some(reasoning_effort) = body
        .get("reasoning_effort")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return;
    };

    let normalized_effort = match reasoning_effort.trim().to_ascii_lowercase().as_str() {
        "none" | "minimal" => "none",
        "low" | "medium" | "high" | "xhigh" | "max" => "high",
        _ => return,
    };
    body["reasoning_effort"] = Value::String(normalized_effort.to_string());
}
