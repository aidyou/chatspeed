//! `cs experiment promotion run|status|reconcile|audit|inspect` — the Phase 2I
//! promotion client.
//!
//! This module is HTTP-only, like the rest of `cs`: it never opens SQLite, never
//! starts a scheduler, an owner or a container, and never touches Git (INV-2).
//! It shares the strict promotion contracts and the canonical-hash helpers with
//! the backend through `chatspeed_lib`, so a request the CLI accepts is exactly
//! the request the backend re-validates.
//!
//! `run` submits one strict `promotion_request.v1` document, waits for the
//! backend's terminal state and exports an atomic, offline-verifiable audit
//! sidecar. `status`, `reconcile` and `audit` are read-only projections.
//! `inspect` is fully offline: it re-verifies the sidecar manifest, the audit
//! integrity digest and the journal digest without reading discovery.

use crate::args::Cli;
use crate::client::ControlPlaneClient;
use crate::error::CliError;
use crate::output::render_result;
use chatspeed_lib::experiment_promotion::types;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Machine-stable error codes for the promotion client.
pub mod code {
    /// The evidence file is missing or is not a strict request document.
    pub const REQUEST_INVALID: &str = "promotion_request_invalid";
    /// The backend answered with a document the client cannot trust.
    pub const RESPONSE_MISMATCH: &str = "promotion_response_mismatch";
    /// The promotion did not reach a terminal state within the wait budget.
    pub const TIMEOUT: &str = "promotion_timeout";
    /// The audit sidecar is missing or does not verify offline.
    pub const AUDIT_INVALID: &str = "promotion_audit_invalid";
}

/// The audit sidecar directory inside `--out`.
const AUDIT_DIR: &str = "promotion-audit";
/// The audit sidecar file name.
const AUDIT_FILE: &str = "audit.json";
/// Manifest domain of the audit sidecar.
const AUDIT_MANIFEST_DOMAIN: &str = "cs-promotion:audit-manifest";

/// The terminal promotion states. A run stops when the backend reports one.
const TERMINAL_STATES: &[&str] = &[
    "rejected",
    "canary_failed",
    "promoted",
    "rolled_back",
    "unknown_manual",
];

fn usage_error(code: &'static str, message: impl Into<String>) -> CliError {
    CliError::usage(format!("{code}: {}", message.into()))
}

fn io_error(code: &'static str, message: impl Into<String>) -> CliError {
    CliError::io(format!("{code}: {}", message.into()))
}

fn audit_dir(out: &Path) -> PathBuf {
    out.join(AUDIT_DIR)
}

/// Reads and strictly validates one `promotion_request.v1` document.
fn read_request(path: &Path) -> Result<Value, CliError> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        io_error(
            code::REQUEST_INVALID,
            format!("failed to read {}: {error}", path.display()),
        )
    })?;
    let value: Value = serde_json::from_str(&text).map_err(|error| {
        io_error(
            code::REQUEST_INVALID,
            format!("{} is not valid JSON: {error}", path.display()),
        )
    })?;
    // The CLI refuses to send anything the backend would reject anyway, so a
    // typo is a local usage error instead of a round trip.
    let request: types::PromotionRequestV1 =
        serde_json::from_value(value.clone()).map_err(|error| {
            usage_error(
                code::REQUEST_INVALID,
                format!(
                    "{} is not a strict promotion request: {error}",
                    path.display()
                ),
            )
        })?;
    request.validate().map_err(|error| {
        usage_error(
            code::REQUEST_INVALID,
            format!("{}: {}", error.code.as_str(), error.message),
        )
    })?;
    Ok(value)
}

/// Whether a state string is terminal.
fn is_terminal(state: &str) -> bool {
    TERMINAL_STATES.contains(&state)
}

/// `cs experiment promotion run` — submit, wait, and export the audit bundle.
pub async fn run(
    cli: &Cli,
    client: &ControlPlaneClient,
    request_path: &Path,
    out: &Path,
) -> Result<(), CliError> {
    let request = read_request(request_path)?;
    let submitted = client
        .post(
            "/control/v1/promotions",
            request,
            Some(&crate::new_idempotency_key()),
        )
        .await?;
    let promotion_id = submitted
        .get("promotion_id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            io_error(
                code::RESPONSE_MISMATCH,
                "the backend did not return a promotion id",
            )
        })?
        .to_string();

    // Poll the backend's own terminal state with a budget derived from the
    // server's canary ceiling. The promotion keeps progressing even if this
    // client exits; the wait is a convenience, never a dependency.
    let promotion_id_for_fetch = promotion_id.clone();
    let projection = wait_for_terminal(
        submitted,
        || {
            let client = client.clone();
            let promotion_id = promotion_id_for_fetch.clone();
            async move {
                client
                    .get(&format!("/control/v1/promotions/{promotion_id}"))
                    .await
            }
        },
        resolve_wait_budget_secs(None),
        POLL_SECONDS,
    )
    .await?;

    let state = projection
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let audit = export_audit(client, &promotion_id, out).await?;
    render_run(cli, &promotion_id, &state, &projection, &audit);
    // A promotion that did not promote is a *result*, not a client failure: the
    // exit code distinguishes them so a caller can branch without parsing prose.
    if state == "promoted" {
        Ok(())
    } else {
        Err(io_error(
            code::RESPONSE_MISMATCH,
            format!("promotion '{promotion_id}' terminated as {state}"),
        ))
    }
}

