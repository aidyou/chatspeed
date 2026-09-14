use crate::db::{MainStore, WorkflowAutomation, WorkflowAutomationRun};
use crate::libs::tsid::TsidGenerator;
use crate::workflow::automation::service::{
    run_automation_now, save_automation, set_automation_enabled,
};
use crate::workflow::automation::types::{
    WorkflowAutomationRequest, WorkflowAutomationRunNowResult,
};
use crate::workflow::react::application::WorkflowApplicationService;
use std::sync::Arc;
use tauri::State;

#[tauri::command]
pub async fn workflow_automation_list(
    state: State<'_, Arc<MainStore>>,
) -> Result<Vec<WorkflowAutomation>, String> {
    let store = &*state;
    store.list_workflow_automations().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workflow_automation_get(
    state: State<'_, Arc<MainStore>>,
    id: String,
) -> Result<Option<WorkflowAutomation>, String> {
    let store = &*state;
    store
        .get_workflow_automation(&id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workflow_automation_save(
    tsid_generator: State<'_, Arc<TsidGenerator>>,
    state: State<'_, Arc<MainStore>>,
    request: WorkflowAutomationRequest,
) -> Result<WorkflowAutomation, String> {
    let store = &*state;
    save_automation(tsid_generator.inner(), &store, request)
}

#[tauri::command]
pub async fn workflow_automation_delete(
    state: State<'_, Arc<MainStore>>,
    id: String,
) -> Result<(), String> {
    let store = &*state;
    store
        .delete_workflow_automation(&id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workflow_automation_set_enabled(
    state: State<'_, Arc<MainStore>>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    let store = &*state;
    set_automation_enabled(&store, &id, enabled)
}

#[tauri::command]
pub async fn workflow_automation_list_runs(
    state: State<'_, Arc<MainStore>>,
    automation_id: String,
) -> Result<Vec<WorkflowAutomationRun>, String> {
    let store = &*state;
    store
        .list_workflow_automation_runs(&automation_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workflow_automation_run_now(
    svc: State<'_, Arc<WorkflowApplicationService>>,
    automation_id: String,
) -> Result<WorkflowAutomationRunNowResult, String> {
    run_automation_now(svc, automation_id).await
}
