//! `cs experiment campaign schedule|jobs|job|cancel|reconcile` — the durable
//! campaign scheduling client (Phase 2G+2H).
//!
//! This module is HTTP-only, like the rest of `cs`: it never opens SQLite,
//! never starts a scheduler or an owner, and never creates a second lifecycle
//! path (INV-1). It shares the strict plan parser, the canonical-hash helpers
//! and the checked-in fixture resolver with the backend through
//! `chatspeed_lib`, so a request the CLI accepts is exactly the request the
//! backend re-validates.
//!
//! The client also never sends the fixture instruction: it resolves the plan's
//! suite/task locally only to derive the durable fixture *refs* (digests), so
//! the database never stores a prompt (INV-6).

use crate::args::Cli;
use crate::client::ControlPlaneClient;
use crate::error::CliError;
use crate::output::render_result;
use chatspeed_lib::campaign;
use chatspeed_lib::experiment_schedule::{fixture, types};
use serde_json::{json, Value};
use std::path::Path;

/// Machine-stable error codes for the schedule client.
pub mod code {
    /// The plan file is missing or is not a strict `campaign_plan.v1`.
    pub const PLAN_INVALID: &str = "plan_invalid";
    /// The checked-in fixture could not be resolved for the plan's suite/task.
    pub const FIXTURE_INVALID: &str = "fixture_invalid";
    /// The local request failed its own strict validation.
    pub const REQUEST_INVALID: &str = "schedule_request_invalid";
    /// The backend answered with a document the client cannot trust.
    pub const RESPONSE_MISMATCH: &str = "schedule_response_mismatch";
}

fn usage_error(code: &'static str, message: impl Into<String>) -> CliError {
    CliError::usage(format!("{code}: {}", message.into()))
}

fn io_error(code: &'static str, message: impl Into<String>) -> CliError {
    CliError::io(format!("{code}: {}", message.into()))
}

/// Reads and strictly validates the frozen plan from a JSON file.
fn read_plan(plan_path: &Path) -> Result<(Value, campaign::CampaignPlanV1), CliError> {
    let text = std::fs::read_to_string(plan_path).map_err(|error| {
        io_error(
            code::PLAN_INVALID,
            format!("failed to read {}: {error}", plan_path.display()),
        )
    })?;
    let value: Value = serde_json::from_str(&text).map_err(|error| {
        io_error(
            code::PLAN_INVALID,
            format!("{} is not valid JSON: {error}", plan_path.display()),
        )
    })?;
    let plan = campaign::parse_and_validate_campaign_plan(&value).map_err(|error| {
        usage_error(
            code::PLAN_INVALID,
            format!("{}: {}", error.code.as_str(), error.message),
        )
    })?;
    Ok((value, plan))
}

/// Builds the strict durable schedule request for a plan and an execution
/// profile reference.
///
/// The fixture refs are derived from the checked-in catalog and carry digests
/// only — never the instruction body.
fn build_schedule_request(
    plan_value: Value,
    plan: &campaign::CampaignPlanV1,
    execution_profile_ref: &str,
    bundle_refs: Vec<String>,
) -> Result<types::CampaignScheduleRequestV1, CliError> {
    let resolved = fixture::resolve_task(&plan.suite, &plan.task).map_err(|error| {
        usage_error(
            code::FIXTURE_INVALID,
            format!("{}: {}", error.code, error.message),
        )
    })?;
    let request = json!({
        "schema_version": types::CAMPAIGN_SCHEDULE_V1,
        "plan": plan_value,
        "fixture_refs": [serde_json::to_value(resolved.task_ref()).map_err(|error| {
            io_error(code::REQUEST_INVALID, format!("fixture ref is not serializable: {error}"))
        })?],
        "execution_profile_ref": execution_profile_ref,
        "bundle_refs": bundle_refs,
    });
    types::parse_and_validate_campaign_schedule_request(&request).map_err(|error| {
        usage_error(
            code::REQUEST_INVALID,
            format!("{}: {}", error.code.as_str(), error.message),
        )
    })
}

/// Runs `cs experiment campaign schedule`: persist one durable schedule (plan,
/// fixture refs, execution profile and bundle refs) and its ordered jobs.
pub async fn schedule(
    cli: &Cli,
    client: &ControlPlaneClient,
    plan_path: &Path,
    execution_profile_ref: &str,
    bundle_refs: Vec<String>,
) -> Result<(), CliError> {
    let (plan_value, plan) = read_plan(plan_path)?;
    let request = build_schedule_request(plan_value, &plan, execution_profile_ref, bundle_refs)?;
    let campaign_id = types::campaign_id_for_schedule(&request);
    // The idempotency key is the canonical schedule hash, so re-running the
    // same command replays the same acceptance instead of enqueueing twice.
    let idempotency = types::schedule_request_hash(&request);
    let body = serde_json::to_value(&request)
        .map_err(|error| io_error(code::REQUEST_INVALID, error.to_string()))?;

    let result = client
        .post(
            &format!("/control/v1/campaigns/{campaign_id}/schedule"),
            body,
            Some(&idempotency),
        )
        .await?;
    let accepted = result
        .get("campaign_id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            io_error(
                code::RESPONSE_MISMATCH,
                "schedule response has no campaign_id",
            )
        })?;
    if accepted != campaign_id {
        return Err(io_error(
            code::RESPONSE_MISMATCH,
            "schedule response campaign id does not match the derived plan hash",
        ));
    }
    let job_ids = result
        .get("job_ids")
        .and_then(Value::as_array)
        .map(|ids| ids.len())
        .unwrap_or(0);
    eprintln!(
        "cs: durable schedule accepted for {campaign_id} ({job_ids} ordered job(s), profile {execution_profile_ref})"
    );
    render_result(cli.output, &result);
    Ok(())
}

