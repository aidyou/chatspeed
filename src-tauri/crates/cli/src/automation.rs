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

use crate::args::{AutomationCommand, AutomationFriendlyArgs, Cli};
use crate::automation_input;
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
            friendly,
            spec_json,
            spec_file,
            intent,
            automation_id,
        } => {
            let inputs = SpecInputs {
                json: spec_json.clone(),
                file: spec_file.clone(),
                friendly,
            };
            draft(cli, client, inputs, intent, automation_id).await
        }
        AutomationCommand::Apply {
            plan_json,
            plan_file,
            expected_plan_hash,
            acknowledge_permission_changes,
            idempotency_key,
        } => {
            apply(
                cli,
                client,
                plan_json,
                plan_file,
                expected_plan_hash,
                *acknowledge_permission_changes,
                idempotency_key,
            )
            .await
        }
        AutomationCommand::Create {
            friendly,
            spec_json,
            spec_file,
            idempotency_key,
        } => {
            let inputs = SpecInputs {
                json: spec_json.clone(),
                file: spec_file.clone(),
                friendly,
            };
            create(cli, client, inputs, idempotency_key).await
        }
        AutomationCommand::Update {
            automation_id,
            friendly,
            spec_json,
            spec_file,
            expected_revision,
            idempotency_key,
        } => {
            let inputs = SpecInputs {
                json: spec_json.clone(),
                file: spec_file.clone(),
                friendly,
            };
            update(
                cli,
                client,
                automation_id,
                inputs,
                *expected_revision,
                idempotency_key,
            )
            .await
        }
        AutomationCommand::Enable {
            automation_id,
            idempotency_key,
        } => {
            simple_mutation(
                cli,
                client,
                automation_id,
                "enable",
                json!({}),
                idempotency_key,
            )
            .await
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
            let value =
                mutation_request(client, automation_id, "run", json!({}), idempotency_key).await?;
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

/// The narrow control-plane surface the automation commands need.
///
/// This is a seam, not a second client: `ControlPlaneClient` is the only
/// production implementation, and an in-memory test plane makes the friendly
/// GET-then-POST shaping (revision handling, and no retry after a conflict)
/// verifiable without a live runtime.
trait AutomationIo {
    async fn get_json(&self, path: &str) -> Result<Value, CliError>;

    async fn post_json(
        &self,
        path: &str,
        body: Value,
        idempotency_key: Option<&str>,
    ) -> Result<Value, CliError>;
}

impl AutomationIo for ControlPlaneClient {
    async fn get_json(&self, path: &str) -> Result<Value, CliError> {
        self.get(path).await
    }

    async fn post_json(
        &self,
        path: &str,
        body: Value,
        idempotency_key: Option<&str>,
    ) -> Result<Value, CliError> {
        self.post(path, body, idempotency_key).await
    }
}

/// Reads a raw spec/plan document when one was given, or `None` so the friendly
/// flag path can take over.
fn optional_document(
    inline: &Option<String>,
    file: &Option<std::path::PathBuf>,
) -> Result<Option<Value>, CliError> {
    match (inline, file) {
        (None, None) => Ok(None),
        _ => read_document(inline, file).map(Some),
    }
}

/// The structured inputs a `create`/`update`/`draft` request accepts: a raw
/// document (`--spec-json`/`--spec-file`) or the friendly field flags.
///
/// The two are mutually exclusive, so grouping them keeps the raw document
/// options from being threaded beside the friendly flags through every request
/// path and keeps each handler's argument list short.
struct SpecInputs<'a> {
    /// The raw `--spec-json` document, if one was supplied inline.
    json: Option<String>,
    /// The raw `--spec-file` path, if one was supplied (`-` reads stdin).
    file: Option<std::path::PathBuf>,
    /// The friendly field flags, the alternative to a raw document.
    friendly: &'a AutomationFriendlyArgs,
}

impl SpecInputs<'_> {
    /// Reads the raw document when one was supplied, or `None` so the friendly
    /// flag path can take over.
    fn document(&self) -> Result<Option<Value>, CliError> {
        optional_document(&self.json, &self.file)
    }

    /// Whether any friendly field flag was given.
    fn friendly_requested(&self) -> bool {
        automation_input::friendly_requested(self.friendly)
    }
}

/// Reads one automation view, the authority the friendly path overlays onto.
async fn fetch_view(io: &impl AutomationIo, automation_id: &str) -> Result<Value, CliError> {
    io.get_json(&format!("/control/v1/automations/{automation_id}"))
        .await
}