/// How long `run` waits for a terminal state, and how often it polls.
///
/// The default budget is *derived from the server's own canary ceiling*, not
/// chosen ad hoc: the widest legal canary is `MAX_CANARY_STAGES` stages, each up
/// to `MAX_CANARY_TIMEOUT_MS`, executed for **two** arms, plus the supervisor's
/// own tick overhead. A budget below that would give up on a promotion the
/// backend is still legitimately running, so a caller-supplied value can only
/// raise it (AC-8).
pub fn minimum_wait_budget_secs() -> u64 {
    let canary_ceiling_secs =
        types::MAX_CANARY_STAGES as u64 * (types::MAX_CANARY_TIMEOUT_MS / 1000) * 2;
    canary_ceiling_secs + 300
}

/// Resolves the effective wait budget from an optional caller override.
pub fn resolve_wait_budget_secs(configured: Option<u64>) -> u64 {
    match configured {
        Some(value) => value.max(minimum_wait_budget_secs()),
        None => minimum_wait_budget_secs(),
    }
}

/// Poll cadence for `run`. The budget is what bounds the wait, not the cadence.
const POLL_SECONDS: u64 = 2;

/// Polls one promotion until it reaches a terminal state.
///
/// Factored out of [`run`] so the budget and the terminal-state contract are
/// testable without HTTP: the *initial* projection is consumed first (no
/// redundant fetch), then the poller fetches at least once more and stops on the
/// first terminal state or when the budget is exhausted.
async fn wait_for_terminal<F, Fut>(
    initial: Value,
    mut fetch: F,
    budget_secs: u64,
    poll_secs: u64,
) -> Result<Value, CliError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Value, CliError>>,
{
    let mut projection = initial;
    if is_terminal(
        projection
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or(""),
    ) {
        return Ok(projection);
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(budget_secs.max(1));
    loop {
        // At least one more fetch, so a promotion that reached its terminal
        // state between the submission and the first poll is not missed.
        tokio::time::sleep(std::time::Duration::from_secs(poll_secs.max(1))).await;
        projection = fetch().await?;
        if is_terminal(
            projection
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or(""),
        ) {
            return Ok(projection);
        }
        if std::time::Instant::now() >= deadline {
            return Err(timeout_error(projection));
        }
    }
}

fn timeout_error(projection: Value) -> CliError {
    let promotion_id = projection
        .get("promotion_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    io_error(
        code::TIMEOUT,
        format!(
            "promotion '{promotion_id}' did not reach a terminal state; query it with \
             `cs experiment promotion status --promotion-id {promotion_id}`"
        ),
    )
}

/// Fetches the audit bundle and exports it as an atomic sidecar.
async fn export_audit(
    client: &ControlPlaneClient,
    promotion_id: &str,
    out: &Path,
) -> Result<Value, CliError> {
    let audit = client
        .get(&format!("/control/v1/promotions/{promotion_id}/audit"))
        .await?;
    // Re-verify before writing: the operator must never receive a bundle that
    // does not verify, even if the transport was tampered with.
    types::verify_audit_document(&audit).map_err(|error| {
        io_error(
            code::AUDIT_INVALID,
            format!("{}: {}", error.code.as_str(), error.message),
        )
    })?;
    let directory = audit_dir(out);
    let body = serde_json::to_string_pretty(&audit).map_err(|error| {
        io_error(
            code::AUDIT_INVALID,
            format!("the audit bundle could not be serialised: {error}"),
        )
    })?;
    let bundle = crate::evaluate::SidecarBundle {
        file_name: AUDIT_FILE.to_string(),
        body,
        schema_version: crate::verifier::SIDECAR_MANIFEST_SCHEMA_VERSION,
        algorithm: crate::artifact::HASH_ALGORITHM,
        manifest_domain: AUDIT_MANIFEST_DOMAIN,
    };
    // Atomic: the sidecar is written to a staging directory and swapped in, so an
    // interrupted export never leaves a half-written bundle.
    crate::evaluate::write_sidecar(&bundle, &directory)
        .map_err(|error| io_error(error.code, error.message))?;
    Ok(audit)
}

/// `cs experiment promotion status` — the status projection.
pub async fn status(
    cli: &Cli,
    client: &ControlPlaneClient,
    promotion_id: &str,
) -> Result<(), CliError> {
    let projection = client
        .get(&format!("/control/v1/promotions/{promotion_id}"))
        .await?;
    let state = projection
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    log::info!("cs promotion status: promotion {promotion_id}: {state}");
    render_result(
        cli.output,
        &json!({
            "promotion_id": promotion_id,
            "promotion": projection,
        }),
    );
    Ok(())
}

/// `cs experiment promotion reconcile` — the evidence-only reconcile document.
pub async fn reconcile(
    cli: &Cli,
    client: &ControlPlaneClient,
    promotion_id: &str,
) -> Result<(), CliError> {
    let reconcile = client
        .post(
            &format!("/control/v1/promotions/{promotion_id}/reconcile"),
            json!({}),
            None,
        )
        .await?;
    log::info!(
        "cs promotion reconcile: promotion {promotion_id}: recovery={}",
        reconcile
            .get("recovery")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
    );
    render_result(cli.output, &reconcile);
    Ok(())
}

/// `cs experiment promotion audit` — export the audit bundle.
pub async fn audit(
    cli: &Cli,
    client: &ControlPlaneClient,
    promotion_id: &str,
    out: &Path,
) -> Result<(), CliError> {
    let audit = export_audit(client, promotion_id, out).await?;
    log::info!("cs promotion audit: promotion {promotion_id}: audit bundle exported");
    render_result(
        cli.output,
        &json!({
            "promotion_id": promotion_id,
            "audit_dir": audit_dir(out).display().to_string(),
            "audit_hash": audit.pointer("/integrity/audit_hash").cloned(),
            "state": audit.pointer("/promotion/state").cloned(),
        }),
    );
    Ok(())
}

/// `cs experiment promotion inspect` — fully offline re-verification of the
/// exported audit sidecar. It never loads discovery, never contacts the backend
/// and never reads the database.
pub fn inspect(cli: &Cli, out: &Path) -> Result<(), CliError> {
    let directory = audit_dir(out);
    let audit = crate::evaluate::verify_sidecar_dir(&directory, AUDIT_FILE, AUDIT_MANIFEST_DOMAIN)
        .map_err(|error| io_error(error.code, error.message))?;
    let verified = types::verify_audit_document(&audit).map_err(|error| {
        io_error(
            code::AUDIT_INVALID,
            format!("{}: {}", error.code.as_str(), error.message),
        )
    })?;
    // The evidence projection inside the bundle must still be self-consistent.
    verified.evidence.validate().map_err(|error| {
        io_error(
            code::AUDIT_INVALID,
            format!("{}: {}", error.code.as_str(), error.message),
        )
    })?;
    log::info!(
        "cs promotion inspect: promotion {} audit verified offline",
        verified.promotion.promotion_id
    );
    render_result(
        cli.output,
        &json!({
            "promotion_id": verified.promotion.promotion_id,
            "state": verified.promotion.state,
            "audit_dir": directory.display().to_string(),
            "audit_hash": verified.integrity.audit_hash,
            "journal_digest": verified.journal_digest,
            "journal_entries": verified.journal.len(),
            "stages": verified.promotion.stages.len(),
            "evidence_hash": verified.promotion.evidence_hash,
            "checkpoint_commit": verified.promotion.checkpoint_commit,
            "branch_ref": verified.branch_ref,
            "offline": true,
        }),
    );
    Ok(())
}

/// The human-facing summary of one completed run.
fn render_run(cli: &Cli, promotion_id: &str, state: &str, projection: &Value, audit: &Value) {
    log::info!("cs promotion run: promotion {promotion_id}: {state}");
    render_result(
        cli.output,
        &json!({
            "promotion_id": promotion_id,
            "state": state,
            "target_ref": projection.get("target_ref"),
            "checkpoint_commit": projection.get("checkpoint_commit"),
            "checkpoint_ref": projection.get("checkpoint_ref"),
            "canary_result_hash": projection.get("canary_result_hash"),
            "error_code": projection.get("error_code"),
            "audit_hash": audit.pointer("/integrity/audit_hash").cloned(),
            "journal_digest": audit.get("journal_digest"),
        }),
    );
}

#[cfg(test)]
#[path = "promotion_tests.rs"]
mod tests;
