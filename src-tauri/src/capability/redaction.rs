//! Redaction filter for capability payloads.
//!
//! This is a defense in depth layer, not the only one: descriptors reject
//! unknown fields, read DTOs carry booleans instead of secret values, and the
//! journal is written through [`redact_json`] so a caller that forgets cannot
//! persist a secret. `redact_text` is applied to every error message that
//! crosses a log line, DTO or journal boundary (AC-13/INV-6).

use serde_json::{Map, Value};
use std::sync::OnceLock;

/// The replacement marker for any redacted value.
pub const REDACTED: &str = "[redacted]";

/// Key segments that mark a value as secret-bearing. Keys are split on
/// `_`, `-`, `.` and `:` so `bearer_token` matches while `author` does not,
/// and the separator-stripped form is matched too so `api_key` matches
/// `apikey`.
const SECRET_SEGMENTS: &[&str] = &[
    "token",
    "tokens",
    "secret",
    "secrets",
    "password",
    "passwd",
    "pwd",
    "passphrase",
    "credential",
    "credentials",
    "authorization",
    "bearer",
    "cookie",
    "cookies",
    "signature",
    "apikey",
    "privatekey",
    "accesskey",
    "clientsecret",
    "sessionkey",
];

/// Whole key names (lowercased, separators removed) that are secret-bearing.
const SECRET_JOINED_KEYS: &[&str] = &[
    "apikey",
    "privatekey",
    "accesskey",
    "secretkey",
    "clientsecret",
    "signingkey",
    "encryptionkey",
    "authorization",
    "bearer",
    "password",
    "passwd",
];

/// Container keys whose *values* are secret by definition: an MCP `env` block
/// or an HTTP `headers` map is value-bearing even when a single variable name
/// looks innocuous.
const OPAQUE_SECRET_CONTAINERS: &[&str] = &["env", "environment", "headers", "header", "secrets"];

/// Whether a JSON key names a secret-bearing value.
pub fn is_secret_key(key: &str) -> bool {
    let lowered = key.to_ascii_lowercase();
    let joined: String = lowered
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect();
    if SECRET_JOINED_KEYS.contains(&joined.as_str()) {
        return true;
    }
    let segments = lowered
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|segment| !segment.is_empty());
    segments
        .into_iter()
        .any(|segment| SECRET_SEGMENTS.contains(&segment))
}

fn is_opaque_secret_container(key: &str) -> bool {
    OPAQUE_SECRET_CONTAINERS.contains(&key.to_ascii_lowercase().as_str())
}

/// Deeply redacts every secret-bearing leaf of a JSON value.
///
/// Object keys are matched by [`is_secret_key`]; the contents of an `env` or
/// `headers` map are redacted regardless of the individual variable names, and
/// every string leaf additionally passes through [`redact_text`] so an
/// embedded URL credential cannot survive.
///
/// Only values that could *hold* a secret are replaced. A boolean or numeric
/// leaf keeps its value and its type even when its key looks secret-ish, so a
/// structural flag such as `reads_credentials` is not turned into a string —
/// that would corrupt the redacted projection and make a journal replay
/// disagree with the original result.
pub fn redact_json(value: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(redact_text(text)),
        Value::Array(items) => Value::Array(items.iter().map(redact_json).collect()),
        Value::Object(map) => {
            let mut redacted = Map::with_capacity(map.len());
            for (key, item) in map {
                if is_secret_key(key) && can_hold_secret(item) {
                    redacted.insert(key.clone(), Value::String(REDACTED.to_string()));
                } else if is_opaque_secret_container(key) {
                    redacted.insert(key.clone(), redact_container(item));
                } else {
                    redacted.insert(key.clone(), redact_json(item));
                }
            }
            Value::Object(redacted)
        }
        other => other.clone(),
    }
}

/// Whether a value is the kind of thing a secret can be stored in.
fn can_hold_secret(value: &Value) -> bool {
    matches!(
        value,
        Value::String(_) | Value::Array(_) | Value::Object(_)
    )
}

/// Redacts every value of an `env`/`headers`-style container while keeping its
/// keys, so the shape stays diagnosable but no value survives.
fn redact_container(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut redacted = Map::with_capacity(map.len());
            for (key, _value) in map {
                redacted.insert(key.clone(), Value::String(REDACTED.to_string()));
            }
            Value::Object(redacted)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| match item {
                    // A `[name, value]` pair keeps only the variable name.
                    Value::Array(pair) if pair.len() == 2 => {
                        let mut kept = Vec::with_capacity(2);
                        kept.push(pair[0].clone());
                        kept.push(Value::String(REDACTED.to_string()));
                        Value::Array(kept)
                    }
                    _ => Value::String(REDACTED.to_string()),
                })
                .collect(),
        ),
        _ => Value::String(REDACTED.to_string()),
    }
}

