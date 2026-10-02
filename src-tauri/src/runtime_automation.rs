//! Desktop adapters that route the existing Tauri workflow-automation commands
//! to the standalone runtime control plane.
//!
//! The runtime process is the single owner of the automation table, its runs and
//! the scheduler. The Tauri commands therefore stop calling a local
//! `WorkflowApplicationService`/`AutomationApplicationService` and reach the
//! runtime through the typed `RuntimeClient` the [`RuntimeSupervisor`] hands out,
//! using only the documented `/control/v1` automation routes.
//!
//! There is deliberately no local fallback and no second service: when the
//! supervisor holds no lease the call fails instead of silently speaking to a
//! second automation owner. Every mutation travels through the canonical
//! idempotency header, so a transport retry replays the durable receipt instead
//! of applying twice.
//!
//! # Wire mapping
//!
//! The canonical routes answer with the transport-neutral `snake_case`
//! projections (`AutomationView`, `AutomationRunView`, `AutomationPlanV1`,
//! `AutomationMutationResult`). The desktop editor, however, still consumes the
//! historical camelCase rows (`WorkflowAutomation`, `WorkflowAutomationRun`),
//! while `draft`/`apply`/`run_views` already return the exact canonical types.
//! This module re-cases the projections back to the historical rows so the
//! frontend contract does not change.
//!
//! ## Known contract gap
//!
//! `AutomationView` deliberately omits `agent_config`, and no read route exposes
//! it. The reconstructed `WorkflowAutomation.agent_config` is therefore `None`.
//! The write path still forwards the editor's `agent_config` verbatim, so a
//! stored override is honoured when it is sent; it simply cannot be read back
//! through the canonical projection. Closing the gap needs a read-side field (or
//! a small compatibility route) on the control plane, which is outside this
//! module's write scope.

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

use chatspeed_runtime_client::{ClientError, RuntimeClient};

use crate::db::{WorkflowAutomation, WorkflowAutomationRun};
use crate::runtime_client::{RuntimeSupervisor, RuntimeUnavailable};
use crate::workflow::automation::types::{
    AutomationApplyRequest, AutomationDispatchResult, AutomationDraftInput,
    AutomationMutationResult, AutomationPlanV1, AutomationRunView, AutomationSpec, AutomationView,
    WorkflowAutomationRequest, WorkflowAutomationRunNowResult,
};

/// Canonical route for the automation collection (`GET` list, `POST` create).
const AUTOMATIONS_ROUTE: &str = "/control/v1/automations";

/// Canonical side-effect-free plan route.
const AUTOMATION_DRAFT_ROUTE: &str = "/control/v1/automation-draft";

/// Canonical apply-a-reviewed-plan route.
const AUTOMATION_APPLY_ROUTE: &str = "/control/v1/automation-apply";

// ---------------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------------

/// Lists every automation as the historical camelCase rows.
pub async fn automation_list(
    supervisor: &RuntimeSupervisor,
) -> Result<Vec<WorkflowAutomation>, String> {
    let views: Vec<AutomationView> = decode(get_value(supervisor, AUTOMATIONS_ROUTE).await?)?;
    Ok(views.into_iter().map(view_to_row).collect())
}

/// Gets one automation, mapping the runtime's stable 404 to `Ok(None)`.
pub async fn automation_get(
    supervisor: &RuntimeSupervisor,
    id: &str,
) -> Result<Option<WorkflowAutomation>, String> {
    let path = format!("{AUTOMATIONS_ROUTE}/{}", encode_path_segment(id));
    let client = control_plane_client(supervisor).await?;
    match client.get(&path).await {
        Ok(value) => Ok(Some(view_to_row(decode(value)?))),
        Err(ClientError::Server { status, .. }) if status == 404 => Ok(None),
        Err(error) => Err(map_client_error(error)),
    }
}

/// Lists an automation's runs as the historical camelCase rows.
pub async fn automation_list_runs(
    supervisor: &RuntimeSupervisor,
    automation_id: &str,
) -> Result<Vec<WorkflowAutomationRun>, String> {
    let views = fetch_run_views(supervisor, automation_id).await?;
    Ok(views.into_iter().map(run_view_to_row).collect())
}

/// Lists the projected run lifecycle (already the canonical snake_case wire).
pub async fn automation_run_views(
    supervisor: &RuntimeSupervisor,
    automation_id: &str,
) -> Result<Vec<AutomationRunView>, String> {
    fetch_run_views(supervisor, automation_id).await
}

// ---------------------------------------------------------------------------
// Mutations
// ---------------------------------------------------------------------------

