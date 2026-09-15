//! Phase 2D deterministic correctness evaluator.
//!
//! Fully offline: it reads only an already-captured 2A artifact directory,
//! re-verifies it with `artifact::verify_bundle_dir`, projects deterministic
//! correctness facts from the trusted `VerifyReport`, and publishes a
//! versioned evaluation sidecar into a separate output directory. It never
//! touches the network, the database, discovery, any LLM key, or the source
//! artifact bytes, and it never emits a promotion verdict.
//!
//! Sidecar layout (`<output>/`):
//! - `evaluation.json`: versioned evaluation document;
//! - `artifacts/manifest.json`: sidecar manifest covering only the sidecar
//!   data file (never the 2A artifact manifest).
//!
//! Fail-closed contract: on any verification, binding, privacy or output
//! target problem the sidecar is not published and a stable machine code is
//! returned. `created_at` is deliberately excluded from `evaluation_hash` so
//! repeated evaluations of the same artifact are canonical-equivalent.

use crate::args::Cli;
use crate::artifact::{self, ArtifactError, VerifyReport};
use crate::error::CliError;
use crate::output::{eprint_diagnostic, render_result};
use serde_json::{json, Map, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Evaluation sidecar schema version.
pub const EVALUATION_SCHEMA_VERSION: u32 = 1;
/// Fixed evaluation kind label.
pub const EVALUATION_KIND: &str = "cs.evaluation.deterministic";
/// Fixed evaluator identity.
pub const EVALUATOR_ID: &str = "chatspeed-deterministic-evaluator";
/// Evaluator implementation version (bump on any check-semantics change).
pub const EVALUATOR_VERSION: &str = "1";
/// Hash algorithm label reused from the 2A artifact contract.
pub const HASH_ALGORITHM: &str = artifact::HASH_ALGORITHM;

/// Domain-separated hash domains for the evaluation sidecar.
pub const EVALUATION_HASH_DOMAIN: &str = "cs-evaluation:evaluation";
pub const EVALUATION_MANIFEST_DOMAIN: &str = "cs-evaluation:manifest";

/// Maximum total bytes across all published sidecar files.
const MAX_SIDECAR_TOTAL_BYTES: u64 = artifact::MAX_TOTAL_BYTES;

/// Machine-stable error codes specific to the evaluation sidecar. Artifact
/// verification failures reuse the stable `artifact::code` values.
pub mod code {
    /// The evaluation output target overlaps the source artifact directory.
    pub const TARGET_OVERLAP: &str = "evaluation_target_unsafe";
}

// ---------------------------------------------------------------------------
// Deterministic checks
// ---------------------------------------------------------------------------

/// Status of a single deterministic check.
///
/// `pass`/`fail` are factual outcomes; `not_evaluable` means the trusted
/// artifact facts do not allow a deterministic decision (never guessed from
/// model text).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStatus {
    Pass,
    Fail,
    NotEvaluable,
}

impl CheckStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CheckStatus::Pass => "pass",
            CheckStatus::Fail => "fail",
            CheckStatus::NotEvaluable => "not_evaluable",
        }
    }
}

/// What class of fact a check contributes to the correctness status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckClass {
    /// Direct correctness evidence (terminal outcome, artifact completeness).
    Correctness,
    /// Integrity/accounting/infrastructure facts; a failure blocks a
    /// correctness pass but is never itself a correctness failure.
    Infra,
}

impl CheckClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            CheckClass::Correctness => "correctness",
            CheckClass::Infra => "infra",
        }
    }
}

/// One deterministic check fact.
#[derive(Debug, Clone)]
pub struct Check {
    pub check_id: &'static str,
    pub class: CheckClass,
    pub status: CheckStatus,
    pub detail: Value,
}

/// Budget/infra admission machine codes that must never be counted as a
/// correctness success. Scanned only inside the verified structured events.
const BUDGET_REJECTION_TOKENS: &[&str] = &[
    "budget_exceeded",
    "scope_paused",
    "resource_unobservable",
    "experiment admission rejected",
];

/// Runs the deterministic checks over a verified artifact report.
pub fn run_checks(report: &VerifyReport) -> Vec<Check> {
    let schema_version = report
        .run
        .get("schema_version")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let usage = report.result.get("usage").cloned().unwrap_or(Value::Null);
    let usage_present = usage.get("present").and_then(Value::as_bool) == Some(true);
    let totals = usage.get("self_usage").cloned().unwrap_or(Value::Null);
    let totals_present = totals.get("total_tokens").and_then(Value::as_i64).is_some();

    let mut checks = vec![
        Check {
            check_id: "artifact_schema_supported",
            class: CheckClass::Infra,
            status: CheckStatus::Pass,
            detail: json!({ "schema_version": schema_version }),
        },
        Check {
            check_id: "manifest_integrity_verified",
            class: CheckClass::Infra,
            status: CheckStatus::Pass,
            detail: json!({ "verified_files": artifact::REQUIRED_FILE_COUNT }),
        },
        Check {
            check_id: "event_chain_verified",
            class: CheckClass::Infra,
            status: CheckStatus::Pass,
            detail: json!({
                "event_count": report.event_count,
                "chain_head": report.chain_head,
            }),
        },
        Check {
            check_id: "session_binding_verified",
            class: CheckClass::Infra,
            status: CheckStatus::Pass,
            detail: json!({ "session_id": report.run.get("session_id").and_then(Value::as_str) }),
        },
        Check {
            check_id: "artifact_status_complete",
            class: CheckClass::Correctness,
            status: match report.status {
                artifact::ArtifactStatus::Complete => CheckStatus::Pass,
                artifact::ArtifactStatus::Incomplete => CheckStatus::NotEvaluable,
            },
            detail: json!({ "artifact_status": report.status.as_str() }),
        },
        Check {
            check_id: "terminal_status_known",
            class: CheckClass::Correctness,
            status: match report.terminal_status {
                artifact::TerminalStatus::Unknown => CheckStatus::NotEvaluable,
                _ => CheckStatus::Pass,
            },
            detail: json!({ "terminal_status": report.terminal_status.as_str() }),
        },
        Check {
            check_id: "terminal_outcome_completed",
            class: CheckClass::Correctness,
            status: match report.terminal_status {
                artifact::TerminalStatus::Completed => CheckStatus::Pass,
                artifact::TerminalStatus::Failed | artifact::TerminalStatus::Cancelled => {
                    CheckStatus::Fail
                }
                artifact::TerminalStatus::Unknown => CheckStatus::NotEvaluable,
            },
            detail: json!({ "terminal_status": report.terminal_status.as_str() }),
        },
        Check {
            check_id: "cost_status_known",
            class: CheckClass::Infra,
            status: if report.cost_status == "known" {
                CheckStatus::Pass
            } else {
                // Unknown cost is never treated as zero cost or a pass.
                CheckStatus::NotEvaluable
            },
            detail: json!({ "cost_status": report.cost_status }),
        },
        Check {
            check_id: "usage_totals_present",
            class: CheckClass::Infra,
            status: if usage_present && totals_present {
                CheckStatus::Pass
            } else {
                CheckStatus::NotEvaluable
            },
            detail: json!({
                "usage_present": usage_present,
                "self_total_tokens_present": totals_present,
            }),
        },
    ];

    // Budget/infra admission facts: any rejection token inside the verified
    // structured events blocks a correctness pass (infra fact, not success).
    let haystack = serde_json::to_string(&report.events).unwrap_or_default();
    let rejected = BUDGET_REJECTION_TOKENS
        .iter()
        .any(|token| haystack.contains(token));
    checks.push(Check {
        check_id: "no_budget_rejection",
        class: CheckClass::Infra,
        status: if rejected {
            CheckStatus::Fail
        } else {
            CheckStatus::Pass
        },
        detail: json!({ "budget_rejection_tokens_found": rejected }),
    });
    checks
}

