use serde_json::Value;

use super::common::normalize_effort;

pub(super) fn applies_to(model: &str) -> bool {
    model.trim().to_ascii_lowercase().starts_with("sensenova-")
}

pub(super) fn normalize_request(body: &mut Value, _model: &str, _base_url: &str) {
    normalize_effort(body, |effort| {
        effort
            .trim()
            .eq_ignore_ascii_case("xhigh")
            .then_some("high")
    });
}
