//! Phase 2E independent deterministic verifier for `chatspeed-smoke@1`.
//!
//! The verifier is deliberately separate from the adapter and from the model
//! path: it reads only the checked-in fixture (via the adapter's digest-bound
//! resolver) and an already-verified 2A artifact (`VerifyReport`), then
//! computes deterministic score/verdict facts. Model self-reports, free text,
//! and model-generated logs are never score inputs.
//!
//! Contract-level boundary note: this local verifier is a candidate-untrusted
//! *contract* boundary, not the OS/container isolation planned for 2G+2H.
//!
//! Hard-fail contract: any invalid artifact, unknown task/suite, digest
//! mismatch, or unsafe output target returns a stable machine code and never
//! publishes a verdict. A valid but failing run still publishes a verdict
//! with `score = 0.0` (the verdict is a fact, not a promotion decision).

use crate::args::Cli;
use crate::artifact::{self, VerifyReport};
use crate::benchmark::{self, ResolvedTask, SmokeTaskV1};
use crate::error::CliError;
use crate::evaluate::{write_sidecar, SidecarBundle};
use crate::output::{eprint_diagnostic, render_result};
use serde_json::{json, Map, Value};
use std::path::Path;

/// Verdict document schema version. Version 1 remains a valid published
/// artifact; version 2 adds explicit independent-coverage facts for admission
/// caps that artifact v1 cannot measure.
pub const VERDICT_SCHEMA_VERSION: u32 = 2;
/// Sidecar manifest schema is independent of the versioned verdict document.
const SIDECAR_MANIFEST_SCHEMA_VERSION: u32 = 1;
/// Fixed verdict kind label.
pub const VERDICT_KIND: &str = "cs.benchmark.verdict";
/// Verifier identity is inherited from the checked-in manifest.
pub const VERIFIER_HASH_DOMAIN: &str = "cs-benchmark:verifier";
/// Domain for the verdict content hash.
pub const VERDICT_HASH_DOMAIN: &str = "cs-benchmark:verdict";
/// Domain for the verdict sidecar manifest.
pub const VERDICT_MANIFEST_DOMAIN: &str = "cs-benchmark:verdict-manifest";

/// Machine-stable error codes specific to the verdict sidecar. Fixture and
/// artifact failures reuse the stable `benchmark::code`/`artifact::code`.
pub mod code {
    /// The verdict output target overlaps the source artifact directory.
    pub const TARGET_OVERLAP: &str = "verdict_target_unsafe";
}

/// Status of a single deterministic verdict check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictCheckStatus {
    Pass,
    Fail,
    NotApplicable,
}

impl VerdictCheckStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            VerdictCheckStatus::Pass => "pass",
            VerdictCheckStatus::Fail => "fail",
            VerdictCheckStatus::NotApplicable => "not_applicable",
        }
    }
}

/// One deterministic verdict check fact.
#[derive(Debug, Clone)]
pub struct VerdictCheck {
    pub check_id: &'static str,
    pub status: VerdictCheckStatus,
    pub detail: Value,
}

/// Budget/admission rejection tokens that must never yield a passing score.
const BUDGET_REJECTION_TOKENS: &[&str] = &[
    "budget_exceeded",
    "scope_paused",
    "resource_unobservable",
    "experiment admission rejected",
];

