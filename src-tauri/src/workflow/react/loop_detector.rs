use std::collections::VecDeque;

/// Window size for the repetition detector (number of recent tool calls to inspect).
const LOOP_DETECT_WINDOW: usize = 18;

/// Minimum repeat count within the window that triggers a loop warning.
const LOOP_REPEAT_THRESHOLD: usize = 3;

/// Minimum consecutive repeated tool-call sequence count before surfacing a warning.
const LOOP_SEQUENCE_REPEAT_THRESHOLD: usize = 3;

/// Maximum sequence length to consider when detecting repeated tool-call patterns.
const LOOP_SEQUENCE_MAX_LEN: usize = 6;

/// Minimum consecutive identical no-tool responses before surfacing a loop warning.
const NO_TOOL_RESPONSE_REPEAT_THRESHOLD: usize = 3;

/// Detects when the agent repeats the same tool call with identical arguments.
pub(crate) struct LoopDetector {
    /// Sliding window of (tool_name, args_hash) pairs.
    recent_calls: VecDeque<(String, u64)>,
    /// Last normalized assistant response seen on a no-tool turn.
    last_no_tool_response: Option<String>,
    /// Consecutive count of identical no-tool assistant responses.
    consecutive_no_tool_responses: usize,
}

impl LoopDetector {
    pub fn new() -> Self {
        Self {
            recent_calls: VecDeque::with_capacity(LOOP_DETECT_WINDOW),
            last_no_tool_response: None,
            consecutive_no_tool_responses: 0,
        }
    }

