//! Desktop adapters that route the existing Tauri workflow commands to the
//! standalone runtime control plane.
//!
//! The runtime process is the single workflow owner. The Tauri commands must
//! therefore stop calling a local `WorkflowApplicationService`/`MainStore` and
//! reach the runtime through the typed `RuntimeClient` the
//! [`RuntimeSupervisor`] hands out, using the documented `/control/v1` routes
//! only. This module owns the request/response mapping so each command wrapper
//! stays a thin translation of its Tauri wire into one runtime call.
//!
//! There is deliberately no local fallback: when the supervisor holds no lease
//! the call fails instead of silently speaking to a second owner.
//!
//! Two families of routes are used:
//!
//! * The original, canonical snake_case workflow routes
//!   (`/control/v1/workflows...`) serve the seven commands the first child
//!   migrated.
//! * The explicit, allowlisted compatibility route
//!   `/control/v1/workflow-commands/{command}` serves the remaining workflow
//!   commands that previously ran inside the desktop wrappers. It carries a
//!   strongly-typed snake_case parameter body and returns the historical Tauri
//!   camelCase JSON verbatim, so the frontend contract does not change. It is a
//!   desktop compatibility adapter, not a plugin API.
//!
//! Requests use the canonical snake_case HTTP DTOs from
//! [`crate::workflow::react::application`]. Responses from the original routes
//! are converted back to the historical Tauri camelCase wire so existing
//! frontend callers keep the same JSON shape.

use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};

use chatspeed_runtime_client::{ClientError, RuntimeClient};

use crate::commands::workflow::{WorkflowContextFrameResult, WorkspaceFile};
use crate::db::{MainStore, Workflow, WorkflowEfficiencyReport, WorkflowMessage};
use crate::runtime_client::{RuntimeSupervisor, RuntimeUnavailable};
use crate::tools::ShellExecutionMode;
use crate::workflow::react::application::{WorkflowCreateRequest, WorkflowStartRequest};
use crate::workflow::react::dispatcher::DispatcherMetricsSnapshot;
use crate::workflow::react::events::WorkflowEventRecord;
use crate::workflow::react::skills::SkillManifest;

/// Canonical control-plane route for the workflow collection.
const WORKFLOWS_ROUTE: &str = "/control/v1/workflows";

/// Compatibility route for the allowlisted, typed workflow command dispatch.
const WORKFLOW_COMMANDS_ROUTE: &str = "/control/v1/workflow-commands";

/// Response keys whose values are opaque persisted payloads.
///
/// The control plane re-cases every response key to snake_case, including the
/// contents of `serde_json::Value` fields. These payloads already carry their
/// final shape (their inner keys are persisted snake_case), so their keys are
/// re-cased but their contents are copied verbatim instead of being recursed
/// into.
const OPAQUE_RESPONSE_KEYS: [&str; 3] = ["metadata", "execution_context", "event_data"];

/// Lists workflows through the runtime control plane.
pub async fn list_workflows(supervisor: &RuntimeSupervisor) -> Result<Vec<Workflow>, String> {
    let client = control_plane_client(supervisor).await?;
    let value = client
        .get(WORKFLOWS_ROUTE)
        .await
        .map_err(map_client_error)?;
    decode_response(value)
}

/// Creates a workflow session through the runtime control plane.
pub async fn create_workflow(
    supervisor: &RuntimeSupervisor,
    request: WorkflowCreateRequest,
) -> Result<String, String> {
    let client = control_plane_client(supervisor).await?;
    let body = serde_json::to_value(&request).map_err(|error| error.to_string())?;
    let response = client
        .post_with_idempotency(WORKFLOWS_ROUTE, &body, &new_idempotency_key())
        .await
        .map_err(map_client_error)?;
    response_string(&response, "session_id")
}

