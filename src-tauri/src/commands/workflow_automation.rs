use crate::ai::interaction::chat_completion::ChatState;
use crate::db::{MainStore, WorkflowAutomation, WorkflowAutomationRun};
use crate::libs::tsid::TsidGenerator;
use crate::workflow::automation::service::{
    run_automation_now, save_automation, set_automation_enabled,
};
use crate::workflow::automation::types::{
    WorkflowAutomationRequest, WorkflowAutomationRunNowResult,
};
use crate::workflow::react::gateway::TauriGateway;
use crate::workflow::react::manager::WorkflowManager;
use crate::workflow::react::orchestrator::SubAgentFactory;
use std::sync::Arc;
use tauri::{AppHandle, State};

#[tauri::command]
pub async fn workflow_automation_list(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
) -> Result<Vec<WorkflowAutomation>, String> {
    let store = state.read().map_err(|e| e.to_string())?;
    store.list_workflow_automations().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workflow_automation_get(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    id: String,
) -> Result<Option<WorkflowAutomation>, String> {
    let store = state.read().map_err(|e| e.to_string())?;
    store
        .get_workflow_automation(&id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workflow_automation_save(
    tsid_generator: State<'_, Arc<TsidGenerator>>,
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    request: WorkflowAutomationRequest,
) -> Result<WorkflowAutomation, String> {
    let store = state.read().map_err(|e| e.to_string())?;
    save_automation(tsid_generator.inner(), &store, request)
}

#[tauri::command]
pub async fn workflow_automation_delete(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    id: String,
) -> Result<(), String> {
    let store = state.read().map_err(|e| e.to_string())?;
    store
        .delete_workflow_automation(&id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workflow_automation_set_enabled(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    let store = state.read().map_err(|e| e.to_string())?;
    set_automation_enabled(&store, &id, enabled)
}

#[tauri::command]
pub async fn workflow_automation_list_runs(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    automation_id: String,
) -> Result<Vec<WorkflowAutomationRun>, String> {
    let store = state.read().map_err(|e| e.to_string())?;
    store
        .list_workflow_automation_runs(&automation_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workflow_automation_run_now(
    app: AppHandle,
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    tsid_generator: State<'_, Arc<TsidGenerator>>,
    gateway: State<'_, Arc<TauriGateway>>,
    factory: State<'_, Arc<dyn SubAgentFactory>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    automation_id: String,
) -> Result<WorkflowAutomationRunNowResult, String> {
    run_automation_now(
        app,
        state,
        chat_state,
        tsid_generator,
        gateway,
        factory,
        workflow_manager,
        automation_id,
    )
    .await
}