/// Creates or updates an automation, preserving the historical upsert save.
pub async fn automation_save(
    supervisor: &RuntimeSupervisor,
    request: WorkflowAutomationRequest,
) -> Result<WorkflowAutomation, String> {
    let spec = request_to_spec(&request);
    let provided = request
        .id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty());

    let result = match provided {
        Some(id) => match automation_get(supervisor, id).await? {
            Some(existing) => update_automation(supervisor, id, &spec, existing.revision).await?,
            // The legacy editor save inserted a row when a caller supplied an
            // unknown id; the canonical create assigns the id itself, so a plain
            // create is the closest faithful mapping.
            None => create_automation(supervisor, &spec).await?,
        },
        None => create_automation(supervisor, &spec).await?,
    };

    let view = result
        .automation
        .ok_or_else(|| "Runtime save returned no automation".to_string())?;
    Ok(view_to_row(view))
}

/// Deletes one automation after the caller's explicit confirmation.
pub async fn automation_delete(
    supervisor: &RuntimeSupervisor,
    id: &str,
    confirm: bool,
) -> Result<(), String> {
    let path = format!("{AUTOMATIONS_ROUTE}/{}/delete", encode_path_segment(id));
    post_idempotent(supervisor, &path, &json!({ "confirm": confirm })).await?;
    Ok(())
}

/// Enables or disables one automation.
pub async fn automation_set_enabled(
    supervisor: &RuntimeSupervisor,
    id: &str,
    enabled: bool,
) -> Result<(), String> {
    let action = if enabled { "enable" } else { "disable" };
    let path = format!("{AUTOMATIONS_ROUTE}/{}/{}", encode_path_segment(id), action);
    post_idempotent(supervisor, &path, &json!({})).await?;
    Ok(())
}

/// Runs one automation now, rebuilding the historical camelCase result.
pub async fn automation_run_now(
    supervisor: &RuntimeSupervisor,
    automation_id: &str,
) -> Result<WorkflowAutomationRunNowResult, String> {
    let path = format!(
        "{AUTOMATIONS_ROUTE}/{}/run",
        encode_path_segment(automation_id)
    );
    let dispatch: AutomationDispatchResult =
        decode(post_idempotent(supervisor, &path, &json!({})).await?)?;
    let dispatch_view = dispatch
        .run
        .ok_or_else(|| "Runtime run produced no dispatch view".to_string())?;

    let automation = automation_get(supervisor, automation_id)
        .await?
        .ok_or_else(|| format!("Automation {automation_id} not found"))?;
    let run = automation_list_runs(supervisor, automation_id)
        .await?
        .into_iter()
        .find(|row| row.id == dispatch_view.run_id)
        .ok_or_else(|| "Created run disappeared".to_string())?;
    let workflow_session_id = run.workflow_session_id.clone().unwrap_or_default();

    Ok(WorkflowAutomationRunNowResult {
        automation,
        run,
        workflow_session_id,
    })
}

/// Produces the side-effect-free draft plan (already the canonical wire type).
pub async fn automation_draft(
    supervisor: &RuntimeSupervisor,
    input: AutomationDraftInput,
) -> Result<AutomationPlanV1, String> {
    let body = encode(&input)?;
    let client = control_plane_client(supervisor).await?;
    let value = client
        .post(AUTOMATION_DRAFT_ROUTE, &body)
        .await
        .map_err(map_client_error)?;
    decode(value)
}

