//! CLI-local experiment artifact domain (Phase 2A).
//!
//! This module is pure and self-contained: it never links the workflow runtime,
//! opens the database, or talks to the network. It turns an already-fetched
//! control-plane snapshot + durable events into an immutable, redacted,
//! offline-verifiable artifact bundle, and verifies/replays such a bundle from
//! disk alone.
//!
//! Design contract (see `work/agent-cli-phase-2-implementation-plan.md`):
//! - fixed v1 layout: `run.json`, `snapshot.json`, `events.jsonl`,
//!   `result.json`, `artifacts/manifest.json`;
//! - canonical snake_case JSON, domain-separated SHA-256, durable event hash
//!   chain, and a file manifest that cannot be edited to hide tampering;
//! - conservative recursive redaction: secrets/env/headers are dropped to a
//!   marker, free text and paths are reduced to type/length/hash projections,
//!   safe identifiers and usage numbers are kept;
//! - staging + atomic rename so a partially written or failed bundle can never
//!   be read back as `complete`.

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Artifact schema version this CLI writes and reads.
pub const SCHEMA_VERSION: u32 = 1;
/// Highest schema version this CLI can interpret.
pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;
/// Fixed artifact kind label.
pub const ARTIFACT_KIND: &str = "cs.workflow.run";
/// Hash algorithm label recorded in the manifest.
pub const HASH_ALGORITHM: &str = "sha256-canonical-v1";
/// Genesis marker used as the first event's `prev_hash`.
pub const GENESIS_HASH: &str = "GENESIS:cs-artifact:v1";

/// Maximum number of durable events a single artifact may contain.
pub const MAX_EVENTS: usize = 100_000;
/// Maximum canonical size (bytes) of a single redacted event payload.
pub const MAX_EVENT_DATA_BYTES: usize = 1 << 20;
/// Maximum total bytes across all published artifact files.
pub const MAX_TOTAL_BYTES: u64 = 256 << 20;
/// Maximum bytes accepted for the manifest before parsing it offline.
const MAX_MANIFEST_BYTES: u64 = 1 << 20;
/// Maximum encoded bytes accepted for one event line during verification.
const MAX_EVENT_LINE_BYTES: usize = MAX_EVENT_DATA_BYTES * 2;

/// The exact set of data files a v1 manifest must cover (the manifest itself is
/// stored separately under `artifacts/`). Verification requires this exact set,
/// with no duplicates or extras, so a required file can never be dropped from
/// integrity coverage.
const REQUIRED_FILES: [&str; 4] = ["run.json", "snapshot.json", "events.jsonl", "result.json"];

/// Machine-stable error codes. These are part of the CLI contract: they must
/// not change meaning across releases, and `inspect`/`replay` surface them in
/// structured output alongside a non-zero exit for `invalid`/`incompatible`.
pub mod code {
    pub const TARGET_EXISTS: &str = "target_exists";
    pub const SYMLINK: &str = "symlink_rejected";
    pub const LIMIT: &str = "limit_exceeded";
    pub const INCOMPATIBLE: &str = "incompatible_schema";
    pub const MANIFEST: &str = "manifest_mismatch";
    pub const HASH: &str = "hash_mismatch";
    pub const CHAIN: &str = "chain_break";
    pub const SESSION: &str = "session_mismatch";
    pub const MISSING_FILE: &str = "missing_file";
    pub const IO: &str = "io_error";
    pub const INVALID: &str = "invalid_artifact";
}

/// A fail-closed artifact error carrying a stable machine code.
#[derive(Debug, Clone)]
pub struct ArtifactError {
    pub code: &'static str,
    pub message: String,
}

impl ArtifactError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Artifact lifecycle status. A malformed/`invalid` claim is rejected by the
/// verifier (fail closed) rather than represented as a status here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactStatus {
    Complete,
    Incomplete,
}

impl ArtifactStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ArtifactStatus::Complete => "complete",
            ArtifactStatus::Incomplete => "incomplete",
        }
    }
}

/// Structured terminal status derived from the verified durable event chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalStatus {
    Completed,
    Failed,
    Cancelled,
    Unknown,
}

impl TerminalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TerminalStatus::Completed => "completed",
            TerminalStatus::Failed => "failed",
            TerminalStatus::Cancelled => "cancelled",
            TerminalStatus::Unknown => "unknown",
        }
    }
}

/// Terminal durable event types and their structured status mapping live in
/// `terminal_status_from_event`. A durable terminal event is the sole authority
/// for a run's terminal state; a possibly-stale snapshot never fabricates one.

/// Input for building a bundle: everything already fetched from the control
/// plane. The caller (capture) is responsible for the read-only HTTP fetch.
pub struct CaptureInput<'a> {
    pub session_id: &'a str,
    pub agent_id: &'a str,
    pub server_instance_id: &'a str,
    pub protocol_version: &'a str,
    pub capture_timestamp: &'a str,
    /// Authoritative workflow snapshot (snake_case wire shape).
    pub snapshot: &'a Value,
    /// Durable event records (snake_case wire shape), any page order.
    pub events: &'a [Value],
}

// ---------------------------------------------------------------------------
// Canonical JSON + domain-separated hashing
// ---------------------------------------------------------------------------

/// Serializes a `Value` to canonical JSON: object keys are recursively sorted,
/// arrays keep order, numbers/strings use serde_json's own escaping.
pub fn canonical_json(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            out.push('{');
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&json_string(key));
                out.push(':');
                canonical_json(&map[*key], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                canonical_json(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&json_string_of(other)),
    }
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

fn json_string_of(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_string())
}

/// Computes a domain-separated SHA-256 hex digest: `sha256(domain || 0x00 || bytes)`.
pub fn domain_hash(domain: &str, bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0u8]);
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Canonical-JSON hash of a value within a domain.
pub fn canonical_hash(domain: &str, value: &Value) -> String {
    let mut buffer = String::new();
    canonical_json(value, &mut buffer);
    domain_hash(domain, buffer.as_bytes())
}

/// Hash of a raw byte blob (used for file digests).
pub fn blob_hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

// ---------------------------------------------------------------------------
// Redaction
// ---------------------------------------------------------------------------

/// Keys whose string values are safe identifiers/enums and are preserved.
const SAFE_KEYS: &[&str] = &[
    "id",
    "session_id",
    "parent_session_id",
    "agent_id",
    "run_id",
    "server_instance_id",
    "protocol_version",
    "schema_version",
    "artifact_kind",
    "artifact_status",
    "provenance",
    "cost_status",
    "correctness_status",
    "promotion_status",
    "event_type",
    "event_version",
    "source_event_type",
    "status",
    "terminal_status",
    "state",
    "wait_reason",
    "role",
    "message_kind",
    "message_subtype",
    "error_type",
    "step_type",
    "backend_model",
    "provider_id",
    "pricing_status",
    "tool_name",
    "phase",
    "decision",
    "mode",
    "approval_level",
    "sandbox_execution_mode",
    "sandbox_scheme_id",
    "version",
    "type",
    "created_at",
    "updated_at",
    "capture_timestamp",
    "duration_ms",
    "usage_summary",
    "self_usage",
    "with_sub_agents",
    "model_breakdowns",
    "input_tokens",
    "output_tokens",
    "cache_tokens",
    "cache_write_tokens",
    "reasoning_tokens",
    "audio_input_tokens",
    "audio_output_tokens",
    "total_tokens",
    "estimated_cost",
    "effective_cost_per_million",
    "unpriced_tokens",
];

/// Fixed marker replacing any value under a secret-bearing key.
const SECRET_MARKER: &str = "<redacted-secret>";

fn is_safe_key(key: &str) -> bool {
    SAFE_KEYS.contains(&key)
}

