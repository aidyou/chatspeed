//! `cs automation` — local workflow-automation commands (Phase 3D).
//!
//! The CLI is a thin HTTP client of the app's control plane: it never opens the
//! database, starts a workflow, or runs a shell command itself, so the desktop,
//! the scheduler and the CLI cannot diverge about what an automation is or what
//! its runs did (AC-1/INV-1). Every mutation carries an idempotency key, minted
//! when the caller did not supply one, so a retried request can never double a
//! schedule, a dispatch or a destructive delete (AC-5/AC-10). `draft` is
//! side-effect-free and cannot grant a new shell/path/network/MCP/Skill
//! permission; only an explicit `apply` of a reviewed plan changes state.

use crate::args::{AutomationCommand, Cli};
use crate::capability::{fetch_and_render, render, text};
use crate::client::ControlPlaneClient;
use crate::error::CliError;
use serde_json::{json, Value};

/// Runs one `cs automation` subcommand.
pub async fn run(
    cli: &Cli,
    client: &ControlPlaneClient,
    command: &AutomationCommand,
) -> Result<(), CliError> {
    match command {
        AutomationCommand::List => {
            fetch_and_render(cli, client, "/control/v1/automations", human_automations).await
        }
        AutomationCommand::Get { automation_id } => {
            fetch_and_render(
                cli,
                client,
                &format!("/control/v1/automations/{automation_id}"),
                human_automation,
            )
            .await
        }
        AutomationCommand::Runs { automation_id } => {
            fetch_and_render(
                cli,
                client,
                &format!("/control/v1/automations/{automation_id}/runs"),
                human_runs,
            )
            .await
        }
        AutomationCommand::Draft {
            spec_json,
            spec_file,
            intent,
            automation_id,
        } => draft(cli, client, spec_json, spec_file, intent, automation_id).await,
        AutomationCommand::Apply {
            plan_json,
            plan_file,
            expected_plan_hash,
            acknowledge_permission_changes,
            idempotency_key,
        } => apply(
            cli,
            client,
            plan_json,
            plan_file,
            expected_plan_hash,
            *acknowledge_permission_changes,
            idempotency_key,
        )
        .await,
        AutomationCommand::Create {
            spec_json,
            spec_file,
            idempotency_key,
        } => create(cli, client, spec_json, spec_file, idempotency_key).await,
        AutomationCommand::Update {
            automation_id,
            spec_json,
            spec_file,
            expected_revision,
            idempotency_key,
        } => update(
            cli,
            client,
            automation_id,
            spec_json,
            spec_file,
            *expected_revision,
            idempotency_key,
        )
        .await,
        AutomationCommand::Enable {
            automation_id,
            idempotency_key,
        } => {
            simple_mutation(cli, client, automation_id, "enable", json!({}), idempotency_key).await
        }
        AutomationCommand::Disable {
            automation_id,
            idempotency_key,
        } => {
            simple_mutation(
                cli,
                client,
                automation_id,
                "disable",
                json!({}),
                idempotency_key,
            )
            .await
        }
        AutomationCommand::Run {
            automation_id,
            idempotency_key,
        } => {
            let value = mutation_request(
                client,
                automation_id,
                "run",
                json!({}),
                idempotency_key,
            )
            .await?;
            render(cli, &value, human_dispatch)
        }
        AutomationCommand::Delete {
            automation_id,
            confirm,
            idempotency_key,
        } => {
            // The confirmation is carried in the body so the server, not the
            // CLI, is the single authority on the destructive gate (AC-10).
            let value = mutation_request(
                client,
                automation_id,
                "delete",
                json!({ "confirm": confirm }),
                idempotency_key,
            )
            .await?;
            render(cli, &value, human_mutation)
        }
    }
}

/// `draft` is a POST read that never mutates, so it needs no idempotency key.
/// Exactly one of a structured spec or a constrained intent is accepted.
async fn draft(
    cli: &Cli,
    client: &ControlPlaneClient,
    spec_json: &Option<String>,
    spec_file: &Option<std::path::PathBuf>,
    intent: &Option<String>,
    automation_id: &Option<String>,
) -> Result<(), CliError> {
    let spec = match (spec_json, spec_file) {
        (Some(_), _) | (None, Some(_)) => Some(read_document(spec_json, spec_file)?),
        (None, None) => None,
    };
    if spec.is_none() && intent.as_deref().unwrap_or("").trim().is_empty() {
        return Err(CliError::usage(
            "provide a spec with --spec-json/--spec-file or an --intent",
        ));
    }
    let body = json!({
        "automation_id": automation_id,
        "spec": spec,
        "intent": intent,
    });
    let value = client.post("/control/v1/automation-draft", body, None).await?;
    render(cli, &value, human_plan)
}

