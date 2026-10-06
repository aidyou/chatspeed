use serde_json::Value;

#[cfg(test)]
use super::common::host_matches;

#[cfg(test)]
pub(super) fn applies_to(base_url: &str) -> bool {
    host_matches(base_url, "api.openai.com")
}

pub(super) fn normalize_request(_body: &mut Value, _model: &str, _base_url: &str) {
    // OpenAI reasoning_effort is forwarded unchanged because supported values are model-specific.
}
