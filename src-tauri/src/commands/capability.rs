//! Tauri adapters for the runtime-owned capability and Agent Skill surface.
//!
//! Each command is a thin transport over the standalone runtime control plane:
//! it maps its Tauri wire onto one typed `/control/v1` capability route through
//! the [`RuntimeSupervisor`], so the desktop page, the `cs` CLI and every other
//! client observe the one `CapabilityApplicationService` the runtime owns (AC-1).
//! The commands never touch `MainStore`, never open a journal, never reach the
//! filesystem and never touch an MCP runtime directly.
//!
//! Errors keep the serialized, redacted `{"code","message"}` capability envelope
//! so the frontend can branch on `code` exactly like an HTTP client does (AC-13).
//!
//! # Module wiring
//!
//! `runtime_capability` is the shared desktop adapter and is included from here
//! with an explicit `#[path]` so this unit does not have to edit `lib.rs`
//! (several sibling units touch the module list there). When that concurrency is
//! done the parent may hoist the declaration to `lib.rs` as
//! `#[cfg(feature = "desktop")] mod runtime_capability;` and drop the `#[path]`
//! include below; the call sites then read `crate::runtime_capability`.

use std::sync::Arc;

use serde_json::Value;
use tauri::State;

use crate::capability::error::CapabilityError;
use crate::capability::types::CapabilityOperation;
use crate::runtime_client::RuntimeSupervisor;

#[path = "../runtime_capability.rs"]
pub(crate) mod runtime_capability;

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
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
) -> Result<Value, String> {
    runtime_capability::skill_targets(supervisor.inner().as_ref())
        .await
        .map_err(wire_error)
}

/// The Agent Skill inventory (owned, drifted, discovered and bundled).
#[tauri::command]
pub async fn capability_skill_inventory(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
) -> Result<Value, String> {
    runtime_capability::skill_inventory(supervisor.inner().as_ref())
        .await
        .map_err(wire_error)
}

/// MCP servers with their desired, observed runtime and tools state.
#[tauri::command]
pub async fn capability_mcp_servers(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
) -> Result<Value, String> {
    runtime_capability::mcp_servers(supervisor.inner().as_ref())
        .await
        .map_err(wire_error)
}

/// One durable capability operation, for the UI to restore an in-flight view.
#[tauri::command]
pub async fn capability_operation(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    operation_id: String,
) -> Result<CapabilityOperation, String> {
    runtime_capability::operation(supervisor.inner().as_ref(), &operation_id)
        .await
        .map_err(wire_error)
}

/// The capability doctor report (journal, ownership, runtime, staging).
#[tauri::command]
pub async fn capability_doctor(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
) -> Result<Value, String> {
    runtime_capability::doctor(supervisor.inner().as_ref())
        .await
        .map_err(wire_error)
}

/// Converges capability drift the durable evidence proves, and preserves the
/// rest as `needs_reconcile`.
///
/// This is the one capability adapter that advances state; it delegates to the
/// runtime's exact `CapabilityApplicationService`, so the desktop and
/// `cs doctor capabilities --reconcile` can never converge different facts
/// (AC-1). It never blind-retries or deletes unproven content.
#[tauri::command]
pub async fn capability_reconcile(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
) -> Result<Value, String> {
    runtime_capability::reconcile(supervisor.inner().as_ref())
        .await
        .map_err(wire_error)
}

/// Checks a Skill source with the non-LLM checker, without installing.
#[tauri::command]
pub async fn capability_skill_check(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    source: Value,
) -> Result<Value, String> {
    runtime_capability::skill_check(supervisor.inner().as_ref(), &source)
        .await
        .map_err(wire_error)
}

/// Installs a checked Skill into the selected targets.
///
/// The idempotency key is required: without it a double click or a retried IPC
/// call could install twice (AC-2). It is forwarded verbatim to the runtime so
/// the durable journal replays a retry.
#[tauri::command]
pub async fn capability_skill_install(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    source: Value,
    targets: Option<Vec<String>>,
    idempotency_key: String,
) -> Result<Value, String> {
    runtime_capability::skill_install(
        supervisor.inner().as_ref(),
        &source,
        &targets.unwrap_or_default(),
        &idempotency_key,
    )
    .await
    .map_err(wire_error)
}

/// Uninstalls a Skill that ChatSpeed installed and still owns.
#[tauri::command]
pub async fn capability_skill_uninstall(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    skill_name: String,
    targets: Option<Vec<String>>,
    idempotency_key: String,
) -> Result<Value, String> {
    runtime_capability::skill_uninstall(
        supervisor.inner().as_ref(),
        &skill_name,
        &targets.unwrap_or_default(),
        &idempotency_key,
    )
    .await
    .map_err(wire_error)
}