/// Applies a previously reviewed plan (already the canonical wire type).
pub async fn automation_apply(
    supervisor: &RuntimeSupervisor,
    request: AutomationApplyRequest,
) -> Result<AutomationMutationResult, String> {
    let body = encode(&request)?;
    let value = post_idempotent(supervisor, AUTOMATION_APPLY_ROUTE, &body).await?;
    decode(value)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Creates an automation and returns the canonical mutation result.
async fn create_automation(
    supervisor: &RuntimeSupervisor,
    spec: &AutomationSpec,
) -> Result<AutomationMutationResult, String> {
    let body = encode(spec)?;
    let value = post_idempotent(supervisor, AUTOMATIONS_ROUTE, &body).await?;
    decode(value)
}

/// Compare-and-set updates an automation against its current revision.
async fn update_automation(
    supervisor: &RuntimeSupervisor,
    id: &str,
    spec: &AutomationSpec,
    expected_revision: i64,
) -> Result<AutomationMutationResult, String> {
    let path = format!("{AUTOMATIONS_ROUTE}/{}/update", encode_path_segment(id));
    let body = json!({
        "spec": encode(spec)?,
        "expected_revision": expected_revision,
    });
    decode(post_idempotent(supervisor, &path, &body).await?)
}

/// Reads an automation's projected runs.
async fn fetch_run_views(
    supervisor: &RuntimeSupervisor,
    automation_id: &str,
) -> Result<Vec<AutomationRunView>, String> {
    let path = format!(
        "{AUTOMATIONS_ROUTE}/{}/runs",
        encode_path_segment(automation_id)
    );
    decode(get_value(supervisor, &path).await?)
}

/// Converts the desktop editor's camelCase request into the canonical spec.
fn request_to_spec(request: &WorkflowAutomationRequest) -> AutomationSpec {
    AutomationSpec {
        title: request.title.clone(),
        prompt: request.prompt.clone(),
        prompt_file_path: request.prompt_file_path.clone(),
        agent_id: request.agent_id.clone(),
        agent_config: request.agent_config.clone(),
        allowed_paths: request.allowed_paths.clone(),
        shell_config: request.shell_config.clone(),
        schedule_kind: request.schedule_kind.clone(),
        schedule_config: request.schedule_config.clone(),
        continuous_context: request.continuous_context,
        self_review: request.self_review,
        enabled: request.enabled,
    }
}

/// Re-cases one canonical view back to the historical camelCase row.
fn view_to_row(view: AutomationView) -> WorkflowAutomation {
    WorkflowAutomation {
        id: view.automation_id,
        title: view.title,
        prompt: view.prompt,
        prompt_file_path: view.prompt_file_path,
        agent_id: view.agent_id,
        agent_config: view
            .agent_config
            .map(|config| to_json_string(&config, "null")),
        allowed_paths: to_json_string(&view.allowed_paths, "[]"),
        shell_config: view
            .shell_config
            .map(|shell| to_json_string(&shell, "null")),
        schedule_kind: view.schedule_kind,
        schedule_config: to_json_string(&view.schedule_config, "{}"),
        continuous_context: view.continuous_context,
        current_workflow_session_id: view.current_workflow_session_id,
        self_review: view.self_review,
        enabled: view.enabled,
        next_run_at: view.next_run_at,
        last_run_at: view.last_run_at,
        created_at: view.created_at,
        updated_at: view.updated_at,
        revision: view.revision,
    }
}

/// Re-cases one canonical run view back to the historical camelCase row.
fn run_view_to_row(view: AutomationRunView) -> WorkflowAutomationRun {
    WorkflowAutomationRun {
        id: view.run_id,
        automation_id: view.automation_id,
        workflow_session_id: view.workflow_session_id,
        status: view.status,
        scheduled_for: view.scheduled_for,
        started_at: view.started_at,
        finished_at: view.finished_at,
        error: view.error,
        created_at: view.created_at,
        updated_at: view.updated_at,
        trigger: view.trigger,
        dispatch_key: view.dispatch_key,
    }
}

/// Serializes a persisted JSON field back to the string the row wire carries.
///
/// `Vec<String>` and `Value` serialize infallibly, so the fallback only guards
/// against pathological input; the empty default keeps the frontend parser able
/// to read a well-typed value.
fn to_json_string<T: Serialize>(value: &T, fallback: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| fallback.to_string())
}

/// Resolves the connected control-plane client, or fails when no lease is held.
async fn control_plane_client(supervisor: &RuntimeSupervisor) -> Result<RuntimeClient, String> {
    supervisor.client().await.map_err(map_runtime_error)
}

/// Sends one idempotent mutation with a fresh key.
async fn post_idempotent(
    supervisor: &RuntimeSupervisor,
    path: &str,
    body: &Value,
) -> Result<Value, String> {
    let client = control_plane_client(supervisor).await?;
    client
        .post_with_idempotency(path, body, &new_idempotency_key())
        .await
        .map_err(map_client_error)
}

/// Sends one read and returns its JSON body.
async fn get_value(supervisor: &RuntimeSupervisor, path: &str) -> Result<Value, String> {
    let client = control_plane_client(supervisor).await?;
    client.get(path).await.map_err(map_client_error)
}

/// Fresh idempotency key for one command invocation.
fn new_idempotency_key() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Encodes a typed request body for the wire.
fn encode<T: Serialize>(body: &T) -> Result<Value, String> {
    serde_json::to_value(body).map_err(|error| error.to_string())
}

/// Decodes a canonical snake_case runtime response into its typed shape.
fn decode<T: DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|error| format!("Unexpected runtime response: {error}"))
}

