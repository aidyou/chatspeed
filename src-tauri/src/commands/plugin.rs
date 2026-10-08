//! Tauri adapters for the runtime-owned plugin-management surface.
//!
//! Each command is a thin transport over the standalone runtime control plane:
//! it maps its Tauri wire onto one typed `/control/v1/plugins/agent-skills`
//! route through the [`RuntimeSupervisor`], so the desktop never resolves a
//! plugin path, never reads or writes the plugin bundle and never opens a
//! database. The runtime's `PluginService` is the only plugin-management owner.
//!
//! The plugin UI commands drive the native host that renders a verified bundle
//! in the Workflow window. They keep the tab bookkeeping in Rust: a tab is
//! identified by a UUID, remembers the bundle it shows and its capability grant,
//! and a repeated open of the same bundle re-proves the runtime inventory and
//! re-places the live tab instead of revoking and rebuilding it. The capability
//! token and the resolved URLs never leave the host.
//!
//! Every UI command is serialised with the plugin lifecycle mutations through
//! the runtime's gate, so an open can never interleave a close or the
//! disable/uninstall cleanup. The native operations are posted to the platform
//! main thread; a lifecycle mutation that invalidates the bundle then revokes
//! every capability, detaches every panel and broadcasts `cs://plugins-changed`
//! so the settings and workflow views can refresh.
//!
//! Errors keep the serialized `{"code","message"}` envelope so the frontend can
//! branch on the same code token an HTTP client observes. The plugin UI commands
//! return a stable token the frontend maps to a localized message.

use std::sync::Arc;

use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State, Window};

use crate::chat_hub::CHAT_HUB_HOST_WINDOW_LABEL;
use crate::plugin_types::PluginError;
use crate::plugin_ui::{
    is_valid_tab_id, plan_tab_open, PluginUiBounds, PluginUiHost, PluginUiRuntime,
    PluginUiTabPlan, PluginUiTabSession,
};
use crate::runtime_client::RuntimeSupervisor;

#[path = "../runtime_plugin.rs"]
pub(crate) mod runtime_plugin;

/// Broadcast when a plugin mutation changes the installed bundle.
///
/// Both the settings and the workflow views listen for it, so a mutation made in
/// one window refreshes the other without polling. It is emitted only after the
/// runtime confirmed the mutation.
const PLUGINS_CHANGED_EVENT: &str = "cs://plugins-changed";

/// Serializes a plugin error into its stable wire envelope.
fn wire_error(error: PluginError) -> String {
    serde_json::to_string(&error).unwrap_or_else(|_| {
        format!(
            "{{\"code\":\"{}\",\"message\":\"plugin error could not be serialized\"}}",
            error.code
        )
    })
}