/// Derives the overall correctness status from the deterministic checks.
///
/// - any correctness-class `fail` → `fail`;
/// - otherwise any `fail` (infra) or `not_evaluable` → `not_evaluable`;
/// - otherwise `pass`.
pub fn correctness_status(checks: &[Check]) -> &'static str {
    let mut not_evaluable = false;
    for check in checks {
        match (check.class, check.status) {
            (CheckClass::Correctness, CheckStatus::Fail) => return "fail",
            (CheckClass::Correctness, CheckStatus::NotEvaluable)
            | (CheckClass::Infra, CheckStatus::NotEvaluable) => not_evaluable = true,
            (CheckClass::Infra, CheckStatus::Fail) => not_evaluable = true,
            (CheckClass::Correctness, CheckStatus::Pass)
            | (CheckClass::Infra, CheckStatus::Pass) => {}
        }
    }
    if not_evaluable {
        "not_evaluable"
    } else {
        "pass"
    }
}

// ---------------------------------------------------------------------------
// Evaluation document
// ---------------------------------------------------------------------------

/// Builds the `evaluation.json` document from a verified report.
///
/// `created_at` is recorded but excluded from `evaluation_hash`, so repeated
/// evaluations of the same artifact produce the same canonical digest.
pub fn build_evaluation(report: &VerifyReport, artifact_dir: &Path, created_at: &str) -> Value {
    let checks = run_checks(report);
    let status = correctness_status(&checks);
    let check_values: Vec<Value> = checks
        .iter()
        .map(|check| {
            json!({
                "check_id": check.check_id,
                "class": check.class.as_str(),
                "status": check.status.as_str(),
                "detail": check.detail,
            })
        })
        .collect();

    // path_hint is the artifact directory's file name only: never an absolute
    // path (privacy: sensitive absolute paths must not enter the sidecar).
    let path_hint = artifact_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");

    let mut evaluation = Map::new();
    evaluation.insert("schema_version".into(), json!(EVALUATION_SCHEMA_VERSION));
    evaluation.insert("evaluation_kind".into(), json!(EVALUATION_KIND));
    evaluation.insert("evaluator_id".into(), json!(EVALUATOR_ID));
    evaluation.insert("evaluator_version".into(), json!(EVALUATOR_VERSION));
    evaluation.insert(
        "source_artifact".into(),
        json!({
            "path_hint": path_hint,
            "run_id": report.run.get("run_id").and_then(Value::as_str).unwrap_or(""),
            "session_id": report.run.get("session_id").and_then(Value::as_str).unwrap_or(""),
            "artifact_schema_version": report.run.get("schema_version").and_then(Value::as_u64).unwrap_or(0),
            "chain_head": report.chain_head,
        }),
    );
    evaluation.insert("checks".into(), Value::Array(check_values));
    evaluation.insert("correctness_status".into(), json!(status));
    evaluation.insert("provenance".into(), json!(["artifact", "evaluator"]));
    evaluation.insert("created_at".into(), json!(created_at));

    // Integrity digest over the deterministic content only: `created_at` is
    // removed before hashing (and re-inserted after) so re-evaluation of the
    // same artifact is canonical-equivalent.
    let deterministic_content = {
        let mut content = evaluation.clone();
        content.remove("created_at");
        Value::Object(content)
    };
    let evaluation_hash = artifact::canonical_hash(EVALUATION_HASH_DOMAIN, &deterministic_content);
    evaluation.insert(
        "integrity".into(),
        json!({
            "algorithm": HASH_ALGORITHM,
            "evaluation_hash": evaluation_hash,
        }),
    );
    Value::Object(evaluation)
}

// ---------------------------------------------------------------------------
// Sidecar writer (shared with the 2E verdict sidecar)
// ---------------------------------------------------------------------------

/// A sidecar bundle: one data file plus a generated sidecar manifest.
pub(crate) struct SidecarBundle {
    /// Data file name inside the sidecar directory (e.g. `evaluation.json`).
    pub file_name: &'static str,
    /// Exact body of the data file.
    pub body: String,
    /// Sidecar schema version recorded in the manifest.
    pub schema_version: u32,
    /// Hash algorithm label recorded in the manifest.
    pub algorithm: &'static str,
    /// Domain for the sidecar `manifest_hash`.
    pub manifest_domain: &'static str,
}

impl SidecarBundle {
    fn manifest_json(&self) -> String {
        let files = json!([{
            "path": self.file_name,
            "size": self.body.len() as u64,
            "sha256": artifact::blob_hash(self.body.as_bytes()),
        }]);
        let manifest_hash = artifact::canonical_hash(self.manifest_domain, &files);
        let manifest = json!({
            "schema_version": self.schema_version,
            "algorithm": self.algorithm,
            "status": "complete",
            "files": files,
            "manifest_hash": manifest_hash,
        });
        serde_json::to_string_pretty(&manifest).unwrap_or_else(|_| "{}".to_string())
    }
}