/// Percent-encodes one path segment so an id cannot inject a separator or query.
fn encode_path_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        let unreserved = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~');
        if unreserved {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// Maps a transport/domain client error to the Tauri string error.
///
/// A structured runtime error keeps its bare message so the Tauri wire stays as
/// close as possible to the previous `AutomationError::message` string; every
/// other failure keeps the client's classified (redacted) description.
fn map_client_error(error: ClientError) -> String {
    match error {
        ClientError::Server { message, .. } => message,
        other => other.to_string(),
    }
}

/// Maps a supervisor availability failure to the Tauri string error.
fn map_runtime_error(error: RuntimeUnavailable) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_view() -> AutomationView {
        AutomationView {
            automation_id: "auto-1".to_string(),
            title: "Nightly".to_string(),
            prompt: Some("do work".to_string()),
            prompt_file_path: None,
            agent_id: "agent-1".to_string(),
            agent_config: Some(json!({ "temperature": 0.2 })),
            allowed_paths: vec!["/tmp/a".to_string(), "/tmp/b".to_string()],
            shell_config: Some(json!({ "command": "echo hi" })),
            schedule_kind: "interval".to_string(),
            schedule_config: json!({ "interval_minutes": 60 }),
            continuous_context: true,
            self_review: false,
            enabled: true,
            current_workflow_session_id: Some("wf-1".to_string()),
            next_run_at: Some("2026-01-01T00:00:00Z".to_string()),
            last_run_at: None,
            revision: 3,
            created_at: Some("2025-12-01T00:00:00Z".to_string()),
            updated_at: None,
        }
    }

    fn sample_run_view() -> AutomationRunView {
        AutomationRunView {
            run_id: "run-1".to_string(),
            automation_id: "auto-1".to_string(),
            trigger: "manual".to_string(),
            dispatch_key: None,
            status: "running".to_string(),
            workflow_session_id: Some("wf-1".to_string()),
            scheduled_for: "2026-01-01T00:00:00Z".to_string(),
            started_at: Some("2026-01-01T00:00:01Z".to_string()),
            finished_at: None,
            error: None,
            workflow_status: Some("running".to_string()),
            wait_reason: None,
            created_at: Some("2026-01-01T00:00:00Z".to_string()),
            updated_at: None,
        }
    }

    #[test]
    fn a_view_reconstructs_the_historical_camelcase_row() {
        let row = view_to_row(sample_view());

        // The id is re-cased from `automation_id`.
        assert_eq!(row.id, "auto-1");
        // Persisted JSON fields are re-encoded as the JSON strings the row wire
        // has always carried, so the frontend parser keeps working.
        assert_eq!(row.allowed_paths, r#"["/tmp/a","/tmp/b"]"#);
        assert_eq!(
            row.shell_config.as_deref(),
            Some(r#"{"command":"echo hi"}"#)
        );
        assert_eq!(row.schedule_config, r#"{"interval_minutes":60}"#);
        assert_eq!(row.revision, 3);
        // The canonical projection preserves agent configuration for a lossless
        // editor round-trip.
        assert_eq!(row.agent_config.as_deref(), Some(r#"{"temperature":0.2}"#));
    }

    #[test]
    fn a_run_view_reconstructs_the_historical_camelcase_row() {
        let row = run_view_to_row(sample_run_view());

        assert_eq!(row.id, "run-1");
        assert_eq!(row.automation_id, "auto-1");
        assert_eq!(row.status, "running");
        assert_eq!(row.trigger, "manual");
        assert_eq!(row.workflow_session_id.as_deref(), Some("wf-1"));
    }

    #[test]
    fn a_camelcase_request_maps_to_the_canonical_spec() {
        let request = WorkflowAutomationRequest {
            id: Some("auto-1".to_string()),
            title: "Nightly".to_string(),
            prompt: Some("do work".to_string()),
            prompt_file_path: None,
            agent_id: "agent-1".to_string(),
            agent_config: Some(json!({ "models": { "act": "m" } })),
            allowed_paths: vec!["/tmp".to_string()],
            shell_config: None,
            schedule_kind: "daily".to_string(),
            schedule_config: json!({ "time": "09:00" }),
            continuous_context: false,
            self_review: true,
            enabled: true,
        };

        let spec = request_to_spec(&request);

        // The write path preserves the editor's agent_config verbatim even though
        // the read projection cannot return it.
        assert_eq!(spec.agent_config, Some(json!({ "models": { "act": "m" } })));
        assert_eq!(spec.allowed_paths, vec!["/tmp".to_string()]);
        assert!(spec.self_review);
        // The canonical spec serializes to snake_case keys the route expects.
        let wire = encode(&spec).expect("spec wire");
        assert_eq!(wire["schedule_kind"], json!("daily"));
        assert_eq!(wire["self_review"], json!(true));
    }

    #[test]
    fn path_segments_are_percent_encoded() {
        assert_eq!(encode_path_segment("auto-1_a.b~c"), "auto-1_a.b~c");
        assert_eq!(encode_path_segment("a/b"), "a%2Fb");
        assert_eq!(encode_path_segment("a?b=c"), "a%3Fb%3Dc");
    }

    #[test]
    fn a_server_error_keeps_its_bare_message() {
        let error = map_client_error(ClientError::Server {
            status: 409,
            code: "confirmation_required".to_string(),
            message: "delete requires confirmation".to_string(),
        });
        assert_eq!(error, "delete requires confirmation");
    }

    #[test]
    fn a_missing_lease_maps_to_the_availability_error() {
        assert_eq!(
            map_runtime_error(RuntimeUnavailable::NotConnected),
            "runtime client is not connected"
        );
    }
}
