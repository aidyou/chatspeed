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
    let mut budget = 16_384;
    redact_json_bounded(value, 0, &mut budget)
}

fn redact_json_bounded(value: &Value, depth: usize, budget: &mut usize) -> Value {
    if depth >= 32 || *budget == 0 {
        return Value::String(REDACTED.to_string());
    }
    *budget -= 1;
    match value {
        Value::String(text) => Value::String(redact_text_bounded(text, depth + 1)),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| redact_json_bounded(item, depth + 1, budget))
                .collect(),
        ),
        Value::Object(map) => {
            let mut redacted = Map::with_capacity(map.len());
            for (key, item) in map {
                if is_secret_key(key) && can_hold_secret(item) {
                    redacted.insert(key.clone(), Value::String(REDACTED.to_string()));
                } else if is_opaque_secret_container(key) {
                    redacted.insert(key.clone(), redact_container(item));
                } else {
                    redacted.insert(key.clone(), redact_json_bounded(item, depth + 1, budget));
                }
            }
            Value::Object(redacted)
        }
        other => other.clone(),
    }
}

/// Whether a value is the kind of thing a secret can be stored in.
fn can_hold_secret(value: &Value) -> bool {
    matches!(value, Value::String(_) | Value::Array(_) | Value::Object(_))
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

fn authorization_pattern() -> &'static regex::Regex {
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    PATTERN.get_or_init(|| regex::Regex::new(r#"(?i)\b(?:authorization|proxy-authorization|cookie|set-cookie)\s*[:=]\s*[^\r\n]+|\bBearer\s+[^\s\"'<>;,}]+"#).expect("authorization pattern must compile"))
}

fn quoted_secret_pattern() -> &'static regex::Regex {
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    PATTERN.get_or_init(|| regex::Regex::new(r#"(?i)([\"'](?:api[_-]?key|token|secret|password|authorization)[\"']\s*:\s*)[\"'][^\"']*[\"']"#).expect("quoted secret pattern must compile"))
}

fn query_secret_pattern() -> &'static regex::Regex {
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        regex::Regex::new(
            r"(?i)([?&](?:key|token|access[_-]?token|refresh[_-]?token|secret|password|signature|credential|api[_-]?key|apikey|authorization|bearer)=)[^&#\s]*",
        )
        .expect("query secret pattern must compile")
    })
}

/// Redacts obvious credentials embedded in free text: URL userinfo, query
/// parameters and `token=...`-style assignments.
pub fn redact_text(text: &str) -> String {
    redact_text_bounded(text, 0)
}

fn redact_text_bounded(text: &str, depth: usize) -> String {
    // Fail closed rather than parsing or copying an unbounded diagnostic payload.
    if depth >= 32 || text.len() > 262_144 {
        return REDACTED.to_string();
    }
    if text.is_empty() {
        return String::new();
    }
    // Escaped JSON also occurs without an enclosing JSON string in upstream errors.
    if text.contains("\\\"") {
        let decoded = text.replace("\\\"", "\"");
        let redacted = redact_text_bounded(&decoded, depth + 1);
        if redacted != decoded {
            return redacted;
        }
    }
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    while cursor < text.len() {
        let start = cursor;
        let byte = text.as_bytes()[cursor];
        if byte == b'"' || byte == b'\'' || byte.is_ascii_alphabetic() || byte == b'_' {
            let quoted = byte == b'"' || byte == b'\'';
            let end = if quoted {
                text_value_end(text, cursor)
            } else {
                let mut end = cursor + 1;
                while end < text.len()
                    && (text.as_bytes()[end].is_ascii_alphanumeric()
                        || b"_-.:".contains(&text.as_bytes()[end]))
                {
                    // A colon separates a key from its value, not a key segment here.
                    if text.as_bytes()[end] == b':' {
                        break;
                    }
                    end += 1;
                }
                end
            };
            let key = if quoted && end > cursor + 1 && text.as_bytes().get(end - 1) == Some(&byte) {
                &text[cursor + 1..end - 1]
            } else {
                &text[cursor..end]
            };
            let decoded_key = if quoted && byte == b'"' {
                serde_json::from_str::<String>(&text[cursor..end]).ok()
            } else {
                None
            };
            let key = decoded_key.as_deref().unwrap_or(key);
            let separator = skip_space(text, end);
            if separator < text.len()
                && b":=".contains(&text.as_bytes()[separator])
                && (is_secret_key(key) || is_opaque_secret_container(key))
            {
                let value_start = skip_space(text, separator + 1);
                let value_end = text_value_end(text, value_start);
                output.push_str(&text[start..value_start]);
                output.push_str("\"[redacted]\"");
                cursor = value_end;
                continue;
            }
            if quoted && byte == b'"' {
                if let Ok(decoded) = serde_json::from_str::<String>(&text[start..end]) {
                    let redacted = redact_text_bounded(&decoded, depth + 1);
                    if redacted != decoded {
                        output.push_str(&Value::String(redacted).to_string());
                        cursor = end;
                        continue;
                    }
                }
            }
            output.push_str(&text[start..end]);
            cursor = end;
        } else {
            let character = text[cursor..].chars().next().unwrap_or_default();
            output.push(character);
            cursor += character.len_utf8();
        }
    }
    let with_urls = url_credential_pattern().replace_all(&output, "${1}[redacted]@");
    let with_queries = query_secret_pattern().replace_all(&with_urls, "${1}[redacted]");
    let with_auth = authorization_pattern().replace_all(&with_queries, "[redacted]");
    let with_quoted = quoted_secret_pattern().replace_all(&with_auth, "${1}\"[redacted]\"");
    inline_secret_pattern()
        .replace_all(&with_quoted, "$1=[redacted]")
        .into_owned()
}

