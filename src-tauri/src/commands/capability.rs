//! Tauri read-only adapters for the Phase 3 capability service.
//!
//! These commands are thin transports: each one calls the single
//! `CapabilityApplicationService` the control plane also uses, so the desktop
//! page and the `cs` CLI can never observe different facts (AC-1). They never
//! mutate anything, never touch `MainStore` directly and never reach the
//! filesystem or the MCP runtime on their own.
//!
//! Errors are returned as the serialized, redacted `{"code","message"}`
//! capability envelope so the frontend can branch on `code` exactly like an
//! HTTP client does (AC-13).

use std::sync::Arc;

use tauri::State;

use crate::capability::doctor::CapabilityDoctorReport;
use crate::capability::error::CapabilityError;
use crate::capability::mcp_service::McpServerView;
use crate::capability::reconcile::CapabilityReconcileReport;
use crate::capability::skill::checker::SkillCheckReport;
use crate::capability::skill::orchestrator::SkillMutationResult;
use crate::capability::skill_inventory::SkillInventory;
use crate::capability::targets::ResolvedSkillTarget;
use crate::capability::types::CapabilityOperation;
use crate::capability::CapabilityApplicationService;

/// Serializes a capability error into its stable wire envelope.
fn wire_error(error: CapabilityError) -> String {
    serde_json::to_string(&error).unwrap_or_else(|_| {
        format!(
            "{{\"code\":\"{}\",\"message\":\"capability error could not be serialized\"}}",
            error.code()
        )
    })
}

/// The closed Skill install-target registry.
#[tauri::command]
pub async fn capability_skill_targets(
    capability: State<'_, Arc<CapabilityApplicationService>>,
) -> Result<Vec<ResolvedSkillTarget>, String> {
    Ok(capability.skill_targets())
}

/// The Agent Skill inventory (owned, drifted, discovered and bundled).
#[tauri::command]
pub async fn capability_skill_inventory(
    capability: State<'_, Arc<CapabilityApplicationService>>,
) -> Result<SkillInventory, String> {
    capability.skill_inventory().map_err(wire_error)
}

/// MCP servers with their desired, observed runtime and tools state.
#[tauri::command]
pub async fn capability_mcp_servers(
    capability: State<'_, Arc<CapabilityApplicationService>>,
) -> Result<Vec<McpServerView>, String> {
    capability.mcp_servers().await.map_err(wire_error)
}

/// One durable capability operation, for the UI to restore an in-flight view.
#[tauri::command]
pub async fn capability_operation(
    capability: State<'_, Arc<CapabilityApplicationService>>,
    operation_id: String,
) -> Result<CapabilityOperation, String> {
    capability.operation(&operation_id).map_err(wire_error)
}

/// The capability doctor report (journal, ownership, runtime, staging).
#[tauri::command]
pub async fn capability_doctor(
    capability: State<'_, Arc<CapabilityApplicationService>>,
) -> Result<CapabilityDoctorReport, String> {
    capability.doctor().await.map_err(wire_error)
}

/// Converges capability drift the durable evidence proves, and preserves the
/// rest as `needs_reconcile`.
///
/// This is the one capability adapter that advances state; it delegates to the
/// exact same `CapabilityApplicationService` the control plane and CLI use, so
/// the desktop and `cs doctor capabilities --reconcile` can never converge
/// different facts (AC-1). It never blind-retries or deletes unproven content.
#[tauri::command]
pub async fn capability_reconcile(
    capability: State<'_, Arc<CapabilityApplicationService>>,
) -> Result<CapabilityReconcileReport, String> {
    capability.reconcile().await.map_err(wire_error)
}

/// The durable journal scope of a mutation started from the desktop.
///
/// The scope is part of the idempotency key, so a desktop click and a CLI retry
/// with the same key stay distinct operations.
pub const ACTOR_SCOPE_DESKTOP: &str = "desktop";

/// Checks a Skill source with the non-LLM checker, without installing.
#[tauri::command]
pub async fn capability_skill_check(
    capability: State<'_, Arc<CapabilityApplicationService>>,
    source: serde_json::Value,
) -> Result<SkillCheckReport, String> {
    capability.skill_check(&source).await.map_err(wire_error)
}

/// Installs a checked Skill into the selected targets.
///
/// The idempotency key is required: without it a double click or a retried
/// IPC call could install twice (AC-2).
#[tauri::command]
pub async fn capability_skill_install(
    capability: State<'_, Arc<CapabilityApplicationService>>,
    source: serde_json::Value,
    targets: Option<Vec<String>>,
    idempotency_key: String,
) -> Result<SkillMutationResult, String> {
    capability
        .skill_install(
            &source,
            &targets.unwrap_or_default(),
            &idempotency_key,
            ACTOR_SCOPE_DESKTOP,
        )
        .await
        .map_err(wire_error)
}

/// Uninstalls a Skill that ChatSpeed installed and still owns.
#[tauri::command]
pub async fn capability_skill_uninstall(
    capability: State<'_, Arc<CapabilityApplicationService>>,
    skill_name: String,
    targets: Option<Vec<String>>,
    idempotency_key: String,
) -> Result<SkillMutationResult, String> {
    capability
        .skill_uninstall(
            &skill_name,
            &targets.unwrap_or_default(),
            &idempotency_key,
            ACTOR_SCOPE_DESKTOP,
        )
        .await
        .map_err(wire_error)
}
