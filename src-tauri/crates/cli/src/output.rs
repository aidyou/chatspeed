//! Output rendering for the `cs` CLI.
//!
//! stdout carries only results/events; diagnostics go to stderr. `json` and
//! `jsonl` are machine contracts (never localized); `human` is localized and
//! may change between releases.

use crate::args::OutputFormat;
use serde_json::Value;
use std::io::Write;

/// Renders a result value according to the selected output format.
pub fn render_result(format: OutputFormat, value: &Value) {
    match format {
        OutputFormat::Json => print_json(value),
        OutputFormat::Jsonl => print_jsonl(value),
        OutputFormat::Human => print_human(value),
    }
}

/// Prints one JSON document (pretty) to stdout.
pub fn print_json(value: &Value) {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    let rendered = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string());
    let _ = writeln!(handle, "{}", rendered);
}

/// Prints one compact JSON line to stdout (JSONL contract: one event per line).
pub fn print_jsonl(value: &Value) {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    let rendered = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string());
    let _ = writeln!(handle, "{}", rendered);
}

/// Human rendering: compact key facts instead of raw JSON.
fn print_human(value: &Value) {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    let _ = writeln!(handle, "{}", human_value(value));
}

/// Renders a JSON value as compact human-readable text.
fn human_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(items) => items.iter().map(human_value).collect::<Vec<_>>().join("\n"),
        Value::Object(map) => {
            // Prefer common identity fields for a compact single-line summary.
            for key in ["session_id", "id", "agent_id", "name"] {
                if let Some(inner) = map.get(key) {
                    if let Some(text) = inner.as_str() {
                        return text.to_string();
                    }
                }
            }
            serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string())
        }
        other => other.to_string(),
    }
}

/// Writes a diagnostic line to stderr (never stdout).
pub fn eprint_diagnostic(message: &str) {
    let stderr = std::io::stderr();
    let mut handle = stderr.lock();
    let _ = writeln!(handle, "{}", message);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn human_value_prefers_identity_fields() {
        assert_eq!(human_value(&json!({"session_id": "abc"})), "abc");
        assert_eq!(human_value(&json!({"id": "x", "name": "y"})), "x");
        assert_eq!(human_value(&json!("plain")), "plain");
    }

    #[test]
    fn human_value_falls_back_to_pretty_json() {
        let rendered = human_value(&json!({"other": 1}));
        assert!(rendered.contains("other"));
    }
}
