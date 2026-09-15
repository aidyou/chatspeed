//! Phase 2E benchmark adapter for the fixed `chatspeed-smoke@1` fixture.
//!
//! The adapter is a pure, deterministic resolver between a checked-in fixture
//! and the existing 2C experiment-run control plane. It never runs an
//! executor, opens the database, or talks to a provider itself; the only
//! external effect of `benchmark run` is the existing single budgeted
//! `POST /control/v1/experiments:run` (INV-2/INV-3).
//!
//! Fixture identity is fixed at compile time via `include_str!`: the adapter
//! can only accept tasks declared in the checked-in manifest, and no runtime
//! path, environment variable, or model output can swap the manifest, the
//! verifier identity, or the resource profile.
//!
//! Runner contract: `runner_kind = "local_control_plane"` for this phase. A
//! future Harbor installed-agent runner must reuse the same task/verdict
//! contract and only replace the transport/runner, never the score schema.

use crate::artifact;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::OnceLock;

/// The only suite id this adapter accepts.
pub const SUITE_ID: &str = "chatspeed-smoke";
/// Fixed dataset identity (Phase 2E baseline).
pub const DATASET_ID: &str = "chatspeed-smoke";
pub const DATASET_VERSION: u32 = 1;
pub const SPLIT: &str = "smoke";
/// Runner kind recorded in adapter metadata. Real Harbor installed-agent
/// execution is out of scope until 2G+2H and must not be claimed here.
pub const RUNNER_KIND: &str = "local_control_plane";

/// Checked-in fixture files (compiled into the CLI; no runtime path).
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

/// Per-task hard caps over the 2C-observable dimensions only. `None` is the
/// explicit `not_applicable` (never unlimited); disk/network have no field at
/// all because they cannot be observed before 2G and are never faked as zero.
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

/// Core resolver over explicit fixture texts (also the tamper-test seam).
fn resolve_task_from(
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
    let instruction_hash =
        artifact::domain_hash(INSTRUCTION_HASH_DOMAIN, task.instruction.as_bytes());
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
    let task_digest = artifact::canonical_hash(TASK_HASH_DOMAIN, &task_value);
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
    let manifest_digest = artifact::canonical_hash(MANIFEST_HASH_DOMAIN, &combined);

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

// ---------------------------------------------------------------------------
// CLI entry points
// ---------------------------------------------------------------------------

/// Runs `cs experiment benchmark run`: resolve the checked-in fixture task,
/// build the strict 2C spec from its resource profile, and submit exactly one
/// budgeted experiment run through the existing control plane. The task
/// instruction is used only as the run prompt and is never persisted raw.
pub async fn run(
    cli: &crate::args::Cli,
    client: &crate::client::ControlPlaneClient,
    suite: &str,
    task_id: &str,
    agent: &str,
    model: Option<&str>,
    artifact_dir: Option<&std::path::Path>,
) -> Result<(), crate::error::CliError> {
    let resolved = resolve_task(suite, task_id).map_err(to_cli_error)?;
    // Operator-visible adapter facts (stderr only): exactly what identity is
    // being run, so a benchmark run can always be audited back to the fixture.
    let metadata = adapter_metadata(&resolved);
    crate::output::eprint_diagnostic(&format!(
        "cs: benchmark adapter metadata: {}",
        serde_json::to_string(&metadata).unwrap_or_default()
    ));
    let spec = build_run_spec(&resolved.task, model);
    let prompt = resolved.task.instruction.clone();
    crate::experiment::run_with_spec(cli, client, agent, spec, prompt, false, artifact_dir).await
}

/// Maps an adapter error to a CLI error: unknown suite/task are usage errors
/// (exit 2); fixture integrity problems are I/O-class failures (exit 1).
fn to_cli_error(error: BenchmarkError) -> crate::error::CliError {
    use crate::error::CliError;
    let rendered = format!("{}: {}", error.code, error.message);
    match error.code {
        code::UNKNOWN_SUITE | code::UNKNOWN_TASK => CliError::usage(rendered),
        _ => CliError::io(rendered),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_manifest_parses_strictly() {
        let manifest = embedded_manifest().as_ref().expect("manifest parses");
        assert_eq!(manifest.dataset_id, "chatspeed-smoke");
        assert_eq!(manifest.dataset_version, 1);
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
        assert_eq!(resolved.manifest_digest, resolved.manifest_digest);
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

    /// Golden digests for the checked-in `chatspeed-smoke@1` fixture. Any
    /// fixture change must bump the dataset version and update these values
    /// plus the route document (AC-5).
    #[test]
    fn fixture_digests_match_golden_values() {
        let resolved = resolve_task(SUITE_ID, "smoke_reply_ok").expect("resolves");
        assert_eq!(
            resolved.manifest_digest,
            "619ae2a20b4e5a7263a627a7ec579b00d4bafc90d11b74e5346b4f33519dc723"
        );
        assert_eq!(
            resolved.task_digest,
            "3fcbf6edc2d98007c8e538037904fc599c751f638df6c8cd9632740bb3d9e8a0"
        );
        let ping = resolve_task(SUITE_ID, "smoke_echo_ping").expect("resolves");
        assert_eq!(ping.manifest_digest, resolved.manifest_digest);
        assert_eq!(
            ping.task_digest,
            "d0d7bb1250b6b5366044ac47c8dfc02aa3dd07e86f106165cb6a062a5538b1fb"
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
            "\"verifier_version\": \"1\"",
            "\"verifier_version\": \"1\", \"score\": 1.0",
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
            .replace("\"verifier_version\": \"1\"", "\"verifier_version\": \"2\"");
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
        assert_eq!(metadata["dataset_version"], json!(1));
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
        let expected = artifact::canonical_hash(TASK_HASH_DOMAIN, &task_value);
        assert_eq!(resolved.task_digest, expected);
        // A different task has a different digest.
        let other = resolve_task(SUITE_ID, "smoke_echo_ping").expect("resolves");
        assert_ne!(resolved.task_digest, other.task_digest);
    }
}
