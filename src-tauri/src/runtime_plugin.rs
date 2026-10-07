//! Desktop adapters that route the plugin-management Tauri commands to the
//! standalone runtime control plane.
//!
//! The runtime process is the single owner of the `agent-skills` plugin bundle,
//! its resolved root and its lifecycle state, so the Tauri commands must stop
//! touching the filesystem and reach the runtime through the typed
//! `RuntimeClient` the [`RuntimeSupervisor`] hands out, using only the
//! documented `/control/v1/plugins/agent-skills` routes. This module owns the
//! request/response mapping so each command wrapper stays a thin translation of
//! its Tauri wire into one runtime call.
//!
//! There is deliberately no local fallback and no second handler: when the
//! supervisor holds no lease the call fails instead of silently speaking to a
//! second plugin owner. Every mutation travels through the canonical
//! control-plane idempotency header, so a transport retry replays the runtime's
//! response instead of applying twice.

use serde_json::{json, Value};

use chatspeed_runtime_client::{ClientError, RuntimeClient};

use crate::plugin_types::{plugin_code, PluginError};
use crate::runtime_client::{RuntimeSupervisor, RuntimeUnavailable};

/// Canonical control-plane routes for the static plugin bundle.
const PLUGIN_INVENTORY_ROUTE: &str = "/control/v1/plugins/agent-skills";
const PLUGIN_LOAD_ROUTE: &str = "/control/v1/plugins/agent-skills/load";
const PLUGIN_DISABLE_ROUTE: &str = "/control/v1/plugins/agent-skills/disable";
const PLUGIN_UNINSTALL_ROUTE: &str = "/control/v1/plugins/agent-skills/uninstall";

/// A plugin result, already carrying the stable wire code the Tauri adapters
/// serialize for the frontend.
type PluginResult<T> = Result<T, PluginError>;

/// The static bundle inventory (install state, assets and isolation contract).
pub async fn inventory(supervisor: &RuntimeSupervisor) -> PluginResult<Value> {
    let client = control_plane_client(supervisor).await?;
    client
        .get(PLUGIN_INVENTORY_ROUTE)
        .await
        .map_err(map_client_error)
}

/// Stages, verifies and atomically publishes the embedded bundle.
pub async fn load(supervisor: &RuntimeSupervisor) -> PluginResult<Value> {
    post_idempotent(supervisor, PLUGIN_LOAD_ROUTE).await
}

/// Marks the installed bundle disabled without touching its assets.
pub async fn disable(supervisor: &RuntimeSupervisor) -> PluginResult<Value> {
    post_idempotent(supervisor, PLUGIN_DISABLE_ROUTE).await
}

/// Removes only the plugin-owned bundle; never the managed skills directory or
/// any installed target.
pub async fn uninstall(supervisor: &RuntimeSupervisor) -> PluginResult<Value> {
    post_idempotent(supervisor, PLUGIN_UNINSTALL_ROUTE).await
}

/// Resolves the connected control-plane client, or fails when no lease is held.
async fn control_plane_client(supervisor: &RuntimeSupervisor) -> PluginResult<RuntimeClient> {
    supervisor.client().await.map_err(map_runtime_error)
}

/// Performs an idempotent `POST` with an empty body and returns the decoded
/// JSON body. The route takes no arguments, so the body only carries the
/// idempotency dedup hash.
async fn post_idempotent(supervisor: &RuntimeSupervisor, path: &str) -> PluginResult<Value> {
    let client = control_plane_client(supervisor).await?;
    client
        .post_with_idempotency(path, &json!({}), &new_idempotency_key())
        .await
        .map_err(map_client_error)
}

/// Fresh idempotency key for one command invocation.
fn new_idempotency_key() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Maps a transport/domain client error to the plugin wire error.
///
/// A structured runtime error keeps its own code and message so the frontend
/// branches on the exact token the service produced; every other failure keeps
/// the client's classified (redacted) description.
fn map_client_error(error: ClientError) -> PluginError {
    match error {
        ClientError::Server { code, message, .. } => PluginError::new(&code, message),
        ClientError::Auth(message) => PluginError::new("unauthorized", message),
        other => PluginError::new(plugin_code::INTERNAL, other.to_string()),
    }
}

/// Maps a supervisor availability failure to the plugin wire error.
fn map_runtime_error(error: RuntimeUnavailable) -> PluginError {
    PluginError::unavailable(error.to_string())
}
