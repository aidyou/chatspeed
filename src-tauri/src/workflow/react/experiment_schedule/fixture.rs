//! Shared `chatspeed-smoke` fixture resolver (Phase 2E contract, 2G+2H owner).
//!
//! The resolver used to live only inside the `cs` CLI. Phase 2G+2H makes the
//! durable campaign scheduler resolve the *same* checked-in fixture inside the
//! backend, because a durable schedule request stores only fixture refs and
//! digests — never the raw instruction. Backend and CLI therefore share this
//! one strict parser and one canonical-hash implementation, so validation can
//! never drift between the two adapters (INV-1/INV-3).
//!
//! The resolver is pure and deterministic: it never opens the database, never
//! runs an executor and never talks to a provider. Its only outputs are the
//! digest-bound [`ResolvedTask`] and the transport-neutral projections built
//! from it.
//!
//! Fixture identity is fixed at compile time via `include_str!`: no runtime
//! path, environment variable, or model output can swap the manifest, the
//! verifier identity, or the resource profile.

use crate::workflow::react::campaign::{canonical_hash, domain_hash};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::OnceLock;

/// The only suite id this adapter accepts.
pub const SUITE_ID: &str = "chatspeed-smoke";
/// Fixed dataset identity (Phase 2E baseline).
pub const DATASET_ID: &str = "chatspeed-smoke";
pub const DATASET_VERSION: u32 = 2;
pub const SPLIT: &str = "smoke";
/// Runner kind recorded in adapter metadata. Real Harbor installed-agent
/// execution is the 2G+2H `HarborTaskOwner` path; this literal stays the
/// local control-plane adapter identity so the frozen 2E digests are stable.
pub const RUNNER_KIND: &str = "local_control_plane";

/// Checked-in fixture files (compiled in; no runtime path).
const MANIFEST_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../work/agent-cli-smoke-benchmark/manifest.json"
));
const TASK_SMOKE_REPLY_OK_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../work/agent-cli-smoke-benchmark/tasks/smoke_reply_ok.json"
));
const TASK_SMOKE_ECHO_PING_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../work/agent-cli-smoke-benchmark/tasks/smoke_echo_ping.json"
));

/// Domain-separated hash domains for benchmark identity.
pub const MANIFEST_HASH_DOMAIN: &str = "cs-benchmark:manifest";
pub const TASK_HASH_DOMAIN: &str = "cs-benchmark:task";
pub const INSTRUCTION_HASH_DOMAIN: &str = "cs-benchmark:instruction";

/// Machine-stable error codes for adapter failures.
pub mod code {
    pub const UNKNOWN_SUITE: &str = "unknown_suite";
    pub const UNKNOWN_TASK: &str = "unknown_task";
    pub const MANIFEST_INVALID: &str = "manifest_invalid";
    pub const TASK_INVALID: &str = "task_invalid";
    pub const DIGEST_MISMATCH: &str = "digest_mismatch";
    pub const UNSUPPORTED_RUNNER: &str = "unsupported_runner";
}

/// Adapter error carrying a stable machine code.
#[derive(Debug, Clone)]
pub struct BenchmarkError {
    pub code: &'static str,
    pub message: String,
}

impl BenchmarkError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for BenchmarkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for BenchmarkError {}

// ---------------------------------------------------------------------------
// Strict fixture schemas (snake_case, unknown fields rejected)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct BenchmarkManifestV1 {
    pub schema_version: u32,
    pub dataset_id: String,
    pub dataset_version: u32,
    pub split: String,
    pub runner_kind: String,
    pub adapter_id: String,
    pub adapter_version: String,
    pub verifier_id: String,
    pub verifier_version: String,
    /// Fixed task id order; duplicates are rejected at parse time.
    pub tasks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct SmokeTaskV1 {
    pub schema_version: u32,
    pub task_id: String,
    pub instruction: String,
    /// Domain-separated SHA-256 of `instruction`; validated at resolve time.
    pub instruction_hash: String,
    pub expected: SmokeExpectedV1,
    pub resource_profile: SmokeResourceProfileV1,
    pub verifier_id: String,
    pub verifier_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct SmokeExpectedV1 {
    pub terminal_status: String,
    pub cost_status: String,
    pub no_budget_rejection: bool,
}

/// Per-task hard caps submitted to the 2C admission/runtime boundary. `None`
/// is explicit `not_applicable` (never unlimited); disk/network have no field
/// because they cannot be observed before 2G and are never faked as zero.
///
/// Artifact v1 independently projects only token totals and wall time. The
/// verifier therefore reports `tool_calls`, `processes`, and `concurrency` as
/// admission-only caps instead of treating their absence from an artifact as
/// a pass; see `verifier::run_verdict_checks`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct SmokeResourceProfileV1 {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: Option<u64>,
    #[serde(default)]
    pub cache_write_tokens: Option<u64>,
    pub wall_time_ms: u64,
    pub tool_calls: u64,
    pub processes: u64,
    pub concurrency: u64,
}

