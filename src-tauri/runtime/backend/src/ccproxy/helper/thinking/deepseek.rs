use serde_json::Value;

#[cfg(test)]
use super::common::host_matches;
use super::common::{ensure_reasoning_replay, normalize_effort};

#[cfg(test)]
pub(super) fn applies_to(base_url: &str) -> bool {
    host_matches(base_url, "api.deepseek.com") || host_matches(base_url, "api.deepseek.cn")
}

pub(super) fn normalize_request(body: &mut Value, _model: &str, _base_url: &str) {
    normalize_effort(body, |effort| {
        match effort.trim().to_ascii_lowercase().as_str() {
            "low" => Some("low"),
            "medium" | "high" | "xhigh" => Some("high"),
            "max" => Some("max"),
            _ => None,
        }
    });
    ensure_reasoning_replay(body);
}
