#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JsonOutput {
    pub compact_content: String,
    pub was_minified: bool,
}

/// Detects a complete JSON object or array emitted exclusively on stdout and
/// removes only insignificant whitespace outside JSON strings.
///
/// The lexical minifier preserves key order, duplicate keys, number literals,
/// escape sequences, and all characters inside strings. Parsing is used only
/// to validate that the complete stdout is JSON before whitespace is removed.
pub(crate) fn detect_json_stdout(stdout: &str, stderr: &str) -> Option<JsonOutput> {
    if !stderr.trim().is_empty() {
        return None;
    }

    let trimmed = stdout.trim();
    if !matches!(trimmed.as_bytes().first(), Some(b'{') | Some(b'[')) {
        return None;
    }
    serde_json::from_str::<serde_json::Value>(trimmed).ok()?;

    let compact_content = minify_json_whitespace(trimmed);
    Some(JsonOutput {
        was_minified: compact_content.len() < stdout.len(),
        compact_content,
    })
}

fn minify_json_whitespace(content: &str) -> String {
    let mut output = String::with_capacity(content.len());
    let mut in_string = false;
    let mut escaped = false;

    for character in content.chars() {
        if in_string {
            output.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
        } else if character == '"' {
            in_string = true;
            output.push(character);
        } else if !character.is_ascii_whitespace() {
            output.push(character);
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::detect_json_stdout;

    #[test]
    fn minifies_only_insignificant_json_whitespace() {
        let stdout = "{\n  \"first\": 1.0,\n  \"first\": 2,\n  \"text\": \"keep  spaces \\\\ and \\\"quotes\\\"\"\n}\n";

        let result = detect_json_stdout(stdout, "").expect("complete JSON should be detected");

        assert_eq!(
            result.compact_content,
            r#"{"first":1.0,"first":2,"text":"keep  spaces \\ and \"quotes\""}"#
        );
        assert!(result.was_minified);
    }

    #[test]
    fn preserves_compact_json_without_claiming_minification() {
        let stdout = r#"[{"id":12345678901234567890,"value":"x y"}]"#;

        let result = detect_json_stdout(stdout, "").expect("complete JSON should be detected");

        assert_eq!(result.compact_content, stdout);
        assert!(!result.was_minified);
    }

    #[test]
    fn rejects_invalid_json_non_structured_json_and_mixed_stderr() {
        assert!(detect_json_stdout("{not json}", "").is_none());
        assert!(detect_json_stdout("\"json string\"", "").is_none());
        assert!(detect_json_stdout("{\"ok\":true}", "warning").is_none());
    }
}