/// A fully resolved, digest-bound task ready to run or verify.
#[derive(Debug, Clone)]
pub struct ResolvedTask {
    pub manifest: BenchmarkManifestV1,
    pub task: SmokeTaskV1,
    /// Canonical digest of the whole fixture (manifest + all tasks in order).
    pub manifest_digest: String,
    /// Canonical digest of this task document.
    pub task_digest: String,
}

/// The durable, instruction-free identity of a resolved fixture task. This is
/// what a durable schedule request stores: refs and digests only, so the raw
/// instruction never reaches the database (INV-6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct FixtureTaskRefV1 {
    pub suite: String,
    pub dataset_id: String,
    pub dataset_version: u32,
    pub split: String,
    pub task_id: String,
    pub manifest_digest: String,
    pub task_digest: String,
    pub instruction_hash: String,
}

impl ResolvedTask {
    /// Projects the fixture identity that may be persisted. The instruction
    /// body is deliberately absent.
    pub fn task_ref(&self) -> FixtureTaskRefV1 {
        FixtureTaskRefV1 {
            suite: SUITE_ID.to_string(),
            dataset_id: self.manifest.dataset_id.clone(),
            dataset_version: self.manifest.dataset_version,
            split: self.manifest.split.clone(),
            task_id: self.task.task_id.clone(),
            manifest_digest: self.manifest_digest.clone(),
            task_digest: self.task_digest.clone(),
            instruction_hash: self.task.instruction_hash.clone(),
        }
    }
}

/// Re-resolves a persisted fixture ref against the pinned checked-in catalog
/// and fails closed on any drift. This is the only way a durable scheduler may
/// recover the instruction: it is recomputed from the compile-time fixture and
/// must match every recorded digest, otherwise the job fails before dispatch.
pub fn resolve_task_ref(reference: &FixtureTaskRefV1) -> Result<ResolvedTask, BenchmarkError> {
    if reference.suite != SUITE_ID
        || reference.dataset_id != DATASET_ID
        || reference.dataset_version != DATASET_VERSION
        || reference.split != SPLIT
    {
        return Err(BenchmarkError::new(
            code::DIGEST_MISMATCH,
            format!(
                "fixture ref identity {}/{}@{}/{} does not match the pinned catalog",
                reference.suite, reference.dataset_id, reference.dataset_version, reference.split
            ),
        ));
    }
    let resolved = resolve_task(&reference.suite, &reference.task_id)?;
    let current = resolved.task_ref();
    if current.manifest_digest != reference.manifest_digest
        || current.task_digest != reference.task_digest
        || current.instruction_hash != reference.instruction_hash
    {
        return Err(BenchmarkError::new(
            code::DIGEST_MISMATCH,
            format!(
                "fixture digest drift for task '{}': the pinned catalog no longer \
                 matches the persisted refs",
                reference.task_id
            ),
        ));
    }
    Ok(resolved)
}

// ---------------------------------------------------------------------------
// Parsing and resolution
// ---------------------------------------------------------------------------