/// `apply` re-submits a reviewed plan. The authorized hash defaults to the
/// plan's own hash unless the caller pins a different one (a mismatch is a
/// server-side conflict).
async fn apply(
    cli: &Cli,
    client: &ControlPlaneClient,
    plan_json: &Option<String>,
    plan_file: &Option<std::path::PathBuf>,
    expected_plan_hash: &Option<String>,
    acknowledge_permission_changes: bool,
    idempotency_key: &Option<String>,
) -> Result<(), CliError> {
    let plan = read_document(plan_json, plan_file)?;
    let expected = expected_plan_hash
        .clone()
        .or_else(|| plan.get("plan_hash").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_default();
    let body = json!({
        "plan": plan,
        "expected_plan_hash": expected,
        "acknowledge_permission_changes": acknowledge_permission_changes,
    });
    let key = generated_idempotency_key(idempotency_key, "apply");
    let value = client
        .post("/control/v1/automation-apply", body, Some(&key))
        .await?;
    render(cli, &value, human_mutation)
}

/// `create` sends the spec document untouched; the service is the validator and
/// mints the id, so the CLI never grows a laxer notion of a valid automation.
async fn create(
    cli: &Cli,
    client: &ControlPlaneClient,
    spec_json: &Option<String>,
    spec_file: &Option<std::path::PathBuf>,
    idempotency_key: &Option<String>,
) -> Result<(), CliError> {
    let spec = read_document(spec_json, spec_file)?;
    let key = generated_idempotency_key(idempotency_key, "create");
    let value = client
        .post("/control/v1/automations", spec, Some(&key))
        .await?;
    render(cli, &value, human_mutation)
}

/// `update` is a compare-and-set on the caller's read revision.
async fn update(
    cli: &Cli,
    client: &ControlPlaneClient,
    automation_id: &str,
    spec_json: &Option<String>,
    spec_file: &Option<std::path::PathBuf>,
    expected_revision: i64,
    idempotency_key: &Option<String>,
) -> Result<(), CliError> {
    let spec = read_document(spec_json, spec_file)?;
    let value = mutation_request(
        client,
        automation_id,
        "update",
        json!({ "spec": spec, "expected_revision": expected_revision }),
        idempotency_key,
    )
    .await?;
    render(cli, &value, human_mutation)
}

/// enable/disable post an empty body through the shared mutation helper.
async fn simple_mutation(
    cli: &Cli,
    client: &ControlPlaneClient,
    automation_id: &str,
    action: &str,
    body: Value,
    idempotency_key: &Option<String>,
) -> Result<(), CliError> {
    let value = mutation_request(client, automation_id, action, body, idempotency_key).await?;
    render(cli, &value, human_mutation)
}

/// Posts one id-addressed automation mutation with a generated key.
async fn mutation_request(
    client: &ControlPlaneClient,
    automation_id: &str,
    action: &str,
    body: Value,
    idempotency_key: &Option<String>,
) -> Result<Value, CliError> {
    let key = generated_idempotency_key(idempotency_key, action);
    client
        .post(
            &format!("/control/v1/automations/{automation_id}/{action}"),
            body,
            Some(&key),
        )
        .await
}

/// Uses the caller's key, or mints one so a mutation is always idempotent.
fn generated_idempotency_key(provided: &Option<String>, action: &str) -> String {
    match provided {
        Some(key) if !key.trim().is_empty() => key.clone(),
        _ => format!("cs-automation-{action}-{}", uuid::Uuid::new_v4().simple()),
    }
}

/// Reads a JSON document from an inline string, a file path, or stdin (`-`).
fn read_document(
    inline: &Option<String>,
    file: &Option<std::path::PathBuf>,
) -> Result<Value, CliError> {
    use std::io::Read;
    let raw = match (inline, file) {
        (Some(text), None) => text.clone(),
        (None, Some(path)) if path.as_os_str() == "-" => {
            let mut buffer = String::new();
            std::io::stdin()
                .read_to_string(&mut buffer)
                .map_err(|e| CliError::io(format!("Failed to read the document from stdin: {e}")))?;
            buffer
        }
        (None, Some(path)) => std::fs::read_to_string(path).map_err(|e| {
            CliError::io(format!("Failed to read the document {}: {e}", path.display()))
        })?,
        (None, None) => return Err(CliError::usage("a JSON document is required")),
        _ => return Err(CliError::usage("inline and file inputs are mutually exclusive")),
    };
    serde_json::from_str(&raw).map_err(|e| CliError::usage(format!("document is not valid JSON: {e}")))
}

fn enabled_text(value: &Value) -> &'static str {
    if value.get("enabled") == Some(&Value::Bool(true)) {
        "enabled"
    } else {
        "disabled"
    }
}