fn is_secret_key(key: &str) -> bool {
    let lowered = key.to_ascii_lowercase();
    if lowered == "key" || lowered.ends_with("_key") {
        return true;
    }
    [
        "token",
        "secret",
        "password",
        "passwd",
        "apikey",
        "api_key",
        "auth",
        "cookie",
        "bearer",
        "credential",
        "private_key",
        "access_key",
        "client_secret",
        "header",
        "env",
        "environment",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
}

/// Detects obvious secret values even under non-secret keys.
fn looks_like_secret_value(text: &str) -> bool {
    let prefix = ["sk-", "xox", "ghp_", "glpat-", "AKIA", "Bearer "];
    prefix.iter().any(|p| text.starts_with(p))
}

fn looks_like_path(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    let bytes = text.as_bytes();
    if bytes[0] == b'/' || text.starts_with("~/") || text.starts_with("\\\\") {
        return true;
    }
    // Windows drive letter: `C:\` or `C:/`.
    bytes.len() >= 3
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
        && bytes[0].is_ascii_alphabetic()
}

/// Recursively redacts a value into a safe projection.
///
/// - object/array: recurse, dropping secret keys to a marker;
/// - safe identifier keys: keep the scalar/string as-is;
/// - secret values or secret keys: fixed marker (no hash, to avoid dictionary
///   recovery of low-entropy secrets);
/// - path-like strings: `{type:"path", sha256}`;
/// - other strings (free text): `{type:"string", len, sha256}`;
/// - numbers/bools/null: kept (usage numbers are factual, not sensitive).
pub fn redact_value(key_hint: &str, value: &Value) -> Value {
    if is_secret_key(key_hint) && !is_safe_key(key_hint) {
        return Value::String(SECRET_MARKER.to_string());
    }
    match value {
        Value::Object(map) => {
            let mut out = Map::with_capacity(map.len());
            for (key, inner) in map {
                out.insert(key.clone(), redact_value(key, inner));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| redact_value(key_hint, item))
                .collect(),
        ),
        Value::String(text) => {
            if is_safe_key(key_hint) {
                Value::String(text.clone())
            } else if looks_like_secret_value(text) {
                Value::String(SECRET_MARKER.to_string())
            } else if looks_like_path(text) {
                json!({
                    "type": "path",
                    "sha256": domain_hash("cs-artifact:path", text.as_bytes()),
                })
            } else {
                json!({
                    "type": "string",
                    "len": text.chars().count() as u64,
                    "sha256": domain_hash("cs-artifact:text", text.as_bytes()),
                })
            }
        }
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------------
// Event hash chain
// ---------------------------------------------------------------------------

/// Maps a terminal durable event type to its structured terminal status.
///
/// Failure and cancellation are preserved distinctly from completion; a
/// `complete` artifact is one that reached *any* terminal state, not only a
/// successful one.
fn terminal_status_from_event(event_type: &str) -> Option<TerminalStatus> {
    match event_type {
        "task_completed" | "workflow_completed" => Some(TerminalStatus::Completed),
        "workflow_failed" => Some(TerminalStatus::Failed),
        "workflow_cancelled" => Some(TerminalStatus::Cancelled),
        _ => None,
    }
}

/// Builds the `events.jsonl` records and returns
/// `(lines, head_hash, terminal_status_from_events)`.
///
/// Records are ordered by ascending durable id; duplicate ids are rejected.
/// Each `record_hash` binds the durable id, session id, event version, payload
/// hash and previous hash, so reordering, deletion, or session mixing breaks
/// the chain deterministically. The third element is the terminal status derived
/// from the last terminal durable event, if any.
fn build_event_chain(
    session_id: &str,
    events: &[Value],
) -> Result<(Vec<String>, String, Option<TerminalStatus>), ArtifactError> {
    if events.len() > MAX_EVENTS {
        return Err(ArtifactError::new(
            code::LIMIT,
            format!("event count {} exceeds limit {}", events.len(), MAX_EVENTS),
        ));
    }

    // Sort by durable id ascending (server returns ASC, but be robust).
    let mut ordered: Vec<&Value> = events.iter().collect();
    ordered.sort_by_key(|event| event.get("id").and_then(Value::as_i64).unwrap_or(i64::MIN));

    let mut lines = Vec::with_capacity(ordered.len());
    let mut prev_hash = GENESIS_HASH.to_string();
    let mut last_id: Option<i64> = None;
    let mut terminal: Option<TerminalStatus> = None;

    for event in ordered {
        let durable_id = event.get("id").and_then(Value::as_i64).ok_or_else(|| {
            ArtifactError::new(code::INVALID, "durable event is missing an integer id")
        })?;
        if let Some(last) = last_id {
            if durable_id <= last {
                return Err(ArtifactError::new(
                    code::INVALID,
                    format!(
                        "durable event ids must be strictly increasing ({} after {})",
                        durable_id, last
                    ),
                ));
            }
        }
        last_id = Some(durable_id);

        let event_session = event
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        if event_session != session_id {
            return Err(ArtifactError::new(
                code::SESSION,
                format!(
                    "event {} belongs to session '{}' not '{}'",
                    durable_id, event_session, session_id
                ),
            ));
        }

        let event_type = event
            .get("event_type")
            .and_then(Value::as_str)
            .unwrap_or("");
        let event_version = event
            .get("event_version")
            .and_then(Value::as_str)
            .unwrap_or("");
        let created_at = event
            .get("created_at")
            .and_then(Value::as_str)
            .unwrap_or("");
        let raw_data = event.get("event_data").cloned().unwrap_or(Value::Null);
        if let Some(status) = terminal_status_from_event(event_type) {
            terminal = Some(status);
        }

        let data = redact_value("event_data", &raw_data);
        let mut canonical = String::new();
        canonical_json(&data, &mut canonical);
        if canonical.len() > MAX_EVENT_DATA_BYTES {
            return Err(ArtifactError::new(
                code::LIMIT,
                format!(
                    "event {} payload projection exceeds {} bytes",
                    durable_id, MAX_EVENT_DATA_BYTES
                ),
            ));
        }
        let payload_hash = domain_hash("cs-artifact:payload", canonical.as_bytes());
        let record_hash = canonical_hash(
            "cs-artifact:event",
            &json!({
                "durable_id": durable_id.to_string(),
                "session_id": event_session,
                "event_version": event_version,
                "payload_hash": payload_hash,
                "prev_hash": prev_hash,
            }),
        );

        let line = json!({
            "durable_id": durable_id.to_string(),
            "session_id": event_session,
            "event_type": event_type,
            "event_version": event_version,
            "created_at": created_at,
            "provenance": "workflow_runtime",
            "data": data,
            "payload_hash": payload_hash,
            "prev_hash": prev_hash,
            "record_hash": record_hash,
        });
        lines.push(serde_json::to_string(&line).unwrap_or_else(|_| "{}".to_string()));
        prev_hash = record_hash;
    }

    Ok((lines, prev_hash, terminal))
}

// ---------------------------------------------------------------------------
// Usage / cost projection
// ---------------------------------------------------------------------------

/// Extracts and projects the usage summary from the latest `task_completed` event.
/// `data_key` is `event_data` during capture and `data` during offline verify.
fn project_usage(events: &[Value], data_key: &str) -> (Value, &'static str) {
    let summary = events
        .iter()
        .rev()
        .find(|event| event.get("event_type").and_then(Value::as_str) == Some("task_completed"))
        .and_then(|event| event.get(data_key))
        .and_then(|data| data.get("usage_summary"))
        .filter(|summary| !summary.is_null());

    let Some(summary) = summary else {
        return (json!({ "present": false }), "unknown");
    };

    let projection = json!({
        "present": true,
        "version": summary.get("version").and_then(Value::as_u64),
        "terminal_status": summary.get("terminal_status").and_then(Value::as_str),
        "duration_ms": summary.get("duration_ms").and_then(Value::as_i64),
        "is_partial": summary.get("is_partial").and_then(Value::as_bool).unwrap_or(true),
        "has_sub_agents": summary.get("has_sub_agents").and_then(Value::as_bool).unwrap_or(false),
        "self_usage": project_totals(summary.get("self_usage").unwrap_or(&Value::Null)),
        "with_sub_agents": project_totals(summary.get("with_sub_agents").unwrap_or(&Value::Null)),
        "model_breakdowns": project_breakdowns(summary.get("model_breakdowns")),
    });
    (projection, usage_cost_status(summary))
}

fn usage_cost_status(summary: &Value) -> &'static str {
    let complete = summary.get("is_partial").and_then(Value::as_bool) == Some(false);
    let self_known = usage_totals_are_priced(summary.get("self_usage"));
    let sub_agents_known = usage_totals_are_priced(summary.get("with_sub_agents"));
    let breakdowns_known = summary
        .get("model_breakdowns")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().all(|item| {
                item.get("pricing_status").and_then(Value::as_str) == Some("priced")
                    && item.get("estimated_cost").and_then(Value::as_f64).is_some()
            })
        });
    if complete && self_known && sub_agents_known && breakdowns_known {
        "known"
    } else {
        "unknown"
    }
}

fn usage_totals_are_priced(value: Option<&Value>) -> bool {
    let Some(totals) = value else {
        return false;
    };
    totals.get("unpriced_tokens").and_then(Value::as_i64) == Some(0)
        && totals
            .get("estimated_cost")
            .and_then(Value::as_f64)
            .is_some()
}

fn project_totals(totals: &Value) -> Value {
    if totals.is_null() {
        return Value::Null;
    }
    json!({
        "input_tokens": totals.get("input_tokens").and_then(Value::as_i64),
        "output_tokens": totals.get("output_tokens").and_then(Value::as_i64),
        "cache_tokens": totals.get("cache_tokens").and_then(Value::as_i64),
        "cache_write_tokens": totals.get("cache_write_tokens").and_then(Value::as_i64),
        "reasoning_tokens": totals.get("reasoning_tokens").and_then(Value::as_i64),
        "audio_input_tokens": totals.get("audio_input_tokens").and_then(Value::as_i64),
        "audio_output_tokens": totals.get("audio_output_tokens").and_then(Value::as_i64),
        "total_tokens": totals.get("total_tokens").and_then(Value::as_i64),
        "estimated_cost": totals.get("estimated_cost").and_then(Value::as_f64),
        "effective_cost_per_million": totals.get("effective_cost_per_million").and_then(Value::as_f64),
        "unpriced_tokens": totals.get("unpriced_tokens").and_then(Value::as_i64),
    })
}

