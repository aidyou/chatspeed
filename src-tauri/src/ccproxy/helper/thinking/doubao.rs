use serde_json::Value;

pub(super) fn applies_to(model: &str) -> bool {
    let model = model.trim().to_ascii_lowercase();
    model.contains("doubao-seed-1.6") || model.contains("doubao-1.5-thinking-vision-pro")
}

pub(super) fn normalize_request(body: &mut Value, _model: &str, _base_url: &str) {
    if let Some(body) = body.as_object_mut() {
        body.remove("reasoning_effort");
        body.remove("thinking_budget");
    }
}
