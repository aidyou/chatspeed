use serde_json::Value;

pub(super) fn host_matches(base_url: &str, expected_host: &str) -> bool {
    reqwest::Url::parse(base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .is_some_and(|host| host.eq_ignore_ascii_case(expected_host))
}

pub(super) fn normalize_effort(body: &mut Value, map: impl FnOnce(&str) -> Option<&'static str>) {
    let Some(reasoning_effort) = body
        .get("reasoning_effort")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return;
    };

    let Some(normalized_effort) = map(&reasoning_effort) else {
        return;
    };
    body["reasoning_effort"] = Value::String(normalized_effort.to_string());
}

pub(super) fn reasoning_enabled(body: &Value) -> bool {
    if let Some(thinking_enabled) = body
        .get("thinking")
        .and_then(|thinking| thinking.get("type"))
        .and_then(Value::as_str)
        .map(|value| !value.eq_ignore_ascii_case("disabled"))
    {
        return thinking_enabled;
    }

    if let Some(enable_thinking) = body.get("enable_thinking").and_then(Value::as_bool) {
        return enable_thinking;
    }

    body.get("reasoning_effort")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
        || body
            .get("thinking_budget")
            .and_then(Value::as_i64)
            .is_some_and(|value| value > 0)
}

pub(super) fn ensure_reasoning_replay(body: &mut Value) {
    if !reasoning_enabled(body) {
        return;
    }

    let has_tool_round = body
        .get("messages")
        .and_then(Value::as_array)
        .is_some_and(|messages| {
            messages.iter().any(|message| {
                message
                    .get("role")
                    .and_then(Value::as_str)
                    .is_some_and(|role| role == "tool")
                    || message
                        .get("tool_calls")
                        .and_then(Value::as_array)
                        .is_some_and(|tool_calls| !tool_calls.is_empty())
            })
        });

    if !has_tool_round {
        return;
    }

    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };

    for message in messages {
        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }

        let Some(message) = message.as_object_mut() else {
            continue;
        };

        if message.contains_key("reasoning_content") {
            continue;
        }

        let reasoning_content = message
            .get("thinking")
            .cloned()
            .unwrap_or_else(|| Value::String(String::new()));
        message.insert("reasoning_content".to_string(), reasoning_content);
    }
}

pub(super) fn normalize_request(_body: &mut Value, _model: &str, _base_url: &str) {}