fn parse_manifest(text: &str) -> Result<BenchmarkManifestV1, BenchmarkError> {
    let manifest: BenchmarkManifestV1 = serde_json::from_str(text).map_err(|error| {
        BenchmarkError::new(
            code::MANIFEST_INVALID,
            format!(
                "benchmark manifest is not a valid strict v1 document: {}",
                error
            ),
        )
    })?;
    if manifest.schema_version != 1 {
        return Err(BenchmarkError::new(
            code::MANIFEST_INVALID,
            format!(
                "unsupported benchmark manifest schema_version {}",
                manifest.schema_version
            ),
        ));
    }
    if manifest.dataset_id != DATASET_ID
        || manifest.dataset_version != DATASET_VERSION
        || manifest.split != SPLIT
    {
        return Err(BenchmarkError::new(
            code::MANIFEST_INVALID,
            format!(
                "benchmark manifest identity mismatch: expected {DATASET_ID}@{DATASET_VERSION}/{SPLIT}"
            ),
        ));
    }
    if manifest.runner_kind != RUNNER_KIND {
        return Err(BenchmarkError::new(
            code::UNSUPPORTED_RUNNER,
            format!(
                "unsupported runner_kind '{}' (expected '{RUNNER_KIND}')",
                manifest.runner_kind
            ),
        ));
    }
    let mut seen: Vec<&str> = Vec::with_capacity(manifest.tasks.len());
    for task_id in &manifest.tasks {
        if seen.contains(&task_id.as_str()) {
            return Err(BenchmarkError::new(
                code::MANIFEST_INVALID,
                format!("duplicate task id in manifest: {}", task_id),
            ));
        }
        seen.push(task_id);
    }
    if manifest.tasks.is_empty() {
        return Err(BenchmarkError::new(
            code::MANIFEST_INVALID,
            "benchmark manifest declares no tasks",
        ));
    }
    Ok(manifest)
}

fn parse_task(text: &str) -> Result<SmokeTaskV1, BenchmarkError> {
    let task: SmokeTaskV1 = serde_json::from_str(text).map_err(|error| {
        BenchmarkError::new(
            code::TASK_INVALID,
            format!(
                "benchmark task is not a valid strict v1 document: {}",
                error
            ),
        )
    })?;
    if task.schema_version != 1 {
        return Err(BenchmarkError::new(
            code::TASK_INVALID,
            format!(
                "unsupported benchmark task schema_version {}",
                task.schema_version
            ),
        ));
    }
    if task.task_id.is_empty() || task.instruction.is_empty() {
        return Err(BenchmarkError::new(
            code::TASK_INVALID,
            "benchmark task is missing task_id or instruction",
        ));
    }
    Ok(task)
}

/// Returns the embedded task fixture body for a manifest-declared task id.
fn embedded_task_json(task_id: &str) -> Option<&'static str> {
    match task_id {
        "smoke_reply_ok" => Some(TASK_SMOKE_REPLY_OK_JSON),
        "smoke_echo_ping" => Some(TASK_SMOKE_ECHO_PING_JSON),
        _ => None,
    }
}

/// Core resolver over explicit fixture texts. It is `pub` because the CLI and
/// the backend negative tests both drive it with tampered fixture bodies.
pub fn resolve_task_from(
    suite: &str,
    task_id: &str,
    manifest_text: &str,
    task_text: Option<&str>,
) -> Result<ResolvedTask, BenchmarkError> {
    if suite != SUITE_ID {
        return Err(BenchmarkError::new(
            code::UNKNOWN_SUITE,
            format!("unknown benchmark suite '{}'", suite),
        ));
    }
    let manifest = parse_manifest(manifest_text)?;
    if !manifest.tasks.iter().any(|declared| declared == task_id) {
        return Err(BenchmarkError::new(
            code::UNKNOWN_TASK,
            format!(
                "task '{}' is not declared in the {}@{} {} manifest",
                task_id, manifest.dataset_id, manifest.dataset_version, manifest.split
            ),
        ));
    }
    let task_text = task_text.ok_or_else(|| {
        BenchmarkError::new(
            code::UNKNOWN_TASK,
            format!("no checked-in fixture for task '{}'", task_id),
        )
    })?;
    let task = parse_task(task_text)?;

    // The task document must be exactly the requested task (no file swap).
    if task.task_id != task_id {
        return Err(BenchmarkError::new(
            code::TASK_INVALID,
            format!(
                "task document declares task_id '{}' but '{}' was requested",
                task.task_id, task_id
            ),
        ));
    }
    // Verifier identity is fixed by the manifest, not by the caller.
    if task.verifier_id != manifest.verifier_id
        || task.verifier_version != manifest.verifier_version
    {
        return Err(BenchmarkError::new(
            code::TASK_INVALID,
            "task verifier identity does not match the manifest",
        ));
    }
    // The instruction is pinned by its domain-separated hash.
    let instruction_hash = domain_hash(INSTRUCTION_HASH_DOMAIN, task.instruction.as_bytes());
    if instruction_hash != task.instruction_hash {
        return Err(BenchmarkError::new(
            code::DIGEST_MISMATCH,
            format!(
                "instruction hash mismatch for task '{}': expected {}",
                task_id, task.instruction_hash
            ),
        ));
    }

    // Canonical digests: the manifest digest binds the manifest plus every
    // task document in declared order; the task digest binds this task alone.
    let task_value: Value = serde_json::from_str(task_text).map_err(|error| {
        BenchmarkError::new(
            code::TASK_INVALID,
            format!("task reparse failed: {}", error),
        )
    })?;
    let task_digest = canonical_hash(TASK_HASH_DOMAIN, &task_value);
    let mut combined_tasks = Vec::with_capacity(manifest.tasks.len());
    for declared in &manifest.tasks {
        let text = embedded_task_json(declared).ok_or_else(|| {
            BenchmarkError::new(
                code::MANIFEST_INVALID,
                format!("no checked-in fixture for declared task '{}'", declared),
            )
        })?;
        let value: Value = serde_json::from_str(text).map_err(|error| {
            BenchmarkError::new(
                code::TASK_INVALID,
                format!("task reparse failed: {}", error),
            )
        })?;
        combined_tasks.push(value);
    }
    let combined = json!({
        "manifest": serde_json::to_value(&manifest)
            .map_err(|error| BenchmarkError::new(code::MANIFEST_INVALID, format!("manifest reparse failed: {}", error)))?,
        "tasks": combined_tasks,
    });
    let manifest_digest = canonical_hash(MANIFEST_HASH_DOMAIN, &combined);

    Ok(ResolvedTask {
        manifest,
        task,
        manifest_digest,
        task_digest,
    })
}