/// Reads the authoritative snapshot for one session.
pub async fn get_workflow_snapshot(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
) -> Result<Value, String> {
    let client = control_plane_client(supervisor).await?;
    let path = format!("{WORKFLOWS_ROUTE}/{}", encode_path_segment(session_id));
    let value = client.get(&path).await.map_err(map_client_error)?;
    Ok(restore_tauri_wire(value))
}

/// Starts (or resumes) a workflow session.
pub async fn workflow_start(
    supervisor: &RuntimeSupervisor,
    request: WorkflowStartRequest,
) -> Result<String, String> {
    let client = control_plane_client(supervisor).await?;
    let body = serde_json::to_value(&request).map_err(|error| error.to_string())?;
    let path = format!(
        "{WORKFLOWS_ROUTE}/{}/start",
        encode_path_segment(&request.session_id)
    );
    let response = client
        .post_with_idempotency(&path, &body, &new_idempotency_key())
        .await
        .map_err(map_client_error)?;
    response_string(&response, "session_id")
}

/// Submits a typed signal to a workflow session.
///
/// The control plane forwards the request body verbatim to the application
/// service. A non-JSON signal is rejected here instead of being wrapped in a
/// JSON string, which would silently change its meaning and mask caller bugs:
/// every canonical signal is a JSON object (CONSTITUTION.md §6.1).
pub async fn workflow_signal(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
    signal: String,
) -> Result<String, String> {
    let body = serde_json::from_str::<Value>(&signal)
        .map_err(|error| format!("Invalid signal JSON: {error}"))?;
    let client = control_plane_client(supervisor).await?;
    let path = format!(
        "{WORKFLOWS_ROUTE}/{}/signal",
        encode_path_segment(session_id)
    );
    let response = client
        .post_with_idempotency(&path, &body, &new_idempotency_key())
        .await
        .map_err(map_client_error)?;
    response_string(&response, "result")
}

/// Stops a workflow session (active, waiting or retrying).
pub async fn workflow_stop(supervisor: &RuntimeSupervisor, session_id: &str) -> Result<(), String> {
    let client = control_plane_client(supervisor).await?;
    let path = format!("{WORKFLOWS_ROUTE}/{}/stop", encode_path_segment(session_id));
    // The path parameter is authoritative; the control plane ignores the body.
    client
        .post_with_idempotency(&path, &Value::Object(Map::new()), &new_idempotency_key())
        .await
        .map_err(map_client_error)?;
    Ok(())
}

/// Reads the durable workflow events for one session.
///
/// This is the bounded durable-event query, not the live gateway stream. The
/// route applies a server-side default page bound, so the adapter pages through
/// it with the maximum page size until the whole durable list has been read:
/// the Tauri wire has always returned the full list and must keep doing so.
pub async fn get_workflow_events(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
) -> Result<Vec<WorkflowEventRecord>, String> {
    const PAGE_LIMIT: u32 = MainStore::WORKFLOW_EVENTS_MAX_LIMIT;
    let client = control_plane_client(supervisor).await?;
    let base = format!(
        "{WORKFLOWS_ROUTE}/{}/events",
        encode_path_segment(session_id)
    );
    let mut events: Vec<WorkflowEventRecord> = Vec::new();
    let mut after: Option<i64> = None;
    loop {
        let path = match after {
            Some(cursor) => format!("{base}?after={cursor}&limit={PAGE_LIMIT}"),
            None => format!("{base}?limit={PAGE_LIMIT}"),
        };
        let value = client.get(&path).await.map_err(map_client_error)?;
        let page: Vec<WorkflowEventRecord> = decode_event_page(value)?;
        let page_len = page.len() as u32;
        match page.last() {
            Some(last) => after = Some(last.id),
            None => break,
        }
        events.extend(page);
        if page_len < PAGE_LIMIT {
            break;
        }
    }
    Ok(events)
}

// ---------------------------------------------------------------------------
// Compatibility command route
// ---------------------------------------------------------------------------

