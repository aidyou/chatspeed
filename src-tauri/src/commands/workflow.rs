//! Desktop Tauri wire for the workflow commands.
//!
//! The transport-neutral command cores and the shared workflow wire types live
//! in the runtime backend (`chatspeed_runtime_backend::commands::workflow`) and
//! are re-exported here. Each `#[tauri::command]` wrapper below translates the
//! Tauri wire into one `crate::runtime_workflow` call, which reaches the runtime
//! control plane, so the desktop runs no workflow logic of its own.

pub use chatspeed_runtime_backend::commands::workflow::*;

use std::sync::Arc;

use serde_json::Value;
use tauri::State;

use crate::db::{Workflow, WorkflowEfficiencyReport, WorkflowMessage};
use crate::workflow::react::application::WorkflowCreateRequest;
use crate::workflow::react::dispatcher::DispatcherMetricsSnapshot;
use crate::workflow::react::skills::SkillManifest;

#[tauri::command]
pub async fn create_workflow(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    request: CreateWorkflowRequest,
) -> Result<String, String> {
    crate::runtime_workflow::create_workflow(
        supervisor.inner().as_ref(),
        WorkflowCreateRequest {
            user_query: request.user_query,
            agent_id: request.agent_id,
            allowed_paths: request.allowed_paths,
            auto_approve_plan: request.auto_approve_plan,
            final_audit: request.final_audit,
            inherited_agent_config: request.inherited_agent_config,
        },
    )
    .await
}
#[tauri::command]
pub async fn list_workflows(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
) -> Result<Vec<Workflow>, String> {
    crate::runtime_workflow::list_workflows(supervisor.inner().as_ref()).await
}
#[tauri::command]
pub async fn delete_workflow(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
) -> Result<(), String> {
    crate::runtime_workflow::delete_workflow(supervisor.inner().as_ref(), &session_id).await
}
#[tauri::command]
pub async fn delete_last_workflow_message(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
) -> Result<bool, String> {
    crate::runtime_workflow::delete_last_workflow_message(supervisor.inner().as_ref(), &session_id)
        .await
}
#[tauri::command]
pub async fn workflow_begin_new_context_frame(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
) -> Result<WorkflowContextFrameResult, String> {
    crate::runtime_workflow::workflow_begin_new_context_frame(
        supervisor.inner().as_ref(),
        &session_id,
    )
    .await
}
#[tauri::command]
pub async fn list_pending_sub_agent_approvals(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
) -> Result<Vec<Value>, String> {
    crate::runtime_workflow::list_pending_sub_agent_approvals(supervisor.inner().as_ref()).await
}
#[tauri::command]
pub async fn get_workflow_snapshot(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
) -> Result<Value, String> {
    crate::runtime_workflow::get_workflow_snapshot(supervisor.inner().as_ref(), &session_id).await
}
#[tauri::command]
pub async fn get_earlier_workflow_message_page(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
    before_message_id: String,
) -> Result<Value, String> {
    crate::runtime_workflow::get_earlier_workflow_message_page(
        supervisor.inner().as_ref(),
        &session_id,
        &before_message_id,
    )
    .await
}
#[tauri::command]
pub async fn get_earlier_workflow_messages(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
    before_message_id: String,
) -> Result<Value, String> {
    crate::runtime_workflow::get_earlier_workflow_messages(
        supervisor.inner().as_ref(),
        &session_id,
        &before_message_id,
    )
    .await
}
#[tauri::command]
pub async fn get_workflow_agent_config(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
) -> Result<Value, String> {
    crate::runtime_workflow::get_workflow_agent_config(supervisor.inner().as_ref(), &session_id)
        .await
}
#[tauri::command]
pub async fn add_workflow_message(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    message: WorkflowMessage,
) -> Result<i64, String> {
    crate::runtime_workflow::add_workflow_message(supervisor.inner().as_ref(), &message).await
}
#[tauri::command]
pub async fn update_workflow_title(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
    title: String,
) -> Result<(), String> {
    crate::runtime_workflow::update_workflow_title(supervisor.inner().as_ref(), &session_id, title)
        .await
}
#[tauri::command]
pub async fn update_workflow_title_and_query(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
    title: String,
    user_query: String,
) -> Result<(), String> {
    crate::runtime_workflow::update_workflow_title_and_query(
        supervisor.inner().as_ref(),
        &session_id,
        title,
        user_query,
    )
    .await
}
#[tauri::command]
pub async fn update_workflow_query(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
    user_query: String,
) -> Result<(), String> {
    crate::runtime_workflow::update_workflow_query(
        supervisor.inner().as_ref(),
        &session_id,
        user_query,
    )
    .await
}
#[tauri::command]
pub async fn update_workflow_status(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
    status: String,
) -> Result<(), String> {
    crate::runtime_workflow::update_workflow_status(
        supervisor.inner().as_ref(),
        &session_id,
        status,
    )
    .await
}
#[tauri::command]
pub async fn workflow_start(
    app: tauri::AppHandle,
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
    agent_id: String,
    initial_prompt: Option<String>,
    initial_metadata: Option<Value>,
    initial_attached_context: Option<String>,
    planning_mode: Option<bool>,
) -> Result<String, String> {
    // Subscribe to the runtime's live workflow stream before the workflow can
    // publish anything. The desktop owns no runtime state, so this forwarding
    // task is the only path from the runtime SSE broker to the webview; if it
    // cannot be established the start is refused rather than running a workflow
    // no UI can observe.
    let supervisor = supervisor.inner().as_ref();
    supervisor
        .ensure_workflow_event_stream(app, &session_id)
        .await
        .map_err(|error| error.to_string())?;
    crate::runtime_workflow::workflow_start(
        supervisor,
        crate::workflow::react::application::WorkflowStartRequest {
            session_id,
            agent_id,
            initial_prompt,
            initial_metadata,
            initial_attached_context,
            planning_mode,
        },
    )
    .await
}
#[tauri::command]
pub async fn workflow_approve_plan(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
    agent_id: String,
    plan: String,
) -> Result<(), String> {
    crate::runtime_workflow::workflow_approve_plan(
        supervisor.inner().as_ref(),
        &session_id,
        agent_id,
        plan,
    )
    .await
}
#[tauri::command]
pub async fn workflow_subscribe(
    app: tauri::AppHandle,
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
) -> Result<(), String> {
    supervisor
        .inner()
        .as_ref()
        .ensure_workflow_event_stream(app, &session_id)
        .await
        .map_err(|error| error.to_string())
}
#[tauri::command]
pub async fn workflow_signal(
    app: tauri::AppHandle,
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
    signal: String,
) -> Result<String, String> {
    let supervisor = supervisor.inner().as_ref();
    // Signals can resume a waiting or recently completed session after the
    // previous reader has reached a terminal state. Establish the transport
    // before forwarding the signal so resumed execution remains observable.
    supervisor
        .ensure_workflow_event_stream(app, &session_id)
        .await
        .map_err(|error| error.to_string())?;
    crate::runtime_workflow::workflow_signal(supervisor, &session_id, signal).await
}
#[tauri::command]
pub async fn workflow_stop(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
) -> Result<(), String> {
    crate::runtime_workflow::workflow_stop(supervisor.inner().as_ref(), &session_id).await
}
#[tauri::command]
pub async fn workflow_get_tasks(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
) -> Result<Vec<Value>, String> {
    crate::runtime_workflow::workflow_get_tasks(supervisor.inner().as_ref(), &session_id).await
}
#[tauri::command]
pub async fn update_workflow_todo_list(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
    todo_list: String,
) -> Result<(), String> {
    crate::runtime_workflow::update_workflow_todo_list(
        supervisor.inner().as_ref(),
        &session_id,
        todo_list,
    )
    .await
}
#[tauri::command]
pub async fn search_workspace_files(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    paths: Vec<String>,
    query: String,
) -> Result<Vec<WorkspaceFile>, String> {
    crate::runtime_workflow::search_workspace_files(supervisor.inner().as_ref(), paths, query).await
}
#[tauri::command]
pub async fn get_system_skills(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
) -> Result<Vec<SkillManifest>, String> {
    crate::runtime_workflow::get_system_skills(supervisor.inner().as_ref()).await
}
#[tauri::command]
pub async fn update_workflow_allowed_paths(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
    allowed_paths: Value,
) -> Result<(), String> {
    crate::runtime_workflow::update_workflow_allowed_paths(
        supervisor.inner().as_ref(),
        &session_id,
        allowed_paths,
    )
    .await
}
#[tauri::command]
pub async fn get_workflow_session_key(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    workflow_id: String,
) -> Result<String, String> {
    crate::runtime_workflow::get_workflow_session_key(supervisor.inner().as_ref(), &workflow_id)
        .await
}
#[tauri::command]
pub async fn update_workflow_final_audit(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
    final_audit: bool,
) -> Result<(), String> {
    crate::runtime_workflow::update_workflow_final_audit(
        supervisor.inner().as_ref(),
        &session_id,
        final_audit,
    )
    .await
}
#[tauri::command]
pub async fn update_workflow_auto_compress(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
    auto_compress: bool,
) -> Result<(), String> {
    crate::runtime_workflow::update_workflow_auto_compress(
        supervisor.inner().as_ref(),
        &session_id,
        auto_compress,
    )
    .await
}
#[tauri::command]
pub async fn update_workflow_personality(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
    personality: String,
) -> Result<(), String> {
    crate::runtime_workflow::update_workflow_personality(
        supervisor.inner().as_ref(),
        &session_id,
        personality,
    )
    .await
}
#[tauri::command]
pub async fn update_workflow_model_config(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
    configs: Value,
) -> Result<(), String> {
    crate::runtime_workflow::update_workflow_model_config(
        supervisor.inner().as_ref(),
        &session_id,
        configs,
    )
    .await
}
#[tauri::command]
pub async fn update_workflow_skills_config(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
    skill_enabled: bool,
    selected_skills: Vec<String>,
) -> Result<(), String> {
    crate::runtime_workflow::update_workflow_skills_config(
        supervisor.inner().as_ref(),
        &session_id,
        skill_enabled,
        selected_skills,
    )
    .await
}
#[tauri::command]
pub async fn update_workflow_approval_level(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
    approval_level: String,
) -> Result<(), String> {
    crate::runtime_workflow::update_workflow_approval_level(
        supervisor.inner().as_ref(),
        &session_id,
        approval_level,
    )
    .await
}
#[tauri::command]
pub async fn update_workflow_phase(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
    phase: String,
) -> Result<(), String> {
    crate::runtime_workflow::update_workflow_phase(supervisor.inner().as_ref(), &session_id, phase)
        .await
}
#[tauri::command]
pub async fn update_workflow_sandbox_config(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
    execution_mode: crate::tools::ShellExecutionMode,
    sandbox_scheme_id: Option<String>,
) -> Result<(), String> {
    crate::runtime_workflow::update_workflow_sandbox_config(
        supervisor.inner().as_ref(),
        &session_id,
        execution_mode,
        sandbox_scheme_id,
    )
    .await
}
#[tauri::command]
pub async fn update_workflow_agent_config(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
    agent_config: String,
) -> Result<(), String> {
    crate::runtime_workflow::update_workflow_agent_config(
        supervisor.inner().as_ref(),
        &session_id,
        agent_config,
    )
    .await
}
#[tauri::command]
pub async fn update_workflow_agent_id(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
    agent_id: String,
) -> Result<String, String> {
    crate::runtime_workflow::update_workflow_agent_id(
        supervisor.inner().as_ref(),
        &session_id,
        agent_id,
    )
    .await
}
#[tauri::command]
pub async fn get_auto_approved_tools(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
) -> Result<Vec<String>, String> {
    crate::runtime_workflow::get_auto_approved_tools(supervisor.inner().as_ref(), &session_id).await
}
#[tauri::command]
pub async fn remove_auto_approved_tool(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
    tool_name: String,
) -> Result<(), String> {
    crate::runtime_workflow::remove_auto_approved_tool(
        supervisor.inner().as_ref(),
        &session_id,
        tool_name,
    )
    .await
}
#[tauri::command]
pub async fn remove_shell_policy_item(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
    pattern: String,
) -> Result<(), String> {
    crate::runtime_workflow::remove_shell_policy_item(
        supervisor.inner().as_ref(),
        &session_id,
        pattern,
    )
    .await
}
#[tauri::command]
pub async fn get_workflow_events(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
) -> Result<Vec<crate::workflow::react::events::WorkflowEventRecord>, String> {
    // The Tauri wire keeps returning the full durable event list.
    crate::runtime_workflow::get_workflow_events(supervisor.inner().as_ref(), &session_id).await
}
#[tauri::command]
pub async fn get_workflow_dispatcher_metrics(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
) -> Result<DispatcherMetricsSnapshot, String> {
    crate::runtime_workflow::get_workflow_dispatcher_metrics(
        supervisor.inner().as_ref(),
        &session_id,
    )
    .await
}
#[tauri::command]
pub async fn get_workflow_efficiency_report(
    supervisor: State<'_, Arc<crate::runtime_client::RuntimeSupervisor>>,
    session_id: String,
) -> Result<WorkflowEfficiencyReport, String> {
    crate::runtime_workflow::get_workflow_efficiency_report(
        supervisor.inner().as_ref(),
        &session_id,
    )
    .await
}