/// Runs `cs experiment campaign jobs`: list the durable jobs of a campaign.
pub async fn jobs(
    cli: &Cli,
    client: &ControlPlaneClient,
    campaign_id: &str,
) -> Result<(), CliError> {
    let result = client
        .get(&format!("/control/v1/campaigns/{campaign_id}/jobs"))
        .await?;
    render_result(cli.output, &result);
    Ok(())
}

/// Runs `cs experiment campaign job`: read one durable job.
pub async fn job(cli: &Cli, client: &ControlPlaneClient, job_id: &str) -> Result<(), CliError> {
    let result = client
        .get(&format!("/control/v1/campaign-jobs/{job_id}"))
        .await?;
    render_result(cli.output, &result);
    Ok(())
}

/// Runs `cs experiment campaign cancel`: cancel pre-dispatch work and stop
/// admitting new work for the campaign.
pub async fn cancel(
    cli: &Cli,
    client: &ControlPlaneClient,
    campaign_id: &str,
    reason: &str,
) -> Result<(), CliError> {
    let result = client
        .post(
            &format!("/control/v1/campaigns/{campaign_id}/cancel"),
            json!({ "reason": reason }),
            Some(&crate::new_idempotency_key()),
        )
        .await?;
    render_result(cli.output, &result);
    Ok(())
}

/// Runs `cs experiment campaign reconcile`: evidence-only classification of the
/// campaign's non-terminal jobs.
pub async fn reconcile(
    cli: &Cli,
    client: &ControlPlaneClient,
    campaign_id: &str,
) -> Result<(), CliError> {
    let result = client
        .post(
            &format!("/control/v1/campaigns/{campaign_id}/reconcile"),
            json!({}),
            Some(&crate::new_idempotency_key()),
        )
        .await?;
    render_result(cli.output, &result);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn smoke_plan() -> Value {
        json!({
            "schema_version": "campaign_plan.v1",
            "campaign_key": "p2gh-smoke",
            "stage": "stage_0_manual",
            "agent_id": "agent-1",
            "suite": "chatspeed-smoke",
            "task": "smoke_reply_ok",
            "concurrency": 1,
            "budget": {
                "money_mode": { "mode": "token_resource_only" },
                "caps": { "input_tokens": 1024, "output_tokens": 1024 },
                "required_dimensions": [],
                "max_attempts": 1
            },
            "candidates": [
                { "candidate_key": "baseline", "kind": "baseline" },
                { "candidate_key": "cand-a", "kind": "candidate",
                  "mutable_surface": ["agent_prompt_ref"],
                  "agent_prompt_ref": "smoke-terse-v1",
                  "prompt_hash": "bb41d700c9a2cdd26bffe26b5a3deac849188ce1966414c32700e38d51d2bf88" }
            ]
        })
    }

    #[test]
    fn a_valid_plan_builds_a_digest_bound_instruction_free_request() {
        let value = smoke_plan();
        let plan = campaign::parse_and_validate_campaign_plan(&value).expect("plan");
        let request =
            build_schedule_request(value, &plan, "smoke-local", vec!["smoke-tools".to_string()])
                .expect("request");

        // The request carries refs and digests only: no instruction anywhere.
        let serialized = serde_json::to_string(&request).expect("serialize");
        assert!(!serialized.contains("Reply with exactly"));
        assert_eq!(request.fixture_refs.len(), 1);
        assert_eq!(request.fixture_refs[0].task_id, "smoke_reply_ok");
        assert_eq!(request.execution_profile_ref, "smoke-local");
        // The campaign identity is derived locally from the same shared hash the
        // backend re-derives, so the path parameter can never disagree.
        let campaign_id = types::campaign_id_for_schedule(&request);
        assert!(campaign::validate_campaign_id(&campaign_id).is_ok());
        // The idempotency key is stable for the same plan.
        assert_eq!(
            types::schedule_request_hash(&request),
            types::schedule_request_hash(&request)
        );
    }

    #[test]
    fn an_unknown_fixture_task_is_a_usage_error() {
        let mut value = smoke_plan();
        value["task"] = json!("not_a_task");
        let plan = campaign::parse_and_validate_campaign_plan(&value).expect("plan parses");
        let error = build_schedule_request(value, &plan, "smoke-local", Vec::new())
            .expect_err("unknown task");
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn a_malformed_plan_file_is_rejected_before_any_request() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("plan.json");
        std::fs::write(&path, b"{ not json }").expect("write");
        let error = read_plan(&path).expect_err("invalid plan");
        assert_eq!(error.exit_code(), 1);

        let missing = directory.path().join("absent.json");
        let error = read_plan(&missing).expect_err("missing plan");
        assert_eq!(error.exit_code(), 1);

        let forbidden = directory.path().join("forbidden.json");
        let mut value = smoke_plan();
        value["allowed_paths"] = json!(["/etc"]);
        std::fs::write(&forbidden, serde_json::to_vec(&value).expect("serialize")).expect("write");
        let error = read_plan(&forbidden).expect_err("forbidden field");
        assert_eq!(error.exit_code(), 2);
    }
}