/// Deletes a workflow session and its owned runtime resources.
pub async fn delete_workflow(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
) -> Result<(), String> {
    command_call(
        supervisor,
        "delete_workflow",
        json!({ "session_id": session_id }),
    )
    .await
    .map(|_| ())
}

/// Deletes the last workflow message.
pub async fn delete_last_workflow_message(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
) -> Result<bool, String> {
    let value = command_call(
        supervisor,
        "delete_last_workflow_message",
        json!({ "session_id": session_id }),
    )
    .await?;
    value
        .get("deleted")
        .and_then(Value::as_bool)
        .ok_or_else(|| "Runtime response is missing the `deleted` field".to_string())
}

/// Starts a manual clear-context frame (or no-ops when already cleared).
pub async fn workflow_begin_new_context_frame(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
) -> Result<WorkflowContextFrameResult, String> {
    let value = command_call(
        supervisor,
        "workflow_begin_new_context_frame",
        json!({ "session_id": session_id }),
    )
    .await?;
    decode_typed(value)
}

/// Lists pending sub-agent approvals across the owner's child workflows.
pub async fn list_pending_sub_agent_approvals(
    supervisor: &RuntimeSupervisor,
) -> Result<Vec<Value>, String> {
    let value = command_call(supervisor, "list_pending_sub_agent_approvals", json!({})).await?;
    decode_typed(value)
}

/// Loads an older page of workflow messages.
pub async fn get_earlier_workflow_message_page(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
    before_message_id: &str,
) -> Result<Value, String> {
    command_call(
        supervisor,
        "get_earlier_workflow_message_page",
        json!({ "session_id": session_id, "before_message_id": before_message_id }),
    )
    .await
}

/// Loads an older completed-task window of workflow messages.
pub async fn get_earlier_workflow_messages(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
    before_message_id: &str,
) -> Result<Value, String> {
    command_call(
        supervisor,
        "get_earlier_workflow_messages",
        json!({ "session_id": session_id, "before_message_id": before_message_id }),
    )
    .await
}

/// Reads the persisted workflow agent config.
pub async fn get_workflow_agent_config(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
) -> Result<Value, String> {
    command_call(
        supervisor,
        "get_workflow_agent_config",
        json!({ "session_id": session_id }),
    )
    .await
}

/// Appends a workflow message and returns its id.
pub async fn add_workflow_message(
    supervisor: &RuntimeSupervisor,
    message: &WorkflowMessage,
) -> Result<i64, String> {
    let message = serde_json::to_value(message).map_err(|error| error.to_string())?;
    let value = command_call(
        supervisor,
        "add_workflow_message",
        json!({ "message": message }),
    )
    .await?;
    value
        .get("id")
        .and_then(Value::as_i64)
        .ok_or_else(|| "Runtime response is missing the `id` field".to_string())
}

/// Updates a workflow title.
pub async fn update_workflow_title(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
    title: String,
) -> Result<(), String> {
    command_call(
        supervisor,
        "update_workflow_title",
        json!({ "session_id": session_id, "title": title }),
    )
    .await
    .map(|_| ())
}

/// Updates a workflow title and user query together.
pub async fn update_workflow_title_and_query(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
    title: String,
    user_query: String,
) -> Result<(), String> {
    command_call(
        supervisor,
        "update_workflow_title_and_query",
        json!({ "session_id": session_id, "title": title, "user_query": user_query }),
    )
    .await
    .map(|_| ())
}

/// Updates a workflow user query.
pub async fn update_workflow_query(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
    user_query: String,
) -> Result<(), String> {
    command_call(
        supervisor,
        "update_workflow_query",
        json!({ "session_id": session_id, "user_query": user_query }),
    )
    .await
    .map(|_| ())
}

/// Updates a workflow status.
pub async fn update_workflow_status(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
    status: String,
) -> Result<(), String> {
    command_call(
        supervisor,
        "update_workflow_status",
        json!({ "session_id": session_id, "status": status }),
    )
    .await
    .map(|_| ())
}