fn skip_space(text: &str, mut cursor: usize) -> usize {
    while cursor < text.len() && text.as_bytes()[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    cursor
}

// Scan balanced containers without recursion, including malformed diagnostic snippets.
// An unterminated secret consumes the remainder rather than exposing a partial value.
fn text_value_end(text: &str, start: usize) -> usize {
    let bytes = text.as_bytes();
    if start >= bytes.len() {
        return start;
    }
    let first = bytes[start];
    let container = first == b'{' || first == b'[';
    let mut nesting = 0usize;
    let mut quote = None;
    let mut escaped = false;
    for (cursor, &byte) in bytes.iter().enumerate().skip(start) {
        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == delimiter {
                quote = None;
                if !container {
                    return cursor + 1;
                }
            }
        } else if byte == b'"' || byte == b'\'' {
            quote = Some(byte);
        } else if container {
            if byte == b'{' || byte == b'[' {
                nesting += 1;
            } else if byte == b'}' || byte == b']' {
                nesting = nesting.saturating_sub(1);
                if nesting == 0 {
                    return cursor + 1;
                }
            }
        } else if b",;\r\n}]".contains(&byte) {
            return cursor;
        }
    }
    text.len()
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

    #[test]
    fn free_text_authorization_and_query_values_are_not_partial_redactions() {
        for text in [
            "Authorization: Bearer CANARY",
            "Bearer CANARY",
            "https://a.test/?access_token=CANARY",
            "Cookie: arbitrary=CANARY",
            "prefix {\"api_key\":\"CANARY\"} suffix",
            "headers: {\n\"custom\":\"CANARY\"\n}",
        ] {
            assert!(
                !redact_text(text).contains("CANARY"),
                "{}",
                redact_text(text)
            );
        }
    }

    #[test]
    fn parameterized_container_and_escaped_canaries_do_not_survive() {
        for key in ["env", "environment", "headers", "header", "ENV", "Headers"] {
            for value in [
                r#"{"custom":"CANARY"}"#,
                "{\n custom:CANARY\n}",
                r#"{"nested":{"custom":["CANARY"]}}"#,
                r#"[["custom","CANARY"],["other","CANARY"]]"#,
                r#"["CANARY", {"custom":"CANARY"}]"#,
                "{custom:CANARY",
            ] {
                for text in [
                    format!("failure {key}: {value}"),
                    format!("failure {{\"{key}\":{value}}} suffix"),
                ] {
                    let redacted = redact_text(&text);
                    assert!(!redacted.contains("CANARY"), "{text} => {redacted}");
                }
            }
        }
        for text in [
            r#"failed {\"api_key\":\"CANARY\"}"#,
            r#"failed {"api\u005fkey":"CANARY"}"#,
            r#""{\"headers\":{\"custom\":\"CANARY\"}}""#,
            "https://user:CANARY@example.test/path?access_token=CANARY&ok=yes",
            "Authorization: Bearer CANARY",
            "Bearer CANARY",
            "\"Bearer CANARY\"",
        ] {
            let redacted = redact_text(text);
            assert!(!redacted.contains("CANARY"), "{text} => {redacted}");
        }
    }

    #[test]
    fn plain_diagnostics_and_scalar_json_are_not_reformatted() {
        for text in [
            "plain error",
            "https://example.test:8443/path?ok=yes#anchor",
            "failed: connection refused\nretry: later",
            "中文错误：连接失败\nnext: retry",
            "\"未闭合中文",
            r#""string""#,
            r#"{"message":"plain error","count":3}"#,
        ] {
            assert_eq!(redact_text(text), text);
        }
    }

    #[test]
    fn deep_and_oversized_inputs_are_bounded_and_fail_closed() {
        let nested = format!(
            "{}{{\"headers\":{{\"custom\":\"CANARY\"}}}}{}",
            "[".repeat(2048),
            "]".repeat(2048)
        );
        assert!(!redact_text(&nested).contains("CANARY"));
        assert_eq!(redact_text(&"CANARY".repeat(50_000)), REDACTED);
        let mut value = json!({"custom":"CANARY"});
        for _ in 0..64 {
            value = json!({"nested":value});
        }
        assert!(!redact_json(&value).to_string().contains("CANARY"));
        let mut encoded = "Bearer CANARY".to_string();
        for _ in 0..12 {
            encoded = serde_json::to_string(&encoded).unwrap();
        }
        assert!(!redact_text(&encoded).contains("CANARY"));
    }

    #[test]
    fn redact_text_handles_embedded_json_and_all_credential_surfaces() {
        let text = r#"failed {"Authorization":"Bearer canary","env":{"HOME":"canary-env"},"headers":{"X-Test":"canary-header"}} https://user:canary-pass@example.test/?key=canary-key"#;
        let redacted = redact_text(text);
        assert!(!redacted.contains("canary"), "got {redacted}");
        assert!(redacted.contains("[redacted]"));
    }
}
