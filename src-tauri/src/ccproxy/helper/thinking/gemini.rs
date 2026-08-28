use serde_json::{Map, Value};

use super::common::host_matches;

pub(super) fn applies_to(base_url: &str) -> bool {
    host_matches(base_url, "generativelanguage.googleapis.com")
}

fn supported_thinking_levels(model: &str) -> Option<&'static [&'static str]> {
    let model = model.trim().to_ascii_lowercase();
    if model.contains("gemini-3.7-flash")
        || model.contains("gemini-3.1-pro-preview")
        || model.contains("gemini-2.5-pro")
        || model.contains("gemini-2.5-flash")
        || model.contains("gemini-2.5-flash-lite")
    {
        Some(&["low", "medium", "high"])
    } else if model.contains("gemini-3.6-flash")
        || model.contains("gemini-3.5-flash-lite")
        || model.contains("gemini-3-flash-preview")
        || model.contains("gemini-3.5-flash")
    {
        Some(&["minimal", "low", "medium", "high"])
    } else if model.contains("gemini-3.1-flash-lite-image") {
        Some(&["minimal", "high"])
    } else if model.contains("gemini-3-pro-preview") {
        Some(&["low", "high"])
    } else {
        None
    }
}

fn normalize_thinking_level(level: &str, supported: &[&'static str]) -> Option<&'static str> {
    let level = level.trim().to_ascii_lowercase();
    if supported.contains(&level.as_str()) {
        return supported
            .iter()
            .find(|candidate| **candidate == level)
            .copied();
    }

    let preferred = match level.as_str() {
        "none" | "minimal" => ["minimal", "low", "medium", "high"],
        "low" => ["low", "minimal", "medium", "high"],
        "medium" => ["medium", "high", "low", "minimal"],
        "high" | "xhigh" | "max" => ["high", "medium", "low", "minimal"],
        _ => return None,
    };
    preferred
        .iter()
        .find(|level| supported.contains(level))
        .copied()
}

pub(super) fn normalize_request(body: &mut Value, model: &str, _base_url: &str) {
    let Some(supported) = supported_thinking_levels(model) else {
        return;
    };

    let Some(generation_config) = body
        .as_object_mut()
        .and_then(|body| body.get_mut("generationConfig"))
        .and_then(Value::as_object_mut)
    else {
        return;
    };

    let thinking_config = generation_config
        .entry("thinkingConfig".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(thinking_config) = thinking_config.as_object_mut() else {
        return;
    };

    let level = thinking_config
        .get("thinkingLevel")
        .and_then(Value::as_str)
        .or_else(|| {
            thinking_config
                .get("thinking_level")
                .and_then(Value::as_str)
        });
    let Some(level) = level else {
        return;
    };
    let Some(level) = normalize_thinking_level(level, supported) else {
        return;
    };

    thinking_config.remove("thinking_level");
    thinking_config.insert(
        "thinkingLevel".to_string(),
        Value::String(level.to_string()),
    );
}
