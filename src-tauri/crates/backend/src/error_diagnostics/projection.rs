//! Safe user-message projection from known response shapes.

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn known_json_projection_is_whitelisted_not_deepest_message_search() {
        for value in [
            json!({"error":{"message":"overloaded: https://a.test/x\nretry"}}),
            json!({"errors":{"message":"overloaded: https://a.test/x\nretry"}}),
            json!({"details":"overloaded: https://a.test/x\nretry"}),
        ] {
            assert_eq!(
                project_user_message(&value, "fallback"),
                "overloaded: https://a.test/x\nretry"
            );
            assert_eq!(
                sanitize_user_message(&value.to_string()),
                "overloaded: https://a.test/x\nretry"
            );
        }
        assert_eq!(
            project_user_message(&json!({"payload":{"message":"hidden"}}), "fallback"),
            "fallback"
        );
        assert_eq!(
            project_user_message(&json!({"error.message":"not a path"}), "fallback"),
            "fallback"
        );
    }

    #[test]
    fn user_projection_is_unicode_bounded_and_html_or_empty_has_localized_fallback() {
        assert_eq!(
            sanitize_user_message(&"𠀀".repeat(4097)),
            format!("{}…", "𠀀".repeat(4096))
        );
        assert_eq!(
            sanitize_user_message("plain &amp; text: https://a.test:123/a\nnext"),
            "plain &amp; text: https://a.test:123/a\nnext"
        );
        assert_eq!(
            sanitize_user_message(" "),
            rust_i18n::t!("chat.error_feedback")
        );
        assert_eq!(
            sanitize_user_message("<html>do not display the error page</html>"),
            rust_i18n::t!("chat.error_feedback")
        );
    }
}
use serde_json::Value;

use crate::capability::redaction::redact_text;

const MAX_USER: usize = 4096;

fn clamp(value: &str) -> String {
    let mut chars = value.chars();
    let output: String = chars.by_ref().take(MAX_USER).collect();
    if chars.next().is_some() {
        format!("{output}…")
    } else {
        output
    }
}

pub fn sanitize_user_message(value: &str) -> String {
    let selected = serde_json::from_str::<Value>(value)
        .ok()
        .and_then(|parsed| project_json_message(&parsed));
    let candidate = selected.as_deref().unwrap_or(value);
    let trimmed = candidate.trim_start().to_ascii_lowercase();
    if candidate.trim().is_empty()
        || [
            "<!doctype html",
            "<html",
            "<body",
            "<head",
            "<div",
            "<script",
        ]
        .iter()
        .any(|tag| trimmed.starts_with(tag))
    {
        return rust_i18n::t!("chat.error_feedback").to_string();
    }
    clamp(&redact_text(candidate))
}

pub fn project_user_message(value: &Value, fallback: impl Into<String>) -> String {
    let fallback = fallback.into();
    sanitize_user_message(project_json_message(value).as_deref().unwrap_or(&fallback))
}

pub fn project_json_message(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    for key in ["error", "errors"] {
        if let Some(Value::Object(error)) = object.get(key) {
            if let Some(Value::String(text)) = error.get("message") {
                if !text.trim().is_empty() {
                    return Some(text.clone());
                }
            }
        }
    }
    for key in ["message", "details"] {
        if let Some(Value::String(text)) = object.get(key) {
            if !text.trim().is_empty() {
                return Some(text.clone());
            }
        }
    }
    None
}