/// Cached parsed embedded manifest.
fn embedded_manifest() -> &'static Result<BenchmarkManifestV1, BenchmarkError> {
    static MANIFEST: OnceLock<Result<BenchmarkManifestV1, BenchmarkError>> = OnceLock::new();
    MANIFEST.get_or_init(|| parse_manifest(MANIFEST_JSON))
}

/// Resolves a suite/task pair against the checked-in fixture.
pub fn resolve_task(suite: &str, task_id: &str) -> Result<ResolvedTask, BenchmarkError> {
    // Fail fast on a malformed embedded manifest before anything else.
    embedded_manifest()
        .as_ref()
        .map_err(|error| BenchmarkError {
            code: error.code,
            message: error.message.clone(),
        })?;
    resolve_task_from(suite, task_id, MANIFEST_JSON, embedded_task_json(task_id))
}

/// The manifest-declared task order of the checked-in fixture. Used by the
/// durable scheduler to build the ordered job list without re-parsing texts.
pub fn manifest_task_ids() -> Result<Vec<String>, BenchmarkError> {
    let manifest = embedded_manifest()
        .as_ref()
        .map_err(|error| BenchmarkError {
            code: error.code,
            message: error.message.clone(),
        })?;
    Ok(manifest.tasks.clone())
}

// ---------------------------------------------------------------------------
// Adapter projection: 2C spec + transport-neutral metadata
// ---------------------------------------------------------------------------

/// Builds the strict `experiment_run_spec.v1` document for a resolved task.
///
/// The budget envelope uses only 2C-observable dimensions from the task's
/// resource profile; disk/network are never capped (they cannot be observed
/// before 2G). Exactly one attempt; no retry. `model` is an optional
/// act-phase override forwarded as the 2C workflow override (a run-time knob
/// like the agent id; it is not part of the frozen fixture digest).
pub fn build_run_spec(task: &SmokeTaskV1, model: Option<&str>) -> Value {
    let profile = &task.resource_profile;
    let workflow = match model {
        Some(model) => json!({ "model": model }),
        None => json!({}),
    };
    json!({
        "schema_version": "experiment_run_spec.v1",
        "planning_mode": false,
        "workflow": workflow,
        "budget": {
            "money_mode": { "mode": "token_resource_only" },
            "caps": {
                "input_tokens": profile.input_tokens,
                "output_tokens": profile.output_tokens,
                "cache_read_tokens": profile.cache_read_tokens,
                "cache_write_tokens": profile.cache_write_tokens,
                "wall_time_ms": profile.wall_time_ms,
                "tool_calls": profile.tool_calls,
                "processes": profile.processes,
                "concurrency": profile.concurrency,
            },
            "required_dimensions": [],
            "max_attempts": 1,
        },
    })
}