    /// Records a tool call and returns a warning message if a loop is detected.
    pub fn record_and_check(
        &mut self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Option<String> {
        use std::hash::{Hash, Hasher};

        // Canonicalize JSON to ensure consistent hashing regardless of key order
        // serde_json::to_string on a Value uses BTreeMap internally which is sorted by default,
        // but being explicit or using a sorted approach is safer if preserve_order is enabled.
        let mut hasher = std::collections::hash_map::DefaultHasher::new();

        // For Objects, we ensure keys are hashed in a stable order
        fn hash_stable<H: Hasher>(v: &serde_json::Value, state: &mut H) {
            match v {
                serde_json::Value::Null => {
                    0u8.hash(state);
                }
                serde_json::Value::Bool(b) => {
                    1u8.hash(state);
                    b.hash(state);
                }
                serde_json::Value::Number(n) => {
                    2u8.hash(state);
                    n.to_string().hash(state);
                }
                serde_json::Value::String(s) => {
                    3u8.hash(state);
                    s.hash(state);
                }
                serde_json::Value::Array(arr) => {
                    4u8.hash(state);
                    for item in arr {
                        hash_stable(item, state);
                    }
                }
                serde_json::Value::Object(map) => {
                    5u8.hash(state);
                    let mut keys: Vec<_> = map.keys().collect();
                    keys.sort();
                    for k in keys {
                        k.hash(state);
                        hash_stable(&map[k], state);
                    }
                }
            }
        }

        hash_stable(args, &mut hasher);
        let args_hash = hasher.finish();

        let key = (tool_name.to_string(), args_hash);
        let repeat_count = self
            .recent_calls
            .iter()
            .filter(|c| c.0 == key.0 && c.1 == key.1)
            .count();

        self.recent_calls.push_back(key);
        if self.recent_calls.len() > LOOP_DETECT_WINDOW {
            self.recent_calls.pop_front();
        }

        if repeat_count >= LOOP_REPEAT_THRESHOLD {
            let task_output_guidance = if tool_name == crate::tools::TOOL_SUB_AGENT_OUTPUT {
                "\nFor sub_agent_output specifically: do NOT call sub_agent_output again for the same missing or unavailable task_id. sub_agent_output only retrieves results for sub-agent IDs returned by sub_agent_run; it is not a final-answer/output tool. If no valid sub-agent exists, continue with another appropriate tool or report the limitation."
            } else {
                ""
            };
            Some(format!(
                "ERROR: LOOP DETECTED\n<SYSTEM_REMINDER>You have called '{}' with identical arguments {} times \
                in the last {} steps. This is unproductive repetition. You MUST change your approach NOW:\n\
                1. If searching the web: try completely different keywords or a different data source.\n\
                2. If fetching a URL: the content may be unavailable — mark the task as 'data_missing' and continue.\n\
                3. If all alternatives are exhausted: accept the limitation and move to the next task.{}\n\
                Do NOT call '{}' with the same parameters again.</SYSTEM_REMINDER>",
                tool_name,
                repeat_count + 1,
                LOOP_DETECT_WINDOW,
                task_output_guidance,
                tool_name
            ))
        } else {
            self.detect_repeated_sequence()
        }
    }

    /// Records a no-tool assistant response and returns a warning if the exact same
    /// normalized response has been repeated consecutively for too long.
    pub fn record_no_tool_response_and_check(&mut self, response_text: &str) -> Option<String> {
        let normalized = normalize_response_text(response_text)?;

        if self.last_no_tool_response.as_deref() == Some(normalized.as_str()) {
            self.consecutive_no_tool_responses += 1;
        } else {
            self.last_no_tool_response = Some(normalized.clone());
            self.consecutive_no_tool_responses = 1;
        }

        if self.consecutive_no_tool_responses >= NO_TOOL_RESPONSE_REPEAT_THRESHOLD {
            Some(format!(
                "<SYSTEM_REMINDER>LOOP DETECTED: You have produced the exact same assistant response {} times in a row without calling any tools.\n\
                Repeating the same text is not progress. You MUST change strategy now:\n\
                1. If you still need information, call an appropriate tool instead of repeating the same sentence.\n\
                2. If the task is actually complete, call the appropriate completion tool for this session.\n\
                3. If you cannot continue because of a real limitation, explain the limitation once and then call 'ask_user' or the appropriate completion tool for this session.\n\
                Do NOT output the same response again without a tool call.</SYSTEM_REMINDER>",
                self.consecutive_no_tool_responses
            ))
        } else {
            None
        }
    }

    /// Clears the no-tool response repetition tracker after progress is made.
    pub fn reset_no_tool_response_history(&mut self) {
        self.last_no_tool_response = None;
        self.consecutive_no_tool_responses = 0;
    }

    fn detect_repeated_sequence(&self) -> Option<String> {
        let calls: Vec<_> = self.recent_calls.iter().cloned().collect();
        let max_sequence_len =
            LOOP_SEQUENCE_MAX_LEN.min(calls.len() / LOOP_SEQUENCE_REPEAT_THRESHOLD);

        for sequence_len in 2..=max_sequence_len {
            let total_len = sequence_len * LOOP_SEQUENCE_REPEAT_THRESHOLD;
            let tail = &calls[calls.len() - total_len..];
            let pattern = &tail[..sequence_len];

            let repeated = (1..LOOP_SEQUENCE_REPEAT_THRESHOLD)
                .all(|index| &tail[index * sequence_len..(index + 1) * sequence_len] == pattern);
            if !repeated {
                continue;
            }

            let tool_summary = pattern
                .iter()
                .map(|(tool_name, _)| tool_name.as_str())
                .collect::<Vec<_>>()
                .join(" -> ");

            return Some(format!(
                "ERROR: LOOP DETECTED\n<SYSTEM_REMINDER>You are repeating the same tool-call sequence {} times in a row: {}.\n\
                This indicates you are re-gathering the same context without making progress. You MUST change strategy now:\n\
                1. Stop repeating the same search/fetch/read cycle.\n\
                2. Use the information already gathered to implement, answer, verify, or explain a real blocker.\n\
                3. Only call additional tools if they gather genuinely new information.\n\
                Do NOT repeat this same tool sequence again.</SYSTEM_REMINDER>",
                LOOP_SEQUENCE_REPEAT_THRESHOLD,
                tool_summary
            ));
        }

        None
    }
}

fn normalize_response_text(response_text: &str) -> Option<String> {
    let normalized = response_text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

#[cfg(test)]
mod tests {
    use super::LoopDetector;
    use serde_json::json;

    #[test]
    fn detects_repeated_tool_calls_with_identical_args() {
        let mut detector = LoopDetector::new();
        let args = json!({"path":"/tmp/demo.txt"});

        assert!(detector.record_and_check("read_file", &args).is_none());
        assert!(detector.record_and_check("read_file", &args).is_none());
        assert!(detector.record_and_check("read_file", &args).is_none());

        let warning = detector.record_and_check("read_file", &args);
        assert!(warning.is_some(), "expected repeated tool call warning");
    }

    #[test]
    fn detects_repeated_no_tool_responses_after_threshold() {
        let mut detector = LoopDetector::new();

        assert!(detector
            .record_no_tool_response_and_check("你好，我无法给到相关内容。")
            .is_none());
        assert!(detector
            .record_no_tool_response_and_check("你好， 我无法给到相关内容。")
            .is_none());

        let warning = detector.record_no_tool_response_and_check("  你好，我无法给到相关内容。  ");
        assert!(
            warning.is_some(),
            "expected repeated no-tool response warning"
        );
    }

    #[test]
    fn reset_clears_no_tool_response_streak() {
        let mut detector = LoopDetector::new();

        detector.record_no_tool_response_and_check("same reply");
        detector.record_no_tool_response_and_check("same reply");
        detector.reset_no_tool_response_history();

        assert!(detector
            .record_no_tool_response_and_check("same reply")
            .is_none());
    }

    #[test]
    fn detects_repeated_tool_sequence() {
        let mut detector = LoopDetector::new();
        let search_args = json!({"query":"single instance"});
        let fetch_args = json!({"url":"https://v2.tauri.app/plugin/single-instance/"});
        let main_args = json!({"file_path":"src-tauri/src/main.rs"});
        let cargo_args = json!({"file_path":"src-tauri/Cargo.toml"});

        let pattern = [
            ("web_search", &search_args),
            ("web_fetch", &fetch_args),
            ("read_file", &main_args),
            ("read_file", &cargo_args),
        ];

        for _ in 0..2 {
            for (tool, args) in &pattern {
                assert!(detector.record_and_check(tool, args).is_none());
            }
        }

        let mut warning = None;
        for (tool, args) in &pattern {
            warning = detector.record_and_check(tool, args);
        }

        assert!(warning.is_some(), "expected repeated tool sequence warning");
    }
}