fn url_credential_pattern() -> &'static regex::Regex {
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        regex::Regex::new(r"(?i)\b([a-z][a-z0-9+.\-]*://)[^/@\s]+@")
            .expect("url credential pattern must compile")
    })
}

fn inline_secret_pattern() -> &'static regex::Regex {
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        regex::Regex::new(
            r#"(?i)\b(token|secret|password|passwd|api[_-]?key|apikey|authorization|bearer|client[_-]?secret)\b\s*[:=]\s*[^\s,;"']+"#,
        )
        .expect("inline secret pattern must compile")
    })
}

/// Redacts obvious credentials embedded in free text: URL userinfo and
/// `token=...`-style assignments. Used for every error message and log-bound
/// string so an upstream interpolation cannot leak.
pub fn redact_text(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let with_urls = url_credential_pattern().replace_all(text, "${1}[redacted]@");
    inline_secret_pattern()
        .replace_all(&with_urls, "$1=[redacted]")
        .into_owned()
}

/// A bounded, redacted JSON projection for persistence or display.
///
/// Values larger than `max_chars` when serialized are replaced by a truncation
/// marker so a journal row can never grow without bound.
pub fn bounded_redacted_json(value: &Value, max_chars: usize) -> Value {
    let redacted = redact_json(value);
    let serialized = redacted.to_string();
    if serialized.chars().count() <= max_chars {
        redacted
    } else {
        Value::String(format!("[truncated {} chars]", serialized.chars().count()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn secret_keys_are_matched_by_segment_not_substring() {
        assert!(is_secret_key("bearer_token"));
        assert!(is_secret_key("API_KEY"));
        assert!(is_secret_key("client-secret"));
        assert!(is_secret_key("authorization"));
        assert!(!is_secret_key("author"));
        assert!(!is_secret_key("monkey"));
        assert!(!is_secret_key("description"));
        assert!(!is_secret_key("resource_key"));
    }

    #[test]
    fn a_non_secret_leaf_keeps_its_value_and_type_even_under_a_secret_like_key() {
        let value = json!({
            "permissions": { "reads_credentials": false, "token_count": 0, "token": "canary" },
        });

        let redacted = redact_json(&value);
        // A boolean or number is a structural flag, not a secret: redacting it
        // would change the type and break a journal replay.
        assert_eq!(redacted["permissions"]["reads_credentials"], json!(false));
        assert_eq!(redacted["permissions"]["token_count"], json!(0));
        // A string under the same key is still redacted.
        assert_eq!(redacted["permissions"]["token"], json!(REDACTED));
        assert!(!redacted.to_string().contains("canary"));
    }

    #[test]
    fn redact_json_replaces_secret_leaves_and_keeps_shape() {
        let value = json!({
            "name": "weather",
            "bearer_token": "canary-secret-value",
            "nested": { "api_key": "canary-key", "target": "chatspeed" },
            "env": { "PUBLIC": "1", "WEIRD": "canary-env" },
            "servers": [{ "token": "canary-in-array" }],
        });

        let redacted = redact_json(&value);
        let serialized = redacted.to_string();
        assert!(!serialized.contains("canary"), "got {serialized}");
        assert_eq!(redacted["name"], json!("weather"));
        assert_eq!(redacted["bearer_token"], json!(REDACTED));
        assert_eq!(redacted["nested"]["target"], json!("chatspeed"));
        assert_eq!(redacted["env"]["PUBLIC"], json!(REDACTED));
        assert_eq!(redacted["servers"][0]["token"], json!(REDACTED));
    }

    #[test]
    fn redact_text_strips_url_userinfo_and_inline_assignments() {
        let text = "connect https://alice:canary-pass@example.test/mcp failed (token=canary-token)";
        let redacted = redact_text(text);
        assert!(!redacted.contains("canary"), "got {redacted}");
        assert!(redacted.contains("example.test"));
    }

    #[test]
    fn bounded_redacted_json_truncates_oversized_payloads() {
        let value = json!({ "note": "x".repeat(64) });
        let bounded = bounded_redacted_json(&value, 16);
        assert!(bounded.is_string(), "expected truncation marker");
    }
}