/// Refuses a plugin UI command that does not come from the Workflow window.
///
/// The native panels are docked into the Workflow window, so a call from any
/// other window is a mistake the host must not honour.
fn ensure_workflow_caller(window: &Window) -> Result<(), String> {
    if window.label() == CHAT_HUB_HOST_WINDOW_LABEL {
        Ok(())
    } else {
        Err("plugin_ui_workflow_window_required".to_string())
    }
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
///
/// A republished bundle keeps its plugin id and entry, so an open tab stays
/// valid; the broadcast tells the views to re-read the bundle they show.
#[tauri::command]
pub async fn plugin_load(
    app: AppHandle,
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    runtime: State<'_, PluginUiRuntime>,
) -> Result<Value, String> {
    let _gate = runtime.serialize().await;
    let result = runtime_plugin::load(supervisor.inner().as_ref())
        .await
        .map_err(wire_error)?;
    broadcast_plugins_changed(&app);
    Ok(result)
}

/// Marks the installed bundle disabled without touching its assets.
#[tauri::command]
pub async fn plugin_disable(
    app: AppHandle,
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    runtime: State<'_, PluginUiRuntime>,
) -> Result<Value, String> {
    let _gate = runtime.serialize().await;
    let result = runtime_plugin::disable(supervisor.inner().as_ref())
        .await
        .map_err(wire_error)?;
    invalidate_plugin_ui(&app, runtime.inner()).await;
    broadcast_plugins_changed(&app);
    Ok(result)
}

/// Removes only the plugin-owned bundle; never the managed skills directory or
/// any installed target.
#[tauri::command]
pub async fn plugin_uninstall(
    app: AppHandle,
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    runtime: State<'_, PluginUiRuntime>,
) -> Result<Value, String> {
    let _gate = runtime.serialize().await;
    let result = runtime_plugin::uninstall(supervisor.inner().as_ref())
        .await
        .map_err(wire_error)?;
    invalidate_plugin_ui(&app, runtime.inner()).await;
    broadcast_plugins_changed(&app);
    Ok(result)
}

/// Revokes every plugin UI capability and detaches every panel after a mutation
/// that invalidates the bundle.
///
/// Cleanup is best-effort: a failure is logged and never replaces or fails the
/// authoritative runtime result the mutation already produced, so the frontend
/// keeps the snapshot the runtime returned.
async fn invalidate_plugin_ui(app: &AppHandle, runtime: &PluginUiRuntime) {
    if let Err(error) = runtime.revoke_all().await {
        log::warn!("[PluginUi] capability cleanup failed: {error}");
    }
    let close_app = app.clone();
    if let Err(error) = run_plugin_host(app, move |host| host.close_all(&close_app)).await {
        log::warn!("[PluginUi] panel cleanup failed: {error}");
    }
}

/// Broadcasts that the installed plugin bundle changed.
fn broadcast_plugins_changed(app: &AppHandle) {
    if let Err(error) = app.emit(PLUGINS_CHANGED_EVENT, ()) {
        log::warn!("[PluginUi] plugins-changed broadcast failed: {error}");
    }
}

/// Shows a plugin UI tab, doubling as the resize and reveal path.
///
/// `tab_id` is a frontend-generated UUID. When the tab already shows the same
/// plugin entry the live tab is re-proved against the runtime inventory and
/// re-placed at `bounds`, keeping its capability grant and its rendered session;
/// another entry replaces the grant. `plugin_id` and `entry` are never trusted
/// blindly: the gateway proves them against the runtime inventory, and the
/// resolved URL never comes from the frontend.
#[tauri::command]
pub async fn plugin_ui_open(
    app: AppHandle,
    window: Window,
    runtime: State<'_, PluginUiRuntime>,
    tab_id: String,
    plugin_id: String,
    entry: String,
    bounds: PluginUiBounds,
) -> Result<(), String> {
    ensure_workflow_caller(&window)?;
    if !is_valid_tab_id(&tab_id) {
        return Err("plugin_ui_tab_id_invalid".to_string());
    }

    let _gate = runtime.serialize().await;
    // The gateway is started once by the runtime startup task; a command never
    // lazily starts it, so a missing gateway is refused instead of racing a
    // second server.
    let gateway = runtime.gateway()?;
    let runtime = runtime.inner();
    let existing = runtime.tabs().session(&tab_id)?;

    match plan_tab_open(
        existing.as_ref(),
        &plugin_id,
        &entry,
        runtime.tabs().count()?,
    ) {
        PluginUiTabPlan::Reuse => {
            let session = existing.ok_or_else(|| "plugin UI tab session is missing".to_string())?;
            // Re-prove the bundle is still an enabled, verified UI before reusing
            // the live tab, so a disabled or replaced bundle cannot keep showing.
            let probe = match gateway.grant(&plugin_id, &entry).await {
                Ok(probe) => probe,
                Err(error) => {
                    // The live panel no longer has an enabled, verified bundle
                    // behind it, so its capability is revoked and the panel is
                    // torn down instead of staying visible with stale content.
                    gateway.revoke(&session.grant.capability).await;
                    runtime.tabs().forget(&tab_id)?;
                    close_plugin_tab(&app, &tab_id).await;
                    return Err(error);
                }
            };
            gateway.revoke(&probe.capability).await;

            let url = session.grant.url;
            let prefix = session.grant.prefix;
            let show_app = app.clone();
            run_plugin_host(&app, move |host| {
                host.show(&show_app, &tab_id, &url, &prefix, bounds)
            })
            .await
        }
        plan => {
            if plan == PluginUiTabPlan::Rejected {
                return Err("plugin_ui_tab_limit_reached".to_string());
            }

            // A different bundle for a known tab replaces its grant. The previous
            // native panel is destroyed on the main thread before the replacement
            // is built, so the fresh page carries the new grant's prefix instead
            // of reusing a page whose navigation handler still pins the old one.
            // Closing a genuinely new tab is a no-op, so the Open plan shares
            // this path.
            let close_tab = tab_id.clone();
            run_plugin_host(&app, move |host| host.close(&close_tab)).await?;

            // The old panel is gone, so its capability can never serve the new
            // entry; revoke it and drop the session before granting the
            // replacement.
            if let Some(previous) = runtime.tabs().forget(&tab_id)? {
                gateway.revoke(&previous.grant.capability).await;
            }
            let grant = gateway.grant(&plugin_id, &entry).await?;
            let url = grant.url.clone();
            let prefix = grant.prefix.clone();
            runtime.tabs().remember(
                &tab_id,
                PluginUiTabSession {
                    plugin_id,
                    entry,
                    grant,
                },
            )?;

            let show_app = app.clone();
            let show_tab = tab_id.clone();
            let result = run_plugin_host(&app, move |host| {
                host.show(&show_app, &show_tab, &url, &prefix, bounds)
            })
            .await;
            if result.is_err() {
                // A failed show must leave neither a grant nor a partially built
                // panel behind, so the tab can be retried cleanly.
                if let Some(session) = runtime.tabs().forget(&tab_id)? {
                    gateway.revoke(&session.grant.capability).await;
                }
                close_plugin_tab(&app, &tab_id).await;
            }
            result
        }
    }
}

/// Hides all plugin panels while preserving their tab sessions.
#[tauri::command]
pub async fn plugin_ui_hide(
    app: AppHandle,
    window: Window,
    runtime: State<'_, PluginUiRuntime>,
) -> Result<(), String> {
    ensure_workflow_caller(&window)?;
    let _gate = runtime.serialize().await;
    run_plugin_host(&app, |host| host.hide()).await
}

/// Closes one plugin tab and revokes its resource capability.
///
/// A tab the host does not know is a no-op, so a close that races the frontend
/// cannot fail.
#[tauri::command]
pub async fn plugin_ui_close(
    app: AppHandle,
    window: Window,
    runtime: State<'_, PluginUiRuntime>,
    tab_id: String,
) -> Result<(), String> {
    ensure_workflow_caller(&window)?;
    if !is_valid_tab_id(&tab_id) {
        return Err("plugin_ui_tab_id_invalid".to_string());
    }

    let _gate = runtime.serialize().await;
    let runtime = runtime.inner();
    if let Some(session) = runtime.tabs().forget(&tab_id)? {
        if let Ok(gateway) = runtime.gateway() {
            gateway.revoke(&session.grant.capability).await;
        }
    }

    run_plugin_host(&app, move |host| host.close(&tab_id)).await
}

/// Revokes all plugin UI capabilities and detaches all native panels.
#[tauri::command]
pub async fn plugin_ui_clear(
    app: AppHandle,
    window: Window,
    runtime: State<'_, PluginUiRuntime>,
) -> Result<(), String> {
    ensure_workflow_caller(&window)?;
    let _gate = runtime.serialize().await;
    runtime.revoke_all().await?;
    let close_app = app.clone();
    run_plugin_host(&app, move |host| host.close_all(&close_app)).await
}

/// Runs one plugin UI host operation on the platform main thread.
///
/// The native webviews and GTK widgets live on that thread, so the operation is
/// posted there and its result is awaited on the caller. A missing host state is
/// reported as a plain error instead of a panic.
async fn run_plugin_host(
    app: &AppHandle,
    operation: impl FnOnce(&PluginUiHost) -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let thread_app = app.clone();
    app.run_on_main_thread(move || {
        let result = match thread_app.try_state::<PluginUiHost>() {
            Some(host) => operation(host.inner()),
            None => Err("plugin_ui_host_unavailable".to_string()),
        };
        let _ = sender.send(result);
    })
    .map_err(|error| error.to_string())?;

    receiver
        .await
        .map_err(|error| format!("plugin UI host task did not report a result: {error}"))?
}

/// Best-effort teardown of one native plugin panel.
///
/// The open paths call this after a rejected reuse proof or a failed show, so a
/// stale or partially built page never stays visible. Closing a tab the host
/// does not know is a no-op, and a real failure is only logged because the
/// caller is already returning the error that triggered the teardown.
async fn close_plugin_tab(app: &AppHandle, tab_id: &str) {
    let close_tab = tab_id.to_string();
    if let Err(error) = run_plugin_host(app, move |host| host.close(&close_tab)).await {
        log::warn!("[PluginUi] failed to close a plugin UI tab: {error}");
    }
}