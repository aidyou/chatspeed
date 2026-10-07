//! Tauri adapters for the runtime-owned plugin-management surface.
//!
//! Each command is a thin transport over the standalone runtime control plane:
//! it maps its Tauri wire onto one typed `/control/v1/plugins/agent-skills`
//! route through the [`RuntimeSupervisor`], so the desktop never resolves a
//! plugin path, never reads or writes the plugin bundle and never opens a
//! database. The runtime's `PluginService` is the only plugin-management owner.
//!
//! Errors keep the serialized `{"code","message"}` envelope so the frontend can
//! branch on the same code token an HTTP client observes.
//!
//! # Module wiring
//!
//! `runtime_plugin` is the shared desktop adapter and is included from here with
//! an explicit `#[path]` so this unit does not have to edit the `lib.rs` module
//! list beyond the command registration (sibling units touch that list too).
//! When that concurrency is done the parent may hoist the declaration to
//! `lib.rs` as `#[cfg(feature = "desktop")] mod runtime_plugin;` and drop the
//! `#[path]` include below; the call sites then read `crate::runtime_plugin`.

use std::sync::Arc;

use serde_json::Value;
use tauri::State;

use crate::plugin_types::PluginError;
use crate::runtime_client::RuntimeSupervisor;

#[path = "../runtime_plugin.rs"]
pub(crate) mod runtime_plugin;

/// Serializes a plugin error into its stable wire envelope.
fn wire_error(error: PluginError) -> String {
    serde_json::to_string(&error).unwrap_or_else(|_| {
        format!(
            "{{\"code\":\"{}\",\"message\":\"plugin error could not be serialized\"}}",
            error.code
        )
    })
}

/// The static bundle inventory (install state, assets and isolation contract).
#[tauri::command]
pub async fn plugin_inventory(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
) -> Result<Value, String> {
    runtime_plugin::inventory(supervisor.inner().as_ref())
        .await
        .map_err(wire_error)
}

/// Stages, verifies and atomically publishes the embedded bundle.
#[tauri::command]
pub async fn plugin_load(supervisor: State<'_, Arc<RuntimeSupervisor>>) -> Result<Value, String> {
    runtime_plugin::load(supervisor.inner().as_ref())
        .await
        .map_err(wire_error)
}

/// Marks the installed bundle disabled without touching its assets.
#[tauri::command]
pub async fn plugin_disable(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
) -> Result<Value, String> {
    runtime_plugin::disable(supervisor.inner().as_ref())
        .await
        .map_err(wire_error)
}

/// Removes only the plugin-owned bundle; never the managed skills directory or
/// any installed target.
#[tauri::command]
pub async fn plugin_uninstall(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
) -> Result<Value, String> {
    runtime_plugin::uninstall(supervisor.inner().as_ref())
        .await
        .map_err(wire_error)
}
