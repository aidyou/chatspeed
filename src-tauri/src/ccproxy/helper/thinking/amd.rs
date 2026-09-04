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
///
/// Only the two models documented for this endpoint are normalized. Other AMD
/// models are passed through unchanged until their behavior is verified.
fn is_deepseek_v4(model: &str) -> bool {
    model.trim().eq_ignore_ascii_case("deepseek-v4-flash")
}

fn is_qwen38_flash(model: &str) -> bool {
    model
        .trim()
        .eq_ignore_ascii_case("qwen3.8-flash-next")
}

fn is_supported_model(model: &str) -> bool {
    is_deepseek_v4(model) || is_qwen38_flash(model)
}

fn supported_efforts(model: &str) -> &'static [&'static str] {
    if is_deepseek_v4(model) {
        &["low", "high"]
    } else {
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
    if !is_supported_model(model) {
        return;
    }

    let Some(object) = body.as_object_mut() else {
        return;
    };

    // AMD rejects these parameters outright; strip them before forwarding.
    let thinking = object.remove("thinking");
    let enable_thinking = object.remove("enable_thinking");
    let requested_budget = object
        .remove("thinking_budget")
        .and_then(|value| value.as_i64());

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
        // AMD interprets an omitted effort as no thinking for DeepSeek. Qwen
        // cannot disable thinking. Omitting the field on Qwen selects its
        // internal default `xhigh` tier, so use the lowest verified effort as
        // the closest available approximation to disabled thinking.
        if is_deepseek_v4(model) {
            object.remove("reasoning_effort");
        } else if let Some(lowest) = supported.first() {
            object.insert(
                "reasoning_effort".to_string(),
                Value::String((*lowest).to_string()),
            );
        }
        return;
    }

    match requested_effort.as_deref() {
        Some(effort) => {
            if let Some(effort) = normalize_effort_level(effort, supported) {
                object.insert(
                    "reasoning_effort".to_string(),
                    Value::String(effort.to_string()),
                );
            } else {
                // Do not forward invalid values to a known AMD model. DeepSeek
                // treats the omission as thinking disabled; Qwen cannot disable
                // thinking, so use its lowest verified effort instead of
                // triggering its xhigh default.
                let fallback = if is_deepseek_v4(model) {
                    None
                } else {
                    supported.first().copied()
                };
                if let Some(effort) = fallback {
                    object.insert(
                        "reasoning_effort".to_string(),
                        Value::String(effort.to_string()),
                    );
                } else {
                    object.remove("reasoning_effort");
                }
            }
        }
        // Qwen uses the provider default (`xhigh`) when thinking is enabled
        // without an explicit effort; preserve that default by omitting the
        // field. DeepSeek is different: it needs an explicit effort to think.
        None if is_deepseek_v4(model) => {
            object.insert(
                "reasoning_effort".to_string(),
                Value::String("high".to_string()),
            );
        }
        None => {}
    }
}
