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
/// branches on the exact token the service produced, and the optional safe
/// presentation is attached flat beside them instead of replacing the legacy
/// pair. Every other failure keeps the client's classified (redacted)
/// description.
fn map_client_error(error: ClientError) -> PluginError {
    match error {
        ClientError::Server(response) => {
            let mut mapped = PluginError::new(&response.code, response.message);
            if let Some(presentation) = response.presentation {
                mapped = mapped.with_user_error(presentation);
            }
            mapped
        }
        ClientError::Auth(auth) => {
            let mut mapped = PluginError::new("unauthorized", auth.to_string());
            if let Some(presentation) = auth.presentation {
                mapped = mapped.with_user_error(presentation);
            }
            mapped
        }
        other => PluginError::new(plugin_code::INTERNAL, other.to_string()),
    }
}

/// Maps a supervisor availability failure to the plugin wire error.
///
/// The availability code remains `unavailable`; the optional presentation
/// carries the cause without changing the existing error classification.
fn map_runtime_error(error: RuntimeUnavailable) -> PluginError {
    match error {
        RuntimeUnavailable::Server(response) => {
            let mut mapped =
                PluginError::unavailable(format!("runtime returned an error: {response}"));
            if let Some(presentation) = response.presentation {
                mapped = mapped.with_user_error(presentation);
            }
            mapped
        }
        other => PluginError::unavailable(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chatspeed_contracts::UserErrorPresentation;
    use chatspeed_runtime_client::{AuthError, ClientResponseError};

    #[test]
    fn a_server_error_keeps_the_flat_pair_and_attaches_the_presentation() {
        let error = map_client_error(ClientError::Server(ClientResponseError {
            legacy_display: "server error 409: plugin bundle is busy (conflict)".to_string(),
            status: 409,
            code: "conflict".to_string(),
            message: "plugin bundle is busy".to_string(),
            presentation: Some(UserErrorPresentation {
                schema_version: 1,
                message: "The plugin bundle is being updated".to_string(),
                reason: None,
                upstream_status: None,
            }),
        }));

        let wire = serde_json::to_value(&error).expect("plugin error wire");
        // The legacy flat contract keeps the runtime code and message.
        assert_eq!(wire["code"], "conflict");
        assert_eq!(wire["message"], "plugin bundle is busy");
        // The safe projection sits beside the legacy pair, never nested.
        assert_eq!(
            wire["presentation"]["message"],
            "The plugin bundle is being updated"
        );
        assert!(wire.get("error").is_none());
    }

    #[test]
    fn a_legacy_server_error_omits_the_presentation_field() {
        let error = map_client_error(ClientError::Server(ClientResponseError {
            legacy_display: "server error 404: bundle is missing (not_installed)".to_string(),
            status: 404,
            code: "not_installed".to_string(),
            message: "bundle is missing".to_string(),
            presentation: None,
        }));

        assert_eq!(
            serde_json::to_value(&error).expect("plugin error wire"),
            serde_json::json!({ "code": "not_installed", "message": "bundle is missing" })
        );
    }

    #[test]
    fn an_auth_failure_keeps_the_unauthorized_code() {
        let error = map_client_error(ClientError::Auth(AuthError {
            status: 401,
            code: "invalid_token".to_string(),
            message: "token rejected".to_string(),
            presentation: None,
        }));

        assert_eq!(error.code, "unauthorized");
        assert!(error.message.contains("token rejected"));
    }

    #[test]
    fn a_runtime_server_failure_keeps_its_availability_code() {
        let error = map_runtime_error(RuntimeUnavailable::Server(
            crate::runtime_client::RuntimeServerError {
                status: 503,
                code: "runtime_stopping".to_string(),
                message: "runtime is stopping".to_string(),
                presentation: None,
            },
        ));

        assert_eq!(error.code, plugin_code::UNAVAILABLE);
        assert_eq!(
            error.message,
            "runtime returned an error: 503 runtime_stopping: runtime is stopping"
        );
    }

    #[test]
    fn a_missing_lease_stays_unavailable() {
        let error = map_runtime_error(RuntimeUnavailable::NotConnected);
        assert_eq!(error.code, plugin_code::UNAVAILABLE);
    }
}
