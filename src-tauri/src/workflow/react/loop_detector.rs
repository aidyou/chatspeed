use std::collections::VecDeque;

/// Window size for the repetition detector (number of recent tool calls to inspect).
const LOOP_DETECT_WINDOW: usize = 8;

/// Minimum repeat count within the window that triggers a loop warning.
const LOOP_REPEAT_THRESHOLD: usize = 3;

/// Detects when the agent repeats the same tool call with identical arguments.
pub(crate) struct LoopDetector {
    /// Sliding window of (tool_name, args_hash) pairs.
    recent_calls: VecDeque<(String, u64)>,
}

impl LoopDetector {
    pub fn new() -> Self {
        Self {
            recent_calls: VecDeque::with_capacity(LOOP_DETECT_WINDOW),
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
            Some(format!(
                "ERROR: LOOP DETECTED\n<SYSTEM_REMINDER>You have called '{}' with identical arguments {} times \
                in the last {} steps. This is unproductive repetition. You MUST change your approach NOW:\n\
                1. If searching the web: try completely different keywords or a different data source.\n\
                2. If fetching a URL: the content may be unavailable — mark the task as 'data_missing' and continue.\n\
                3. If all alternatives are exhausted: accept the limitation and move to the next task.\n\
                Do NOT call '{}' with the same parameters again.</SYSTEM_REMINDER>",
                tool_name,
                repeat_count + 1,
                LOOP_DETECT_WINDOW,
                tool_name
            ))
        } else {
            None
        }
    }
}
