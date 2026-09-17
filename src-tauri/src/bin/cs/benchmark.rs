//! Phase 2E benchmark adapter for the fixed `chatspeed-smoke@2` fixture.
//!
//! The adapter is a pure, deterministic resolver between a checked-in fixture
//! and the existing 2C experiment-run control plane. It never runs an executor,
//! opens the database, or talks to a provider itself; the only external effect
//! of `benchmark run` is the existing single budgeted
//! `POST /control/v1/experiments:run` (INV-2/INV-3).
//!
//! Since Phase 2G+2H the resolver itself lives in the shared library module
//! `chatspeed_lib::experiment_schedule::fixture`, because the durable backend
//! scheduler must resolve the *same* fixture refs to the same instruction and
//! digests. This file keeps the CLI-only pieces: the diagnostic projection and
//! the CLI error mapping, plus the re-exports that make `crate::benchmark` the
//! single CLI-facing name for the shared contract. There is exactly one parser
//! and one hash implementation for both adapters.

/// Re-export of the one shared resolver. A glob keeps this module a faithful
/// alias of the library contract without silently narrowing (or widening) the
/// CLI-facing surface as the contract grows.
pub use chatspeed_lib::experiment_schedule::fixture::*;

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
    use serde_json::json;

    #[test]
    fn cli_uses_the_shared_checked_in_fixture() {
        let resolved = resolve_task(SUITE_ID, "smoke_reply_ok").expect("resolves");
        assert_eq!(resolved.manifest.dataset_id, "chatspeed-smoke");
        assert_eq!(resolved.manifest.dataset_version, 2);
        assert_eq!(resolved.manifest.split, "smoke");
        assert_eq!(resolved.manifest.runner_kind, RUNNER_KIND);
        assert_eq!(
            resolved.manifest.tasks,
            vec!["smoke_reply_ok".to_string(), "smoke_echo_ping".to_string()]
        );
    }

    /// The CLI-visible golden digests must stay identical to the backend's.
    /// Moving the resolver into the shared library must not change the 2E
    /// fixture identity (2G+2H U-1 acceptance).
    #[test]
    fn fixture_digests_match_golden_values() {
        let resolved = resolve_task(SUITE_ID, "smoke_reply_ok").expect("resolves");
        assert_eq!(resolved.task.instruction, "Reply with exactly: OK");
        assert_eq!(
            resolved.manifest_digest,
            "fcedab561697fbce2e095c04969e9313900bc73a7ed2e09c1b35bc89d2738918"
        );
        assert_eq!(
            resolved.task_digest,
            "85acd5e8d744b20f0ce537a17ca6ae51aa2caaa4b3f71853a81fb1713f7d5405"
        );
        assert_eq!(
            resolved.task.instruction_hash,
            "f58cc8905c59cc469e451f19c4141d1f6f0a59dce07a40a37040078236de4b40"
        );
        let ping = resolve_task(SUITE_ID, "smoke_echo_ping").expect("resolves");
        assert_eq!(ping.manifest_digest, resolved.manifest_digest);
        assert_eq!(
            ping.task_digest,
            "5ce34ddef4ad2586e621a858cf810bff388596423c6ccb9bf2040b961425ed04"
        );
    }

    #[test]
    fn unknown_suite_and_task_are_usage_errors() {
        let error = to_cli_error(resolve_task("other-suite", "smoke_reply_ok").expect_err("suite"));
        assert_eq!(error.exit_code(), 2);
        let error = to_cli_error(resolve_task(SUITE_ID, "not_a_task").expect_err("task"));
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn fixture_integrity_failures_are_io_errors() {
        let error = to_cli_error(BenchmarkError::new(code::DIGEST_MISMATCH, "tampered"));
        assert_eq!(error.exit_code(), 1);
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
        assert_eq!(metadata["runner_kind"], json!(RUNNER_KIND));
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

    /// The durable scheduler will persist exactly this projection and must be
    /// able to recover the same instruction from it.
    #[test]
    fn fixture_ref_round_trips_through_the_shared_resolver() {
        let resolved = resolve_task(SUITE_ID, "smoke_echo_ping").expect("resolves");
        let reference = resolved.task_ref();
        let recovered = resolve_task_ref(&reference).expect("recovers");
        assert_eq!(recovered.task.instruction, "Reply with exactly: PONG");
        assert_eq!(recovered.task_digest, resolved.task_digest);

        let mut stale = reference.clone();
        stale.task_digest = "0".repeat(64);
        let error = resolve_task_ref(&stale).expect_err("stale");
        assert_eq!(error.code, code::DIGEST_MISMATCH);
    }
}