/// Reads the persisted todo list for a workflow.
pub async fn workflow_get_tasks(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
) -> Result<Vec<Value>, String> {
    let value = command_call(
        supervisor,
        "workflow_get_tasks",
        json!({ "session_id": session_id }),
    )
    .await?;
    decode_typed(value)
}

/// Persists a workflow todo list.
pub async fn update_workflow_todo_list(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
    todo_list: String,
) -> Result<(), String> {
    command_call(
        supervisor,
        "update_workflow_todo_list",
        json!({ "session_id": session_id, "todo_list": todo_list }),
    )
    .await
    .map(|_| ())
}

/// Updates the allowed paths of a workflow's effective config.
pub async fn update_workflow_allowed_paths(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
    allowed_paths: Value,
) -> Result<(), String> {
    command_call(
        supervisor,
        "update_workflow_allowed_paths",
        json!({ "session_id": session_id, "allowed_paths": allowed_paths }),
    )
    .await
    .map(|_| ())
}

/// Reads the per-session proxy key.
///
/// Only a loopback bearer client of the owning runtime can reach this route.
/// The value is a credential, so it is never logged or embedded in an error.
pub async fn get_workflow_session_key(
    supervisor: &RuntimeSupervisor,
    workflow_id: &str,
) -> Result<String, String> {
    let value = command_call(
        supervisor,
        "get_workflow_session_key",
        json!({ "workflow_id": workflow_id }),
    )
    .await?;
    response_string(&value, "session_key")
}

/// Updates the final-audit flag.
pub async fn update_workflow_final_audit(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
    final_audit: bool,
) -> Result<(), String> {
    command_call(
        supervisor,
        "update_workflow_final_audit",
        json!({ "session_id": session_id, "final_audit": final_audit }),
    )
    .await
    .map(|_| ())
}

/// Updates the auto-compress flag.
pub async fn update_workflow_auto_compress(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
    auto_compress: bool,
) -> Result<(), String> {
    command_call(
        supervisor,
        "update_workflow_auto_compress",
        json!({ "session_id": session_id, "auto_compress": auto_compress }),
    )
    .await
    .map(|_| ())
}

/// Updates the execution-style (personality) selection.
pub async fn update_workflow_personality(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
    personality: String,
) -> Result<(), String> {
    command_call(
        supervisor,
        "update_workflow_personality",
        json!({ "session_id": session_id, "personality": personality }),
    )
    .await
    .map(|_| ())
}

/// Updates the workflow model config.
pub async fn update_workflow_model_config(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
    configs: Value,
) -> Result<(), String> {
    command_call(
        supervisor,
        "update_workflow_model_config",
        json!({ "session_id": session_id, "configs": configs }),
    )
    .await
    .map(|_| ())
}

/// Updates the workflow skill config.
pub async fn update_workflow_skills_config(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
    skill_enabled: bool,
    selected_skills: Vec<String>,
) -> Result<(), String> {
    command_call(
        supervisor,
        "update_workflow_skills_config",
        json!({
            "session_id": session_id,
            "skill_enabled": skill_enabled,
            "selected_skills": selected_skills
        }),
    )
    .await
    .map(|_| ())
}

/// Updates the workflow approval level.
pub async fn update_workflow_approval_level(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
    approval_level: String,
) -> Result<(), String> {
    command_call(
        supervisor,
        "update_workflow_approval_level",
        json!({ "session_id": session_id, "approval_level": approval_level }),
    )
    .await
    .map(|_| ())
}

/// Updates the execution phase.
pub async fn update_workflow_phase(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
    phase: String,
) -> Result<(), String> {
    command_call(
        supervisor,
        "update_workflow_phase",
        json!({ "session_id": session_id, "phase": phase }),
    )
    .await
    .map(|_| ())
}