fn project_breakdowns(value: Option<&Value>) -> Value {
    let Some(Value::Array(items)) = value else {
        return Value::Array(Vec::new());
    };
    Value::Array(
        items
            .iter()
            .map(|item| {
                let input_tokens = item.get("input_tokens").and_then(Value::as_i64);
                let output_tokens = item.get("output_tokens").and_then(Value::as_i64);
                json!({
                    "provider_id": item.get("provider_id").and_then(Value::as_i64),
                    "backend_model": item.get("backend_model").and_then(Value::as_str),
                    "input_tokens": input_tokens,
                    "output_tokens": output_tokens,
                    "cache_tokens": item.get("cache_tokens").and_then(Value::as_i64),
                    "cache_write_tokens": item.get("cache_write_tokens").and_then(Value::as_i64),
                    "reasoning_tokens": item.get("reasoning_tokens").and_then(Value::as_i64),
                    "audio_input_tokens": item.get("audio_input_tokens").and_then(Value::as_i64),
                    "audio_output_tokens": item.get("audio_output_tokens").and_then(Value::as_i64),
                    "total_tokens": input_tokens.zip(output_tokens).map(|(input, output)| input.saturating_add(output)),
                    "pricing_status": item.get("pricing_status").and_then(Value::as_str),
                    "input_per_million": item.get("input_per_million").and_then(Value::as_f64),
                    "output_per_million": item.get("output_per_million").and_then(Value::as_f64),
                    "cache_per_million": item.get("cache_per_million").and_then(Value::as_f64),
                    "reasoning_per_million": item.get("reasoning_per_million").and_then(Value::as_f64),
                    "multiplier": item.get("multiplier").and_then(Value::as_f64),
                    "estimated_cost": item.get("estimated_cost").and_then(Value::as_f64),
                })
            })
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// Snapshot projection
// ---------------------------------------------------------------------------

/// Parses the `agent_config` JSON string (camelCase AgentConfig) if present.
fn parse_agent_config(snapshot: &Value) -> Option<Value> {
    let raw = snapshot.get("workflow")?.get("agent_config")?.as_str()?;
    serde_json::from_str(raw).ok()
}

/// Builds a redacted snapshot projection: identity, status, liveness, counts
/// and a redacted config projection. Never copies messages or raw context.
fn project_snapshot(input: &CaptureInput) -> Value {
    let workflow = input
        .snapshot
        .get("workflow")
        .cloned()
        .unwrap_or(Value::Null);
    let message_count = input
        .snapshot
        .get("messages")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);

    json!({
        "provenance": "control_plane_snapshot",
        "session_id": input.session_id,
        "workflow_id": workflow.get("id").and_then(Value::as_str),
        "agent_id": workflow.get("agent_id").and_then(Value::as_str),
        "status": workflow.get("status").and_then(Value::as_str),
        "wait_reason": workflow.get("wait_reason").and_then(Value::as_str),
        "is_automation_run": workflow.get("is_automation_run").and_then(Value::as_bool).unwrap_or(false),
        "has_live_session": input.snapshot.get("has_live_session").and_then(Value::as_bool).unwrap_or(false),
        "message_count": message_count,
        "hidden_earlier_message_count": input.snapshot.get("hidden_earlier_message_count").and_then(Value::as_u64).unwrap_or(0),
        "config_projection": redact_value("agent_config", &parse_agent_config(input.snapshot).unwrap_or(Value::Null)),
    })
}

/// Computes the run.json provenance hashes from snapshot fields.
fn project_hashes(input: &CaptureInput) -> Value {
    let workflow = input
        .snapshot
        .get("workflow")
        .cloned()
        .unwrap_or(Value::Null);
    let user_query = workflow
        .get("user_query")
        .and_then(Value::as_str)
        .unwrap_or("");
    let config_raw = workflow
        .get("agent_config")
        .and_then(Value::as_str)
        .unwrap_or("");
    let config = parse_agent_config(input.snapshot).unwrap_or(Value::Null);

    let hash_or_null = |domain: &str, value: &Value| -> Value {
        if value.is_null() {
            Value::Null
        } else {
            Value::String(canonical_hash(domain, value))
        }
    };

    json!({
        "prompt_hash": domain_hash("cs-artifact:prompt", user_query.as_bytes()),
        "config_hash": if config_raw.is_empty() { Value::Null } else { Value::String(domain_hash("cs-artifact:config", config_raw.as_bytes())) },
        "model_hash": hash_or_null("cs-artifact:model", config.get("models").unwrap_or(&Value::Null)),
        "tool_hash": hash_or_null("cs-artifact:tool", config.get("availableTools").unwrap_or(&Value::Null)),
        "policy_hash": hash_or_null("cs-artifact:policy", config.get("shellPolicy").unwrap_or(&Value::Null)),
        "workspace_hash": hash_or_null("cs-artifact:workspace", config.get("allowedPaths").unwrap_or(&Value::Null)),
    })
}

// ---------------------------------------------------------------------------
// Bundle construction
// ---------------------------------------------------------------------------

/// A fully built, in-memory artifact bundle: each field is the exact file body.
#[derive(Debug)]
pub struct ArtifactBundle {
    pub run_json: String,
    pub snapshot_json: String,
    pub events_jsonl: String,
    pub result_json: String,
    pub manifest_json: String,
}

impl ArtifactBundle {
    /// Files covered by the manifest, as `(relative path, body)` pairs.
    fn files(&self) -> [(&'static str, &str); 4] {
        [
            ("run.json", &self.run_json),
            ("snapshot.json", &self.snapshot_json),
            ("events.jsonl", &self.events_jsonl),
            ("result.json", &self.result_json),
        ]
    }
}

/// Builds the artifact bundle from a capture input. Fails closed on privacy or
/// integrity problems before anything is written to disk.
pub fn construct_bundle(input: &CaptureInput) -> Result<ArtifactBundle, ArtifactError> {
    let (event_lines, chain_head, chain_terminal) =
        build_event_chain(input.session_id, input.events)?;
    let events_jsonl = if event_lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", event_lines.join("\n"))
    };

    // The durable event chain is the authoritative terminal source: a terminal
    // status is reported only when a terminal durable event was captured. A
    // possibly-stale snapshot status never fabricates a terminal state.
    let terminal_status = chain_terminal.unwrap_or(TerminalStatus::Unknown);
    // A `complete` artifact requires an actual terminal durable event (the
    // authoritative terminal transition). A stale/nonterminal snapshot with no
    // captured terminal event is `incomplete`, never a fabricated completion.
    let artifact_status = match chain_terminal {
        Some(_) => ArtifactStatus::Complete,
        None => ArtifactStatus::Incomplete,
    };

    let (usage_projection, cost_status) = project_usage(input.events, "event_data");
    let workflow = input
        .snapshot
        .get("workflow")
        .ok_or_else(|| ArtifactError::new(code::INVALID, "snapshot is missing workflow"))?;
    let snapshot_id = workflow.get("id").and_then(Value::as_str).unwrap_or("");
    if snapshot_id != input.session_id {
        return Err(ArtifactError::new(
            code::SESSION,
            "snapshot workflow id does not match session_id",
        ));
    }

    let snapshot_json = canonical_pretty(&project_snapshot(input));
    let hashes = project_hashes(input);

    let run_json = canonical_pretty(&json!({
        "schema_version": SCHEMA_VERSION,
        "artifact_kind": ARTIFACT_KIND,
        "run_id": input.session_id,
        "capture_timestamp": input.capture_timestamp,
        "protocol_version": input.protocol_version,
        "server_instance_id": input.server_instance_id,
        "agent_id": input.agent_id,
        "session_id": input.session_id,
        "artifact_status": artifact_status.as_str(),
        "terminal_status": terminal_status.as_str(),
        "provenance": ["control_plane_snapshot", "control_plane_durable_event", "derived_redaction"],
        "hashes": hashes,
    }));

    let result_json = canonical_pretty(&json!({
        "schema_version": SCHEMA_VERSION,
        "run_id": input.session_id,
        "session_id": input.session_id,
        "artifact_status": artifact_status.as_str(),
        "terminal_status": terminal_status.as_str(),
        "usage": usage_projection,
        "cost_status": cost_status,
        "correctness_status": "not_evaluated",
        "promotion_status": "not_applicable",
        "verification": {
            "event_count": event_lines.len(),
            "chain_head": chain_head,
            "algorithm": HASH_ALGORITHM,
        },
    }));

    let bundle = ArtifactBundle {
        run_json,
        snapshot_json,
        events_jsonl,
        result_json,
        manifest_json: String::new(),
    };
    let manifest_json = build_manifest(&bundle);
    Ok(ArtifactBundle {
        manifest_json,
        ..bundle
    })
}

fn canonical_pretty(value: &Value) -> String {
    let mut buffer = String::new();
    canonical_json(value, &mut buffer);
    // Emit canonical (sorted-key) JSON with a trailing newline for readability.
    let parsed: Value = serde_json::from_str(&buffer).unwrap_or_else(|_| value.clone());
    serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| buffer)
}

/// Builds the manifest covering the four data files (never the manifest itself).
fn build_manifest(bundle: &ArtifactBundle) -> String {
    let files: Vec<Value> = bundle
        .files()
        .iter()
        .map(|(path, body)| {
            json!({
                "path": path,
                "size": body.len() as u64,
                "sha256": blob_hash(body.as_bytes()),
            })
        })
        .collect();
    let files_value = Value::Array(files);
    let manifest_hash = canonical_hash("cs-artifact:manifest", &files_value);
    let manifest = json!({
        "schema_version": SCHEMA_VERSION,
        "algorithm": HASH_ALGORITHM,
        "status": "complete",
        "files": files_value,
        "manifest_hash": manifest_hash,
    });
    serde_json::to_string_pretty(&manifest).unwrap_or_else(|_| "{}".to_string())
}

// ---------------------------------------------------------------------------
// Atomic writer
// ---------------------------------------------------------------------------