/// `<id>  <enabled>  <schedule>  <rev>  <next>  <title>` for one automation.
fn automation_row(automation: &Value) -> String {
    format!(
        "{}\t{}\t{}\t{}\t{}\t{}",
        text(automation, "automation_id"),
        enabled_text(automation),
        text(automation, "schedule_kind"),
        text(automation, "revision"),
        text(automation, "next_run_at"),
        text(automation, "title"),
    )
}

fn human_automations(value: &Value) -> Vec<String> {
    let mut rows = vec![rust_i18n::t!("cs.automation_list_header").to_string()];
    let Some(automations) = value.as_array() else {
        rows.push(rust_i18n::t!("cs.no_records").to_string());
        return rows;
    };
    if automations.is_empty() {
        rows.push(rust_i18n::t!("cs.no_records").to_string());
        return rows;
    }
    for automation in automations {
        rows.push(automation_row(automation));
    }
    rows
}

fn human_automation(value: &Value) -> Vec<String> {
    vec![automation_row(value)]
}

fn human_runs(value: &Value) -> Vec<String> {
    let mut rows = vec![rust_i18n::t!("cs.automation_runs_header").to_string()];
    let Some(runs) = value.as_array() else {
        rows.push(rust_i18n::t!("cs.no_records").to_string());
        return rows;
    };
    if runs.is_empty() {
        rows.push(rust_i18n::t!("cs.no_records").to_string());
        return rows;
    }
    for run in runs {
        rows.push(format!(
            "{}\t{}\t{}\t{}\t{}",
            text(run, "status"),
            text(run, "trigger"),
            text(run, "run_id"),
            text(run, "workflow_status"),
            text(run, "started_at"),
        ));
    }
    rows
}

/// A mutation prints its outcome plus the automation id and new revision so an
/// operator can chain the next compare-and-set call.
fn human_mutation(value: &Value) -> Vec<String> {
    let automation = value.get("automation").cloned().unwrap_or(Value::Null);
    vec![format!(
        "{}\t{}\t{}\t{}",
        text(value, "outcome"),
        text(&automation, "automation_id"),
        text(&automation, "revision"),
        text(&automation, "title"),
    )]
}

/// A run dispatch reports the accepted outcome and the projected run status; an
/// accepted start is shown as such, never as a completion (AC-8).
fn human_dispatch(value: &Value) -> Vec<String> {
    let run = value.get("run").cloned().unwrap_or(Value::Null);
    vec![format!(
        "{}\t{}\t{}\t{}",
        text(value, "outcome"),
        text(&run, "run_id"),
        text(&run, "status"),
        text(&run, "workflow_status"),
    )]
}