/// Updates the workflow sandbox override.
pub async fn update_workflow_sandbox_config(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
    execution_mode: ShellExecutionMode,
    sandbox_scheme_id: Option<String>,
) -> Result<(), String> {
    let execution_mode = serde_json::to_value(execution_mode).map_err(|error| error.to_string())?;
    command_call(
        supervisor,
        "update_workflow_sandbox_config",
        json!({
            "session_id": session_id,
            "execution_mode": execution_mode,
            "sandbox_scheme_id": sandbox_scheme_id
        }),
    )
    .await
    .map(|_| ())
}

/// Replaces the workflow agent config.
pub async fn update_workflow_agent_config(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
    agent_config: String,
) -> Result<(), String> {
    command_call(
        supervisor,
        "update_workflow_agent_config",
        json!({ "session_id": session_id, "agent_config": agent_config }),
    )
    .await
    .map(|_| ())
}

/// Rebinds a not-yet-started workflow to another primary agent.
pub async fn update_workflow_agent_id(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
    agent_id: String,
) -> Result<String, String> {
    let value = command_call(
        supervisor,
        "update_workflow_agent_id",
        json!({ "session_id": session_id, "agent_id": agent_id }),
    )
    .await?;
    response_string(&value, "agent_config")
}

/// Lists the workflow's effective auto-approved tools.
pub async fn get_auto_approved_tools(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
) -> Result<Vec<String>, String> {
    let value = command_call(
        supervisor,
        "get_auto_approved_tools",
        json!({ "session_id": session_id }),
    )
    .await?;
    decode_typed(value)
}

/// Removes one tool from the auto-approved set.
pub async fn remove_auto_approved_tool(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
    tool_name: String,
) -> Result<(), String> {
    command_call(
        supervisor,
        "remove_auto_approved_tool",
        json!({ "session_id": session_id, "tool_name": tool_name }),
    )
    .await
    .map(|_| ())
}

/// Removes one entry from the shell policy.
pub async fn remove_shell_policy_item(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
    pattern: String,
) -> Result<(), String> {
    command_call(
        supervisor,
        "remove_shell_policy_item",
        json!({ "session_id": session_id, "pattern": pattern }),
    )
    .await
    .map(|_| ())
}

/// Approves the pending plan through the canonical signal path.
pub async fn workflow_approve_plan(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
    agent_id: String,
    plan: String,
) -> Result<(), String> {
    command_call(
        supervisor,
        "workflow_approve_plan",
        json!({ "session_id": session_id, "agent_id": agent_id, "plan": plan }),
    )
    .await
    .map(|_| ())
}

/// Reads the in-memory dispatcher metrics for one session.
pub async fn get_workflow_dispatcher_metrics(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
) -> Result<DispatcherMetricsSnapshot, String> {
    let value = command_call(
        supervisor,
        "get_workflow_dispatcher_metrics",
        json!({ "session_id": session_id }),
    )
    .await?;
    decode_typed(value)
}

/// Reads the workflow efficiency report.
pub async fn get_workflow_efficiency_report(
    supervisor: &RuntimeSupervisor,
    session_id: &str,
) -> Result<WorkflowEfficiencyReport, String> {
    let value = command_call(
        supervisor,
        "get_workflow_efficiency_report",
        json!({ "session_id": session_id }),
    )
    .await?;
    decode_typed(value)
}

/// Searches the allowed workspace roots for files matching a query.
pub async fn search_workspace_files(
    supervisor: &RuntimeSupervisor,
    paths: Vec<String>,
    query: String,
) -> Result<Vec<WorkspaceFile>, String> {
    let value = command_call(
        supervisor,
        "search_workspace_files",
        json!({ "paths": paths, "query": query }),
    )
    .await?;
    decode_typed(value)
}

/// Lists the runtime's installed system skills.
pub async fn get_system_skills(
    supervisor: &RuntimeSupervisor,
) -> Result<Vec<SkillManifest>, String> {
    let value = command_call(supervisor, "get_system_skills", json!({})).await?;
    decode_typed(value)
}

