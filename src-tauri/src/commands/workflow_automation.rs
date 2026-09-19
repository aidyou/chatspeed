//! Tauri adapters for local workflow automation.
//!
//! These are thin wire-conversion shims. Every read and mutation resolves to the
//! single `AutomationApplicationService` facade (reached through the workflow
//! runtime owner), so the desktop editor can never diverge from the control plane
//! or the scheduler about validation, errors, revision or status (AC-1/INV-2).
//! Command names and the historical camelCase return shapes are preserved; the
//! only additive change is `delete`'s optional `confirm` flag (AC-10).

use crate::db::{WorkflowAutomation, WorkflowAutomationRun};
use crate::workflow::automation::types::{
    AutomationApplyRequest, AutomationDraftInput, AutomationMutationResult, AutomationPlanV1,
    AutomationRunView, WorkflowAutomationRequest, WorkflowAutomationRunNowResult,
    AUTOMATION_ACTOR_SCOPE_DESKTOP,
};
use crate::workflow::react::application::WorkflowApplicationService;
use std::sync::Arc;
use tauri::State;

#[tauri::command]
pub async fn workflow_automation_list(
    svc: State<'_, Arc<WorkflowApplicationService>>,
) -> Result<Vec<WorkflowAutomation>, String> {
    svc.automation()
        .list_rows()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workflow_automation_get(
    svc: State<'_, Arc<WorkflowApplicationService>>,
    id: String,
) -> Result<Option<WorkflowAutomation>, String> {
    svc.automation().get_row(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workflow_automation_save(
    svc: State<'_, Arc<WorkflowApplicationService>>,
    request: WorkflowAutomationRequest,
) -> Result<WorkflowAutomation, String> {
    svc.automation()
        .compat_save(&request)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workflow_automation_delete(
    svc: State<'_, Arc<WorkflowApplicationService>>,
    id: String,
    confirm: Option<bool>,
) -> Result<(), String> {
    svc.automation()
        .delete(
            &id,
            confirm.unwrap_or(false),
            AUTOMATION_ACTOR_SCOPE_DESKTOP,
            None,
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workflow_automation_set_enabled(
    svc: State<'_, Arc<WorkflowApplicationService>>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    svc.automation()
        .set_enabled(&id, enabled, None, AUTOMATION_ACTOR_SCOPE_DESKTOP, None)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workflow_automation_list_runs(
    svc: State<'_, Arc<WorkflowApplicationService>>,
    automation_id: String,
) -> Result<Vec<WorkflowAutomationRun>, String> {
    svc.automation()
        .run_rows(&automation_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workflow_automation_run_now(
    svc: State<'_, Arc<WorkflowApplicationService>>,
    automation_id: String,
) -> Result<WorkflowAutomationRunNowResult, String> {
    // The compatibility command performs the run only through the runtime
    // owner's typed facade: `automation_run_compat` delegates to
    // `automation_run`, which owns the single manual-run kernel. The command
    // itself holds no automation write logic and never reaches the raw service
    // helper (AC-1/INV-2); it only preserves the historical camelCase return
    // shape (INV-9).
    svc.automation_run_compat(automation_id).await
}

/// Structured, side-effect-free plan for the draft/apply preview (AC-3/INV-4).
/// Returns the canonical `snake_case` plan; the store maps it for display.
#[tauri::command]
pub async fn workflow_automation_draft(
    svc: State<'_, Arc<WorkflowApplicationService>>,
    input: AutomationDraftInput,
) -> Result<AutomationPlanV1, String> {
    svc.automation().draft(input).map_err(|e| e.to_string())
}

/// Applies a previously reviewed plan (AC-4). A tampered hash, moved revision or
/// unacknowledged permission expansion returns a stable error string the UI
/// surfaces without a partial write.
#[tauri::command]
pub async fn workflow_automation_apply(
    svc: State<'_, Arc<WorkflowApplicationService>>,
    request: AutomationApplyRequest,
) -> Result<AutomationMutationResult, String> {
    svc.automation()
        .apply(
            &request,
            AUTOMATION_ACTOR_SCOPE_DESKTOP,
            None,
        )
        .map_err(|e| e.to_string())
}

/// The projected run lifecycle (snake_case) joined from the durable workflow
/// snapshot, never from transcript text (AC-7/INV-7). Additive to the legacy
/// camelCase `workflow_automation_list_runs` wire.
#[tauri::command]
pub async fn workflow_automation_run_views(
    svc: State<'_, Arc<WorkflowApplicationService>>,
    automation_id: String,
) -> Result<Vec<AutomationRunView>, String> {
    svc.automation().runs(&automation_id).map_err(|e| e.to_string())
}
