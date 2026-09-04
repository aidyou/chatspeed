use serde_json::Value;

#[cfg(test)]
use super::common::host_matches;

#[cfg(test)]
pub(super) fn applies_to(base_url: &str) -> bool {
    host_matches(base_url, "developer.amd.com.cn")
}

/// AMD Radeon developer endpoint thinking behavior (verified against the vendor
/// docs on 2026-09-04):
/// - `thinking`, `enable_thinking`, and `thinking_budget` are unsupported on
///   `/v1/chat/completions` and rejected with 400 `unsupported_parameter`;
///   thinking is controlled through `reasoning_effort` only.
/// - DeepSeek-V4-Flash accepts every effort value but behaves as two tiers
///   (low-ish and high-ish); omitting `reasoning_effort` disables thinking.
/// - Qwen3.8-Flash only accepts `low` and `medium` end to end (`high` passes
///   schema validation but is rejected by the model; `xhigh`/`max`/`minimal`
///   fail deserialization). Thinking cannot be disabled: omitting the parameter
///   falls back to the model default, which is the longest tier.
fn is_deepseek_v4(model: &str) -> bool {
    model.trim().to_ascii_lowercase().contains("deepseek-v4")
}

fn supported_efforts(model: &str) -> &'static [&'static str] {
    if is_deepseek_v4(model) {
        &["low", "high"]
    } else {
        // Qwen3.8-Flash and unknown AMD-hosted models keep the conservative
        // intersection of values that pass the endpoint schema and the model.
        &["low", "medium"]
    }
}

fn normalize_effort_level(effort: &str, supported: &[&'static str]) -> Option<&'static str> {
    let effort = effort.trim().to_ascii_lowercase();
    let preferred: &[&str] = match effort.as_str() {
        "none" | "minimal" => &["minimal", "low", "medium", "high"],
        "low" => &["low", "medium", "high"],
        "medium" => &["medium", "low", "high"],
        "high" | "xhigh" | "max" => &["high", "medium", "low"],
        _ => return None,
    };
    preferred
        .iter()
        .copied()
        .find(|candidate| supported.contains(candidate))
}

pub(super) fn normalize_request(body: &mut Value, model: &str, _base_url: &str) {
    let Some(object) = body.as_object_mut() else {
        return;
    };

    // AMD rejects these parameters outright; strip them before forwarding.
    let thinking = object.remove("thinking");
    let enable_thinking = object.remove("enable_thinking");
    let requested_budget = object.remove("thinking_budget").and_then(|value| value.as_i64());

    let explicit_enabled = thinking
        .as_ref()
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str)
        .map(|value| !value.eq_ignore_ascii_case("disabled"))
        .or_else(|| enable_thinking.as_ref().and_then(Value::as_bool));
    let requested_effort = object
        .get("reasoning_effort")
        .and_then(Value::as_str)
        .map(str::trim)
        .map(str::to_owned);

    let thinking_requested = match explicit_enabled {
        Some(enabled) => enabled,
        None => match requested_effort.as_deref() {
            Some(effort) => !effort.eq_ignore_ascii_case("none"),
            None => requested_budget.is_some_and(|budget| budget > 0),
        },
    };

    let supported = supported_efforts(model);
    if !thinking_requested {
        if is_deepseek_v4(model) {
            // Omitting reasoning_effort disables thinking on this model.
            object.remove("reasoning_effort");
        } else if let Some(lowest) = supported.first() {
            // Thinking cannot be disabled on Qwen3.8-Flash (and unknown models);
            // the lowest supported tier is the closest approximation.
            object.insert(
                "reasoning_effort".to_string(),
                Value::String((*lowest).to_string()),
            );
        }
        return;
    }

    let mapped = match requested_effort.as_deref() {
        Some(effort) => normalize_effort_level(effort, supported),
        // DeepSeek-V4-Flash needs an explicit effort to think at all; its
        // documented default tier is `high`. Qwen3.8-Flash already thinks by
        // default, so keep the provider default by omitting the parameter.
        None if is_deepseek_v4(model) => Some("high"),
        None => None,
    };
    if let Some(effort) = mapped {
        object.insert(
            "reasoning_effort".to_string(),
            Value::String(effort.to_string()),
        );
    }
}