/// Posts one allowlisted workflow command and returns the camelCase response.
async fn command_call(
    supervisor: &RuntimeSupervisor,
    command: &str,
    params: Value,
) -> Result<Value, String> {
    let client = control_plane_client(supervisor).await?;
    let path = format!("{WORKFLOW_COMMANDS_ROUTE}/{}", encode_path_segment(command));
    client
        .post_with_idempotency(&path, &params, &new_idempotency_key())
        .await
        .map_err(map_client_error)
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Resolves the connected control-plane client, or fails when no lease is held.
async fn control_plane_client(supervisor: &RuntimeSupervisor) -> Result<RuntimeClient, String> {
    supervisor.client().await.map_err(map_runtime_error)
}

/// Fresh idempotency key for one command invocation.
fn new_idempotency_key() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Percent-encodes one path segment so a caller-controlled id can never inject
/// extra path components, a query or a fragment into the control-plane URL.
fn encode_path_segment(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                output.push(byte as char)
            }
            _ => output.push_str(&format!("%{byte:02X}")),
        }
    }
    output
}

/// Decodes a snake_case runtime response into a Tauri-wire type.
fn decode_response<T: DeserializeOwned>(value: Value) -> Result<T, String> {
    let restored = restore_tauri_wire(value);
    serde_json::from_value(restored)
        .map_err(|error| format!("Unexpected runtime response: {error}"))
}

/// Decodes a camelCase-preserving compatibility response into a Tauri type.
fn decode_typed<T: DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|error| format!("Unexpected runtime response: {error}"))
}

/// Decodes a durable event page from its canonical snake_case HTTP wire.
///
/// The durable route snake-cases every key, so a generic camelCase round-trip
/// would corrupt the typed record (its `event_data` payload is opaque). The
/// explicit wire struct decodes the canonical shape once and maps it to the
/// Tauri `WorkflowEventRecord`, leaving `event_data` untouched.
fn decode_event_page(value: Value) -> Result<Vec<WorkflowEventRecord>, String> {
    let page: Vec<WorkflowEventRecordWire> = serde_json::from_value(value)
        .map_err(|error| format!("Unexpected runtime event response: {error}"))?;
    Ok(page.into_iter().map(WorkflowEventRecord::from).collect())
}

/// Canonical snake_case wire shape of one durable workflow event.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
struct WorkflowEventRecordWire {
    id: i64,
    session_id: String,
    event_type: String,
    event_version: String,
    #[serde(default)]
    event_data: Value,
    created_at: String,
}

impl From<WorkflowEventRecordWire> for WorkflowEventRecord {
    fn from(wire: WorkflowEventRecordWire) -> Self {
        Self {
            id: wire.id,
            session_id: wire.session_id,
            event_type: wire.event_type,
            event_version: wire.event_version,
            event_data: wire.event_data,
            created_at: wire.created_at,
        }
    }
}

/// Reads a required string field from a runtime response object.
fn response_string(response: &Value, field: &str) -> Result<String, String> {
    response
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("Runtime response is missing the `{field}` field"))
}

/// Maps a transport/domain client error to the Tauri string error.
///
/// A structured runtime error keeps its bare message so the Tauri wire stays as
/// close as possible to the previous `ApplicationError::message` string; every
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

/// Converts a snake_case runtime response back to the Tauri camelCase wire.
///
/// The control plane serializes every response with
/// `dto::to_snake_case_keys`; the Tauri commands historically emit camelCase.
/// Structural keys are re-cased here while values under
/// [`OPAQUE_RESPONSE_KEYS`] are copied verbatim.
fn restore_tauri_wire(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut converted = Map::with_capacity(map.len());
            for (key, val) in map {
                let camel = snake_to_camel(&key);
                if OPAQUE_RESPONSE_KEYS.contains(&key.as_str()) {
                    converted.insert(camel, val);
                } else {
                    converted.insert(camel, restore_tauri_wire(val));
                }
            }
            Value::Object(converted)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(restore_tauri_wire).collect()),
        other => other,
    }
}

