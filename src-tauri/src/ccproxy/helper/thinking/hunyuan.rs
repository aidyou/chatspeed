use serde_json::Value;

use super::common::normalize_effort;

#[cfg(test)]
pub(super) fn applies_to(model: &str) -> bool {
    let model = model.trim().to_ascii_lowercase();
    model.starts_with("hy3") || model.starts_with("hy4") || model.contains("hunyuan")
}

pub(super) fn normalize_request(body: &mut Value, model: &str, _base_url: &str) {
    if !model.trim().to_ascii_lowercase().starts_with("hy4-preview") {
        return;
    }

    normalize_effort(body, |effort| {
        match effort.trim().to_ascii_lowercase().as_str() {
            "none" | "minimal" => Some("none"),
            "low" | "medium" | "high" | "xhigh" | "max" => Some("high"),
            _ => None,
        }
    });
}