/// Verifies a published/staged sidecar directory: exact single-file manifest,
/// hash integrity, and no symlink/traversal tricks. Returns the parsed data
/// document.
pub(crate) fn verify_sidecar_dir(
    dir: &Path,
    file_name: &str,
    manifest_domain: &str,
) -> Result<Value, ArtifactError> {
    if dir
        .symlink_metadata()
        .map(|meta| meta.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(ArtifactError::new(
            artifact::code::SYMLINK,
            "sidecar directory is a symlink",
        ));
    }
    let manifest_path = dir.join("artifacts").join("manifest.json");
    let manifest_text = fs::read_to_string(&manifest_path).map_err(|_| {
        ArtifactError::new(
            artifact::code::MISSING_FILE,
            format!("missing sidecar manifest {}", manifest_path.display()),
        )
    })?;
    if manifest_text.len() as u64 > MAX_SIDECAR_TOTAL_BYTES {
        return Err(ArtifactError::new(
            artifact::code::LIMIT,
            "sidecar manifest exceeds size limit",
        ));
    }
    let manifest: Value = serde_json::from_str(&manifest_text).map_err(|error| {
        ArtifactError::new(
            artifact::code::MANIFEST,
            format!("sidecar manifest is not valid JSON: {}", error),
        )
    })?;
    if manifest.get("schema_version").and_then(Value::as_u64) != Some(1) {
        return Err(ArtifactError::new(
            artifact::code::INCOMPATIBLE,
            "unsupported sidecar manifest schema_version",
        ));
    }
    if manifest.get("algorithm").and_then(Value::as_str) != Some(HASH_ALGORITHM) {
        return Err(ArtifactError::new(
            artifact::code::MANIFEST,
            "sidecar manifest algorithm mismatch",
        ));
    }
    if manifest.get("status").and_then(Value::as_str) != Some("complete") {
        return Err(ArtifactError::new(
            artifact::code::MANIFEST,
            "sidecar manifest is not complete",
        ));
    }
    let files = manifest
        .get("files")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ArtifactError::new(
                artifact::code::MANIFEST,
                "sidecar manifest has no files array",
            )
        })?;
    if files.len() != 1 {
        return Err(ArtifactError::new(
            artifact::code::MANIFEST,
            "sidecar manifest must cover exactly one data file",
        ));
    }
    let entry = &files[0];
    let path = entry.get("path").and_then(Value::as_str).unwrap_or("");
    if path != file_name {
        return Err(ArtifactError::new(
            artifact::code::MANIFEST,
            format!("sidecar manifest path is not {:?}: {:?}", file_name, path),
        ));
    }
    let files_value = Value::Array(files.clone());
    let expected_hash = artifact::canonical_hash(manifest_domain, &files_value);
    if manifest
        .get("manifest_hash")
        .and_then(Value::as_str)
        .unwrap_or("")
        != expected_hash
    {
        return Err(ArtifactError::new(
            artifact::code::MANIFEST,
            "sidecar manifest_hash does not match files list",
        ));
    }
    let data_path = dir.join(file_name);
    let meta = data_path.symlink_metadata().map_err(|_| {
        ArtifactError::new(
            artifact::code::MISSING_FILE,
            format!("missing sidecar data file {}", file_name),
        )
    })?;
    if meta.file_type().is_symlink() {
        return Err(ArtifactError::new(
            artifact::code::SYMLINK,
            "sidecar data file is a symlink",
        ));
    }
    let bytes = fs::read(&data_path).map_err(|error| {
        ArtifactError::new(
            artifact::code::IO,
            format!("cannot read sidecar data file: {}", error),
        )
    })?;
    if bytes.len() as u64
        != entry
            .get("size")
            .and_then(Value::as_u64)
            .unwrap_or(u64::MAX)
    {
        return Err(ArtifactError::new(
            artifact::code::HASH,
            "sidecar data file size mismatch",
        ));
    }
    if artifact::blob_hash(&bytes) != entry.get("sha256").and_then(Value::as_str).unwrap_or("") {
        return Err(ArtifactError::new(
            artifact::code::HASH,
            "sidecar data file hash mismatch",
        ));
    }
    serde_json::from_slice(&bytes).map_err(|error| {
        ArtifactError::new(
            artifact::code::INVALID,
            format!("sidecar data file is not valid JSON: {}", error),
        )
    })
}

/// Publishes a sidecar via staging + re-verify + atomic rename.
///
/// On any failure the staging directory is removed and the final path is never
/// created, so a partial sidecar cannot be mistaken for a valid evaluation.
pub(crate) fn write_sidecar(bundle: &SidecarBundle, final_dir: &Path) -> Result<(), ArtifactError> {
    let total_bytes = bundle.body.len() as u64 + bundle.manifest_json().len() as u64;
    if total_bytes > MAX_SIDECAR_TOTAL_BYTES {
        return Err(ArtifactError::new(
            artifact::code::LIMIT,
            format!(
                "sidecar total size {} bytes exceeds limit {}",
                total_bytes, MAX_SIDECAR_TOTAL_BYTES
            ),
        ));
    }
    if final_dir.symlink_metadata().is_ok() {
        return Err(ArtifactError::new(
            artifact::code::TARGET_EXISTS,
            format!("sidecar target already exists: {}", final_dir.display()),
        ));
    }
    // Defense in depth: a symlinked parent would redirect the atomic rename
    // (possibly into the immutable 2A artifact directory), so every existing
    // ancestor must be a real directory.
    reject_symlink_ancestors(final_dir)?;
    let parent = final_dir
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    if !parent.exists() {
        fs::create_dir_all(&parent).map_err(|error| {
            ArtifactError::new(
                artifact::code::IO,
                format!("cannot create parent {}: {}", parent.display(), error),
            )
        })?;
    }

    let name = final_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("sidecar");
    let staging = parent.join(format!(".{}.staging-{}", name, uuid::Uuid::new_v4()));
    fs::create_dir_all(staging.join("artifacts")).map_err(|error| {
        ArtifactError::new(
            artifact::code::IO,
            format!("cannot create staging: {}", error),
        )
    })?;

    let write_result = (|| -> Result<(), ArtifactError> {
        let manifest_json = bundle.manifest_json();
        write_sidecar_file(&staging.join(bundle.file_name), bundle.body.as_bytes())?;
        write_sidecar_file(
            &staging.join("artifacts").join("manifest.json"),
            manifest_json.as_bytes(),
        )?;
        // Re-verify the staged sidecar before publishing.
        verify_sidecar_dir(&staging, bundle.file_name, bundle.manifest_domain)?;
        Ok(())
    })();

    if let Err(error) = write_result {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }

    fs::rename(&staging, final_dir).map_err(|error| {
        let _ = fs::remove_dir_all(&staging);
        ArtifactError::new(
            artifact::code::IO,
            format!(
                "atomic rename failed for {}: {}",
                final_dir.display(),
                error
            ),
        )
    })
}

