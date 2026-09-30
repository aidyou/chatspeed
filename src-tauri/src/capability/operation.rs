//! Canonical request hashing, stable id minting and per-resource serialization.
//!
//! The canonical hash is what makes an idempotency key trustworthy across
//! restarts: two requests with the same key must produce the same hash, and a
//! different hash under the same key is a conflict rather than a replay
//! (AC-2). The hash is computed over a canonicalized JSON projection, so key
//! order, insignificant whitespace and object identity cannot change it.

use dashmap::DashMap;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use super::error::CapabilityError;
use super::types::CapabilityKind;

/// Milliseconds since the Unix epoch, the single clock for the journal.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

/// Mints a stable, sortable operation id for one capability family.
pub fn new_operation_id(capability: CapabilityKind) -> String {
    format!("op-{}-{}", capability.as_str(), uuid::Uuid::now_v7())
}

/// Mints a stable id for one effect journal row.
pub fn new_effect_id() -> String {
    format!("eff-{}", uuid::Uuid::now_v7())
}

/// Serializes a JSON value into a canonical, order-independent string.
///
/// Object keys are emitted in sorted order and strings are escaped with the
/// JSON encoder, so the output is a pure function of the value's meaning.
pub fn canonical_json(value: &serde_json::Value) -> String {
    let mut output = String::new();
    write_canonical(value, &mut output);
    output
}

fn write_canonical(value: &serde_json::Value, output: &mut String) {
    match value {
        serde_json::Value::Null => output.push_str("null"),
        serde_json::Value::Bool(flag) => {
            output.push_str(if *flag { "true" } else { "false" })
        }
        serde_json::Value::Number(number) => output.push_str(&number.to_string()),
        serde_json::Value::String(text) => {
            // `to_string` on a Value::String always yields a JSON string literal.
            match serde_json::to_string(text) {
                Ok(escaped) => output.push_str(&escaped),
                Err(_) => output.push_str("\"\""),
            }
        }
        serde_json::Value::Array(items) => {
            output.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                write_canonical(item, output);
            }
            output.push(']');
        }
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            output.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                match serde_json::to_string(key) {
                    Ok(escaped) => output.push_str(&escaped),
                    Err(_) => output.push_str("\"\""),
                }
                output.push(':');
                if let Some(item) = map.get(key) {
                    write_canonical(item, output);
                }
            }
            output.push('}');
        }
    }
}

/// The canonical SHA-256 hash of a request projection, as lowercase hex.
pub fn canonical_request_hash(value: &serde_json::Value) -> String {
    hex::encode(Sha256::digest(canonical_json(value).as_bytes()))
}

/// A stable content digest over an ordered list of `(relative_path, sha256)`.
pub fn content_digest(entries: &[(String, String)]) -> String {
    let mut hasher = Sha256::new();
    for (path, digest) in entries {
        hasher.update(path.as_bytes());
        hasher.update([0u8]);
        hasher.update(digest.as_bytes());
        hasher.update([b'\n']);
    }
    hex::encode(hasher.finalize())
}

/// Validates that an idempotency key is usable, returning the canonical code
/// when it is not. Every mutation path shares this check (AC-2).
pub fn require_idempotency_key(key: &str) -> Result<String, CapabilityError> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err(CapabilityError::new(
            super::error::code::IDEMPOTENCY_KEY_REQUIRED,
            "a non-empty idempotency key is required for every capability mutation",
        ));
    }
    if trimmed.len() > 200 {
        return Err(CapabilityError::invalid_request(
            "idempotency key must be at most 200 characters",
        ));
    }
    Ok(trimmed.to_string())
}

/// Per-resource serialization for in-process single-flight.
///
/// The durable unique constraint remains the final authority; this only keeps
/// concurrent callers from racing on the same resource and burning work. Keys
/// are bounded by the number of managed resources (Skill names / MCP server
/// ids), so entries are intentionally retained for the process lifetime.
pub struct ResourceLocks {
    locks: DashMap<String, Arc<tokio::sync::Mutex<()>>>,
}

impl ResourceLocks {
    pub fn new() -> Self {
        Self {
            locks: DashMap::new(),
        }
    }

    /// Acquires the lock for one resource key, waiting for an in-flight peer.
    pub async fn lock(&self, key: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let mutex = self
            .locks
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        mutex.lock_owned().await
    }

    /// Number of tracked resource keys, exposed for diagnostics.
    pub fn tracked_keys(&self) -> usize {
        self.locks.len()
    }
}

impl Default for ResourceLocks {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_json_is_key_order_independent() {
        let first = json!({ "b": 1, "a": { "y": [1, 2], "x": "s" } });
        let second = json!({ "a": { "x": "s", "y": [1, 2] }, "b": 1 });
        assert_eq!(canonical_json(&first), canonical_json(&second));
        assert_eq!(
            canonical_request_hash(&first),
            canonical_request_hash(&second)
        );
    }

    #[test]
    fn canonical_request_hash_changes_with_content() {
        let first = json!({ "name": "weather" });
        let second = json!({ "name": "weather2" });
        assert_ne!(
            canonical_request_hash(&first),
            canonical_request_hash(&second)
        );
    }

    #[test]
    fn idempotency_key_must_be_present_and_bounded() {
        assert!(require_idempotency_key("  ").is_err());
        assert!(require_idempotency_key(&"k".repeat(201)).is_err());
        assert_eq!(
            require_idempotency_key(" key ").expect("trimmed"),
            "key".to_string()
        );
    }

    #[tokio::test]
    async fn resource_locks_serialize_the_same_key() {
        let locks = Arc::new(ResourceLocks::new());
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..4 {
            let locks = locks.clone();
            let counter = counter.clone();
            handles.push(tokio::spawn(async move {
                let _guard = locks.lock("skill:demo").await;
                let current = counter.load(std::sync::atomic::Ordering::SeqCst);
                tokio::task::yield_now().await;
                counter.store(current + 1, std::sync::atomic::Ordering::SeqCst);
            }));
        }
        for handle in handles {
            handle.await.expect("lock task must not panic");
        }
        // Serialization means each task observed the previous result, so the
        // counter ends at the number of tasks instead of a lost-update value.
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 4);
        assert_eq!(locks.tracked_keys(), 1);
    }
}
