use serde_json::Value;

use super::common::ensure_reasoning_replay;

#[cfg(test)]
pub(super) fn applies_to(model: &str) -> bool {
    let model = model.trim().to_ascii_lowercase();
    model.contains("mimo-v2.5-pro") || model.contains("mimo-v2.5")
}

pub(super) fn normalize_request(body: &mut Value, _model: &str, _base_url: &str) {
    if let Some(body) = body.as_object_mut() {
        body.remove("reasoning_effort");
        body.remove("thinking_budget");
    }
    ensure_reasoning_replay(body);
}