/// `draft` is a POST read that never mutates, so it needs no idempotency key.
/// Exactly one input is accepted: a raw spec, a constrained intent or the
/// friendly flags (optionally shaped against an existing automation id).
async fn draft(
    cli: &Cli,
    io: &impl AutomationIo,
    inputs: SpecInputs<'_>,
    intent: &Option<String>,
    automation_id: &Option<String>,
) -> Result<(), CliError> {
    let raw = inputs.document()?;
    let has_intent = intent
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty());
    // Clap already excludes a raw document next to an intent; the friendly flags
    // are excluded here so the three inputs stay mutually exclusive.
    if has_intent && (raw.is_some() || inputs.friendly_requested()) {
        return Err(CliError::usage(
            rust_i18n::t!("cs.automation_draft_intent_conflict").to_string(),
        ));
    }
    let body = if has_intent {
        json!({ "automation_id": automation_id, "spec": Value::Null, "intent": intent })
    } else if let Some(document) = raw {
        json!({ "automation_id": automation_id, "spec": document, "intent": Value::Null })
    } else if inputs.friendly_requested() {
        let spec = match automation_id.as_deref() {
            Some(id) => {
                let view = fetch_view(io, id).await?;
                automation_input::overlay_spec(&view, inputs.friendly)?
            }
            None => automation_input::build_create_spec(inputs.friendly)?,
        };
        json!({ "automation_id": automation_id, "spec": spec, "intent": Value::Null })
    } else {
        return Err(CliError::usage(
            rust_i18n::t!("cs.automation_draft_input_required").to_string(),
        ));
    };
    let value = io
        .post_json("/control/v1/automation-draft", body, None)
        .await?;
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
        .or_else(|| {
            plan.get("plan_hash")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
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

/// `create` sends either the raw spec document untouched or the friendly-flag
/// spec; the service stays the only validator and mints the id, so the CLI never
/// grows a laxer notion of a valid automation.
async fn create(
    cli: &Cli,
    io: &impl AutomationIo,
    inputs: SpecInputs<'_>,
    idempotency_key: &Option<String>,
) -> Result<(), CliError> {
    let body = match inputs.document()? {
        // The raw document is posted exactly as supplied.
        Some(document) => document,
        None => {
            if !inputs.friendly_requested() {
                return Err(CliError::usage(
                    rust_i18n::t!("cs.automation_create_input_required").to_string(),
                ));
            }
            automation_input::build_create_spec(inputs.friendly)?
        }
    };
    let key = generated_idempotency_key(idempotency_key, "create");
    let value = io
        .post_json("/control/v1/automations", body, Some(&key))
        .await?;
    render(cli, &value, human_mutation)
}

/// `update` is a compare-and-set on the caller's read revision.
///
/// A raw spec keeps requiring `--expected-revision`. The friendly flags read the
/// current view first, use its revision unless the caller pinned the same value,
/// and overlay only the fields that were given.
async fn update(
    cli: &Cli,
    io: &impl AutomationIo,
    automation_id: &str,
    inputs: SpecInputs<'_>,
    expected_revision: Option<i64>,
    idempotency_key: &Option<String>,
) -> Result<(), CliError> {
    let body = match inputs.document()? {
        Some(document) => {
            let revision = expected_revision.ok_or_else(|| {
                CliError::usage(
                    rust_i18n::t!("cs.automation_expected_revision_required").to_string(),
                )
            })?;
            json!({ "spec": document, "expected_revision": revision })
        }
        None => {
            if !inputs.friendly_requested() {
                return Err(CliError::usage(
                    rust_i18n::t!("cs.automation_update_fields_required").to_string(),
                ));
            }
            let view = fetch_view(io, automation_id).await?;
            let current = automation_input::view_revision(&view)?;
            if let Some(given) = expected_revision {
                if given != current {
                    return Err(CliError::usage(
                        rust_i18n::t!(
                            "cs.automation_expected_revision_mismatch",
                            given = given.to_string(),
                            current = current.to_string()
                        )
                        .to_string(),
                    ));
                }
            }
            let spec = automation_input::overlay_spec(&view, inputs.friendly)?;
            json!({ "spec": spec, "expected_revision": current })
        }
    };
    // A conflict from the compare-and-set is surfaced as-is: the revision is never
    // re-read and the mutation is never retried behind the caller's back.
    let value = mutation_request(io, automation_id, "update", body, idempotency_key).await?;
    render(cli, &value, human_mutation)
}

/// enable/disable post an empty body through the shared mutation helper.
async fn simple_mutation(
    cli: &Cli,
    io: &impl AutomationIo,
    automation_id: &str,
    action: &str,
    body: Value,
    idempotency_key: &Option<String>,
) -> Result<(), CliError> {
    let value = mutation_request(io, automation_id, action, body, idempotency_key).await?;
    render(cli, &value, human_mutation)
}

/// Posts one id-addressed automation mutation with a generated key.
async fn mutation_request(
    io: &impl AutomationIo,
    automation_id: &str,
    action: &str,
    body: Value,
    idempotency_key: &Option<String>,
) -> Result<Value, CliError> {
    let key = generated_idempotency_key(idempotency_key, action);
    io.post_json(
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
    read_document_with(inline, file, &mut std::io::stdin().lock())
}

/// Reads a JSON document from an inline string, a file path, or the given stdin
/// reader (`-`).
///
/// Split out from [`read_document`] so the stdin branch is verifiable without a
/// real terminal or runtime.
fn read_document_with(
    inline: &Option<String>,
    file: &Option<std::path::PathBuf>,
    stdin: &mut impl std::io::Read,
) -> Result<Value, CliError> {
    let raw = match (inline, file) {
        (Some(text), None) => text.clone(),
        (None, Some(path)) if path.as_os_str() == "-" => {
            let mut buffer = String::new();
            stdin.read_to_string(&mut buffer).map_err(|e| {
                CliError::io(format!("Failed to read the document from stdin: {e}"))
            })?;
            buffer
        }
        (None, Some(path)) => std::fs::read_to_string(path).map_err(|e| {
            CliError::io(format!(
                "Failed to read the document {}: {e}",
                path.display()
            ))
        })?,
        (None, None) => return Err(CliError::usage("a JSON document is required")),
        _ => {
            return Err(CliError::usage(
                "inline and file inputs are mutually exclusive",
            ))
        }
    };
    serde_json::from_str(&raw)
        .map_err(|e| CliError::usage(format!("document is not valid JSON: {e}")))
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
    let summary = value
        .get("permission_summary")
        .cloned()
        .unwrap_or(Value::Null);
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
    use crate::args::Command;
    use clap::Parser as _;
    use serde_json::json;
    use std::cell::RefCell;

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
        assert!(rows
            .iter()
            .any(|row| row.contains("requires_explicit_mutation")));
    }

    #[test]
    fn a_document_can_be_inlined_named_or_rejected_when_ambiguous() {
        let inline = Some(json!({ "title": "T" }).to_string());
        assert_eq!(read_document(&inline, &None).expect("inline")["title"], "T");
        assert!(read_document(&None, &None).is_err());
        assert!(read_document(&inline, &Some(std::path::PathBuf::from("-"))).is_err());
    }

    /// The `-` document convention reads the JSON text from stdin.
    #[test]
    fn a_dash_document_reads_stdin() {
        let mut stdin: &[u8] = br#"{"title":"piped","enabled":true}"#;
        let document = read_document_with(&None, &Some(std::path::PathBuf::from("-")), &mut stdin)
            .expect("stdin document");
        assert_eq!(document["title"], "piped");
        assert_eq!(document["enabled"], true);

        // Text that is not JSON stays a usage error, not a silent empty document.
        let mut garbage: &[u8] = b"not json";
        let error = read_document_with(&None, &Some(std::path::PathBuf::from("-")), &mut garbage)
            .expect_err("invalid json");
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn an_explicit_idempotency_key_is_never_replaced() {
        assert_eq!(
            generated_idempotency_key(&Some("keep-me".to_string()), "create"),
            "keep-me"
        );
        let generated = generated_idempotency_key(&None, "create");
        assert!(
            generated.starts_with("cs-automation-create-"),
            "{generated}"
        );
        assert_ne!(generated, generated_idempotency_key(&None, "create"));
    }

    /// An `AutomationView` shaped like the control-plane GET response.
    fn view() -> Value {
        json!({
            "automation_id": "auto-1",
            "title": "Stored",
            "prompt": "old",
            "prompt_file_path": null,
            "agent_id": "agent-1",
            "agent_config": { "approvalLevel": "full" },
            "allowed_paths": ["/keep"],
            "shell_config": null,
            "schedule_kind": "interval",
            "schedule_config": { "interval_minutes": 30 },
            "continuous_context": false,
            "self_review": false,
            "enabled": false,
            "current_workflow_session_id": null,
            "next_run_at": null,
            "last_run_at": null,
            "revision": 7,
            "created_at": null,
            "updated_at": null,
        })
    }

    /// One recorded POST from the in-memory plane.
    struct Posted {
        path: String,
        body: Value,
        key: Option<String>,
    }

    /// A scriptable in-memory control plane. It records the exact route sequence,
    /// so the friendly GET-then-POST shaping is asserted without a live runtime and
    /// without a network mock.
    struct MockIo {
        view: Value,
        post_status: Option<u16>,
        gets: RefCell<Vec<String>>,
        posts: RefCell<Vec<Posted>>,
    }

    impl MockIo {
        fn new(view: Value) -> Self {
            Self {
                view,
                post_status: None,
                gets: RefCell::new(Vec::new()),
                posts: RefCell::new(Vec::new()),
            }
        }
    }

    impl AutomationIo for MockIo {
        async fn get_json(&self, path: &str) -> Result<Value, CliError> {
            self.gets.borrow_mut().push(path.to_string());
            Ok(self.view.clone())
        }

        async fn post_json(
            &self,
            path: &str,
            body: Value,
            idempotency_key: Option<&str>,
        ) -> Result<Value, CliError> {
            self.posts.borrow_mut().push(Posted {
                path: path.to_string(),
                body,
                key: idempotency_key.map(str::to_string),
            });
            match self.post_status {
                Some(status) => Err(CliError::Server {
                    status,
                    code: "automation_revision_conflict".to_string(),
                    message: "revision moved".to_string(),
                }),
                None => Ok(json!({
                    "outcome": "applied",
                    "automation": {
                        "automation_id": "auto-1",
                        "revision": 8,
                        "title": "Stored",
                    },
                })),
            }
        }
    }

    /// Parses a friendly automation command line and returns the client config
    /// plus the extracted friendly flags.
    fn parse_friendly(argv: &[&str]) -> (Cli, AutomationFriendlyArgs) {
        let cli = Cli::try_parse_from(argv).expect("parse");
        let friendly = match &cli.command {
            Command::Automation { command } => match &**command {
                AutomationCommand::Create { friendly, .. }
                | AutomationCommand::Update { friendly, .. }
                | AutomationCommand::Draft { friendly, .. } => friendly.clone(),
                _ => panic!("expected a friendly automation command"),
            },
            _ => panic!("expected an automation command"),
        };
        (cli, friendly)
    }

    /// The spec inputs for a friendly-only request: no raw document, the given
    /// flags.
    fn friendly_inputs(friendly: &AutomationFriendlyArgs) -> SpecInputs<'_> {
        SpecInputs {
            json: None,
            file: None,
            friendly,
        }
    }

    /// The spec inputs for a raw-document request: the given document beside the
    /// parsed friendly flags (empty when `--spec-json` was used).
    fn raw_inputs<'a>(
        json: &Option<String>,
        friendly: &'a AutomationFriendlyArgs,
    ) -> SpecInputs<'a> {
        SpecInputs {
            json: json.clone(),
            file: None,
            friendly,
        }
    }

    #[tokio::test]
    async fn a_friendly_create_posts_the_translated_spec_without_reading_a_view() {
        let (cli, friendly) = parse_friendly(&[
            "cscli",
            "automation",
            "create",
            "--title",
            "Nightly",
            "--agent",
            "agent-1",
            "--prompt",
            "hello",
            "--every",
            "1h",
            "--allowed-path",
            "/tmp/a",
        ]);
        let io = MockIo::new(view());
        create(
            &cli,
            &io,
            friendly_inputs(&friendly),
            &Some("k-1".to_string()),
        )
        .await
        .expect("create");

        assert!(io.gets.borrow().is_empty(), "create must not read a view");
        let posts = io.posts.borrow();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].path, "/control/v1/automations");
        assert_eq!(posts[0].key.as_deref(), Some("k-1"));
        assert_eq!(posts[0].body["title"], "Nightly");
        assert_eq!(posts[0].body["agent_id"], "agent-1");
        assert_eq!(posts[0].body["enabled"], false);
        assert_eq!(posts[0].body["allowed_paths"], json!(["/tmp/a"]));
        assert_eq!(posts[0].body["schedule_config"]["interval_minutes"], 60);
    }

    #[tokio::test]
    async fn a_friendly_update_reads_the_view_and_cas_on_its_revision() {
        let (cli, friendly) = parse_friendly(&[
            "cscli",
            "automation",
            "update",
            "auto-1",
            "--title",
            "Renamed",
        ]);
        let io = MockIo::new(view());
        update(&cli, &io, "auto-1", friendly_inputs(&friendly), None, &None)
            .await
            .expect("update");

        assert_eq!(
            io.gets.borrow().as_slice(),
            ["/control/v1/automations/auto-1"]
        );
        let posts = io.posts.borrow();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].path, "/control/v1/automations/auto-1/update");
        assert_eq!(posts[0].body["expected_revision"], 7);
        assert_eq!(posts[0].body["spec"]["title"], "Renamed");
        // Unspecified spec fields come from the view, and no view metadata leaks.
        assert_eq!(posts[0].body["spec"]["agent_id"], "agent-1");
        assert_eq!(
            posts[0].body["spec"]["agent_config"],
            json!({ "approvalLevel": "full" })
        );
        assert_eq!(posts[0].body["spec"]["allowed_paths"], json!(["/keep"]));
        assert!(posts[0].body["spec"].get("revision").is_none());
    }

    /// A same-kind update may restate schedule parameters only: the stored kind
    /// and its untouched parameters survive beside the restated window.
    #[tokio::test]
    async fn a_friendly_update_may_restate_only_a_window() {
        let (cli, friendly) = parse_friendly(&[
            "cscli",
            "automation",
            "update",
            "auto-1",
            "--start-date",
            "2026-03-01",
            "--end-date",
            "2026-04-01",
        ]);
        let io = MockIo::new(view());
        update(&cli, &io, "auto-1", friendly_inputs(&friendly), None, &None)
            .await
            .expect("window-only update");

        assert_eq!(io.gets.borrow().len(), 1);
        let posts = io.posts.borrow();
        let config = &posts[0].body["spec"]["schedule_config"];
        assert_eq!(posts[0].body["spec"]["schedule_kind"], "interval");
        assert_eq!(config["interval_minutes"], 30);
        assert_eq!(config["start_date"], "2026-03-01");
        assert_eq!(config["end_date"], "2026-04-01");
        assert_eq!(posts[0].body["expected_revision"], 7);
    }

    /// A parameter the stored kind cannot express is refused before any POST, so
    /// an inapplicable window never reaches the service.
    #[tokio::test]
    async fn a_window_only_update_on_a_stored_once_is_rejected_before_any_post() {
        let mut once_view = view();
        once_view["schedule_kind"] = json!("once");
        once_view["schedule_config"] = json!({ "run_at": "2026-06-25T10:00:00Z" });
        let (cli, friendly) =
            parse_friendly(&["cscli", "automation", "update", "auto-1", "--weekdays", "1"]);
        let io = MockIo::new(once_view);
        let error = update(&cli, &io, "auto-1", friendly_inputs(&friendly), None, &None)
            .await
            .expect_err("inapplicable window");
        assert_eq!(error.exit_code(), 2);
        assert!(
            io.posts.borrow().is_empty(),
            "an inapplicable window must not mutate"
        );
    }

    /// The same window-only shaping applies to an update-shaped draft, which reads
    /// the view and keeps the stored kind and interval length.
    #[tokio::test]
    async fn a_friendly_draft_with_an_id_may_restate_only_a_window() {
        let (cli, friendly) = parse_friendly(&[
            "cscli",
            "automation",
            "draft",
            "--automation-id",
            "auto-1",
            "--weekdays",
            "1,3,5",
        ]);
        let io = MockIo::new(view());
        draft(
            &cli,
            &io,
            friendly_inputs(&friendly),
            &None,
            &Some("auto-1".to_string()),
        )
        .await
        .expect("window-only draft");

        assert_eq!(io.gets.borrow().len(), 1);
        let posts = io.posts.borrow();
        assert_eq!(posts[0].path, "/control/v1/automation-draft");
        let config = &posts[0].body["spec"]["schedule_config"];
        assert_eq!(posts[0].body["spec"]["schedule_kind"], "interval");
        assert_eq!(config["interval_minutes"], 30);
        assert_eq!(config["weekdays"], json!([1, 3, 5]));
    }

    #[tokio::test]
    async fn a_pinned_revision_must_match_the_view_and_a_stale_pin_never_mutates() {
        let (cli, friendly) = parse_friendly(&[
            "cscli",
            "automation",
            "update",
            "auto-1",
            "--title",
            "Renamed",
        ]);

        let matching = MockIo::new(view());
        update(
            &cli,
            &matching,
            "auto-1",
            friendly_inputs(&friendly),
            Some(7),
            &None,
        )
        .await
        .expect("matching revision");
        assert_eq!(matching.posts.borrow()[0].body["expected_revision"], 7);

        let stale = MockIo::new(view());
        let error = update(
            &cli,
            &stale,
            "auto-1",
            friendly_inputs(&friendly),
            Some(6),
            &None,
        )
        .await
        .expect_err("stale pin");
        assert_eq!(error.exit_code(), 2);
        assert!(
            stale.posts.borrow().is_empty(),
            "a stale pin must not mutate"
        );
        // The view is still read once, to compare the pin against.
        assert_eq!(stale.gets.borrow().len(), 1);
    }

    #[tokio::test]
    async fn a_raw_update_requires_the_revision_and_posts_the_document_unchanged() {
        let document = json!({
            "title": "Raw",
            "prompt": "p",
            "prompt_file_path": null,
            "agent_id": "a",
            "agent_config": null,
            "allowed_paths": [],
            "shell_config": null,
            "schedule_kind": "daily",
            "schedule_config": { "times": ["09:00"] },
            "continuous_context": false,
            "self_review": false,
            "enabled": true,
        });
        let raw = document.to_string();
        let (cli, friendly) = parse_friendly(&[
            "cscli",
            "automation",
            "update",
            "auto-1",
            "--spec-json",
            &raw,
        ]);
        let spec_json = Some(raw.clone());

        let io = MockIo::new(view());
        let error = update(
            &cli,
            &io,
            "auto-1",
            raw_inputs(&spec_json, &friendly),
            None,
            &None,
        )
        .await
        .expect_err("revision required");
        assert_eq!(error.exit_code(), 2);
        assert!(io.posts.borrow().is_empty());

        let io = MockIo::new(view());
        update(
            &cli,
            &io,
            "auto-1",
            raw_inputs(&spec_json, &friendly),
            Some(3),
            &None,
        )
        .await
        .expect("raw update");
        assert!(
            io.gets.borrow().is_empty(),
            "the raw path never reads the view"
        );
        let posts = io.posts.borrow();
        assert_eq!(
            posts[0].body,
            json!({ "spec": document, "expected_revision": 3 })
        );
    }

    #[tokio::test]
    async fn a_conflict_is_surfaced_once_and_never_re_read_or_retried() {
        let (cli, friendly) = parse_friendly(&[
            "cscli",
            "automation",
            "update",
            "auto-1",
            "--title",
            "Renamed",
        ]);
        let mut io = MockIo::new(view());
        io.post_status = Some(409);
        let error = update(&cli, &io, "auto-1", friendly_inputs(&friendly), None, &None)
            .await
            .expect_err("conflict");
        match error {
            CliError::Server { status, .. } => assert_eq!(status, 409),
            other => panic!("expected a server conflict, got {other:?}"),
        }
        assert_eq!(
            io.gets.borrow().len(),
            1,
            "a conflict must not re-read the view"
        );
        assert_eq!(io.posts.borrow().len(), 1, "a conflict must not be retried");
    }

    #[tokio::test]
    async fn a_friendly_update_requires_at_least_one_field() {
        let (cli, friendly) = parse_friendly(&["cscli", "automation", "update", "auto-1"]);
        let io = MockIo::new(view());
        let error = update(&cli, &io, "auto-1", friendly_inputs(&friendly), None, &None)
            .await
            .expect_err("no fields");
        assert_eq!(error.exit_code(), 2);
        assert!(io.gets.borrow().is_empty());
        assert!(io.posts.borrow().is_empty());
    }

    #[tokio::test]
    async fn a_friendly_draft_with_an_id_reads_the_view_and_overlays() {
        let (cli, friendly) = parse_friendly(&[
            "cscli",
            "automation",
            "draft",
            "--automation-id",
            "auto-1",
            "--title",
            "Renamed",
        ]);
        let io = MockIo::new(view());
        draft(
            &cli,
            &io,
            friendly_inputs(&friendly),
            &None,
            &Some("auto-1".to_string()),
        )
        .await
        .expect("draft");
        assert_eq!(io.gets.borrow().len(), 1);
        let posts = io.posts.borrow();
        assert_eq!(posts[0].path, "/control/v1/automation-draft");
        assert_eq!(posts[0].body["automation_id"], "auto-1");
        assert_eq!(posts[0].body["spec"]["title"], "Renamed");
        assert_eq!(posts[0].body["intent"], Value::Null);
        assert!(posts[0].key.is_none(), "draft carries no idempotency key");
    }

    #[tokio::test]
    async fn a_friendly_draft_without_an_id_builds_a_fresh_spec_without_reading() {
        let (cli, friendly) = parse_friendly(&[
            "cscli",
            "automation",
            "draft",
            "--title",
            "Nightly",
            "--agent",
            "a",
            "--prompt",
            "p",
            "--daily",
            "09:00",
        ]);
        let io = MockIo::new(view());
        draft(&cli, &io, friendly_inputs(&friendly), &None, &None)
            .await
            .expect("draft");
        assert!(io.gets.borrow().is_empty());
        let posts = io.posts.borrow();
        assert_eq!(posts[0].body["spec"]["schedule_kind"], "daily");
        assert_eq!(
            posts[0].body["spec"]["schedule_config"]["times"],
            json!(["09:00"])
        );
    }

    #[tokio::test]
    async fn a_raw_or_intent_draft_keeps_its_original_wire_shape() {
        let document = json!({ "title": "Raw" });
        let raw = document.to_string();
        let (cli, friendly) = parse_friendly(&[
            "cscli",
            "automation",
            "draft",
            "--spec-json",
            &raw,
            "--automation-id",
            "auto-1",
        ]);
        let io = MockIo::new(view());
        draft(
            &cli,
            &io,
            raw_inputs(&Some(raw.clone()), &friendly),
            &None,
            &Some("auto-1".to_string()),
        )
        .await
        .expect("raw draft");
        assert!(
            io.gets.borrow().is_empty(),
            "a raw draft must not read the view"
        );
        assert_eq!(
            io.posts.borrow()[0].body,
            json!({ "automation_id": "auto-1", "spec": document, "intent": Value::Null })
        );

        let (cli, friendly) = parse_friendly(&[
            "cscli",
            "automation",
            "draft",
            "--intent",
            "run daily at nine",
        ]);
        let io = MockIo::new(view());
        draft(
            &cli,
            &io,
            friendly_inputs(&friendly),
            &Some("run daily at nine".to_string()),
            &None,
        )
        .await
        .expect("intent draft");
        assert_eq!(
            io.posts.borrow()[0].body,
            json!({
                "automation_id": Value::Null,
                "spec": Value::Null,
                "intent": "run daily at nine",
            })
        );
    }

    #[tokio::test]
    async fn a_draft_rejects_an_ambiguous_input_and_requires_one() {
        let (cli, friendly) = parse_friendly(&["cscli", "automation", "draft", "--title", "T"]);
        let io = MockIo::new(view());
        let error = draft(
            &cli,
            &io,
            friendly_inputs(&friendly),
            &Some("intent".to_string()),
            &None,
        )
        .await
        .expect_err("ambiguous input");
        assert_eq!(error.exit_code(), 2);
        assert!(io.posts.borrow().is_empty());

        let (cli, friendly) = parse_friendly(&["cscli", "automation", "draft"]);
        let io = MockIo::new(view());
        let error = draft(&cli, &io, friendly_inputs(&friendly), &None, &None)
            .await
            .expect_err("no input");
        assert_eq!(error.exit_code(), 2);
        assert!(io.posts.borrow().is_empty());
    }

    /// Source guard (AC-1/AC-11/INV-1): the `cs` CLI modules are HTTP/rendering
    /// adapters only. Nothing in `cli/src/` may name the database store, a
    /// SQL driver, the workflow runtime/manager or a process launcher, so the
    /// CLI can never grow a second automation authority or execute a shell.
    #[test]
    fn the_cli_never_reaches_a_local_execution_or_storage_authority() {
        let cs_dir = crate::source_guard_dir();
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

    // --- Real loopback control plane ----------------------------------------
    //
    // The in-memory plane above proves the shaping logic; this fixture proves the
    // same paths against an actual socket and the production `ControlPlaneClient`,
    // so the GET response envelope, the CAS body and the no-retry-on-409 rule are
    // verified end to end instead of only through the `AutomationIo` seam.

    /// Instance id and pid the loopback plane reports on `/meta`.
    const LOOPBACK_INSTANCE: &str = "loopback-instance";
    const LOOPBACK_PID: u32 = 4242;

    /// One request the loopback plane received.
    #[derive(Debug, Clone)]
    struct RecordedRequest {
        method: String,
        path: String,
        body: Value,
    }

    /// An isolated HTTP control plane bound to a loopback port.
    struct LoopbackPlane {
        port: u16,
        requests: std::sync::Arc<std::sync::Mutex<Vec<RecordedRequest>>>,
        task: tokio::task::JoinHandle<()>,
    }

    impl LoopbackPlane {
        /// Serves the readiness handshake, one automation GET and one update POST.
        ///
        /// `update_status` selects the POST reply so a conflict can be exercised.
        async fn start(view: Value, update_status: Option<u16>) -> Self {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .expect("bind loopback");
            let port = listener.local_addr().expect("local addr").port();
            let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let recorded = std::sync::Arc::clone(&requests);
            let task = tokio::spawn(async move {
                use tokio::io::AsyncWriteExt;
                loop {
                    let Ok((mut stream, _)) = listener.accept().await else {
                        return;
                    };
                    let Some(request) = read_http_request(&mut stream).await else {
                        continue;
                    };
                    let response = respond(&request, &view, update_status);
                    recorded.lock().expect("lock").push(request);
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.flush().await;
                    let _ = stream.shutdown().await;
                }
            });
            Self {
                port,
                requests,
                task,
            }
        }

        /// The requests received so far, in order.
        fn recorded(&self) -> Vec<RecordedRequest> {
            self.requests.lock().expect("lock").clone()
        }

        async fn shutdown(self) {
            self.task.abort();
            let _ = self.task.await;
        }
    }

    /// Connects the production client to the loopback plane, completing the same
    /// readiness handshake every command performs.
    async fn connect_to(plane: &LoopbackPlane) -> ControlPlaneClient {
        let discovery = crate::discovery::ControlPlaneDiscovery {
            protocol_version: "1".to_string(),
            server_instance_id: LOOPBACK_INSTANCE.to_string(),
            pid: LOOPBACK_PID,
            host: "127.0.0.1".to_string(),
            port: plane.port,
            token: "secret".to_string(),
            started_at: "now".to_string(),
        };
        ControlPlaneClient::connect(&discovery)
            .await
            .expect("connect to the loopback plane")
    }

    /// Reads one HTTP/1.1 request, headers plus the `Content-Length` body.
    async fn read_http_request(stream: &mut tokio::net::TcpStream) -> Option<RecordedRequest> {
        use tokio::io::AsyncReadExt;
        let mut chunk = [0u8; 1024];
        let mut buffer = Vec::new();
        let header_end = loop {
            let read = stream.read(&mut chunk).await.ok()?;
            if read == 0 {
                return None;
            }
            buffer.extend_from_slice(&chunk[..read]);
            if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                break position;
            }
        };
        let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
        let mut lines = head.split("\r\n");
        let mut request_line = lines.next().unwrap_or_default().split_whitespace();
        let method = request_line.next().unwrap_or_default().to_string();
        let path = request_line.next().unwrap_or_default().to_string();
        let content_length = lines
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, value)| value.trim().parse::<usize>().ok())
            .unwrap_or(0);

        let mut body = buffer[header_end + 4..].to_vec();
        while body.len() < content_length {
            let read = stream.read(&mut chunk).await.ok()?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..read]);
        }
        let body = serde_json::from_slice(&body).unwrap_or(Value::Null);
        Some(RecordedRequest { method, path, body })
    }

    /// The canned reply for one recorded request.
    fn respond(request: &RecordedRequest, view: &Value, update_status: Option<u16>) -> String {
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/control/v1/meta") => json_response(
                200,
                &json!({
                    "service": "chatspeed-runtime",
                    "protocol_version": "1",
                    "schema_version": 1,
                    "server_instance_id": LOOPBACK_INSTANCE,
                    "pid": LOOPBACK_PID,
                }),
            ),
            ("GET", "/control/v1/automations/auto-1") => json_response(200, view),
            ("POST", "/control/v1/automations/auto-1/update") => match update_status {
                Some(status) => json_response(
                    status,
                    &json!({
                        "error": {
                            "code": "automation_revision_conflict",
                            "message": "revision moved",
                        }
                    }),
                ),
                None => json_response(
                    200,
                    &json!({
                        "outcome": "applied",
                        "automation": {
                            "automation_id": "auto-1",
                            "revision": 8,
                            "title": "Stored",
                        },
                    }),
                ),
            },
            _ => json_response(
                404,
                &json!({ "error": { "code": "not_found", "message": "unexpected route" } }),
            ),
        }
    }

    /// Renders one JSON reply; the connection is closed so the client never reuses
    /// it after the fixture's routing decision.
    fn json_response(status: u16, body: &Value) -> String {
        let reason = match status {
            200 => "OK",
            404 => "Not Found",
            409 => "Conflict",
            _ => "Error",
        };
        let body = body.to_string();
        format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    #[tokio::test]
    async fn a_friendly_update_over_real_http_maps_the_view_and_cas_body() {
        let plane = LoopbackPlane::start(view(), None).await;
        let client = connect_to(&plane).await;
        let (cli, friendly) = parse_friendly(&[
            "cscli",
            "automation",
            "update",
            "auto-1",
            "--title",
            "Renamed",
        ]);
        update(
            &cli,
            &client,
            "auto-1",
            friendly_inputs(&friendly),
            None,
            &None,
        )
        .await
        .expect("update over http");

        let requests = plane.recorded();
        let routes: Vec<(&str, &str)> = requests
            .iter()
            .map(|request| (request.method.as_str(), request.path.as_str()))
            .collect();
        assert_eq!(
            routes,
            [
                ("GET", "/control/v1/meta"),
                ("GET", "/control/v1/automations/auto-1"),
                ("POST", "/control/v1/automations/auto-1/update"),
            ]
        );
        let body = &requests[2].body;
        assert_eq!(body["expected_revision"], 7);
        assert_eq!(body["spec"]["title"], "Renamed");
        // Unspecified fields come from the real GET response envelope.
        assert_eq!(body["spec"]["agent_id"], "agent-1");
        assert_eq!(body["spec"]["allowed_paths"], json!(["/keep"]));
        plane.shutdown().await;
    }

    #[tokio::test]
    async fn a_friendly_update_may_restate_only_a_window_over_real_http() {
        let plane = LoopbackPlane::start(view(), None).await;
        let client = connect_to(&plane).await;
        let (cli, friendly) = parse_friendly(&[
            "cscli",
            "automation",
            "update",
            "auto-1",
            "--weekdays",
            "3,5",
        ]);
        update(
            &cli,
            &client,
            "auto-1",
            friendly_inputs(&friendly),
            None,
            &None,
        )
        .await
        .expect("window-only update over http");

        let requests = plane.recorded();
        let body = &requests.last().expect("post").body;
        // The stored interval length is kept; only the window is replaced.
        assert_eq!(body["spec"]["schedule_kind"], "interval");
        assert_eq!(body["spec"]["schedule_config"]["interval_minutes"], 30);
        assert_eq!(body["spec"]["schedule_config"]["weekdays"], json!([3, 5]));
        plane.shutdown().await;
    }

    #[tokio::test]
    async fn a_conflict_over_real_http_is_not_retried() {
        let plane = LoopbackPlane::start(view(), Some(409)).await;
        let client = connect_to(&plane).await;
        let (cli, friendly) = parse_friendly(&[
            "cscli",
            "automation",
            "update",
            "auto-1",
            "--title",
            "Renamed",
        ]);
        let error = update(
            &cli,
            &client,
            "auto-1",
            friendly_inputs(&friendly),
            None,
            &None,
        )
        .await
        .expect_err("conflict over http");
        match error {
            CliError::Server { status, code, .. } => {
                assert_eq!(status, 409);
                assert_eq!(code, "automation_revision_conflict");
            }
            other => panic!("expected a server conflict, got {other:?}"),
        }
        let requests = plane.recorded();
        assert_eq!(
            requests.iter().filter(|r| r.method == "POST").count(),
            1,
            "a 409 must not be retried"
        );
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.path == "/control/v1/automations/auto-1")
                .count(),
            1,
            "a conflict must not re-read the view"
        );
        plane.shutdown().await;
    }

    #[tokio::test]
    async fn a_raw_update_over_real_http_posts_the_document_unchanged() {
        let document = json!({
            "title": "Raw",
            "prompt": "p",
            "prompt_file_path": null,
            "agent_id": "a",
            "agent_config": null,
            "allowed_paths": [],
            "shell_config": null,
            "schedule_kind": "daily",
            "schedule_config": { "times": ["09:00"] },
            "continuous_context": false,
            "self_review": false,
            "enabled": true,
        });
        let raw = document.to_string();
        let plane = LoopbackPlane::start(view(), None).await;
        let client = connect_to(&plane).await;
        let (cli, friendly) = parse_friendly(&[
            "cscli",
            "automation",
            "update",
            "auto-1",
            "--spec-json",
            &raw,
        ]);
        update(
            &cli,
            &client,
            "auto-1",
            raw_inputs(&Some(raw.clone()), &friendly),
            Some(3),
            &None,
        )
        .await
        .expect("raw update over http");

        let requests = plane.recorded();
        // Only the readiness handshake and the CAS POST: the raw path never reads
        // the automation view, so its document cannot be rewritten by an envelope.
        assert!(requests
            .iter()
            .all(|request| request.path != "/control/v1/automations/auto-1"));
        let post = requests
            .iter()
            .find(|request| request.method == "POST")
            .expect("cas post");
        assert_eq!(
            post.body,
            json!({ "spec": document, "expected_revision": 3 })
        );
        plane.shutdown().await;
    }
}