/// Converts one snake_case key to camelCase.
fn snake_to_camel(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut uppercase_next = false;
    for ch in input.chars() {
        if ch == '_' {
            uppercase_next = true;
        } else if uppercase_next {
            output.extend(ch.to_uppercase());
            uppercase_next = false;
        } else {
            output.push(ch);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn snake_to_camel_converts_identifiers() {
        assert_eq!(snake_to_camel("session_id"), "sessionId");
        assert_eq!(snake_to_camel("has_live_session"), "hasLiveSession");
        assert_eq!(snake_to_camel("id"), "id");
    }

    #[test]
    fn encode_path_segment_escapes_separators_and_queries() {
        assert_eq!(encode_path_segment("wf-123_ab"), "wf-123_ab");
        assert_eq!(encode_path_segment("a/b"), "a%2Fb");
        assert_eq!(encode_path_segment("../x"), "..%2Fx");
        assert_eq!(encode_path_segment("a?b=c"), "a%3Fb%3Dc");
        assert_eq!(encode_path_segment("a b"), "a%20b");
    }

    #[test]
    fn decode_event_page_maps_the_real_snake_case_wire() {
        // Exactly what `GET /control/v1/workflows/{id}/events` returns: a
        // snake_case array whose nested persisted payload stays snake_case.
        let wire = json!([
            {
                "id": 7,
                "session_id": "s1",
                "event_type": "workflow_started",
                "event_version": "1",
                "event_data": { "agent_id": "a1", "pending_tools": [] },
                "created_at": "2026-01-01T00:00:00Z"
            }
        ]);

        let records = decode_event_page(wire).expect("decode real event page");

        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.id, 7);
        assert_eq!(record.session_id, "s1");
        assert_eq!(record.event_type, "workflow_started");
        assert_eq!(record.event_version, "1");
        assert_eq!(record.created_at, "2026-01-01T00:00:00Z");
        // The persisted payload keeps its canonical snake_case keys verbatim.
        assert_eq!(record.event_data["agent_id"], json!("a1"));
        assert_eq!(record.event_data["pending_tools"], json!([]));
    }

    #[test]
    fn decode_event_page_rejects_a_camelcase_wire() {
        // A camelCase document must not be silently accepted: the durable route
        // is snake_case, so a mismatch is a real protocol error.
        let wire = json!([
            { "id": 1, "sessionId": "s1", "eventType": "x", "eventVersion": "1", "createdAt": "t" }
        ]);
        assert!(decode_event_page(wire).is_err());
    }

    #[test]
    fn restore_tauri_wire_camelizes_structure_and_preserves_opaque_payloads() {
        let response = json!({
            "message_window_before_id": "12",
            "has_live_session": true,
            "workflow": { "agent_id": "a1", "user_query": "hi" },
            "execution_context": { "session_id": "s1", "pending_tools": [] },
            "messages": [
                { "message_kind": "tool", "is_error": false, "metadata": { "tool_call_id": "t1" } }
            ]
        });

        let restored = restore_tauri_wire(response);

        assert_eq!(restored["messageWindowBeforeId"], json!("12"));
        assert_eq!(restored["hasLiveSession"], json!(true));
        assert_eq!(restored["workflow"]["agentId"], json!("a1"));
        assert_eq!(restored["workflow"]["userQuery"], json!("hi"));
        // The recovered execution context keeps its persisted snake_case shape.
        assert_eq!(restored["executionContext"]["session_id"], json!("s1"));
        assert_eq!(restored["executionContext"]["pending_tools"], json!([]));
        assert_eq!(restored["messages"][0]["messageKind"], json!("tool"));
        assert_eq!(restored["messages"][0]["isError"], json!(false));
        // Message metadata keeps its persisted snake_case keys verbatim.
        assert_eq!(
            restored["messages"][0]["metadata"]["tool_call_id"],
            json!("t1")
        );
    }
}