fn write_sidecar_file(path: &Path, bytes: &[u8]) -> Result<(), ArtifactError> {
    let mut file = fs::File::create(path).map_err(|error| {
        ArtifactError::new(
            artifact::code::IO,
            format!("cannot write {}: {}", path.display(), error),
        )
    })?;
    file.write_all(bytes).map_err(|error| {
        ArtifactError::new(
            artifact::code::IO,
            format!("write failed {}: {}", path.display(), error),
        )
    })?;
    file.flush().map_err(|error| {
        ArtifactError::new(
            artifact::code::IO,
            format!("flush failed {}: {}", path.display(), error),
        )
    })?;
    file.sync_all().map_err(|error| {
        ArtifactError::new(
            artifact::code::IO,
            format!("sync failed {}: {}", path.display(), error),
        )
    })?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Output target safety
// ---------------------------------------------------------------------------

/// Rejects any symlink on the existing portion of the output path (including
/// the final component). A symlinked parent could redirect the atomic rename
/// into the immutable 2A artifact directory, so this fails closed.
pub(crate) fn reject_symlink_ancestors(final_dir: &Path) -> Result<(), ArtifactError> {
    for ancestor in final_dir.ancestors() {
        if let Ok(meta) = fs::symlink_metadata(ancestor) {
            if meta.file_type().is_symlink() {
                return Err(ArtifactError::new(
                    artifact::code::SYMLINK,
                    format!(
                        "output path contains a symlink component: {}",
                        ancestor.display()
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Resolves `path` against the filesystem: canonicalizes the deepest existing
/// ancestor and re-appends the non-existing tail. Returns `None` when the
/// path cannot be anchored (the caller's later checks fail closed instead).
pub(crate) fn canonicalize_output(path: &Path) -> Option<PathBuf> {
    let mut existing = path.to_path_buf();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    loop {
        match fs::canonicalize(&existing) {
            Ok(canonical) => {
                let mut resolved = canonical;
                for component in tail.iter().rev() {
                    resolved.push(component);
                }
                return Some(resolved);
            }
            Err(_) => {
                let name = existing.file_name()?.to_os_string();
                tail.push(name);
                existing = existing.parent()?.to_path_buf();
            }
        }
    }
}

/// Rejects an evaluation output target that overlaps the source artifact
/// directory (either containing it or contained by it), so evaluating can
/// never modify or nest inside the immutable 2A bundle. Sibling directories
/// under a common parent are allowed. The check is both lexical and
/// filesystem-resolved: a symlinked parent that points into the artifact
/// directory is rejected instead of silently redirecting the sidecar.
pub(crate) fn reject_target_overlap(
    artifact_dir: &Path,
    output_dir: &Path,
    target_kind: &str,
) -> Result<(), ArtifactError> {
    // Lexical containment first (cheap, no filesystem access).
    if output_dir.starts_with(artifact_dir) || artifact_dir.starts_with(output_dir) {
        return Err(ArtifactError::new(
            code::TARGET_OVERLAP,
            format!(
                "{} output {} overlaps the source artifact directory {}",
                target_kind,
                output_dir.display(),
                artifact_dir.display()
            ),
        ));
    }
    // Filesystem-resolved containment: canonicalize the artifact directory
    // and the deepest existing ancestor of the output path, then compare.
    let Some(artifact_canonical) = canonicalize_output(artifact_dir) else {
        // A missing artifact fails closed later in verify_bundle_dir.
        return Ok(());
    };
    let Some(output_canonical) = canonicalize_output(output_dir) else {
        // No anchorable root; write_sidecar re-checks symlinks before writing.
        return Ok(());
    };
    if output_canonical.starts_with(&artifact_canonical)
        || artifact_canonical.starts_with(&output_canonical)
    {
        return Err(ArtifactError::new(
            code::TARGET_OVERLAP,
            format!(
                "{} output {} resolves inside the source artifact directory {}",
                target_kind,
                output_dir.display(),
                artifact_dir.display()
            ),
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// CLI entry point
// ---------------------------------------------------------------------------

/// Runs `cs experiment evaluate`: verify the artifact offline, project
/// deterministic correctness facts, publish the evaluation sidecar.
pub fn evaluate(cli: &Cli, artifact_dir: &Path, output_dir: &Path) -> Result<(), CliError> {
    // Fail closed on any symlink component of the output path, then on any
    // (lexical or filesystem-resolved) overlap with the artifact directory.
    reject_symlink_ancestors(output_dir).map_err(to_cli_error)?;
    reject_target_overlap(artifact_dir, output_dir, "evaluation").map_err(to_cli_error)?;

    let report = match artifact::verify_bundle_dir(artifact_dir) {
        Ok(report) => report,
        Err(error) => {
            // Fail closed: render the structured error projection (json/jsonl)
            // or a stderr diagnostic (human), and never publish a sidecar.
            let projection = json!({
                "source_artifact_dir": artifact_dir.display().to_string(),
                "artifact_status": "invalid",
                "code": error.code,
                "message": error.message,
                "correctness_status": "not_evaluable",
                "published": false,
            });
            if cli.output == crate::args::OutputFormat::Human {
                eprint_diagnostic(&format!("cs: {}: {}", error.code, error.message));
            } else {
                render_result(cli.output, &projection);
            }
            return Err(CliError::io(format!("{}: {}", error.code, error.message)));
        }
    };

    let created_at = chrono::Utc::now().to_rfc3339();
    let evaluation = build_evaluation(&report, artifact_dir, &created_at);
    let body = serde_json::to_string_pretty(&evaluation).unwrap_or_else(|_| "{}".to_string());
    let bundle = SidecarBundle {
        file_name: "evaluation.json",
        body,
        schema_version: EVALUATION_SCHEMA_VERSION,
        algorithm: HASH_ALGORITHM,
        manifest_domain: EVALUATION_MANIFEST_DOMAIN,
    };
    write_sidecar(&bundle, output_dir).map_err(to_cli_error)?;

    render_evaluation(cli, artifact_dir, output_dir, &evaluation);
    Ok(())
}

fn render_evaluation(cli: &Cli, artifact_dir: &Path, output_dir: &Path, evaluation: &Value) {
    let projection = json!({
        "evaluation_dir": output_dir.display().to_string(),
        "source_artifact_dir": artifact_dir.display().to_string(),
        "run_id": evaluation["source_artifact"]["run_id"],
        "session_id": evaluation["source_artifact"]["session_id"],
        "chain_head": evaluation["source_artifact"]["chain_head"],
        "evaluator_id": evaluation["evaluator_id"],
        "evaluator_version": evaluation["evaluator_version"],
        "correctness_status": evaluation["correctness_status"],
        "evaluation_hash": evaluation["integrity"]["evaluation_hash"],
    });
    match cli.output {
        crate::args::OutputFormat::Human => {
            eprint_diagnostic(&format!(
                "cs: evaluation sidecar published to {} (correctness_status={})",
                output_dir.display(),
                evaluation["correctness_status"].as_str().unwrap_or("?"),
            ));
            render_result(cli.output, &projection);
        }
        _ => render_result(cli.output, &projection),
    }
}

/// Maps an artifact/sidecar error to a CLI error, preserving the machine code.
fn to_cli_error(error: ArtifactError) -> CliError {
    CliError::io(format!("{}: {}", error.code, error.message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::OutputFormat;
    use clap::Parser as _;

    // ------------------------------------------------------------------
    // Fixtures
    // ------------------------------------------------------------------

    fn usage_summary(unpriced_tokens: i64) -> Value {
        json!({
            "version": 1, "terminal_status": "completed", "duration_ms": 10,
            "is_partial": false, "has_sub_agents": false,
            "self_usage": {
                "total_tokens": 5, "unpriced_tokens": unpriced_tokens,
                "estimated_cost": 0.001
            },
            "with_sub_agents": {
                "total_tokens": 5, "unpriced_tokens": unpriced_tokens,
                "estimated_cost": 0.001
            },
            "model_breakdowns": []
        })
    }

    fn capture_events(terminal: Option<&str>, unpriced_tokens: i64) -> Vec<Value> {
        let mut events = vec![json!({
            "id": 1, "session_id": "s1", "event_type": "workflow_started",
            "event_version": "1.0.0", "created_at": "t0",
            "event_data": { "agent_id": "builtin:coding" }
        })];
        if let Some(terminal_type) = terminal {
            let mut data = json!({});
            if terminal_type == "task_completed" {
                data = json!({ "usage_summary": usage_summary(unpriced_tokens) });
            }
            events.push(json!({
                "id": 2, "session_id": "s1", "event_type": terminal_type,
                "event_version": "1.0.0", "created_at": "t1",
                "event_data": data
            }));
        }
        events
    }

    fn write_artifact(dir: &Path, name: &str, terminal: Option<&str>, unpriced: i64) -> PathBuf {
        let snapshot = json!({
            "workflow": {
                "id": "s1", "agent_id": "builtin:coding", "status": "completed",
                "wait_reason": null, "user_query": "secret prompt text",
                "agent_config": "{\"models\":{\"act\":{\"id\":0,\"model\":\"cs@free:ds-v4-flash\"}}}",
                "is_automation_run": false
            },
            "messages": [], "has_live_session": false
        });
        let events = capture_events(terminal, unpriced);
        let input = artifact::CaptureInput {
            session_id: "s1",
            agent_id: "builtin:coding",
            server_instance_id: "inst",
            protocol_version: "1.0",
            capture_timestamp: "now",
            snapshot: &snapshot,
            events: &events,
        };
        let bundle = artifact::construct_bundle(&input).expect("construct");
        let target = dir.join(name);
        artifact::write_bundle(&bundle, &target).expect("write");
        target
    }

    fn evaluate_cli(artifact_dir: &Path, output_dir: &Path) -> Cli {
        Cli::try_parse_from([
            "cs",
            "experiment",
            "evaluate",
            artifact_dir.display().to_string().as_str(),
            "--evaluation-dir",
            output_dir.display().to_string().as_str(),
        ])
        .expect("valid args")
    }

    fn dir_hash(dir: &Path) -> Vec<(String, String)> {
        let mut entries: Vec<(String, String)> = Vec::new();
        for relative in [
            "run.json",
            "snapshot.json",
            "events.jsonl",
            "result.json",
            "artifacts/manifest.json",
        ] {
            let bytes = fs::read(dir.join(relative)).expect("artifact file");
            entries.push((relative.to_string(), artifact::blob_hash(&bytes)));
        }
        entries
    }

    // ------------------------------------------------------------------
    // Positive paths
    // ------------------------------------------------------------------

    #[test]
    fn evaluate_publishes_pass_sidecar_for_completed_artifact() {
        let tmp = tempfile::tempdir().unwrap();
        let artifact_dir = write_artifact(tmp.path(), "artifact", Some("task_completed"), 0);
        let output_dir = tmp.path().join("evaluation");

        let cli = evaluate_cli(&artifact_dir, &output_dir);
        evaluate(&cli, &artifact_dir, &output_dir).expect("evaluate ok");

        let published =
            verify_sidecar_dir(&output_dir, "evaluation.json", EVALUATION_MANIFEST_DOMAIN)
                .expect("published sidecar verifies");
        assert_eq!(
            published["schema_version"],
            json!(EVALUATION_SCHEMA_VERSION)
        );
        assert_eq!(published["evaluation_kind"], json!(EVALUATION_KIND));
        assert_eq!(published["correctness_status"], json!("pass"));
        assert_eq!(published["source_artifact"]["run_id"], json!("s1"));
        assert_eq!(published["source_artifact"]["session_id"], json!("s1"));
        assert_eq!(published["provenance"], json!(["artifact", "evaluator"]));
        let checks = published["checks"].as_array().expect("checks array");
        assert!(checks.iter().all(|check| check["status"] == json!("pass")));
        // No promotion verdict may appear anywhere in the sidecar.
        let rendered = published.to_string();
        assert!(!rendered.contains("promotion"));
    }

    #[test]
    fn evaluate_is_canonical_equivalent_across_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let artifact_dir = write_artifact(tmp.path(), "artifact", Some("task_completed"), 0);
        let first = tmp.path().join("eval-1");
        let second = tmp.path().join("eval-2");
        let cli = evaluate_cli(&artifact_dir, &first);
        evaluate(&cli, &artifact_dir, &first).expect("first evaluate");
        let cli = evaluate_cli(&artifact_dir, &second);
        evaluate(&cli, &artifact_dir, &second).expect("second evaluate");

        let read_hash = |dir: &Path| -> String {
            let value = verify_sidecar_dir(dir, "evaluation.json", EVALUATION_MANIFEST_DOMAIN)
                .expect("sidecar");
            value["integrity"]["evaluation_hash"]
                .as_str()
                .expect("evaluation_hash")
                .to_string()
        };
        assert_eq!(read_hash(&first), read_hash(&second));
    }

    #[test]
    fn evaluate_does_not_modify_source_artifact() {
        let tmp = tempfile::tempdir().unwrap();
        let artifact_dir = write_artifact(tmp.path(), "artifact", Some("task_completed"), 0);
        let before = dir_hash(&artifact_dir);
        let output_dir = tmp.path().join("evaluation");
        let cli = evaluate_cli(&artifact_dir, &output_dir);
        evaluate(&cli, &artifact_dir, &output_dir).expect("evaluate ok");
        assert_eq!(before, dir_hash(&artifact_dir));
    }

    #[test]
    fn evaluate_output_is_privacy_safe() {
        let tmp = tempfile::tempdir().unwrap();
        let artifact_dir = write_artifact(tmp.path(), "artifact", Some("task_completed"), 0);
        let output_dir = tmp.path().join("evaluation");
        let cli = evaluate_cli(&artifact_dir, &output_dir);
        evaluate(&cli, &artifact_dir, &output_dir).expect("evaluate ok");

        let body = fs::read_to_string(output_dir.join("evaluation.json")).unwrap();
        for forbidden in [
            "secret prompt text",  // raw prompt must never appear
            "cs@free:ds-v4-flash", // config projection stays in 2A only
            "Bearer ",
            "sk-",
            "BEGIN PRIVATE KEY",
        ] {
            assert!(!body.contains(forbidden), "sidecar leaked {:?}", forbidden);
        }
        // path_hint is the directory file name, never an absolute path.
        assert!(!body.contains(tmp.path().display().to_string().as_str()));
    }

    // ------------------------------------------------------------------
    // Structured status distinctions
    // ------------------------------------------------------------------

    #[test]
    fn failed_terminal_maps_to_correctness_fail() {
        let tmp = tempfile::tempdir().unwrap();
        let artifact_dir = write_artifact(tmp.path(), "artifact", Some("workflow_failed"), 0);
        let output_dir = tmp.path().join("evaluation");
        let cli = evaluate_cli(&artifact_dir, &output_dir);
        evaluate(&cli, &artifact_dir, &output_dir).expect("evaluate publishes sidecar");
        let published =
            verify_sidecar_dir(&output_dir, "evaluation.json", EVALUATION_MANIFEST_DOMAIN).unwrap();
        assert_eq!(published["correctness_status"], json!("fail"));
        let outcome = published["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|check| check["check_id"] == json!("terminal_outcome_completed"))
            .unwrap();
        assert_eq!(outcome["status"], json!("fail"));
    }

    #[test]
    fn cancelled_terminal_maps_to_correctness_fail() {
        let tmp = tempfile::tempdir().unwrap();
        let artifact_dir = write_artifact(tmp.path(), "artifact", Some("workflow_cancelled"), 0);
        let output_dir = tmp.path().join("evaluation");
        let cli = evaluate_cli(&artifact_dir, &output_dir);
        evaluate(&cli, &artifact_dir, &output_dir).expect("evaluate publishes sidecar");
        let published =
            verify_sidecar_dir(&output_dir, "evaluation.json", EVALUATION_MANIFEST_DOMAIN).unwrap();
        assert_eq!(published["correctness_status"], json!("fail"));
    }

    #[test]
    fn incomplete_artifact_maps_to_not_evaluable() {
        let tmp = tempfile::tempdir().unwrap();
        // No terminal event: artifact is incomplete with unknown terminal.
        let artifact_dir = write_artifact(tmp.path(), "artifact", None, 0);
        let output_dir = tmp.path().join("evaluation");
        let cli = evaluate_cli(&artifact_dir, &output_dir);
        evaluate(&cli, &artifact_dir, &output_dir).expect("evaluate publishes sidecar");
        let published =
            verify_sidecar_dir(&output_dir, "evaluation.json", EVALUATION_MANIFEST_DOMAIN).unwrap();
        assert_eq!(published["correctness_status"], json!("not_evaluable"));
    }

    #[test]
    fn unknown_cost_maps_to_not_evaluable_not_pass() {
        let tmp = tempfile::tempdir().unwrap();
        // unpriced_tokens > 0 forces cost_status=unknown in the 2A projection.
        let artifact_dir = write_artifact(tmp.path(), "artifact", Some("task_completed"), 7);
        let output_dir = tmp.path().join("evaluation");
        let cli = evaluate_cli(&artifact_dir, &output_dir);
        evaluate(&cli, &artifact_dir, &output_dir).expect("evaluate publishes sidecar");
        let published =
            verify_sidecar_dir(&output_dir, "evaluation.json", EVALUATION_MANIFEST_DOMAIN).unwrap();
        assert_eq!(published["correctness_status"], json!("not_evaluable"));
        let cost = published["checks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|check| check["check_id"] == json!("cost_status_known"))
            .unwrap();
        assert_eq!(cost["status"], json!("not_evaluable"));
        assert_eq!(cost["detail"]["cost_status"], json!("unknown"));
    }

    #[test]
    fn budget_rejection_token_blocks_correctness_pass() {
        let tmp = tempfile::tempdir().unwrap();
        let snapshot = json!({
            "workflow": {
                "id": "s1", "agent_id": "builtin:coding", "status": "completed",
                "wait_reason": null, "user_query": "prompt",
                "agent_config": "{\"models\":{\"act\":{\"id\":0,\"model\":\"m\"}}}",
                "is_automation_run": false
            },
            "messages": [], "has_live_session": false
        });
        let events = vec![
            json!({
                "id": 1, "session_id": "s1", "event_type": "workflow_started",
                "event_version": "1.0.0", "created_at": "t0", "event_data": {}
            }),
            json!({
                "id": 2, "session_id": "s1", "event_type": "tool_observation",
                "event_version": "1.0.0", "created_at": "t1",
                "event_data": { "error_type": "budget_exceeded" }
            }),
            json!({
                "id": 3, "session_id": "s1", "event_type": "task_completed",
                "event_version": "1.0.0", "created_at": "t2",
                "event_data": { "usage_summary": usage_summary(0) }
            }),
        ];
        let input = artifact::CaptureInput {
            session_id: "s1",
            agent_id: "builtin:coding",
            server_instance_id: "inst",
            protocol_version: "1.0",
            capture_timestamp: "now",
            snapshot: &snapshot,
            events: &events,
        };
        let bundle = artifact::construct_bundle(&input).expect("construct");
        let artifact_dir = tmp.path().join("artifact");
        artifact::write_bundle(&bundle, &artifact_dir).expect("write");

        let output_dir = tmp.path().join("evaluation");
        let cli = evaluate_cli(&artifact_dir, &output_dir);
        evaluate(&cli, &artifact_dir, &output_dir).expect("evaluate publishes sidecar");
        let published =
            verify_sidecar_dir(&output_dir, "evaluation.json", EVALUATION_MANIFEST_DOMAIN).unwrap();
        // A budget rejection is an infra fact: it must never become a pass.
        assert_eq!(published["correctness_status"], json!("not_evaluable"));
    }

    // ------------------------------------------------------------------
    // Fail-closed negative paths
    // ------------------------------------------------------------------

    #[test]
    fn evaluate_fails_closed_on_tampered_artifact_without_publishing() {
        let tmp = tempfile::tempdir().unwrap();
        let artifact_dir = write_artifact(tmp.path(), "artifact", Some("task_completed"), 0);
        let run_path = artifact_dir.join("run.json");
        let mut run: Value = serde_json::from_str(&fs::read_to_string(&run_path).unwrap()).unwrap();
        run["agent_id"] = json!("evil-agent");
        fs::write(&run_path, serde_json::to_string_pretty(&run).unwrap()).unwrap();

        let output_dir = tmp.path().join("evaluation");
        let cli = evaluate_cli(&artifact_dir, &output_dir);
        let error = evaluate(&cli, &artifact_dir, &output_dir).expect_err("must fail closed");
        assert!(matches!(error, CliError::Io(_)));
        assert!(
            !output_dir.exists(),
            "no sidecar may be published on failure"
        );
    }

    #[test]
    fn evaluate_fails_closed_on_future_schema() {
        let tmp = tempfile::tempdir().unwrap();
        let artifact_dir = write_artifact(tmp.path(), "artifact", Some("task_completed"), 0);
        // Bump the schema version and refresh the manifest so only the schema
        // gate can reject it (the manifest itself stays internally consistent).
        let run_path = artifact_dir.join("run.json");
        let mut run: Value = serde_json::from_str(&fs::read_to_string(&run_path).unwrap()).unwrap();
        run["schema_version"] = json!(artifact::SUPPORTED_SCHEMA_VERSION + 1);
        let text = serde_json::to_string_pretty(&run).unwrap();
        fs::write(&run_path, &text).unwrap();
        refresh_manifest(&artifact_dir);

        let output_dir = tmp.path().join("evaluation");
        let cli = evaluate_cli(&artifact_dir, &output_dir);
        let error = evaluate(&cli, &artifact_dir, &output_dir).expect_err("must fail closed");
        let message = error.to_string();
        assert!(
            message.contains(artifact::code::INCOMPATIBLE),
            "expected incompatible_schema code, got: {}",
            message
        );
        assert!(!output_dir.exists());
    }

    #[test]
    fn evaluate_fails_closed_on_missing_artifact() {
        let tmp = tempfile::tempdir().unwrap();
        let artifact_dir = tmp.path().join("missing-artifact");
        let output_dir = tmp.path().join("evaluation");
        let cli = evaluate_cli(&artifact_dir, &output_dir);
        let error = evaluate(&cli, &artifact_dir, &output_dir).expect_err("must fail closed");
        assert!(error.to_string().contains(artifact::code::MISSING_FILE));
        assert!(!output_dir.exists());
    }

    #[test]
    fn evaluate_rejects_existing_output_target() {
        let tmp = tempfile::tempdir().unwrap();
        let artifact_dir = write_artifact(tmp.path(), "artifact", Some("task_completed"), 0);
        let output_dir = tmp.path().join("evaluation");
        fs::create_dir_all(&output_dir).unwrap();
        let cli = evaluate_cli(&artifact_dir, &output_dir);
        let error = evaluate(&cli, &artifact_dir, &output_dir).expect_err("must fail closed");
        assert!(error.to_string().contains(artifact::code::TARGET_EXISTS));
    }

    #[test]
    fn evaluate_rejects_output_inside_artifact_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let artifact_dir = write_artifact(tmp.path(), "artifact", Some("task_completed"), 0);
        let output_dir = artifact_dir.join("nested-evaluation");
        let cli = evaluate_cli(&artifact_dir, &output_dir);
        let error = evaluate(&cli, &artifact_dir, &output_dir).expect_err("must fail closed");
        assert!(error.to_string().contains(code::TARGET_OVERLAP));
        assert!(!output_dir.exists());
    }

    #[test]
    fn evaluate_rejects_artifact_inside_output_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let artifact_dir = write_artifact(tmp.path(), "artifact", Some("task_completed"), 0);
        let output_dir = tmp.path();
        let cli = evaluate_cli(&artifact_dir, &output_dir);
        let error = evaluate(&cli, &artifact_dir, &output_dir).expect_err("must fail closed");
        assert!(error.to_string().contains(code::TARGET_OVERLAP));
    }

    #[test]
    fn evaluate_rejects_symlinked_output_target() {
        let tmp = tempfile::tempdir().unwrap();
        let artifact_dir = write_artifact(tmp.path(), "artifact", Some("task_completed"), 0);
        let link_target = tmp.path().join("elsewhere");
        fs::create_dir_all(&link_target).unwrap();
        let output_dir = tmp.path().join("evaluation-link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&link_target, &output_dir).unwrap();

        let cli = evaluate_cli(&artifact_dir, &output_dir);
        let error = evaluate(&cli, &artifact_dir, &output_dir).expect_err("must fail closed");
        // The symlink component itself is rejected before the exists check.
        assert!(error.to_string().contains(artifact::code::SYMLINK));
    }

    #[test]
    fn evaluate_rejects_symlinked_parent_targeting_artifact() {
        let tmp = tempfile::tempdir().unwrap();
        let artifact_dir = write_artifact(tmp.path(), "artifact", Some("task_completed"), 0);
        let before = dir_hash(&artifact_dir);
        // A symlinked parent that resolves into the artifact directory must
        // never redirect the sidecar into the immutable 2A bundle.
        #[cfg(unix)]
        std::os::unix::fs::symlink(&artifact_dir, tmp.path().join("link")).unwrap();
        let output_dir = tmp.path().join("link").join("evaluation");

        let cli = evaluate_cli(&artifact_dir, &output_dir);
        let error = evaluate(&cli, &artifact_dir, &output_dir).expect_err("must fail closed");
        assert!(error.to_string().contains(artifact::code::SYMLINK));
        assert!(!output_dir.exists(), "no sidecar may be published");
        assert_eq!(
            before,
            dir_hash(&artifact_dir),
            "artifact must be unchanged"
        );
    }

    #[test]
    fn evaluate_allows_sibling_output_under_real_parent() {
        // Regression guard for the canonical overlap check: sibling output
        // directories under a real (non-symlink) parent stay allowed.
        let tmp = tempfile::tempdir().unwrap();
        let artifact_dir = write_artifact(tmp.path(), "artifact", Some("task_completed"), 0);
        let output_dir = tmp.path().join("evaluation");
        let cli = evaluate_cli(&artifact_dir, &output_dir);
        evaluate(&cli, &artifact_dir, &output_dir).expect("sibling output is allowed");
    }

    // ------------------------------------------------------------------
    // Sidecar writer integrity
    // ------------------------------------------------------------------

    #[test]
    fn sidecar_verifier_rejects_tampered_data_file() {
        let tmp = tempfile::tempdir().unwrap();
        let artifact_dir = write_artifact(tmp.path(), "artifact", Some("task_completed"), 0);
        let output_dir = tmp.path().join("evaluation");
        let cli = evaluate_cli(&artifact_dir, &output_dir);
        evaluate(&cli, &artifact_dir, &output_dir).expect("evaluate ok");

        let data_path = output_dir.join("evaluation.json");
        let mut value: Value =
            serde_json::from_str(&fs::read_to_string(&data_path).unwrap()).unwrap();
        // Tamper with a field that currently holds a different value so the
        // rewrite actually changes the bytes.
        value["evaluator_version"] = json!("tampered");
        fs::write(&data_path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

        let error = verify_sidecar_dir(&output_dir, "evaluation.json", EVALUATION_MANIFEST_DOMAIN)
            .expect_err("tampered sidecar must fail");
        assert_eq!(error.code, artifact::code::HASH);
    }

    #[test]
    fn sidecar_verifier_rejects_extra_manifest_files() {
        let tmp = tempfile::tempdir().unwrap();
        let artifact_dir = write_artifact(tmp.path(), "artifact", Some("task_completed"), 0);
        let output_dir = tmp.path().join("evaluation");
        let cli = evaluate_cli(&artifact_dir, &output_dir);
        evaluate(&cli, &artifact_dir, &output_dir).expect("evaluate ok");

        // Rewrite the manifest with an extra file entry and a matching hash.
        let manifest_path = output_dir.join("artifacts").join("manifest.json");
        let mut manifest: Value =
            serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
        let extra = json!({
            "path": "extra.json",
            "size": 2,
            "sha256": artifact::blob_hash(b"{}"),
        });
        manifest["files"].as_array_mut().unwrap().push(extra);
        let files_value = manifest["files"].clone();
        manifest["manifest_hash"] = json!(artifact::canonical_hash(
            EVALUATION_MANIFEST_DOMAIN,
            &files_value
        ));
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let error = verify_sidecar_dir(&output_dir, "evaluation.json", EVALUATION_MANIFEST_DOMAIN)
            .expect_err("extra file must fail");
        assert_eq!(error.code, artifact::code::MANIFEST);
    }

    #[test]
    fn sidecar_writer_never_publishes_on_staging_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = SidecarBundle {
            file_name: "evaluation.json",
            body: "{}".to_string(),
            schema_version: EVALUATION_SCHEMA_VERSION,
            algorithm: HASH_ALGORITHM,
            manifest_domain: EVALUATION_MANIFEST_DOMAIN,
        };
        // A body that is not valid JSON must fail the staged re-verify.
        let broken = SidecarBundle {
            body: "not-json".to_string(),
            ..bundle
        };
        let output_dir = tmp.path().join("evaluation");
        let error = write_sidecar(&broken, &output_dir).expect_err("must fail");
        assert_eq!(error.code, artifact::code::INVALID);
        assert!(!output_dir.exists());
        // No staging leftovers.
        let leftovers: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().contains(".staging-"))
            .collect();
        assert!(leftovers.is_empty(), "staging directories must be cleaned");
    }

    #[test]
    fn correctness_status_derivation_is_structured() {
        let pass = |class, status| Check {
            check_id: "x",
            class,
            status,
            detail: json!({}),
        };
        assert_eq!(correctness_status(&[]), "pass");
        assert_eq!(
            correctness_status(&[pass(CheckClass::Correctness, CheckStatus::Pass)]),
            "pass"
        );
        assert_eq!(
            correctness_status(&[pass(CheckClass::Correctness, CheckStatus::Fail)]),
            "fail"
        );
        // Infra failure blocks a pass but is not a correctness failure.
        assert_eq!(
            correctness_status(&[
                pass(CheckClass::Correctness, CheckStatus::Pass),
                pass(CheckClass::Infra, CheckStatus::Fail),
            ]),
            "not_evaluable"
        );
        assert_eq!(
            correctness_status(&[pass(CheckClass::Correctness, CheckStatus::NotEvaluable)]),
            "not_evaluable"
        );
    }

    #[test]
    fn human_output_renders_without_panicking() {
        let tmp = tempfile::tempdir().unwrap();
        let artifact_dir = write_artifact(tmp.path(), "artifact", Some("task_completed"), 0);
        let output_dir = tmp.path().join("evaluation");
        let cli = evaluate_cli(&artifact_dir, &output_dir);
        assert_eq!(cli.output, OutputFormat::Human);
        evaluate(&cli, &artifact_dir, &output_dir).expect("evaluate ok");
    }

    /// Rebuilds the 2A manifest after an intentional test tamper so only the
    /// targeted gate (schema) rejects the artifact.
    fn refresh_manifest(artifact_dir: &Path) {
        let files: Vec<Value> = ["run.json", "snapshot.json", "events.jsonl", "result.json"]
            .iter()
            .map(|relative| {
                let bytes = fs::read(artifact_dir.join(relative)).unwrap();
                json!({
                    "path": relative,
                    "size": bytes.len() as u64,
                    "sha256": artifact::blob_hash(&bytes),
                })
            })
            .collect();
        let files_value = Value::Array(files);
        let manifest = json!({
            "schema_version": artifact::SCHEMA_VERSION,
            "algorithm": artifact::HASH_ALGORITHM,
            "status": "complete",
            "files": files_value,
            "manifest_hash": artifact::canonical_hash("cs-artifact:manifest", &files_value),
        });
        fs::write(
            artifact_dir.join("artifacts").join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }
}