/// Computes the deterministic verdict checks for a resolved task against a
/// verified artifact report. Only structured, trusted facts are consulted.
pub fn run_verdict_checks(resolved: &ResolvedTask, report: &VerifyReport) -> Vec<VerdictCheck> {
    let expected = &resolved.task.expected;
    let profile = &resolved.task.resource_profile;
    let usage = report.result.get("usage").cloned().unwrap_or(Value::Null);
    // `with_sub_agents` totals cover the whole run (self + sub-agents).
    let totals = usage.get("with_sub_agents").cloned().unwrap_or(Value::Null);

    let haystack = serde_json::to_string(&report.events).unwrap_or_default();
    let budget_rejected = BUDGET_REJECTION_TOKENS
        .iter()
        .any(|token| haystack.contains(token));

    let mut checks = vec![
        VerdictCheck {
            check_id: "artifact_complete",
            status: match report.status {
                artifact::ArtifactStatus::Complete => VerdictCheckStatus::Pass,
                artifact::ArtifactStatus::Incomplete => VerdictCheckStatus::Fail,
            },
            detail: json!({ "artifact_status": report.status.as_str() }),
        },
        VerdictCheck {
            check_id: "terminal_status_matches_expected",
            status: if expected.terminal_status == report.terminal_status.as_str() {
                VerdictCheckStatus::Pass
            } else {
                VerdictCheckStatus::Fail
            },
            detail: json!({
                "expected": expected.terminal_status,
                "actual": report.terminal_status.as_str(),
            }),
        },
        VerdictCheck {
            check_id: "cost_status_matches_expected",
            status: if expected.cost_status == report.cost_status {
                VerdictCheckStatus::Pass
            } else {
                VerdictCheckStatus::Fail
            },
            detail: json!({
                "expected": expected.cost_status,
                "actual": report.cost_status,
            }),
        },
        VerdictCheck {
            check_id: "no_budget_rejection",
            status: if budget_rejected == !expected.no_budget_rejection {
                VerdictCheckStatus::Pass
            } else {
                VerdictCheckStatus::Fail
            },
            detail: json!({
                "expected_no_rejection": expected.no_budget_rejection,
                "rejection_tokens_found": budget_rejected,
            }),
        },
    ];

    // Resource-cap facts independently observable from the 2A artifact usage
    // projection. A `None` cap is explicit not_applicable.
    let cap_checks: [(&'static str, Option<u64>, Option<i64>); 4] = [
        (
            "usage_within_input_cap",
            Some(profile.input_tokens),
            totals.get("input_tokens").and_then(Value::as_i64),
        ),
        (
            "usage_within_output_cap",
            Some(profile.output_tokens),
            totals.get("output_tokens").and_then(Value::as_i64),
        ),
        (
            "usage_within_cache_read_cap",
            profile.cache_read_tokens,
            totals.get("cache_read_tokens").and_then(Value::as_i64),
        ),
        (
            "usage_within_cache_write_cap",
            profile.cache_write_tokens,
            totals.get("cache_write_tokens").and_then(Value::as_i64),
        ),
    ];
    for (check_id, cap, actual) in cap_checks {
        checks.push(match (cap, actual) {
            (Some(limit), Some(used)) => VerdictCheck {
                check_id,
                status: if used >= 0 && (used as u64) <= limit {
                    VerdictCheckStatus::Pass
                } else {
                    VerdictCheckStatus::Fail
                },
                detail: json!({ "cap": limit, "actual": used }),
            },
            (None, _) => VerdictCheck {
                check_id,
                status: VerdictCheckStatus::NotApplicable,
                detail: json!({ "cap": Value::Null }),
            },
            (Some(limit), None) => VerdictCheck {
                check_id,
                status: VerdictCheckStatus::Fail,
                detail: json!({ "cap": limit, "actual": Value::Null }),
            },
        });
    }

    // Wall-time cap: the artifact usage projection records duration_ms.
    let duration_ms = usage.get("duration_ms").and_then(Value::as_i64);
    checks.push(match (profile.wall_time_ms, duration_ms) {
        (limit, Some(actual)) => VerdictCheck {
            check_id: "usage_within_wall_time_cap",
            status: if actual >= 0 && (actual as u64) <= limit {
                VerdictCheckStatus::Pass
            } else {
                VerdictCheckStatus::Fail
            },
            detail: json!({ "cap": limit, "actual": actual }),
        },
        (limit, None) => VerdictCheck {
            check_id: "usage_within_wall_time_cap",
            status: VerdictCheckStatus::Fail,
            detail: json!({ "cap": limit, "actual": Value::Null }),
        },
    });

    // `tool_calls`, `processes`, and peak `concurrency` remain admission
    // controls in 2C, but artifact v1 does not carry independently verifiable
    // actuals for them. Publish that coverage boundary explicitly rather than
    // allowing a score consumer to infer a false pass from omitted checks.
    for (check_id, limit) in [
        ("usage_within_tool_calls_cap", profile.tool_calls),
        ("usage_within_processes_cap", profile.processes),
        ("usage_within_concurrency_cap", profile.concurrency),
    ] {
        checks.push(VerdictCheck {
            check_id,
            status: VerdictCheckStatus::NotApplicable,
            detail: json!({
                "cap": limit,
                "actual": Value::Null,
                "reason": "not_recorded_in_artifact_v1",
            }),
        });
    }

    checks
}

/// The deterministic score: 1.0 only when every independently verifiable
/// check passes. Explicit `not_applicable` checks document admission controls
/// that artifact v1 cannot independently measure; they are not pass evidence.
pub fn score(checks: &[VerdictCheck]) -> f64 {
    if checks.iter().all(|check| {
        matches!(
            check.status,
            VerdictCheckStatus::Pass | VerdictCheckStatus::NotApplicable
        )
    }) {
        1.0
    } else {
        0.0
    }
}

/// Digest binding the verifier identity to the exact expected-facts contract
/// it enforces for this task (id/version + expected facts + resource profile).
pub fn verifier_digest(task: &SmokeTaskV1) -> String {
    artifact::canonical_hash(
        VERIFIER_HASH_DOMAIN,
        &json!({
            "verifier_id": task.verifier_id,
            "verifier_version": task.verifier_version,
            "expected": task.expected,
            "resource_profile": task.resource_profile,
        }),
    )
}

/// Builds the `verdict.json` document. `created_at` is excluded from
/// `verdict_hash` so repeated verification of the same artifact/fixture is
/// canonical-equivalent.
pub fn build_verdict(
    resolved: &ResolvedTask,
    report: &VerifyReport,
    artifact_dir: &Path,
    created_at: &str,
) -> Value {
    let checks = run_verdict_checks(resolved, report);
    let profile = &resolved.task.resource_profile;
    let total = score(&checks);
    let check_values: Vec<Value> = checks
        .iter()
        .map(|check| {
            json!({
                "check_id": check.check_id,
                "status": check.status.as_str(),
                "detail": check.detail,
            })
        })
        .collect();

    let safety_fail = checks.iter().any(|check| {
        check.check_id == "no_budget_rejection" && check.status == VerdictCheckStatus::Fail
    });
    let infra_fail = checks.iter().any(|check| {
        check.check_id != "no_budget_rejection"
            && check.check_id != "terminal_status_matches_expected"
            && check.status == VerdictCheckStatus::Fail
    });

    let path_hint = artifact_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");

    let mut verdict = Map::new();
    verdict.insert("schema_version".into(), json!(VERDICT_SCHEMA_VERSION));
    verdict.insert("verdict_kind".into(), json!(VERDICT_KIND));
    verdict.insert("dataset_id".into(), json!(resolved.manifest.dataset_id));
    verdict.insert(
        "dataset_version".into(),
        json!(resolved.manifest.dataset_version),
    );
    verdict.insert("dataset_digest".into(), json!(resolved.manifest_digest));
    verdict.insert("split".into(), json!(resolved.manifest.split));
    verdict.insert("task_id".into(), json!(resolved.task.task_id));
    verdict.insert("task_digest".into(), json!(resolved.task_digest));
    verdict.insert("verifier_id".into(), json!(resolved.task.verifier_id));
    verdict.insert(
        "verifier_version".into(),
        json!(resolved.task.verifier_version),
    );
    verdict.insert(
        "verifier_digest".into(),
        json!(verifier_digest(&resolved.task)),
    );
    verdict.insert(
        "source_artifact".into(),
        json!({
            "path_hint": path_hint,
            "run_id": report.run.get("run_id").and_then(Value::as_str).unwrap_or(""),
            "session_id": report.run.get("session_id").and_then(Value::as_str).unwrap_or(""),
            "chain_head": report.chain_head,
        }),
    );
    verdict.insert("score".into(), json!(total));
    verdict.insert("metrics".into(), Value::Array(check_values));
    verdict.insert(
        "safety_status".into(),
        json!(if safety_fail { "fail" } else { "pass" }),
    );
    verdict.insert(
        "infra_status".into(),
        json!(if infra_fail { "fail" } else { "pass" }),
    );
    verdict.insert(
        "budget_facts".into(),
        json!({
            "cost_status": report.cost_status,
            "money_mode": "token_resource_only",
            "admission_caps": resolved.task.resource_profile,
            "independently_verified_caps": {
                "input_tokens": profile.input_tokens,
                "output_tokens": profile.output_tokens,
                "cache_read_tokens": profile.cache_read_tokens,
                "cache_write_tokens": profile.cache_write_tokens,
                "wall_time_ms": profile.wall_time_ms,
            },
            "unverified_admission_caps": {
                "tool_calls": {
                    "cap": profile.tool_calls,
                    "reason": "not_recorded_in_artifact_v1",
                },
                "processes": {
                    "cap": profile.processes,
                    "reason": "not_recorded_in_artifact_v1",
                },
                "concurrency": {
                    "cap": profile.concurrency,
                    "reason": "not_recorded_in_artifact_v1",
                },
            },
        }),
    );
    verdict.insert(
        "provenance".into(),
        json!(["artifact", "benchmark_adapter", "independent_verifier"]),
    );
    verdict.insert("created_at".into(), json!(created_at));

    let deterministic_content = {
        let mut content = verdict.clone();
        content.remove("created_at");
        Value::Object(content)
    };
    let verdict_hash = artifact::canonical_hash(VERDICT_HASH_DOMAIN, &deterministic_content);
    verdict.insert(
        "integrity".into(),
        json!({
            "algorithm": artifact::HASH_ALGORITHM,
            "verdict_hash": verdict_hash,
        }),
    );
    Value::Object(verdict)
}

// ---------------------------------------------------------------------------
// CLI entry point
// ---------------------------------------------------------------------------

/// Runs `cs experiment benchmark verify`: resolve the fixed fixture task,
/// verify the artifact offline, compute the deterministic verdict, and
/// publish the verdict sidecar. Never produces a promotion decision.
pub fn verify(
    cli: &Cli,
    suite: &str,
    task_id: &str,
    artifact_dir: &Path,
    verdict_dir: &Path,
) -> Result<(), CliError> {
    // Output target safety first: fail closed on any symlink component of the
    // verdict path, then on any (lexical or filesystem-resolved) overlap with
    // the immutable 2A artifact directory.
    crate::evaluate::reject_symlink_ancestors(verdict_dir)
        .map_err(|error| CliError::io(format!("{}: {}", error.code, error.message)))?;
    crate::evaluate::reject_target_overlap(artifact_dir, verdict_dir, "verdict").map_err(
        |error| {
            // Keep the verdict-specific machine code for overlap failures.
            let code = if error.code == crate::evaluate::code::TARGET_OVERLAP {
                code::TARGET_OVERLAP
            } else {
                error.code
            };
            CliError::io(format!("{}: {}", code, error.message))
        },
    )?;

    // Fixed fixture identity (digest-bound; unknown task/suite hard-fails).
    let resolved = benchmark::resolve_task(suite, task_id)
        .map_err(|error| CliError::io(format!("{}: {}", error.code, error.message)))?;

    // Trusted artifact facts only.
    let report = match artifact::verify_bundle_dir(artifact_dir) {
        Ok(report) => report,
        Err(error) => {
            render_verify_failure(cli, artifact_dir, verdict_dir, error.code, &error.message);
            return Err(CliError::io(format!("{}: {}", error.code, error.message)));
        }
    };

    let created_at = chrono::Utc::now().to_rfc3339();
    let verdict = build_verdict(&resolved, &report, artifact_dir, &created_at);
    let body = serde_json::to_string_pretty(&verdict).unwrap_or_else(|_| "{}".to_string());
    let bundle = SidecarBundle {
        file_name: "verdict.json",
        body,
        schema_version: SIDECAR_MANIFEST_SCHEMA_VERSION,
        algorithm: artifact::HASH_ALGORITHM,
        manifest_domain: VERDICT_MANIFEST_DOMAIN,
    };
    write_sidecar(&bundle, verdict_dir)
        .map_err(|error| CliError::io(format!("{}: {}", error.code, error.message)))?;

    render_verdict(cli, artifact_dir, verdict_dir, &verdict);
    Ok(())
}

fn render_verify_failure(
    cli: &Cli,
    artifact_dir: &Path,
    verdict_dir: &Path,
    error_code: &str,
    message: &str,
) {
    let projection = json!({
        "source_artifact_dir": artifact_dir.display().to_string(),
        "verdict_dir": verdict_dir.display().to_string(),
        "verdict_status": "invalid",
        "code": error_code,
        "message": message,
        "published": false,
    });
    if cli.output == crate::args::OutputFormat::Human {
        eprint_diagnostic(&format!("cs: {}: {}", error_code, message));
    } else {
        render_result(cli.output, &projection);
    }
}

fn render_verdict(cli: &Cli, artifact_dir: &Path, verdict_dir: &Path, verdict: &Value) {
    let projection = json!({
        "verdict_dir": verdict_dir.display().to_string(),
        "source_artifact_dir": artifact_dir.display().to_string(),
        "dataset_id": verdict["dataset_id"],
        "dataset_version": verdict["dataset_version"],
        "dataset_digest": verdict["dataset_digest"],
        "split": verdict["split"],
        "task_id": verdict["task_id"],
        "task_digest": verdict["task_digest"],
        "verifier_id": verdict["verifier_id"],
        "verifier_version": verdict["verifier_version"],
        "run_id": verdict["source_artifact"]["run_id"],
        "session_id": verdict["source_artifact"]["session_id"],
        "chain_head": verdict["source_artifact"]["chain_head"],
        "score": verdict["score"],
        "safety_status": verdict["safety_status"],
        "infra_status": verdict["infra_status"],
        "verdict_hash": verdict["integrity"]["verdict_hash"],
    });
    match cli.output {
        crate::args::OutputFormat::Human => {
            eprint_diagnostic(&format!(
                "cs: verdict sidecar published to {} (score={}, safety={}, infra={})",
                verdict_dir.display(),
                verdict["score"],
                verdict["safety_status"].as_str().unwrap_or("?"),
                verdict["infra_status"].as_str().unwrap_or("?"),
            ));
            render_result(cli.output, &projection);
        }
        _ => render_result(cli.output, &projection),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::CaptureInput;
    use crate::benchmark::code as benchmark_code;
    use crate::benchmark::SUITE_ID;
    use clap::Parser as _;
    use std::fs;
    use std::path::PathBuf;

    // ------------------------------------------------------------------
    // Fixtures
    // ------------------------------------------------------------------

    fn usage_summary() -> Value {
        json!({
            "version": 1, "terminal_status": "completed", "duration_ms": 1200,
            "is_partial": false, "has_sub_agents": false,
            "self_usage": {
                "input_tokens": 100, "output_tokens": 5, "cache_tokens": 0,
                "cache_write_tokens": 0, "total_tokens": 105,
                "unpriced_tokens": 0, "estimated_cost": 0.0
            },
            "with_sub_agents": {
                "input_tokens": 100, "output_tokens": 5, "cache_tokens": 0,
                "cache_write_tokens": 0, "total_tokens": 105,
                "unpriced_tokens": 0, "estimated_cost": 0.0
            },
            "model_breakdowns": []
        })
    }

    fn write_artifact(dir: &Path, name: &str, terminal: Option<&str>) -> PathBuf {
        let snapshot = json!({
            "workflow": {
                "id": "s1", "agent_id": "builtin:coding", "status": "completed",
                "wait_reason": null, "user_query": "secret prompt text",
                "agent_config": "{\"models\":{\"act\":{\"id\":0,\"model\":\"cs@free:ds-v4-flash\"}}}",
                "is_automation_run": false
            },
            "messages": [], "has_live_session": false
        });
        let mut events = vec![json!({
            "id": 1, "session_id": "s1", "event_type": "workflow_started",
            "event_version": "1.0.0", "created_at": "t0", "event_data": {}
        })];
        if let Some(terminal_type) = terminal {
            let data = if terminal_type == "task_completed" {
                json!({ "usage_summary": usage_summary() })
            } else {
                json!({})
            };
            events.push(json!({
                "id": 2, "session_id": "s1", "event_type": terminal_type,
                "event_version": "1.0.0", "created_at": "t1", "event_data": data
            }));
        }
        let input = CaptureInput {
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

    fn verify_cli(artifact_dir: &Path, verdict_dir: &Path) -> Cli {
        Cli::try_parse_from([
            "cs",
            "experiment",
            "benchmark",
            "verify",
            "--suite",
            SUITE_ID,
            "--task",
            "smoke_reply_ok",
            artifact_dir.display().to_string().as_str(),
            "--verdict-dir",
            verdict_dir.display().to_string().as_str(),
        ])
        .expect("valid args")
    }

    fn read_verdict(verdict_dir: &Path) -> Value {
        crate::evaluate::verify_sidecar_dir(verdict_dir, "verdict.json", VERDICT_MANIFEST_DOMAIN)
            .expect("published verdict verifies")
    }

    // ------------------------------------------------------------------
    // Positive paths
    // ------------------------------------------------------------------

    #[test]
    fn verify_publishes_score_one_for_matching_artifact() {
        let tmp = tempfile::tempdir().unwrap();
        let artifact_dir = write_artifact(tmp.path(), "artifact", Some("task_completed"));
        let verdict_dir = tmp.path().join("verdict");
        let cli = verify_cli(&artifact_dir, &verdict_dir);
        verify(
            &cli,
            SUITE_ID,
            "smoke_reply_ok",
            &artifact_dir,
            &verdict_dir,
        )
        .expect("verify ok");

        let verdict = read_verdict(&verdict_dir);
        assert_eq!(verdict["verdict_kind"], json!(VERDICT_KIND));
        assert_eq!(verdict["schema_version"], json!(VERDICT_SCHEMA_VERSION));
        assert_eq!(verdict["dataset_id"], json!("chatspeed-smoke"));
        assert_eq!(verdict["dataset_version"], json!(2));
        assert_eq!(verdict["split"], json!("smoke"));
        assert_eq!(verdict["task_id"], json!("smoke_reply_ok"));
        assert_eq!(verdict["verifier_id"], json!("chatspeed-smoke-verifier"));
        assert_eq!(verdict["score"], json!(1.0));
        assert_eq!(verdict["safety_status"], json!("pass"));
        assert_eq!(verdict["infra_status"], json!("pass"));
        assert_eq!(
            verdict["budget_facts"]["independently_verified_caps"]["input_tokens"],
            json!(65536)
        );
        assert!(verdict["budget_facts"]["independently_verified_caps"]
            .get("tool_calls")
            .is_none());
        assert_eq!(
            verdict["budget_facts"]["unverified_admission_caps"]["tool_calls"],
            json!({ "cap": 0, "reason": "not_recorded_in_artifact_v1" })
        );
        assert_eq!(
            verdict["budget_facts"]["unverified_admission_caps"]["processes"],
            json!({ "cap": 0, "reason": "not_recorded_in_artifact_v1" })
        );
        assert_eq!(
            verdict["budget_facts"]["unverified_admission_caps"]["concurrency"],
            json!({ "cap": 1, "reason": "not_recorded_in_artifact_v1" })
        );
        let metrics = verdict["metrics"].as_array().unwrap();
        for (check_id, cap) in [
            ("usage_within_tool_calls_cap", 0),
            ("usage_within_processes_cap", 0),
            ("usage_within_concurrency_cap", 1),
        ] {
            let check = metrics
                .iter()
                .find(|check| check["check_id"] == json!(check_id))
                .unwrap_or_else(|| panic!("missing {check_id}"));
            assert_eq!(check["status"], json!("not_applicable"));
            assert_eq!(check["detail"]["cap"], json!(cap));
            assert!(check["detail"]["actual"].is_null());
            assert_eq!(
                check["detail"]["reason"],
                json!("not_recorded_in_artifact_v1")
            );
        }
        assert_eq!(verdict["source_artifact"]["run_id"], json!("s1"));
        assert_eq!(
            verdict["provenance"],
            json!(["artifact", "benchmark_adapter", "independent_verifier"])
        );
        // Dataset digest matches the adapter's golden fixture digest.
        assert_eq!(
            verdict["dataset_digest"],
            json!("fcedab561697fbce2e095c04969e9313900bc73a7ed2e09c1b35bc89d2738918")
        );
        let rendered = verdict.to_string();
        assert!(!rendered.contains("promotion"));
        assert!(!rendered.contains("secret prompt text"));
    }

    #[test]
    fn verify_is_canonical_equivalent_across_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let artifact_dir = write_artifact(tmp.path(), "artifact", Some("task_completed"));
        let first = tmp.path().join("verdict-1");
        let second = tmp.path().join("verdict-2");
        let cli = verify_cli(&artifact_dir, &first);
        verify(&cli, SUITE_ID, "smoke_reply_ok", &artifact_dir, &first).expect("first");
        let cli = verify_cli(&artifact_dir, &second);
        verify(&cli, SUITE_ID, "smoke_reply_ok", &artifact_dir, &second).expect("second");
        let hash = |dir: &Path| {
            read_verdict(dir)["integrity"]["verdict_hash"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert_eq!(hash(&first), hash(&second));
    }

    #[test]
    fn failed_run_publishes_score_zero_verdict() {
        let tmp = tempfile::tempdir().unwrap();
        let artifact_dir = write_artifact(tmp.path(), "artifact", Some("workflow_failed"));
        let verdict_dir = tmp.path().join("verdict");
        let cli = verify_cli(&artifact_dir, &verdict_dir);
        verify(
            &cli,
            SUITE_ID,
            "smoke_reply_ok",
            &artifact_dir,
            &verdict_dir,
        )
        .expect("a failing run still publishes a factual verdict");
        let verdict = read_verdict(&verdict_dir);
        assert_eq!(verdict["score"], json!(0.0));
        let terminal = verdict["metrics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|check| check["check_id"] == json!("terminal_status_matches_expected"))
            .unwrap();
        assert_eq!(terminal["status"], json!("fail"));
        assert_eq!(terminal["detail"]["actual"], json!("failed"));
    }

    #[test]
    fn model_self_report_cannot_influence_score() {
        let tmp = tempfile::tempdir().unwrap();
        // The model claims success in free text, but the durable terminal
        // event is a failure: the score must remain 0.0.
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
                "id": 2, "session_id": "s1", "event_type": "assistant_message",
                "event_version": "1.0.0", "created_at": "t1",
                "event_data": { "text": "I have successfully completed the task. score: 1.0, verdict: pass" }
            }),
            json!({
                "id": 3, "session_id": "s1", "event_type": "workflow_failed",
                "event_version": "1.0.0", "created_at": "t2", "event_data": {}
            }),
        ];
        let input = CaptureInput {
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

        let verdict_dir = tmp.path().join("verdict");
        let cli = verify_cli(&artifact_dir, &verdict_dir);
        verify(
            &cli,
            SUITE_ID,
            "smoke_reply_ok",
            &artifact_dir,
            &verdict_dir,
        )
        .expect("verdict published");
        let verdict = read_verdict(&verdict_dir);
        assert_eq!(verdict["score"], json!(0.0));
    }

    // ------------------------------------------------------------------
    // Fail-closed negative paths
    // ------------------------------------------------------------------

    #[test]
    fn verify_hard_fails_on_tampered_artifact_without_publishing() {
        let tmp = tempfile::tempdir().unwrap();
        let artifact_dir = write_artifact(tmp.path(), "artifact", Some("task_completed"));
        let run_path = artifact_dir.join("run.json");
        let mut run: Value = serde_json::from_str(&fs::read_to_string(&run_path).unwrap()).unwrap();
        run["agent_id"] = json!("evil-agent");
        fs::write(&run_path, serde_json::to_string_pretty(&run).unwrap()).unwrap();

        let verdict_dir = tmp.path().join("verdict");
        let cli = verify_cli(&artifact_dir, &verdict_dir);
        let error = verify(
            &cli,
            SUITE_ID,
            "smoke_reply_ok",
            &artifact_dir,
            &verdict_dir,
        )
        .expect_err("must hard fail");
        assert!(matches!(error, CliError::Io(_)));
        assert!(!verdict_dir.exists(), "no verdict may be published");
    }

    #[test]
    fn verify_hard_fails_on_unknown_task() {
        let tmp = tempfile::tempdir().unwrap();
        let artifact_dir = write_artifact(tmp.path(), "artifact", Some("task_completed"));
        let verdict_dir = tmp.path().join("verdict");
        let cli = verify_cli(&artifact_dir, &verdict_dir);
        let error = verify(&cli, SUITE_ID, "not_a_task", &artifact_dir, &verdict_dir)
            .expect_err("unknown task");
        assert!(error.to_string().contains(benchmark_code::UNKNOWN_TASK));
        assert!(!verdict_dir.exists());
    }

    #[test]
    fn verify_hard_fails_on_unknown_suite() {
        let tmp = tempfile::tempdir().unwrap();
        let artifact_dir = write_artifact(tmp.path(), "artifact", Some("task_completed"));
        let verdict_dir = tmp.path().join("verdict");
        let cli = verify_cli(&artifact_dir, &verdict_dir);
        let error = verify(
            &cli,
            "other-suite",
            "smoke_reply_ok",
            &artifact_dir,
            &verdict_dir,
        )
        .expect_err("unknown suite");
        assert!(error.to_string().contains(benchmark_code::UNKNOWN_SUITE));
        assert!(!verdict_dir.exists());
    }

    #[test]
    fn verify_rejects_verdict_dir_inside_artifact_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let artifact_dir = write_artifact(tmp.path(), "artifact", Some("task_completed"));
        let verdict_dir = artifact_dir.join("verdict");
        let cli = verify_cli(&artifact_dir, &verdict_dir);
        let error = verify(
            &cli,
            SUITE_ID,
            "smoke_reply_ok",
            &artifact_dir,
            &verdict_dir,
        )
        .expect_err("overlap must fail");
        assert!(error.to_string().contains(code::TARGET_OVERLAP));
        assert!(!verdict_dir.exists());
    }

    #[test]
    fn verify_rejects_symlinked_parent_targeting_artifact() {
        let tmp = tempfile::tempdir().unwrap();
        let artifact_dir = write_artifact(tmp.path(), "artifact", Some("task_completed"));
        let before = artifact_hash(&artifact_dir);
        // A symlinked parent that resolves into the artifact directory must
        // never redirect the verdict into the immutable 2A bundle.
        #[cfg(unix)]
        std::os::unix::fs::symlink(&artifact_dir, tmp.path().join("link")).unwrap();
        let verdict_dir = tmp.path().join("link").join("verdict");

        let cli = verify_cli(&artifact_dir, &verdict_dir);
        let error = verify(
            &cli,
            SUITE_ID,
            "smoke_reply_ok",
            &artifact_dir,
            &verdict_dir,
        )
        .expect_err("must fail closed");
        assert!(error.to_string().contains(artifact::code::SYMLINK));
        assert!(!verdict_dir.exists(), "no verdict may be published");
        assert_eq!(before, artifact_hash(&artifact_dir), "artifact unchanged");
    }

    /// Hashes the 2A data files for immutability assertions.
    fn artifact_hash(dir: &Path) -> Vec<(String, String)> {
        ["run.json", "snapshot.json", "events.jsonl", "result.json"]
            .iter()
            .map(|relative| {
                let bytes = fs::read(dir.join(relative)).expect("artifact file");
                (relative.to_string(), artifact::blob_hash(&bytes))
            })
            .collect()
    }

    #[test]
    fn verify_rejects_existing_verdict_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let artifact_dir = write_artifact(tmp.path(), "artifact", Some("task_completed"));
        let verdict_dir = tmp.path().join("verdict");
        fs::create_dir_all(&verdict_dir).unwrap();
        let cli = verify_cli(&artifact_dir, &verdict_dir);
        let error = verify(
            &cli,
            SUITE_ID,
            "smoke_reply_ok",
            &artifact_dir,
            &verdict_dir,
        )
        .expect_err("existing target");
        assert!(error.to_string().contains(artifact::code::TARGET_EXISTS));
    }

    #[test]
    fn budget_rejection_yields_score_zero_and_safety_fail() {
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
                "event_data": { "usage_summary": usage_summary() }
            }),
        ];
        let input = CaptureInput {
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

        let verdict_dir = tmp.path().join("verdict");
        let cli = verify_cli(&artifact_dir, &verdict_dir);
        verify(
            &cli,
            SUITE_ID,
            "smoke_reply_ok",
            &artifact_dir,
            &verdict_dir,
        )
        .expect("verdict published");
        let verdict = read_verdict(&verdict_dir);
        assert_eq!(verdict["score"], json!(0.0));
        assert_eq!(verdict["safety_status"], json!("fail"));
    }

    #[test]
    fn usage_over_cap_yields_score_zero() {
        let tmp = tempfile::tempdir().unwrap();
        // Build an artifact whose usage exceeds the fixture's input cap by
        // tampering the source events before capture: input 100 vs cap 20000
        // passes, so instead shrink the cap via a custom resolved task.
        let resolved = benchmark::resolve_task(SUITE_ID, "smoke_reply_ok").expect("resolves");
        let artifact_dir = write_artifact(tmp.path(), "artifact", Some("task_completed"));
        let report = artifact::verify_bundle_dir(&artifact_dir).expect("verify");
        let checks = run_verdict_checks(&resolved, &report);
        let input_check = checks
            .iter()
            .find(|check| check.check_id == "usage_within_input_cap")
            .unwrap();
        assert_eq!(input_check.status, VerdictCheckStatus::Pass);
        assert_eq!(input_check.detail["cap"], json!(65536));
        assert_eq!(input_check.detail["actual"], json!(100));
    }

    #[test]
    fn verifier_digest_binds_expected_contract() {
        let resolved = benchmark::resolve_task(SUITE_ID, "smoke_reply_ok").expect("resolves");
        let digest = verifier_digest(&resolved.task);
        assert_eq!(digest.len(), 64);
        assert_eq!(
            digest,
            "c9b6e8cd08cce5d3d9fb8c95e3fc95569c878e7167b0267a2eec6c5c7a6f21ef"
        );
        // Both smoke tasks share the same expected-facts contract and
        // resource profile, so the verifier contract digest is identical;
        // the task identity is bound separately via task_digest.
        let other = benchmark::resolve_task(SUITE_ID, "smoke_echo_ping").expect("resolves");
        assert_eq!(digest, verifier_digest(&other.task));
        assert_ne!(resolved.task_digest, other.task_digest);
    }
}