/// A plan reports its review status, hash, base revision and permission
/// summary, plus any blocking warnings (AC-3/AC-4).
fn human_plan(value: &Value) -> Vec<String> {
    let summary = value.get("permission_summary").cloned().unwrap_or(Value::Null);
    let mut rows = vec![
        rust_i18n::t!("cs.automation_plan_header").to_string(),
        format!(
            "{}\t{}\t{}\t{}",
            text(value, "status"),
            text(value, "plan_hash"),
            text(value, "base_revision"),
            if summary.get("permission_expansion") == Some(&Value::Bool(true)) {
                "permission_change"
            } else {
                "no_permission_change"
            },
        ),
    ];
    let warnings = value
        .get("warnings")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for warning in warnings {
        rows.push(format!(
            "warning\t{}\t{}",
            text(&warning, "code"),
            text(&warning, "message"),
        ));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_list_row_shows_state_schedule_and_revision() {
        let value = json!([
            {
                "automation_id": "01abc",
                "enabled": true,
                "schedule_kind": "interval",
                "revision": 3,
                "next_run_at": "2026-06-25 10:00:00",
                "title": "Nightly",
            }
        ]);
        let rows = human_automations(&value);
        assert!(rows[1].contains("enabled"));
        assert!(rows[1].contains("interval"));
        assert!(rows[1].contains("01abc"));
        assert!(rows[1].contains("Nightly"));
    }

    #[test]
    fn a_run_row_reports_status_and_trigger() {
        let rows = human_runs(&json!([
            { "status": "running", "trigger": "scheduled", "run_id": "r1", "workflow_status": "running", "started_at": "x" }
        ]));
        assert!(rows[1].contains("running"));
        assert!(rows[1].contains("scheduled"));
        assert!(rows[1].contains("r1"));
    }

    /// A dispatch result is rendered by outcome; an accepted start is never
    /// labeled completed by the CLI (it echoes the backend field).
    #[test]
    fn a_dispatch_row_echoes_backend_status_without_inventing_completion() {
        let rows = human_dispatch(&json!({
            "outcome": "accepted",
            "run": { "run_id": "r2", "status": "running", "workflow_status": "running" }
        }));
        assert!(rows[0].contains("accepted"));
        assert!(rows[0].contains("running"));
        assert!(!rows[0].contains("completed"));
    }

    /// A blocked plan surfaces its status and warning so the operator sees that
    /// the parser refused to mint a permission (INV-3/INV-4).
    #[test]
    fn a_blocked_plan_renders_status_and_warnings() {
        let rows = human_plan(&json!({
            "status": "blocked",
            "plan_hash": "",
            "base_revision": null,
            "permission_summary": { "permission_expansion": false },
            "warnings": [{ "code": "requires_explicit_mutation", "message": "cannot add shell" }],
        }));
        assert!(rows[1].contains("blocked"));
        assert!(rows.iter().any(|row| row.contains("requires_explicit_mutation")));
    }

    #[test]
    fn a_document_can_be_inlined_named_or_rejected_when_ambiguous() {
        let inline = Some(json!({ "title": "T" }).to_string());
        assert_eq!(read_document(&inline, &None).expect("inline")["title"], "T");
        assert!(read_document(&None, &None).is_err());
        assert!(read_document(&inline, &Some(std::path::PathBuf::from("-"))).is_err());
    }

    #[test]
    fn an_explicit_idempotency_key_is_never_replaced() {
        assert_eq!(
            generated_idempotency_key(&Some("keep-me".to_string()), "create"),
            "keep-me"
        );
        let generated = generated_idempotency_key(&None, "create");
        assert!(generated.starts_with("cs-automation-create-"), "{generated}");
        assert_ne!(generated, generated_idempotency_key(&None, "create"));
    }

    /// Source guard (AC-1/AC-11/INV-1): the `cs` CLI modules are HTTP/rendering
    /// adapters only. Nothing in `src/bin/cs/` may name the database store, a
    /// SQL driver, the workflow runtime/manager or a process launcher, so the
    /// CLI can never grow a second automation authority or execute a shell.
    #[test]
    fn the_cli_never_reaches_a_local_execution_or_storage_authority() {
        let cs_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/cs");
        // Assembled via `concat!` so the forbidden token never appears as a
        // contiguous literal in this file's own source (which the guard scans).
        let forbidden = [
            concat!("Main", "Store"),
            concat!("rus", "qlite"),
            concat!("Tool", "Manager"),
            concat!("Workflow", "Manager"),
            concat!("Command", "::new"),
            concat!("Db", "Runtime"),
        ];
        let mut scanned = 0usize;
        let entries = std::fs::read_dir(&cs_dir).expect("cs module directory");
        for entry in entries {
            let path = entry.expect("entry").path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("read module source");
            // Strip comments so a doc that names a forbidden authority in prose
            // does not trip the structural guard.
            for raw in source.lines() {
                let line = raw.trim_start();
                if line.starts_with("//") || line.starts_with("///") {
                    continue;
                }
                for token in forbidden {
                    assert!(
                        !line.contains(token),
                        "{} references forbidden authority {token}: {line}",
                        path.display()
                    );
                }
            }
            scanned += 1;
        }
        assert!(scanned > 0, "expected CLI module sources to exist");
    }
}