/// Transport-neutral adapter metadata for a resolved task. A future Harbor
/// runner receives exactly this metadata and must not change its shape.
pub fn adapter_metadata(resolved: &ResolvedTask) -> Value {
    json!({
        "runner_kind": RUNNER_KIND,
        "dataset_id": resolved.manifest.dataset_id,
        "dataset_version": resolved.manifest.dataset_version,
        "split": resolved.manifest.split,
        "manifest_digest": resolved.manifest_digest,
        "task_id": resolved.task.task_id,
        "task_digest": resolved.task_digest,
        "instruction_hash": resolved.task.instruction_hash,
        "adapter_id": resolved.manifest.adapter_id,
        "adapter_version": resolved.manifest.adapter_version,
        "verifier_id": resolved.manifest.verifier_id,
        "verifier_version": resolved.manifest.verifier_version,
        "resource_profile": serde_json::to_value(&resolved.task.resource_profile)
            .unwrap_or(Value::Null),
        "disk_network_enforcement": "not_applicable",
        "expected": serde_json::to_value(&resolved.task.expected).unwrap_or(Value::Null),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_manifest_parses_strictly() {
        let manifest = embedded_manifest().as_ref().expect("manifest parses");
        assert_eq!(manifest.dataset_id, "chatspeed-smoke");
        assert_eq!(manifest.dataset_version, 2);
        assert_eq!(manifest.split, "smoke");
        assert_eq!(manifest.runner_kind, "local_control_plane");
        assert_eq!(
            manifest.tasks,
            vec!["smoke_reply_ok".to_string(), "smoke_echo_ping".to_string()]
        );
    }

    #[test]
    fn resolve_task_returns_digest_bound_task() {
        let resolved = resolve_task(SUITE_ID, "smoke_reply_ok").expect("resolves");
        assert_eq!(resolved.task.task_id, "smoke_reply_ok");
        assert_eq!(resolved.task.instruction, "Reply with exactly: OK");
        assert!(!resolved.task_digest.is_empty());
        assert_eq!(resolved.task.verifier_id, "chatspeed-smoke-verifier");
    }

    #[test]
    fn manifest_digest_is_stable_across_calls() {
        let first = resolve_task(SUITE_ID, "smoke_reply_ok")
            .expect("resolves")
            .manifest_digest;
        let second = resolve_task(SUITE_ID, "smoke_echo_ping")
            .expect("resolves")
            .manifest_digest;
        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
    }

    /// Golden digests for the checked-in `chatspeed-smoke@2` fixture. Moving
    /// the resolver from the CLI into the shared library must not change these
    /// values (2G+2H U-1 acceptance).
    #[test]
    fn fixture_digests_match_golden_values() {
        let resolved = resolve_task(SUITE_ID, "smoke_reply_ok").expect("resolves");
        assert_eq!(
            resolved.manifest_digest,
            "fcedab561697fbce2e095c04969e9313900bc73a7ed2e09c1b35bc89d2738918"
        );
        assert_eq!(
            resolved.task_digest,
            "85acd5e8d744b20f0ce537a17ca6ae51aa2caaa4b3f71853a81fb1713f7d5405"
        );
        let ping = resolve_task(SUITE_ID, "smoke_echo_ping").expect("resolves");
        assert_eq!(ping.manifest_digest, resolved.manifest_digest);
        assert_eq!(
            ping.task_digest,
            "5ce34ddef4ad2586e621a858cf810bff388596423c6ccb9bf2040b961425ed04"
        );
        assert_eq!(
            resolved.task.instruction_hash,
            "f58cc8905c59cc469e451f19c4141d1f6f0a59dce07a40a37040078236de4b40"
        );
    }

    #[test]
    fn unknown_suite_and_task_fail_closed() {
        let error = resolve_task("other-suite", "smoke_reply_ok").expect_err("suite");
        assert_eq!(error.code, code::UNKNOWN_SUITE);
        let error = resolve_task(SUITE_ID, "not_a_task").expect_err("task");
        assert_eq!(error.code, code::UNKNOWN_TASK);
    }

    #[test]
    fn tampered_instruction_hash_fails_closed() {
        let tampered = TASK_SMOKE_REPLY_OK_JSON.replace(
            "f58cc8905c59cc469e451f19c4141d1f6f0a59dce07a40a37040078236de4b40",
            "0000000000000000000000000000000000000000000000000000000000000000",
        );
        assert_ne!(tampered, TASK_SMOKE_REPLY_OK_JSON);
        let error = resolve_task_from(SUITE_ID, "smoke_reply_ok", MANIFEST_JSON, Some(&tampered))
            .expect_err("digest mismatch");
        assert_eq!(error.code, code::DIGEST_MISMATCH);
    }

    #[test]
    fn tampered_instruction_text_fails_closed() {
        let tampered =
            TASK_SMOKE_REPLY_OK_JSON.replace("Reply with exactly: OK", "Reply with exactly: EVIL");
        let error = resolve_task_from(SUITE_ID, "smoke_reply_ok", MANIFEST_JSON, Some(&tampered))
            .expect_err("digest mismatch");
        assert_eq!(error.code, code::DIGEST_MISMATCH);
    }

    #[test]
    fn task_file_swap_fails_closed() {
        // Requesting smoke_reply_ok but supplying the echo_ping document.
        let error = resolve_task_from(
            SUITE_ID,
            "smoke_reply_ok",
            MANIFEST_JSON,
            Some(TASK_SMOKE_ECHO_PING_JSON),
        )
        .expect_err("swap must fail");
        assert_eq!(error.code, code::TASK_INVALID);
    }

    #[test]
    fn undeclared_task_fails_closed_even_with_valid_document() {
        let error = resolve_task_from(
            SUITE_ID,
            "smoke_echo_ping",
            &MANIFEST_JSON.replace("\"smoke_echo_ping\"", "\"something_else\""),
            Some(TASK_SMOKE_ECHO_PING_JSON),
        )
        .expect_err("undeclared task");
        assert_eq!(error.code, code::UNKNOWN_TASK);
    }

    #[test]
    fn duplicate_manifest_tasks_fail_closed() {
        let duplicated = MANIFEST_JSON.replace("\"smoke_echo_ping\"", "\"smoke_reply_ok\"");
        let error = parse_manifest(&duplicated).expect_err("duplicate task ids");
        assert_eq!(error.code, code::MANIFEST_INVALID);
    }

    #[test]
    fn unknown_manifest_field_fails_closed() {
        let tampered = MANIFEST_JSON.replace(
            "\"split\": \"smoke\",",
            "\"split\": \"smoke\", \"extra_field\": true,",
        );
        let error = parse_manifest(&tampered).expect_err("unknown field");
        assert_eq!(error.code, code::MANIFEST_INVALID);
    }

    #[test]
    fn unknown_task_field_fails_closed() {
        let tampered = TASK_SMOKE_REPLY_OK_JSON.replace(
            "\"verifier_version\": \"2\"",
            "\"verifier_version\": \"2\", \"score\": 1.0",
        );
        assert_ne!(tampered, TASK_SMOKE_REPLY_OK_JSON);
        let error = parse_task(&tampered).expect_err("unknown field");
        assert_eq!(error.code, code::TASK_INVALID);
    }

    #[test]
    fn wrong_runner_kind_fails_closed() {
        let tampered = MANIFEST_JSON.replace("local_control_plane", "harbor_installed_agent");
        let error = parse_manifest(&tampered).expect_err("runner kind");
        assert_eq!(error.code, code::UNSUPPORTED_RUNNER);
    }

    #[test]
    fn verifier_identity_mismatch_fails_closed() {
        let tampered = TASK_SMOKE_REPLY_OK_JSON
            .replace("\"verifier_version\": \"2\"", "\"verifier_version\": \"3\"");
        let error = resolve_task_from(SUITE_ID, "smoke_reply_ok", MANIFEST_JSON, Some(&tampered))
            .expect_err("verifier identity");
        assert_eq!(error.code, code::TASK_INVALID);
    }

    #[test]
    fn adapter_metadata_is_deterministic() {
        let first = resolve_task(SUITE_ID, "smoke_reply_ok").expect("resolves");
        let second = resolve_task(SUITE_ID, "smoke_reply_ok").expect("resolves");
        assert_eq!(
            adapter_metadata(&first).to_string(),
            adapter_metadata(&second).to_string()
        );
        let metadata = adapter_metadata(&first);
        assert_eq!(metadata["runner_kind"], json!("local_control_plane"));
        assert_eq!(metadata["dataset_id"], json!("chatspeed-smoke"));
        assert_eq!(metadata["dataset_version"], json!(2));
        assert_eq!(metadata["split"], json!("smoke"));
        assert_eq!(
            metadata["disk_network_enforcement"],
            json!("not_applicable")
        );
        assert_eq!(metadata["task_id"], json!("smoke_reply_ok"));
    }

    #[test]
    fn run_spec_maps_profile_to_2c_caps() {
        let resolved = resolve_task(SUITE_ID, "smoke_reply_ok").expect("resolves");
        let spec = build_run_spec(&resolved.task, None);
        assert_eq!(spec["schema_version"], json!("experiment_run_spec.v1"));
        assert_eq!(spec["workflow"], json!({}));
        assert_eq!(spec["budget"]["max_attempts"], json!(1));
        assert_eq!(
            spec["budget"]["money_mode"]["mode"],
            json!("token_resource_only")
        );
        let caps = &spec["budget"]["caps"];
        assert_eq!(caps["input_tokens"], json!(65536));
        assert_eq!(caps["output_tokens"], json!(128000));
        assert_eq!(caps["wall_time_ms"], json!(300000));
        assert_eq!(caps["tool_calls"], json!(0));
        assert_eq!(caps["processes"], json!(0));
        assert_eq!(caps["concurrency"], json!(1));
        // Disk/network are never capped before 2G.
        assert!(caps.get("disk_bytes").is_none());
        assert!(caps.get("network_bytes").is_none());
    }

    #[test]
    fn run_spec_forwards_model_override() {
        // The model is a run-time knob (like the agent id): it is forwarded
        // as the 2C workflow override and never enters the fixture digest.
        let resolved = resolve_task(SUITE_ID, "smoke_reply_ok").expect("resolves");
        let spec = build_run_spec(&resolved.task, Some("cs@free:ds-v4-flash"));
        assert_eq!(spec["workflow"]["model"], json!("cs@free:ds-v4-flash"));
        // The budget envelope is unchanged by the model override.
        assert_eq!(spec["budget"]["caps"]["output_tokens"], json!(128000));
        assert_eq!(spec["budget"]["max_attempts"], json!(1));
    }

    #[test]
    fn task_digest_binds_task_content() {
        let resolved = resolve_task(SUITE_ID, "smoke_reply_ok").expect("resolves");
        let task_value: Value = serde_json::from_str(TASK_SMOKE_REPLY_OK_JSON).unwrap();
        let expected = canonical_hash(TASK_HASH_DOMAIN, &task_value);
        assert_eq!(resolved.task_digest, expected);
        // A different task has a different digest.
        let other = resolve_task(SUITE_ID, "smoke_echo_ping").expect("resolves");
        assert_ne!(resolved.task_digest, other.task_digest);
    }

    #[test]
    fn task_ref_never_carries_the_instruction() {
        let resolved = resolve_task(SUITE_ID, "smoke_reply_ok").expect("resolves");
        let reference = resolved.task_ref();
        let serialized = serde_json::to_string(&reference).expect("serializes");
        assert!(!serialized.contains("Reply with exactly"));
        assert_eq!(reference.task_id, "smoke_reply_ok");
        assert_eq!(reference.instruction_hash, resolved.task.instruction_hash);
        assert_eq!(reference.manifest_digest, resolved.manifest_digest);
        assert_eq!(reference.dataset_version, 2);
    }

    #[test]
    fn resolve_task_ref_round_trips_and_rejects_drift() {
        let resolved = resolve_task(SUITE_ID, "smoke_echo_ping").expect("resolves");
        let reference = resolved.task_ref();
        let recovered = resolve_task_ref(&reference).expect("re-resolves");
        assert_eq!(recovered.task.instruction, "Reply with exactly: PONG");
        assert_eq!(recovered.task_digest, reference.task_digest);

        let mut stale = reference.clone();
        stale.task_digest = "0".repeat(64);
        let error = resolve_task_ref(&stale).expect_err("stale digest");
        assert_eq!(error.code, code::DIGEST_MISMATCH);

        let mut wrong_version = reference;
        wrong_version.dataset_version = 1;
        let error = resolve_task_ref(&wrong_version).expect_err("stale version");
        assert_eq!(error.code, code::DIGEST_MISMATCH);
    }

    #[test]
    fn manifest_task_ids_match_declared_order() {
        assert_eq!(
            manifest_task_ids().expect("manifest"),
            vec!["smoke_reply_ok".to_string(), "smoke_echo_ping".to_string()]
        );
    }
}
