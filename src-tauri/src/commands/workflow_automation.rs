//! Tauri adapters for local workflow automation.
//!
//! These are thin transport shims over the standalone runtime control plane.
//! Every read and mutation resolves to the one automation owner the runtime
//! process holds, so the desktop editor can never diverge from the control plane
//! or the scheduler about validation, errors, revision or status (AC-1/INV-2).
//! Command names and the historical camelCase return shapes are preserved; the
//! only additive change is `delete`'s optional `confirm` flag (AC-10).
//!
//! The commands never inject `WorkflowApplicationService`, `MainStore` or a
//! scheduler: [`runtime_automation`] maps each wire onto the documented
//! `/control/v1` automation routes through the [`RuntimeSupervisor`].
//!
//! ## Module wiring
//!
//! `runtime_automation` is included from here with an explicit `#[path]` so this
//! unit does not have to edit `lib.rs` (several sibling units touch the module
//! list there). When that concurrency is done the parent may hoist the
//! declaration to `lib.rs` as `#[cfg(feature = "desktop")] mod runtime_automation;`.

use crate::db::{WorkflowAutomation, WorkflowAutomationRun};
use crate::runtime_client::RuntimeSupervisor;
use crate::workflow::automation::types::{
    AutomationApplyRequest, AutomationDraftInput, AutomationMutationResult, AutomationPlanV1,
    AutomationRunView, WorkflowAutomationRequest, WorkflowAutomationRunNowResult,
};
use std::sync::Arc;
use tauri::State;

#[path = "../runtime_automation.rs"]
mod runtime_automation;

#[tauri::command]
pub async fn workflow_automation_list(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
) -> Result<Vec<WorkflowAutomation>, String> {
    runtime_automation::automation_list(supervisor.inner().as_ref()).await
}

#[tauri::command]
pub async fn workflow_automation_get(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: String,
) -> Result<Option<WorkflowAutomation>, String> {
    runtime_automation::automation_get(supervisor.inner().as_ref(), &id).await
}

#[tauri::command]
pub async fn workflow_automation_save(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    request: WorkflowAutomationRequest,
) -> Result<WorkflowAutomation, String> {
    runtime_automation::automation_save(supervisor.inner().as_ref(), request).await
}

#[tauri::command]
pub async fn workflow_automation_delete(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: String,
    confirm: Option<bool>,
) -> Result<(), String> {
    runtime_automation::automation_delete(
        supervisor.inner().as_ref(),
        &id,
        confirm.unwrap_or(false),
    )
    .await
}

#[tauri::command]
pub async fn workflow_automation_set_enabled(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    runtime_automation::automation_set_enabled(supervisor.inner().as_ref(), &id, enabled).await
}

#[tauri::command]
pub async fn workflow_automation_list_runs(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    automation_id: String,
) -> Result<Vec<WorkflowAutomationRun>, String> {
    runtime_automation::automation_list_runs(supervisor.inner().as_ref(), &automation_id).await
}

#[tauri::command]
pub async fn workflow_automation_run_now(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    automation_id: String,
) -> Result<WorkflowAutomationRunNowResult, String> {
    // The manual run resolves to the runtime's single manual-run kernel through
    // the canonical `/run` route; the desktop holds no run logic (AC-1/INV-2).
    runtime_automation::automation_run_now(supervisor.inner().as_ref(), &automation_id).await
}

/// Structured, side-effect-free plan for the draft/apply preview (AC-3/INV-4).
/// Returns the canonical `snake_case` plan; the store maps it for display.
#[tauri::command]
pub async fn workflow_automation_draft(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    input: AutomationDraftInput,
) -> Result<AutomationPlanV1, String> {
    runtime_automation::automation_draft(supervisor.inner().as_ref(), input).await
}

/// Applies a previously reviewed plan (AC-4). A tampered hash, moved revision or
/// unacknowledged permission expansion returns a stable error string the UI
/// surfaces without a partial write.
#[tauri::command]
pub async fn workflow_automation_apply(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    request: AutomationApplyRequest,
) -> Result<AutomationMutationResult, String> {
    runtime_automation::automation_apply(supervisor.inner().as_ref(), request).await
}

/// The projected run lifecycle (snake_case) joined from the durable workflow
/// snapshot, never from transcript text (AC-7/INV-7). Additive to the legacy
/// camelCase `workflow_automation_list_runs` wire.
#[tauri::command]
pub async fn workflow_automation_run_views(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    automation_id: String,
) -> Result<Vec<AutomationRunView>, String> {
    runtime_automation::automation_run_views(supervisor.inner().as_ref(), &automation_id).await
}
