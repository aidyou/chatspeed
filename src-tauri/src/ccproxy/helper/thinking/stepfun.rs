use serde_json::Value;

use super::common::{host_matches, normalize_effort};

pub(super) fn applies_to(base_url: &str) -> bool {
    host_matches(base_url, "api.stepfun.ai")
}

pub(super) fn normalize_request(body: &mut Value, _model: &str, _base_url: &str) {
    normalize_effort(body, |effort| {
        match effort.trim().to_ascii_lowercase().as_str() {
            "none" | "minimal" | "low" => Some("low"),
            "medium" => Some("medium"),
            "high" | "xhigh" | "max" => Some("high"),
            _ => None,
        }
    });
}