/// Rejects unsafe targets and writes the bundle via staging + atomic rename.
///
/// On any failure the staging directory is removed and the final path is never
/// created, so a partial bundle cannot be mistaken for a valid artifact.
pub fn write_bundle(bundle: &ArtifactBundle, final_dir: &Path) -> Result<(), ArtifactError> {
    let total_bytes: u64 = bundle
        .files()
        .iter()
        .map(|(_, body)| body.len() as u64)
        .sum::<u64>()
        + bundle.manifest_json.len() as u64;
    if total_bytes > MAX_TOTAL_BYTES {
        return Err(ArtifactError::new(
            code::LIMIT,
            format!(
                "artifact total size {} bytes exceeds limit {}",
                total_bytes, MAX_TOTAL_BYTES
            ),
        ));
    }
    if final_dir.symlink_metadata().is_ok() {
        return Err(ArtifactError::new(
            code::TARGET_EXISTS,
            format!("artifact target already exists: {}", final_dir.display()),
        ));
    }
    let parent = final_dir
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    if !parent.exists() {
        fs::create_dir_all(&parent).map_err(|e| {
            ArtifactError::new(
                code::IO,
                format!("cannot create parent {}: {}", parent.display(), e),
            )
        })?;
    }

    let name = final_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("artifact");
    let staging = parent.join(format!(".{}.staging-{}", name, uuid::Uuid::new_v4()));
    fs::create_dir_all(staging.join("artifacts"))
        .map_err(|e| ArtifactError::new(code::IO, format!("cannot create staging: {}", e)))?;

    let write_result = (|| -> Result<(), ArtifactError> {
        write_file(&staging.join("run.json"), bundle.run_json.as_bytes())?;
        write_file(
            &staging.join("snapshot.json"),
            bundle.snapshot_json.as_bytes(),
        )?;
        write_file(
            &staging.join("events.jsonl"),
            bundle.events_jsonl.as_bytes(),
        )?;
        write_file(&staging.join("result.json"), bundle.result_json.as_bytes())?;
        write_file(
            &staging.join("artifacts").join("manifest.json"),
            bundle.manifest_json.as_bytes(),
        )?;
        // Re-verify the staged bundle before publishing.
        verify_bundle_dir(&staging)?;
        Ok(())
    })();

    if let Err(error) = write_result {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }

    fs::rename(&staging, final_dir).map_err(|e| {
        let _ = fs::remove_dir_all(&staging);
        ArtifactError::new(
            code::IO,
            format!("atomic rename failed for {}: {}", final_dir.display(), e),
        )
    })
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), ArtifactError> {
    let mut file = fs::File::create(path).map_err(|e| {
        ArtifactError::new(code::IO, format!("cannot write {}: {}", path.display(), e))
    })?;
    file.write_all(bytes).map_err(|e| {
        ArtifactError::new(code::IO, format!("write failed {}: {}", path.display(), e))
    })?;
    file.flush().map_err(|e| {
        ArtifactError::new(code::IO, format!("flush failed {}: {}", path.display(), e))
    })?;
    file.sync_all().map_err(|e| {
        ArtifactError::new(code::IO, format!("sync failed {}: {}", path.display(), e))
    })?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Offline verifier
// ---------------------------------------------------------------------------

/// Result of verifying a bundle on disk.
#[derive(Debug)]
pub struct VerifyReport {
    pub status: ArtifactStatus,
    pub terminal_status: TerminalStatus,
    pub cost_status: String,
    pub run: Value,
    pub events: Vec<Value>,
    pub result: Value,
    pub event_count: usize,
    pub chain_head: String,
}

/// Strictly parses an artifact_status claim; anything but the two valid values
/// (including missing/`invalid`/`unknown`) fails closed.
fn parse_artifact_status(value: Option<&str>) -> Result<ArtifactStatus, ArtifactError> {
    match value {
        Some("complete") => Ok(ArtifactStatus::Complete),
        Some("incomplete") => Ok(ArtifactStatus::Incomplete),
        other => Err(ArtifactError::new(
            code::INVALID,
            format!("invalid or missing artifact_status: {:?}", other),
        )),
    }
}

/// Strictly parses a terminal_status claim; only the four valid values are
/// accepted, everything else fails closed.
fn parse_terminal_status(value: Option<&str>) -> Result<TerminalStatus, ArtifactError> {
    match value {
        Some("completed") => Ok(TerminalStatus::Completed),
        Some("failed") => Ok(TerminalStatus::Failed),
        Some("cancelled") => Ok(TerminalStatus::Cancelled),
        Some("unknown") => Ok(TerminalStatus::Unknown),
        other => Err(ArtifactError::new(
            code::INVALID,
            format!("invalid or missing terminal_status: {:?}", other),
        )),
    }
}

/// Reads and verifies an artifact directory offline. Never touches the network,
/// database, or runtime. Returns `Ok(report)` for `complete`/`incomplete` and
/// `Err` for `invalid`/`incompatible` so callers can map to a non-zero exit.
pub fn verify_bundle_dir(dir: &Path) -> Result<VerifyReport, ArtifactError> {
    if dir
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(ArtifactError::new(
            code::SYMLINK,
            "artifact directory is a symlink",
        ));
    }
    verify_input_size(dir)?;
    let run = read_json_file(dir, "run.json", MAX_TOTAL_BYTES)?;
    let snapshot = read_json_file(dir, "snapshot.json", MAX_TOTAL_BYTES)?;
    let result = read_json_file(dir, "result.json", MAX_TOTAL_BYTES)?;
    let manifest = read_json_file(dir, "artifacts/manifest.json", MAX_MANIFEST_BYTES)?;
    let events_raw = read_text_file(dir, "events.jsonl", MAX_TOTAL_BYTES)?;

    // Schema gate first: an unknown future version is `incompatible`, not `invalid`.
    let schema_version = run
        .get("schema_version")
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    if schema_version > SUPPORTED_SCHEMA_VERSION {
        return Err(ArtifactError::new(
            code::INCOMPATIBLE,
            format!(
                "artifact schema_version {} exceeds supported {}",
                schema_version, SUPPORTED_SCHEMA_VERSION
            ),
        ));
    }
    if schema_version == 0 {
        return Err(ArtifactError::new(
            code::INCOMPATIBLE,
            "artifact schema_version missing",
        ));
    }

    // Manifest integrity: recompute each file hash and the manifest hash.
    verify_manifest(&manifest, dir)?;

    // Session binding.
    let session_id = run.get("session_id").and_then(Value::as_str).unwrap_or("");
    let run_id = run.get("run_id").and_then(Value::as_str).unwrap_or("");
    if session_id != run_id {
        return Err(ArtifactError::new(
            code::SESSION,
            "run_id does not match session_id",
        ));
    }
    if snapshot
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        != session_id
    {
        return Err(ArtifactError::new(
            code::SESSION,
            "snapshot session_id mismatch",
        ));
    }
    if snapshot
        .get("workflow_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        != session_id
    {
        return Err(ArtifactError::new(
            code::SESSION,
            "snapshot workflow_id mismatch",
        ));
    }
    if snapshot
        .get("agent_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        != run.get("agent_id").and_then(Value::as_str).unwrap_or("")
    {
        return Err(ArtifactError::new(
            code::SESSION,
            "snapshot agent_id does not match run agent_id",
        ));
    }
    if result
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        != session_id
    {
        return Err(ArtifactError::new(
            code::SESSION,
            "result session_id mismatch",
        ));
    }

    // Event chain verification.
    let events = parse_events(&events_raw)?;
    let chain_head = verify_chain(session_id, &events)?;

    // Result cross-binding: the recorded chain head must match the recomputed one.
    let recorded_head = result
        .get("verification")
        .and_then(|v| v.get("chain_head"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if recorded_head != chain_head {
        return Err(ArtifactError::new(
            code::CHAIN,
            "result chain_head does not match recomputed chain",
        ));
    }
    let recorded_count = result
        .get("verification")
        .and_then(|v| v.get("event_count"))
        .and_then(Value::as_u64)
        .unwrap_or(u64::MAX) as usize;
    if recorded_count != events.len() {
        return Err(ArtifactError::new(
            code::CHAIN,
            "result event_count does not match events.jsonl",
        ));
    }

    // Authoritative terminal/artifact status is derived from the *verified*
    // durable event chain, never trusted from the (tamperable) result/run claims.
    let derived_terminal = events.iter().rev().find_map(|event| {
        terminal_status_from_event(
            event
                .get("event_type")
                .and_then(Value::as_str)
                .unwrap_or(""),
        )
    });
    let expected_terminal = derived_terminal.unwrap_or(TerminalStatus::Unknown);
    let expected_artifact = if derived_terminal.is_some() {
        ArtifactStatus::Complete
    } else {
        ArtifactStatus::Incomplete
    };

    // Strictly parse the status claims; missing/unknown/invalid values fail closed.
    let run_artifact = parse_artifact_status(run.get("artifact_status").and_then(Value::as_str))?;
    let result_artifact =
        parse_artifact_status(result.get("artifact_status").and_then(Value::as_str))?;
    let run_terminal = parse_terminal_status(run.get("terminal_status").and_then(Value::as_str))?;
    let result_terminal =
        parse_terminal_status(result.get("terminal_status").and_then(Value::as_str))?;

    // run.json and result.json must agree with each other...
    if run_artifact != result_artifact {
        return Err(ArtifactError::new(
            code::INVALID,
            "run.json and result.json artifact_status disagree",
        ));
    }
    if run_terminal != result_terminal {
        return Err(ArtifactError::new(
            code::INVALID,
            "run.json and result.json terminal_status disagree",
        ));
    }
    // ...and both must match the status derived from the verified event chain.
    if result_terminal != expected_terminal {
        return Err(ArtifactError::new(
            code::INVALID,
            format!(
                "terminal_status '{}' does not match the durable event chain ('{}')",
                result_terminal.as_str(),
                expected_terminal.as_str()
            ),
        ));
    }
    if result_artifact != expected_artifact {
        return Err(ArtifactError::new(
            code::INVALID,
            format!(
                "artifact_status '{}' is inconsistent with the durable event chain (expected '{}')",
                result_artifact.as_str(),
                expected_artifact.as_str()
            ),
        ));
    }

    let cost_status = match result.get("cost_status").and_then(Value::as_str) {
        Some(value @ ("known" | "unknown")) => value.to_string(),
        other => {
            return Err(ArtifactError::new(
                code::INVALID,
                format!("invalid or missing cost_status: {:?}", other),
            ))
        }
    };
    let (expected_usage, expected_cost_status) = project_usage(&events, "data");
    if result.get("usage").cloned().unwrap_or(Value::Null) != expected_usage
        || cost_status != expected_cost_status
    {
        return Err(ArtifactError::new(
            code::INVALID,
            "result usage or cost_status does not match the durable task_completed event",
        ));
    }
    if result.get("correctness_status").and_then(Value::as_str) != Some("not_evaluated")
        || result.get("promotion_status").and_then(Value::as_str) != Some("not_applicable")
    {
        return Err(ArtifactError::new(
            code::INVALID,
            "artifact contains an unsupported correctness or promotion status",
        ));
    }

    let status = result_artifact;
    let terminal_status = result_terminal;

    let event_count = events.len();
    Ok(VerifyReport {
        status,
        terminal_status,
        cost_status,
        run,
        events,
        result,
        event_count,
        chain_head,
    })
}

fn read_json_file(dir: &Path, relative: &str, max_bytes: u64) -> Result<Value, ArtifactError> {
    let text = read_text_file(dir, relative, max_bytes)?;
    serde_json::from_str(&text).map_err(|e| {
        ArtifactError::new(
            code::INVALID,
            format!("{} is not valid JSON: {}", relative, e),
        )
    })
}

fn read_text_file(dir: &Path, relative: &str, max_bytes: u64) -> Result<String, ArtifactError> {
    let path = dir.join(relative);
    let meta = path.symlink_metadata().map_err(|_| {
        ArtifactError::new(code::MISSING_FILE, format!("missing file {}", relative))
    })?;
    if meta.file_type().is_symlink() {
        return Err(ArtifactError::new(
            code::SYMLINK,
            format!("{} is a symlink", relative),
        ));
    }
    if !meta.file_type().is_file() {
        return Err(ArtifactError::new(
            code::INVALID,
            format!("{} is not a regular file", relative),
        ));
    }
    if meta.len() > max_bytes {
        return Err(ArtifactError::new(
            code::LIMIT,
            format!("{} exceeds {} bytes", relative, max_bytes),
        ));
    }
    let mut file = fs::File::open(&path).map_err(|e| {
        ArtifactError::new(
            code::MISSING_FILE,
            format!("cannot read {}: {}", relative, e),
        )
    })?;
    let mut text = String::with_capacity(meta.len() as usize);
    file.read_to_string(&mut text).map_err(|e| {
        ArtifactError::new(code::INVALID, format!("cannot decode {}: {}", relative, e))
    })?;
    Ok(text)
}

fn verify_manifest(manifest: &Value, dir: &Path) -> Result<(), ArtifactError> {
    let files = manifest
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| ArtifactError::new(code::MANIFEST, "manifest has no files array"))?;

    // Enforce the exact v1 layout and confine every path to a known relative
    // name. This rejects absolute paths, `..` traversal, duplicates, extras,
    // and any attempt to drop a required file from integrity coverage.
    let mut seen: Vec<&str> = Vec::with_capacity(files.len());
    for entry in files {
        let path = entry.get("path").and_then(Value::as_str).unwrap_or("");
        if !REQUIRED_FILES.contains(&path) {
            return Err(ArtifactError::new(
                code::MANIFEST,
                format!("manifest path is not an allowed v1 file: {:?}", path),
            ));
        }
        if seen.contains(&path) {
            return Err(ArtifactError::new(
                code::MANIFEST,
                format!("duplicate manifest path: {}", path),
            ));
        }
        seen.push(path);
    }
    for required in REQUIRED_FILES {
        if !seen.contains(&required) {
            return Err(ArtifactError::new(
                code::MANIFEST,
                format!("manifest is missing required file: {}", required),
            ));
        }
    }

    let files_value = Value::Array(files.clone());
    let expected_hash = canonical_hash("cs-artifact:manifest", &files_value);
    if manifest
        .get("manifest_hash")
        .and_then(Value::as_str)
        .unwrap_or("")
        != expected_hash
    {
        return Err(ArtifactError::new(
            code::MANIFEST,
            "manifest_hash does not match files list",
        ));
    }
    for entry in files {
        let path = entry.get("path").and_then(Value::as_str).unwrap_or("");
        let expected_sha = entry.get("sha256").and_then(Value::as_str).unwrap_or("");
        let expected_size = entry
            .get("size")
            .and_then(Value::as_u64)
            .unwrap_or(u64::MAX);
        let bytes = fs::read(dir.join(path)).map_err(|_| {
            ArtifactError::new(
                code::MISSING_FILE,
                format!("manifest file missing: {}", path),
            )
        })?;
        if bytes.len() as u64 != expected_size {
            return Err(ArtifactError::new(
                code::HASH,
                format!("size mismatch for {}", path),
            ));
        }
        if blob_hash(&bytes) != expected_sha {
            return Err(ArtifactError::new(
                code::HASH,
                format!("hash mismatch for {}", path),
            ));
        }
    }
    Ok(())
}

fn verify_input_size(dir: &Path) -> Result<(), ArtifactError> {
    let mut total = 0u64;
    for relative in REQUIRED_FILES {
        let path = dir.join(relative);
        let meta = path.symlink_metadata().map_err(|_| {
            ArtifactError::new(code::MISSING_FILE, format!("missing file {}", relative))
        })?;
        if meta.file_type().is_symlink() {
            return Err(ArtifactError::new(
                code::SYMLINK,
                format!("{} is a symlink", relative),
            ));
        }
        if !meta.file_type().is_file() {
            return Err(ArtifactError::new(
                code::INVALID,
                format!("{} is not a regular file", relative),
            ));
        }
        total = total.saturating_add(meta.len());
        if total > MAX_TOTAL_BYTES {
            return Err(ArtifactError::new(
                code::LIMIT,
                format!("artifact files exceed {} bytes", MAX_TOTAL_BYTES),
            ));
        }
    }
    Ok(())
}

fn parse_events(raw: &str) -> Result<Vec<Value>, ArtifactError> {
    let mut events = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.len() > MAX_EVENT_LINE_BYTES {
            return Err(ArtifactError::new(
                code::LIMIT,
                format!("event line exceeds {} bytes", MAX_EVENT_LINE_BYTES),
            ));
        }
        if events.len() >= MAX_EVENTS {
            return Err(ArtifactError::new(
                code::LIMIT,
                format!("event count exceeds limit {}", MAX_EVENTS),
            ));
        }
        let value: Value = serde_json::from_str(line)
            .map_err(|e| ArtifactError::new(code::INVALID, format!("bad event line: {}", e)))?;
        events.push(value);
    }
    Ok(events)
}

/// Recomputes the chain and returns the head hash, failing on any break.
fn verify_chain(session_id: &str, events: &[Value]) -> Result<String, ArtifactError> {
    let mut prev_hash = GENESIS_HASH.to_string();
    let mut last_id: Option<i64> = None;
    for event in events {
        let durable_id_str = event
            .get("durable_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        let durable_id: i64 = durable_id_str.parse().map_err(|_| {
            ArtifactError::new(
                code::INVALID,
                format!("durable_id '{}' is not an integer", durable_id_str),
            )
        })?;
        if let Some(last) = last_id {
            if durable_id <= last {
                return Err(ArtifactError::new(
                    code::CHAIN,
                    "durable ids are not strictly increasing",
                ));
            }
        }
        last_id = Some(durable_id);

        if event
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            != session_id
        {
            return Err(ArtifactError::new(
                code::SESSION,
                "event session_id mismatch in chain",
            ));
        }
        if event.get("provenance").and_then(Value::as_str) != Some("workflow_runtime") {
            return Err(ArtifactError::new(
                code::INVALID,
                "event provenance is not workflow_runtime",
            ));
        }

        let data = event.get("data").cloned().unwrap_or(Value::Null);
        let mut canonical = String::new();
        canonical_json(&data, &mut canonical);
        let payload_hash = domain_hash("cs-artifact:payload", canonical.as_bytes());
        if event
            .get("payload_hash")
            .and_then(Value::as_str)
            .unwrap_or("")
            != payload_hash
        {
            return Err(ArtifactError::new(
                code::HASH,
                format!("payload_hash mismatch at event {}", durable_id),
            ));
        }
        if event.get("prev_hash").and_then(Value::as_str).unwrap_or("") != prev_hash {
            return Err(ArtifactError::new(
                code::CHAIN,
                format!("prev_hash mismatch at event {}", durable_id),
            ));
        }
        let record_hash = canonical_hash(
            "cs-artifact:event",
            &json!({
                "durable_id": durable_id_str,
                "session_id": event.get("session_id").and_then(Value::as_str).unwrap_or(""),
                "event_version": event.get("event_version").and_then(Value::as_str).unwrap_or(""),
                "payload_hash": payload_hash,
                "prev_hash": prev_hash,
            }),
        );
        if event
            .get("record_hash")
            .and_then(Value::as_str)
            .unwrap_or("")
            != record_hash
        {
            return Err(ArtifactError::new(
                code::HASH,
                format!("record_hash mismatch at event {}", durable_id),
            ));
        }
        prev_hash = record_hash;
    }
    Ok(prev_hash)
}

// ---------------------------------------------------------------------------
// Structured projections for inspect / replay output
// ---------------------------------------------------------------------------

/// Builds the `inspect` JSON projection (integrity + status summary).
pub fn inspect_projection(report: &VerifyReport) -> Value {
    json!({
        "schema_version": report.run.get("schema_version").and_then(Value::as_u64).unwrap_or(0),
        "artifact_kind": report.run.get("artifact_kind").and_then(Value::as_str),
        "run_id": report.run.get("run_id").and_then(Value::as_str),
        "session_id": report.run.get("session_id").and_then(Value::as_str),
        "agent_id": report.run.get("agent_id").and_then(Value::as_str),
        "server_instance_id": report.run.get("server_instance_id").and_then(Value::as_str),
        "protocol_version": report.run.get("protocol_version").and_then(Value::as_str),
        "capture_timestamp": report.run.get("capture_timestamp").and_then(Value::as_str),
        "artifact_status": report.status.as_str(),
        "terminal_status": report.terminal_status.as_str(),
        "cost_status": report.cost_status,
        "correctness_status": report.result.get("correctness_status").and_then(Value::as_str),
        "promotion_status": report.result.get("promotion_status").and_then(Value::as_str),
        "event_count": report.event_count,
        "chain_head": report.chain_head,
        "hashes": report.run.get("hashes").cloned().unwrap_or(Value::Null),
        "usage": report.result.get("usage").cloned().unwrap_or(Value::Null),
        "integrity": {
            "manifest_verified": true,
            "chain_verified": true,
            "session_bound": true,
        },
    })
}

/// Builds the `replay` JSON projection (timeline + status + usage only).
pub fn replay_projection(report: &VerifyReport) -> Value {
    let timeline: Vec<Value> = report
        .events
        .iter()
        .map(|event| {
            json!({
                "durable_id": event.get("durable_id").and_then(Value::as_str),
                "event_type": event.get("event_type").and_then(Value::as_str),
                "event_version": event.get("event_version").and_then(Value::as_str),
                "created_at": event.get("created_at").and_then(Value::as_str),
            })
        })
        .collect();
    json!({
        "run_id": report.run.get("run_id").and_then(Value::as_str),
        "session_id": report.run.get("session_id").and_then(Value::as_str),
        "artifact_status": report.status.as_str(),
        "terminal_status": report.terminal_status.as_str(),
        "cost_status": report.cost_status,
        "correctness_status": report.result.get("correctness_status").and_then(Value::as_str),
        "promotion_status": report.result.get("promotion_status").and_then(Value::as_str),
        "usage": report.result.get("usage").cloned().unwrap_or(Value::Null),
        "timeline": timeline,
    })
}

/// Builds a structured error projection for a failed verification.
pub fn error_projection(dir: &Path, error: &ArtifactError) -> Value {
    json!({
        "artifact_dir": dir.display().to_string(),
        "artifact_status": if error.code == code::INCOMPATIBLE { "incompatible" } else { "invalid" },
        "code": error.code,
        "message": error.message,
        "correctness_status": "not_evaluated",
        "promotion_status": "not_applicable",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: &str = "sess-abc";

    fn priced_usage() -> Value {
        json!({
            "version": 1,
            "terminal_status": "completed",
            "duration_ms": 1234,
            "is_partial": false,
            "has_sub_agents": false,
            "self_usage": {
                "input_tokens": 100, "output_tokens": 50, "cache_tokens": 0,
                "cache_write_tokens": 0, "reasoning_tokens": 0, "total_tokens": 150,
                "estimated_cost": 0.0012, "effective_cost_per_million": 8.0, "unpriced_tokens": 0
            },
            "with_sub_agents": {
                "input_tokens": 100, "output_tokens": 50, "cache_tokens": 0,
                "cache_write_tokens": 0, "reasoning_tokens": 0, "total_tokens": 150,
                "estimated_cost": 0.0012, "effective_cost_per_million": 8.0, "unpriced_tokens": 0
            },
            "model_breakdowns": [{
                "provider_id": 0, "backend_model": "free:ds-v4-flash",
                "input_tokens": 100, "output_tokens": 50, "total_tokens": 150,
                "pricing_status": "priced", "estimated_cost": 0.0012
            }]
        })
    }

    fn sample_events(usage: Option<Value>) -> Vec<Value> {
        let mut events = vec![
            json!({
                "id": 1, "session_id": SESSION, "event_type": "workflow_started",
                "event_version": "1.0.0", "created_at": "2026-01-01T00:00:00Z",
                "event_data": { "agent_id": "builtin:coding" }
            }),
            json!({
                "id": 2, "session_id": SESSION, "event_type": "tool_completed",
                "event_version": "1.0.0", "created_at": "2026-01-01T00:00:01Z",
                "event_data": {
                    "tool_name": "bash",
                    "arguments": { "command": "echo hello world", "cwd": "/home/xc/secret/dir" },
                    "result": "the raw tool output text that must not be stored verbatim",
                    "api_key": "sk-supersecretvalue123",
                    "Authorization": "Bearer abc.def.ghi"
                }
            }),
        ];
        if let Some(usage) = usage {
            events.push(json!({
                "id": 3, "session_id": SESSION, "event_type": "task_completed",
                "event_version": "1.0.0", "created_at": "2026-01-01T00:00:02Z",
                "event_data": { "terminal": true, "usage_summary": usage }
            }));
        }
        events
    }

    fn sample_snapshot(status: &str) -> Value {
        json!({
            "workflow": {
                "id": SESSION,
                "agent_id": "builtin:coding",
                "status": status,
                "wait_reason": null,
                "user_query": "a private prompt that must never be written to disk",
                "agent_config": "{\"models\":{\"act\":{\"id\":0,\"model\":\"cs@free:ds-v4-flash\"}},\"availableTools\":[\"bash\"],\"allowedPaths\":[\"/home/xc/private/workspace\"],\"shellPolicy\":[]}",
                "is_automation_run": false
            },
            "messages": [{ "role": "user", "message": "raw message body" }],
            "has_live_session": false,
            "hidden_earlier_message_count": 0
        })
    }

    fn capture_input<'a>(snapshot: &'a Value, events: &'a [Value]) -> CaptureInput<'a> {
        CaptureInput {
            session_id: SESSION,
            agent_id: "builtin:coding",
            server_instance_id: "inst-1",
            protocol_version: "1.0",
            capture_timestamp: "2026-01-01T00:00:03Z",
            snapshot,
            events,
        }
    }

    fn write_to_temp(bundle: &ArtifactBundle) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("artifact");
        write_bundle(bundle, &target).expect("write bundle");
        dir
    }

    #[test]
    fn canonical_json_is_key_order_independent() {
        let a = json!({ "b": 2, "a": { "d": 4, "c": 3 } });
        let b = json!({ "a": { "c": 3, "d": 4 }, "b": 2 });
        let mut sa = String::new();
        let mut sb = String::new();
        canonical_json(&a, &mut sa);
        canonical_json(&b, &mut sb);
        assert_eq!(sa, sb);
        assert_eq!(sa, r#"{"a":{"c":3,"d":4},"b":2}"#);
    }

    #[test]
    fn canonical_json_preserves_array_order() {
        let mut s = String::new();
        canonical_json(&json!([3, 1, 2]), &mut s);
        assert_eq!(s, "[3,1,2]");
    }

    #[test]
    fn domain_hash_is_deterministic_and_domain_separated() {
        assert_eq!(domain_hash("x", b"abc"), domain_hash("x", b"abc"));
        assert_ne!(domain_hash("x", b"abc"), domain_hash("y", b"abc"));
    }

    #[test]
    fn redaction_drops_secrets_and_projects_free_text_and_paths() {
        let value = json!({
            "tool_name": "bash",
            "content": "verbatim assistant text",
            "path": "/home/xc/private/file",
            "api_key": "sk-abcdef",
            "Authorization": "Bearer zzz",
            "count": 7,
            "flag": true
        });
        let red = redact_value("event_data", &value);
        // safe identifier kept
        assert_eq!(red["tool_name"], json!("bash"));
        // numbers/bools kept
        assert_eq!(red["count"], json!(7));
        assert_eq!(red["flag"], json!(true));
        // free text -> projection, no raw text
        assert_eq!(red["content"]["type"], json!("string"));
        assert!(red["content"].get("sha256").is_some());
        assert!(!red.to_string().contains("verbatim"));
        // path -> hash projection, no raw path
        assert_eq!(red["path"]["type"], json!("path"));
        assert!(!red.to_string().contains("/home/xc/private/file"));
        // secret keys -> marker
        assert_eq!(red["api_key"], json!(SECRET_MARKER));
        assert_eq!(red["Authorization"], json!(SECRET_MARKER));
    }

    #[test]
    fn secret_value_under_nonsecret_key_is_marked() {
        let red = redact_value("note", &json!("sk-live-secrettoken"));
        assert_eq!(red, json!(SECRET_MARKER));
    }

    #[test]
    fn complete_round_trip_verifies() {
        let snapshot = sample_snapshot("completed");
        let events = sample_events(Some(priced_usage()));
        let bundle = construct_bundle(&capture_input(&snapshot, &events)).expect("construct");
        // artifact + cost status
        let result: Value = serde_json::from_str(&bundle.result_json).unwrap();
        assert_eq!(result["artifact_status"], json!("complete"));
        assert_eq!(result["cost_status"], json!("known"));
        assert_eq!(result["correctness_status"], json!("not_evaluated"));
        assert_eq!(result["promotion_status"], json!("not_applicable"));

        let dir = write_to_temp(&bundle);
        let report = verify_bundle_dir(&dir.path().join("artifact")).expect("verify");
        assert_eq!(report.status, ArtifactStatus::Complete);
        assert_eq!(report.terminal_status, TerminalStatus::Completed);
        assert_eq!(report.event_count, 3);
        assert_eq!(report.cost_status, "known");
    }

    #[test]
    fn no_raw_prompt_or_secret_in_files() {
        let snapshot = sample_snapshot("completed");
        let events = sample_events(Some(priced_usage()));
        let bundle = construct_bundle(&capture_input(&snapshot, &events)).expect("construct");
        let all = format!(
            "{}{}{}",
            bundle.run_json, bundle.snapshot_json, bundle.events_jsonl
        );
        assert!(!all.contains("a private prompt that must never"));
        assert!(!all.contains("raw message body"));
        assert!(!all.contains("sk-supersecretvalue123"));
        assert!(!all.contains("the raw tool output text"));
        assert!(!all.contains("/home/xc/private/workspace"));
        assert!(!all.contains("Bearer abc.def.ghi"));
    }

    #[test]
    fn nonterminal_run_is_incomplete_not_complete() {
        let snapshot = sample_snapshot("running");
        let events = sample_events(None); // no terminal event
        let bundle = construct_bundle(&capture_input(&snapshot, &events)).expect("construct");
        let result: Value = serde_json::from_str(&bundle.result_json).unwrap();
        assert_eq!(result["artifact_status"], json!("incomplete"));
        // cost unknown when no usage summary
        assert_eq!(result["cost_status"], json!("unknown"));

        let dir = write_to_temp(&bundle);
        let report = verify_bundle_dir(&dir.path().join("artifact")).expect("verify incomplete");
        assert_eq!(report.status, ArtifactStatus::Incomplete);
    }

    #[test]
    fn unpriced_usage_yields_unknown_cost() {
        let mut usage = priced_usage();
        usage["self_usage"]["unpriced_tokens"] = json!(42);
        usage["self_usage"]["estimated_cost"] = Value::Null;
        let snapshot = sample_snapshot("completed");
        let events = sample_events(Some(usage));
        let bundle = construct_bundle(&capture_input(&snapshot, &events)).expect("construct");
        let result: Value = serde_json::from_str(&bundle.result_json).unwrap();
        assert_eq!(result["cost_status"], json!("unknown"));
    }

    #[test]
    fn tampering_event_payload_breaks_hash() {
        let snapshot = sample_snapshot("completed");
        let events = sample_events(Some(priced_usage()));
        let bundle = construct_bundle(&capture_input(&snapshot, &events)).expect("construct");
        let dir = write_to_temp(&bundle);
        let artifact = dir.path().join("artifact");
        let path = artifact.join("events.jsonl");
        let body = fs::read_to_string(&path).unwrap();
        // flip one character inside a projected string
        let tampered = body.replace("\"tool_name\":\"bash\"", "\"tool_name\":\"basX\"");
        assert_ne!(body, tampered);
        fs::write(&path, tampered).unwrap();
        let error = verify_bundle_dir(&artifact).expect_err("must fail");
        assert_eq!(error.code, code::HASH);
    }

    #[test]
    fn deleting_an_event_breaks_the_chain() {
        let snapshot = sample_snapshot("completed");
        let events = sample_events(Some(priced_usage()));
        let bundle = construct_bundle(&capture_input(&snapshot, &events)).expect("construct");
        let dir = write_to_temp(&bundle);
        let artifact = dir.path().join("artifact");
        let path = artifact.join("events.jsonl");
        let lines: Vec<String> = fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        // keep first and last, drop middle -> prev_hash chain breaks
        let kept = format!("{}\n{}\n", lines[0], lines[2]);
        fs::write(&path, kept).unwrap();
        // Refresh the manifest so the file-hash layer passes and the chain
        // layer is the one that must reject the edit.
        refresh_manifest(&artifact);
        let error = verify_bundle_dir(&artifact).expect_err("must fail");
        assert_eq!(error.code, code::CHAIN);
    }

    #[test]
    fn reordering_events_is_rejected() {
        let snapshot = sample_snapshot("completed");
        let events = sample_events(Some(priced_usage()));
        let bundle = construct_bundle(&capture_input(&snapshot, &events)).expect("construct");
        let dir = write_to_temp(&bundle);
        let artifact = dir.path().join("artifact");
        let path = artifact.join("events.jsonl");
        let lines: Vec<String> = fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        // swap first two -> durable ids not strictly increasing
        let swapped = format!("{}\n{}\n{}\n", lines[1], lines[0], lines[2]);
        fs::write(&path, swapped).unwrap();
        refresh_manifest(&artifact);
        let error = verify_bundle_dir(&artifact).expect_err("must fail");
        assert_eq!(error.code, code::CHAIN);
    }

    #[test]
    fn manifest_edit_is_rejected() {
        let snapshot = sample_snapshot("completed");
        let events = sample_events(Some(priced_usage()));
        let bundle = construct_bundle(&capture_input(&snapshot, &events)).expect("construct");
        let dir = write_to_temp(&bundle);
        let artifact = dir.path().join("artifact");
        let manifest_path = artifact.join("artifacts").join("manifest.json");
        let manifest: Value =
            serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
        // change a file size without touching manifest_hash -> manifest_hash mismatch
        let mut edited = manifest.clone();
        edited["files"][0]["size"] = json!(manifest["files"][0]["size"].as_u64().unwrap() + 1);
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&edited).unwrap(),
        )
        .unwrap();
        let error = verify_bundle_dir(&artifact).expect_err("must fail");
        assert_eq!(error.code, code::MANIFEST);
    }

    #[test]
    fn missing_file_is_rejected() {
        let snapshot = sample_snapshot("completed");
        let events = sample_events(Some(priced_usage()));
        let bundle = construct_bundle(&capture_input(&snapshot, &events)).expect("construct");
        let dir = write_to_temp(&bundle);
        let artifact = dir.path().join("artifact");
        fs::remove_file(artifact.join("snapshot.json")).unwrap();
        let error = verify_bundle_dir(&artifact).expect_err("must fail");
        assert_eq!(error.code, code::MISSING_FILE);
    }

    #[test]
    fn incompatible_schema_is_reported() {
        let snapshot = sample_snapshot("completed");
        let events = sample_events(Some(priced_usage()));
        let bundle = construct_bundle(&capture_input(&snapshot, &events)).expect("construct");
        let dir = write_to_temp(&bundle);
        let artifact = dir.path().join("artifact");
        // bump run.json schema_version to a future value and refresh its hash in the manifest
        let run_path = artifact.join("run.json");
        let mut run: Value = serde_json::from_str(&fs::read_to_string(&run_path).unwrap()).unwrap();
        run["schema_version"] = json!(99);
        let run_text = serde_json::to_string_pretty(&run).unwrap();
        fs::write(&run_path, &run_text).unwrap();
        // rewrite manifest so only the schema gate (not the manifest) fails
        let manifest_path = artifact.join("artifacts").join("manifest.json");
        let mut manifest: Value =
            serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
        for entry in manifest["files"].as_array_mut().unwrap() {
            if entry["path"] == json!("run.json") {
                entry["size"] = json!(run_text.len() as u64);
                entry["sha256"] = json!(blob_hash(run_text.as_bytes()));
            }
        }
        let files = manifest["files"].clone();
        manifest["manifest_hash"] = json!(canonical_hash("cs-artifact:manifest", &files));
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let error = verify_bundle_dir(&artifact).expect_err("must fail");
        assert_eq!(error.code, code::INCOMPATIBLE);
    }

    #[test]
    fn session_mixed_event_is_rejected_at_capture() {
        let snapshot = sample_snapshot("completed");
        let mut events = sample_events(Some(priced_usage()));
        events[0]["session_id"] = json!("other-session");
        let error = construct_bundle(&capture_input(&snapshot, &events)).expect_err("must fail");
        assert_eq!(error.code, code::SESSION);
    }

    #[test]
    fn complete_claim_without_terminal_event_is_rejected() {
        // Manually craft a bundle that claims complete but has no terminal event.
        let snapshot = sample_snapshot("completed");
        let events = sample_events(Some(priced_usage()));
        let bundle = construct_bundle(&capture_input(&snapshot, &events)).expect("construct");
        let dir = write_to_temp(&bundle);
        let artifact = dir.path().join("artifact");
        // Rewrite events.jsonl to drop the task_completed line, then fix result + manifest.
        let events_path = artifact.join("events.jsonl");
        let lines: Vec<String> = fs::read_to_string(&events_path)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        let dropped = format!("{}\n{}\n", lines[0], lines[1]);
        fs::write(&events_path, &dropped).unwrap();

        let result_path = artifact.join("result.json");
        let mut result: Value =
            serde_json::from_str(&fs::read_to_string(&result_path).unwrap()).unwrap();
        result["verification"]["event_count"] = json!(2);
        let result_text = serde_json::to_string_pretty(&result).unwrap();
        fs::write(&result_path, &result_text).unwrap();

        refresh_manifest(&artifact);

        let error = verify_bundle_dir(&artifact).expect_err("must fail");
        // The recorded chain_head still reflects the 3-event chain, so the
        // chain cross-binding rejects it before the status check is reached.
        assert_eq!(error.code, code::CHAIN);
    }

    /// Recomputes file hashes + manifest_hash so only the intended check fails.
    fn refresh_manifest(artifact: &Path) {
        let manifest_path = artifact.join("artifacts").join("manifest.json");
        let mut manifest: Value =
            serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
        for entry in manifest["files"].as_array_mut().unwrap() {
            let rel = entry["path"].as_str().unwrap().to_string();
            let bytes = fs::read(artifact.join(&rel)).unwrap();
            entry["size"] = json!(bytes.len() as u64);
            entry["sha256"] = json!(blob_hash(&bytes));
        }
        let files = manifest["files"].clone();
        manifest["manifest_hash"] = json!(canonical_hash("cs-artifact:manifest", &files));
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn existing_target_is_rejected() {
        let snapshot = sample_snapshot("completed");
        let events = sample_events(Some(priced_usage()));
        let bundle = construct_bundle(&capture_input(&snapshot, &events)).expect("construct");
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("artifact");
        fs::create_dir(&target).unwrap();
        let error = write_bundle(&bundle, &target).expect_err("must fail");
        assert_eq!(error.code, code::TARGET_EXISTS);
    }

    #[test]
    fn symlink_target_is_rejected() {
        let snapshot = sample_snapshot("completed");
        let events = sample_events(Some(priced_usage()));
        let bundle = construct_bundle(&capture_input(&snapshot, &events)).expect("construct");
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        fs::create_dir(&real).unwrap();
        #[cfg(unix)]
        {
            let link = dir.path().join("artifact");
            std::os::unix::fs::symlink(&real, &link).unwrap();
            let error = write_bundle(&bundle, &link).expect_err("must fail");
            assert_eq!(error.code, code::TARGET_EXISTS);
        }
    }

    #[test]
    fn usage_projection_keeps_numbers_and_drops_nothing_sensitive() {
        let snapshot = sample_snapshot("completed");
        let events = sample_events(Some(priced_usage()));
        let bundle = construct_bundle(&capture_input(&snapshot, &events)).expect("construct");
        let result: Value = serde_json::from_str(&bundle.result_json).unwrap();
        assert_eq!(result["usage"]["self_usage"]["total_tokens"], json!(150));
        assert_eq!(
            result["usage"]["model_breakdowns"][0]["backend_model"],
            json!("free:ds-v4-flash")
        );
        assert_eq!(
            result["usage"]["model_breakdowns"][0]["pricing_status"],
            json!("priced")
        );
    }

    #[test]
    fn sub_agent_unpriced_usage_yields_unknown_cost() {
        let mut usage = priced_usage();
        usage["with_sub_agents"]["unpriced_tokens"] = json!(1);
        usage["with_sub_agents"]["estimated_cost"] = Value::Null;
        let snapshot = sample_snapshot("completed");
        let events = sample_events(Some(usage));
        let bundle = construct_bundle(&capture_input(&snapshot, &events)).expect("construct");
        let result: Value = serde_json::from_str(&bundle.result_json).unwrap();
        assert_eq!(result["cost_status"], json!("unknown"));
    }

    #[test]
    fn result_usage_tampering_is_rejected_after_manifest_refresh() {
        let dir = write_complete_bundle();
        let artifact = dir.path().join("artifact");
        patch_file(&artifact, "result.json", |value| {
            value["usage"]["self_usage"]["total_tokens"] = json!(999);
        });
        refresh_manifest(&artifact);
        let error = verify_bundle_dir(&artifact).expect_err("must fail");
        assert_eq!(error.code, code::INVALID);
    }

    #[test]
    fn snapshot_workflow_id_mismatch_is_rejected_at_capture() {
        let mut snapshot = sample_snapshot("completed");
        snapshot["workflow"]["id"] = json!("other-session");
        let events = sample_events(Some(priced_usage()));
        let error = construct_bundle(&capture_input(&snapshot, &events)).expect_err("must fail");
        assert_eq!(error.code, code::SESSION);
    }

    /// Rewrites the manifest files array and recomputes manifest_hash so only
    /// the layout/structural check can reject it.
    fn write_manifest_files(artifact: &Path, files: Value) {
        let manifest_path = artifact.join("artifacts").join("manifest.json");
        let mut manifest: Value =
            serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
        manifest["files"] = files.clone();
        manifest["manifest_hash"] = json!(canonical_hash("cs-artifact:manifest", &files));
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }

    fn write_complete_bundle() -> tempfile::TempDir {
        let snapshot = sample_snapshot("completed");
        let events = sample_events(Some(priced_usage()));
        let bundle = construct_bundle(&capture_input(&snapshot, &events)).expect("construct");
        write_to_temp(&bundle)
    }

    #[test]
    fn manifest_omitting_required_file_is_rejected() {
        let dir = write_complete_bundle();
        let artifact = dir.path().join("artifact");
        let manifest: Value = serde_json::from_str(
            &fs::read_to_string(artifact.join("artifacts").join("manifest.json")).unwrap(),
        )
        .unwrap();
        // Drop the run.json entry but keep the file on disk and recompute the hash.
        let kept: Vec<Value> = manifest["files"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["path"] != json!("run.json"))
            .cloned()
            .collect();
        write_manifest_files(&artifact, Value::Array(kept));
        let error = verify_bundle_dir(&artifact).expect_err("must fail");
        assert_eq!(error.code, code::MANIFEST);
    }

    #[test]
    fn manifest_absolute_or_traversal_path_is_rejected() {
        let dir = write_complete_bundle();
        let artifact = dir.path().join("artifact");
        let manifest: Value = serde_json::from_str(
            &fs::read_to_string(artifact.join("artifacts").join("manifest.json")).unwrap(),
        )
        .unwrap();
        let mut files = manifest["files"].as_array().unwrap().clone();
        files[0]["path"] = json!("../../etc/passwd");
        write_manifest_files(&artifact, Value::Array(files));
        let error = verify_bundle_dir(&artifact).expect_err("must fail");
        assert_eq!(error.code, code::MANIFEST);
    }

    #[test]
    fn manifest_duplicate_entry_is_rejected() {
        let dir = write_complete_bundle();
        let artifact = dir.path().join("artifact");
        let manifest: Value = serde_json::from_str(
            &fs::read_to_string(artifact.join("artifacts").join("manifest.json")).unwrap(),
        )
        .unwrap();
        let mut files = manifest["files"].as_array().unwrap().clone();
        files.push(files[0].clone());
        write_manifest_files(&artifact, Value::Array(files));
        let error = verify_bundle_dir(&artifact).expect_err("must fail");
        assert_eq!(error.code, code::MANIFEST);
    }

    #[test]
    fn failed_terminal_event_is_not_reported_completed() {
        // Stale/nonterminal snapshot but a durable workflow_failed event: the
        // event is authoritative, so terminal_status must be `failed`.
        let snapshot = sample_snapshot("running");
        let mut events = sample_events(None);
        events.push(json!({
            "id": 3, "session_id": SESSION, "event_type": "workflow_failed",
            "event_version": "1.0.0", "created_at": "t2",
            "event_data": { "reason": "boom" }
        }));
        let bundle = construct_bundle(&capture_input(&snapshot, &events)).expect("construct");
        let result: Value = serde_json::from_str(&bundle.result_json).unwrap();
        assert_eq!(result["terminal_status"], json!("failed"));
        assert_eq!(result["artifact_status"], json!("complete"));

        let dir = write_to_temp(&bundle);
        let report = verify_bundle_dir(&dir.path().join("artifact")).expect("verify");
        assert_eq!(report.terminal_status, TerminalStatus::Failed);
        assert_eq!(report.status, ArtifactStatus::Complete);
    }

    #[test]
    fn cancelled_terminal_event_maps_to_cancelled() {
        let snapshot = sample_snapshot("cancelled");
        let mut events = sample_events(None);
        events.push(json!({
            "id": 3, "session_id": SESSION, "event_type": "workflow_cancelled",
            "event_version": "1.0.0", "created_at": "t2",
            "event_data": {}
        }));
        let bundle = construct_bundle(&capture_input(&snapshot, &events)).expect("construct");
        let result: Value = serde_json::from_str(&bundle.result_json).unwrap();
        assert_eq!(result["terminal_status"], json!("cancelled"));
        assert_eq!(result["artifact_status"], json!("complete"));
    }

    #[test]
    fn stale_completed_snapshot_without_terminal_event_is_incomplete() {
        // Snapshot claims completed but no terminal durable event was captured:
        // must be `incomplete`, never a fabricated completion.
        let snapshot = sample_snapshot("completed");
        let events = sample_events(None);
        let bundle = construct_bundle(&capture_input(&snapshot, &events)).expect("construct");
        let result: Value = serde_json::from_str(&bundle.result_json).unwrap();
        assert_eq!(result["artifact_status"], json!("incomplete"));
    }

    // --- verifier status cross-check hardening ---------------------------

    fn patch_file(artifact: &Path, rel: &str, patch: impl FnOnce(&mut Value)) {
        let path = artifact.join(rel);
        let mut value: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        patch(&mut value);
        fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
    }

    fn write_failed_artifact() -> tempfile::TempDir {
        let snapshot = sample_snapshot("running");
        let mut events = sample_events(None);
        events.push(json!({
            "id": 3, "session_id": SESSION, "event_type": "workflow_failed",
            "event_version": "1.0.0", "created_at": "t2", "event_data": { "reason": "boom" }
        }));
        let bundle = construct_bundle(&capture_input(&snapshot, &events)).expect("construct");
        write_to_temp(&bundle)
    }

    #[test]
    fn verify_derives_terminal_status_from_events() {
        // A failed durable event yields a `failed` report even though the
        // snapshot status was stale.
        let dir = write_failed_artifact();
        let report = verify_bundle_dir(&dir.path().join("artifact")).expect("verify");
        assert_eq!(report.terminal_status, TerminalStatus::Failed);
        assert_eq!(report.status, ArtifactStatus::Complete);
    }

    #[test]
    fn result_terminal_claim_mismatch_is_rejected() {
        // Rewrite both run.json and result.json to claim "completed" while the
        // verified event chain says "failed"; refresh the manifest so only the
        // derived-vs-claimed cross-check can reject it.
        let dir = write_failed_artifact();
        let artifact = dir.path().join("artifact");
        patch_file(&artifact, "run.json", |v| {
            v["terminal_status"] = json!("completed")
        });
        patch_file(&artifact, "result.json", |v| {
            v["terminal_status"] = json!("completed")
        });
        refresh_manifest(&artifact);
        let error = verify_bundle_dir(&artifact).expect_err("must fail");
        assert_eq!(error.code, code::INVALID);
    }

    #[test]
    fn missing_artifact_status_is_rejected() {
        let dir = write_complete_bundle();
        let artifact = dir.path().join("artifact");
        patch_file(&artifact, "result.json", |v| {
            v.as_object_mut().unwrap().remove("artifact_status");
        });
        refresh_manifest(&artifact);
        let error = verify_bundle_dir(&artifact).expect_err("must fail");
        assert_eq!(error.code, code::INVALID);
    }

    #[test]
    fn run_result_status_disagreement_is_rejected() {
        let dir = write_complete_bundle();
        let artifact = dir.path().join("artifact");
        patch_file(&artifact, "run.json", |v| {
            v["artifact_status"] = json!("incomplete")
        });
        refresh_manifest(&artifact);
        let error = verify_bundle_dir(&artifact).expect_err("must fail");
        assert_eq!(error.code, code::INVALID);
    }

    #[test]
    fn invalid_cost_status_is_rejected() {
        let dir = write_complete_bundle();
        let artifact = dir.path().join("artifact");
        patch_file(&artifact, "result.json", |v| {
            v["cost_status"] = json!("free")
        });
        refresh_manifest(&artifact);
        let error = verify_bundle_dir(&artifact).expect_err("must fail");
        assert_eq!(error.code, code::INVALID);
    }
}
