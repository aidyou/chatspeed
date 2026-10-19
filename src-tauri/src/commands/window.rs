//! Window management commands for the application
//!
//! This module implements commands for managing application windows, including
//! creating and focusing note, setting, and URL windows.
//!
//! # Window Creation Strategy
//!
//! Instead of creating windows directly in command handlers, we use an event-based
//! approach where commands emit events that are handled by the main thread. This
//! design choice addresses a critical issue in WebView2 (Windows) where creating
//! windows from IPC handlers can lead to deadlocks.
//!
//! ## Background
//!
//! The issue stems from how WebView2 handles window creation and IPC calls on Windows:
//! - WebView2 requires window creation to happen on the main UI thread
//! - IPC handlers run on a different thread
//! - Attempting to create windows directly from IPC handlers can cause thread
//!   synchronization issues and deadlocks
//!
//! For more details, see the discussion at:
//! <https://github.com/tauri-apps/wry/issues/583>
//!
//! ## Implementation
//!
//! Our solution uses Tauri's event system:
//! 1. Commands emit events (e.g., "create-note-window", "create-setting-window")
//! 2. Event listeners on the main thread handle window creation
//! 3. This ensures windows are always created on the correct thread
//!
//! This approach provides several benefits:
//! - Prevents deadlocks on Windows
//! - Works consistently across all platforms
//! - Maintains clean separation of concerns
//! - Improves code maintainability

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use rust_i18n::t;
use serde_json::Value;
// use tauri::utils::{config::WindowEffectsConfig, WindowEffect};
use crate::constants::{
    ASSISTANT_ALWAYS_ON_TOP, MAIN_WINDOW_ALWAYS_ON_TOP, WORKFLOW_WINDOW_ALWAYS_ON_TOP,
};
use crate::exit_coordinator::{
    ExitCoordinator, QuitPhase, QuitStatus, QuitTransition, QUIT_ERROR_RUNTIME_INSTANCE_CHANGED,
    QUIT_ERROR_RUNTIME_NOT_CONNECTED, QUIT_RELEASE_BUDGET, QUIT_STATUS_EVENT, QUIT_STOP_BUDGET,
};
use crate::plugin_types::PluginError;
use crate::runtime_client::{RuntimeStopOutcome, RuntimeSupervisor, RuntimeUnavailable};
use tauri::{command, Emitter, Manager, State}; //window::Color, WindowEvent,

use crate::error::{AppError, Result};

#[derive(serde::Serialize, Clone)]
struct SettingWindowPayload {
    setting_type: String,
}

#[derive(serde::Serialize, Clone)]
struct ShowWindowPayload<'a> {
    window_label: &'a str,
}

/// Opens the settings window via event system
///
/// IMPORTANT: We use events instead of direct window creation to avoid deadlocks
/// on Windows. This is because WebView2 requires window creation to happen on the
/// main UI thread, while IPC handlers run on a different thread. Using events
/// ensures window creation occurs on the correct thread.
///
/// See: https://github.com/tauri-apps/wry/issues/583
///
/// # Arguments
/// - `app_handle` - Tauri application handle
/// - `setting_type` - Optional setting type to display (defaults to "general")
///
/// # Example
/// ```js
/// import { invoke } from '@tauri-apps/api/core'
///
/// await invoke('open_setting_window', { settingType: 'model' });
/// ```
#[command]
pub async fn open_setting_window(
    app_handle: tauri::AppHandle,
    setting_type: Option<String>,
) -> Result<()> {
    app_handle
        .emit(
            "create-setting-window",
            SettingWindowPayload {
                setting_type: setting_type.unwrap_or_else(|| "general".to_string()),
            },
        )
        .map_err(|e| AppError::General {
            message: t!("main.failed_to_emit_event", error = e.to_string()).to_string(),
        })?;

    Ok(())
}

/// Opens the note window via event system
///
/// IMPORTANT: We use events instead of direct window creation to avoid deadlocks
/// on Windows. This is because WebView2 requires window creation to happen on the
/// main UI thread, while IPC handlers run on a different thread. Using events
/// ensures window creation occurs on the correct thread.
///
/// See: https://github.com/tauri-apps/wry/issues/583
///
/// # Arguments
/// - `app_handle` - Tauri application handle
///
/// # Example
/// ```js
/// import { invoke } from '@tauri-apps/api/core'
///
/// await invoke('open_note_window');
/// ```
#[command]
pub async fn open_note_window(app_handle: tauri::AppHandle) -> Result<()> {
    app_handle
        .emit("create-note-window", ())
        .map_err(|e| AppError::General {
            message: t!("main.failed_to_emit_event", error = e.to_string()).to_string(),
        })?;

    Ok(())
}

/// Opens the proxy switcher window via event system
///
/// # Arguments
/// - `app_handle` - Tauri application handle
///
/// # Example
/// ```js
/// import { invoke } from '@tauri-apps/api/core'
///
/// await invoke('open_proxy_switcher_window');
/// ```
#[command]
pub fn open_proxy_switcher_window(app_handle: tauri::AppHandle) -> Result<()> {
    app_handle
        .emit("create-proxy-switcher-window", ())
        .map_err(|e| AppError::General {
            message: t!("main.failed_to_emit_event", error = e.to_string()).to_string(),
        })?;

    Ok(())
}

/// The built-in plugin whose trusted management surface the docked view renders.
const AGENT_SKILLS_PLUGIN_ID: &str = "agent-skills";

/// Shows the docked Agent Skills surface at the rectangle the frontend measured.
///
/// The runtime inventory is the only authority for whether the surface may open: the built-in
/// `agent-skills` bundle must be enabled with a UI the runtime verified, so a disabled,
/// uninstalled or replaced plugin is never reachable through this path. The check runs while the
/// plugin UI gate is held, so a concurrent lifecycle mutation cannot race the open.
///
/// The surface is docked into the Workflow window, so a call from any other window is refused. It
/// is a same-window child webview, never a second top-level window, and the creation result
/// travels back to the caller instead of being flattened into a log line.
///
/// # Example
/// ```js
/// import { invoke } from '@tauri-apps/api/core'
///
/// await invoke('show_agent_skills_view', {
///   bounds: { x: 100, y: 40, width: 600, height: 700 },
///   cornerRadius: 8
/// });
/// ```
#[command]
pub async fn show_agent_skills_view(
    app: tauri::AppHandle,
    window: tauri::Window,
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    runtime: State<'_, crate::plugin_ui::PluginUiRuntime>,
    bounds: crate::native_dock::DockBounds,
    corner_radius: Option<f64>,
) -> std::result::Result<(), String> {
    crate::commands::plugin::ensure_workflow_caller(&window)?;
    let corner_radius = corner_radius.unwrap_or(0.0);

    let _gate = runtime.serialize().await;
    let inventory = crate::commands::plugin::runtime_plugin::inventory(supervisor.inner().as_ref())
        .await
        .map_err(plugin_wire_error)?;
    if !agent_skills_ui_verified(&inventory) {
        return Err(plugin_wire_error(PluginError::not_installed(
            "the agent-skills plugin is not installed, enabled and verified",
        )));
    }

    crate::agent_skills::run_on_view_thread(&app, move |view, app| {
        view.show(app, bounds, corner_radius)
    })
    .await
}

/// Hides the docked Agent Skills surface while keeping its browsing session.
#[command]
pub async fn hide_agent_skills_view(
    app: tauri::AppHandle,
    window: tauri::Window,
) -> std::result::Result<(), String> {
    crate::commands::plugin::ensure_workflow_caller(&window)?;

    crate::agent_skills::run_on_view_thread(&app, |view, _app| view.hide()).await
}

/// Releases the docked Agent Skills surface, and only that surface.
///
/// The child webview is closed without touching the Workflow window it is docked into, so the
/// workflow UI and every other docked view stay in place.
#[command]
pub async fn close_agent_skills_view(
    app: tauri::AppHandle,
    window: tauri::Window,
) -> std::result::Result<(), String> {
    crate::commands::plugin::ensure_workflow_caller(&window)?;

    crate::agent_skills::run_on_view_thread(&app, |view, app| view.close(app)).await
}

/// Whether the runtime inventory proves the built-in Agent Skills surface may open.
///
/// The check mirrors the frontend filter (see `verifiedPluginUis`): the snapshot
/// must use the schema this desktop understands, and the plugin must be a
/// built-in bundle that is enabled with a verified UI. Anything else fails
/// closed, so an unconfirmed snapshot can never reach the dock.
fn agent_skills_ui_verified(inventory: &Value) -> bool {
    if inventory.get("schema_version").and_then(Value::as_u64) != Some(1) {
        return false;
    }

    inventory
        .get("plugins")
        .and_then(Value::as_array)
        .map(|plugins| {
            plugins.iter().any(|plugin| {
                plugin.get("id").and_then(Value::as_str) == Some(AGENT_SKILLS_PLUGIN_ID)
                    && plugin.get("kind").and_then(Value::as_str) == Some("builtin")
                    && plugin.get("state").and_then(Value::as_str) == Some("enabled")
                    && plugin
                        .get("ui")
                        .and_then(|ui| ui.get("verified"))
                        .and_then(Value::as_bool)
                        == Some(true)
            })
        })
        .unwrap_or(false)
}

/// Serializes a plugin failure into the stable `{"code","message"}` envelope the
/// frontend already branches on.
fn plugin_wire_error(error: PluginError) -> String {
    serde_json::to_string(&error).unwrap_or_else(|_| {
        format!(
            "{{\"code\":\"{}\",\"message\":\"plugin error could not be serialized\"}}",
            error.code
        )
    })
}

/// Show the window by windowLabel
///
/// # Arguments
/// - `app_handle` - Tauri application handle
///
/// # Example
/// ```js
/// import { invoke } from '@tauri-apps/api/core'
///
/// await invoke('show_window');
/// ```
#[command]
pub fn show_window(app_handle: tauri::AppHandle, window_label: &str) -> Result<()> {
    app_handle
        .emit("show-window", ShowWindowPayload { window_label })
        .map_err(|e| AppError::General {
            message: t!("main.failed_to_emit_event", error = e.to_string()).to_string(),
        })?;

    Ok(())
}

#[derive(serde::Serialize, Clone)]
struct UrlWindowPayload {
    url: String,
}
/// Opens a URL in a new window via event system
///
/// IMPORTANT: We use events instead of direct window creation to avoid deadlocks
/// on Windows. This is because WebView2 requires window creation to happen on the
/// main UI thread, while IPC handlers run on a different thread. Using events
/// ensures window creation occurs on the correct thread.
///
/// See: https://github.com/tauri-apps/wry/issues/583
///
/// # Arguments
/// - `app_handle` - Tauri application handle
/// - `url` - URL to open in the window
///
/// # Example
/// ```js
/// import { invoke } from '@tauri-apps/api/core'
///
/// await invoke('open_url', { url: 'https://example.com' });
/// ```
#[command]
pub async fn open_url(app_handle: tauri::AppHandle, url: String) -> Result<()> {
    app_handle
        .emit("create-url-window", UrlWindowPayload { url })
        .map_err(|e| AppError::General {
            message: t!("main.failed_to_emit_event", error = e.to_string()).to_string(),
        })?;

    Ok(())
}

/// Sync the state of the application
///
/// It is used to sync the state of the application.
///
/// # Arguments
/// - `app` - The app handle, automatically injected by Tauri
/// - `sync_type` - The type of sync to perform
/// - `label` - The window label of the sync, the available labels can be found in `src-tauri/tauri.conf.json`: app.windows[window_config_index].label
///
/// # Example
///
/// ```js
/// import { invoke } from '@tauri-apps/api/core';
///
/// await invoke('sync_state', { syncType: 'model', label: 'model' });
/// ```
#[tauri::command]
pub fn sync_state(
    app: tauri::AppHandle,
    sync_type: &str,
    window_label: &str,
    metadata: Option<Value>,
) {
    let mut payload: HashMap<String, Value> = HashMap::new();
    payload.insert("type".to_string(), Value::String(sync_type.to_string()));
    payload.insert(
        "windowLabel".to_string(),
        Value::String(window_label.to_string()),
    );
    if let Some(metadata) = metadata {
        payload.insert("metadata".to_string(), metadata);
    }

    let _ = app.emit("cs://sync-state", payload);
}

/// Toggle the always on top state of a window
///
/// Note: Only the assistant window is supported now.
///
/// # Arguments
/// - `app` - The app handle
/// - `window_label` - The label of the window
/// - `new_state` - The new state to set
///
/// # Returns
/// - `Result<bool, String>` - The new state
#[tauri::command]
pub async fn toggle_window_always_on_top(
    app: tauri::AppHandle,
    window_label: &str,
    new_state: bool,
) -> Result<bool> {
    if window_label == "assistant" || window_label == "main" || window_label == "workflow" {
        let window = app
            .get_webview_window(window_label)
            .ok_or_else(|| AppError::General {
                message: t!(
                    "main.failed_to_find_window_with_label",
                    label = window_label
                )
                .to_string(),
            })?;

        // Set always on top state
        window
            .set_always_on_top(new_state)
            .map_err(|e| AppError::General {
                message: t!(
                    "main.failed_to_set_window_always_on_top",
                    error = e.to_string()
                )
                .to_string(),
            })?;

        // Update global state
        match window_label {
            "assistant" => ASSISTANT_ALWAYS_ON_TOP.store(new_state, Ordering::Relaxed),
            "main" => MAIN_WINDOW_ALWAYS_ON_TOP.store(new_state, Ordering::Relaxed),
            "workflow" => WORKFLOW_WINDOW_ALWAYS_ON_TOP.store(new_state, Ordering::Relaxed),
            _ => {}
        }
    }

    Ok(new_state)
}

/// Get the always on top state of a window.
///
/// Note: Only the assistant window is supported now.
///
/// # Arguments
/// - `window_label` - The label of the window
///
/// # Returns
/// - `bool` - The always on top state
#[tauri::command]
pub fn get_window_always_on_top(window_label: &str) -> bool {
    crate::window::get_user_always_on_top_preference(window_label)
}

/// Opens the shared quit confirmation dialog for the window menu.
#[command]
pub fn quit_window(app: tauri::AppHandle) -> Result<()> {
    crate::exit_coordinator::request_quit(&app);
    Ok(())
}

/// Returns the pending quit prompt, when the tray opened one.
///
/// The dialog host queries this on mount, so a prompt raised while the host was
/// hidden or being recreated is delivered even though its listener did not exist
/// when the request event was emitted.
#[command]
pub fn quit_status(coordinator: State<'_, Arc<ExitCoordinator>>) -> QuitStatus {
    coordinator.status()
}

/// Cancels a matching pending prompt and returns the coordinator to idle.
#[command]
pub fn quit_cancel(
    coordinator: State<'_, Arc<ExitCoordinator>>,
    nonce: String,
    host: String,
) -> QuitStatus {
    coordinator.cancel(&nonce, &host);
    coordinator.status()
}

/// Applies one confirmation submit.
///
/// With `stop_runtime` false the desktop releases its lease (bounded) and commits
/// the exit. With it true the shared runtime is asked to stop, and the exit is
/// committed only once the runtime's own status reports `stopped`; an
/// unconfirmed or failed stop leaves a stable error code so the user can retry or
/// exit the desktop only. A duplicate or stale submit is a no-op.
#[command]
pub async fn quit_confirm(
    app: tauri::AppHandle,
    coordinator: State<'_, Arc<ExitCoordinator>>,
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    nonce: String,
    host: String,
    stop_runtime: bool,
) -> Result<QuitStatus> {
    if !stop_runtime {
        let accepted = match coordinator.phase() {
            QuitPhase::Confirming => {
                coordinator.confirm_exit_desktop(&nonce, &host) == QuitTransition::Applied
            }
            // Abandon a failed stop and take the desktop-only exit instead.
            QuitPhase::StoppingRuntime => {
                coordinator.exit_desktop_after_stop(&nonce, &host) == QuitTransition::Applied
            }
            _ => false,
        };
        if !accepted {
            return Ok(coordinator.status());
        }
        release_lease_bounded(supervisor.inner().as_ref()).await;
        crate::exit_coordinator::commit_exit(&app, coordinator.inner().as_ref());
        return Ok(coordinator.status());
    }

    let accepted = match coordinator.phase() {
        QuitPhase::Confirming => {
            coordinator.confirm_stop_runtime(&nonce, &host) == QuitTransition::Applied
        }
        // A retry clears the recorded failure and re-issues the same request.
        QuitPhase::StoppingRuntime => {
            coordinator.begin_stop_retry(&nonce, &host) == QuitTransition::Applied
        }
        _ => false,
    };
    if !accepted {
        return Ok(coordinator.status());
    }

    let supervisor = supervisor.inner().as_ref();
    // Read the live instance first and keep it as the only target, so a runtime
    // that restarted in between is never addressed as a successor.
    let identity = match supervisor.runtime_identity().await {
        Ok(identity) => identity,
        Err(error) => {
            coordinator.set_stop_failure(runtime_unavailable_code(&error));
            emit_quit_status(&app, coordinator.inner().as_ref());
            return Ok(coordinator.status());
        }
    };
    match coordinator.stop_target() {
        Some((saved, _)) if saved == identity.instance_id => {}
        Some(_) => {
            coordinator.set_stop_failure(QUIT_ERROR_RUNTIME_INSTANCE_CHANGED);
            emit_quit_status(&app, coordinator.inner().as_ref());
            return Ok(coordinator.status());
        }
        None => coordinator.set_stop_target(&identity.instance_id, identity.pid),
    }

    // The key is stable for one prompt, so a retry is idempotent at the runtime.
    let idempotency_key = format!("quit-stop-{nonce}");
    let report = supervisor
        .request_forced_runtime_shutdown(
            &identity.instance_id,
            identity.pid,
            &idempotency_key,
            QUIT_STOP_BUDGET,
        )
        .await;
    if let Some(operation_id) = report.operation_id.as_deref() {
        log::info!("[ExitCoordinator] runtime shutdown accepted as operation {operation_id}");
    }
    if report.outcome == RuntimeStopOutcome::Confirmed {
        crate::exit_coordinator::commit_exit(&app, coordinator.inner().as_ref());
    } else {
        coordinator.set_stop_failure(report.outcome.code());
        emit_quit_status(&app, coordinator.inner().as_ref());
    }
    Ok(coordinator.status())
}

/// Releases the runtime lease within a bounded budget on the desktop-only path.
///
/// A release can block on I/O, so a stuck one must not hold the exit open.
async fn release_lease_bounded(supervisor: &RuntimeSupervisor) {
    if tokio::time::timeout(QUIT_RELEASE_BUDGET, supervisor.release())
        .await
        .is_err()
    {
        log::warn!("[ExitCoordinator] releasing the runtime lease timed out");
    }
}

/// Publishes the coordinator status to the dialog host, logging a failure.
fn emit_quit_status(app: &tauri::AppHandle, coordinator: &ExitCoordinator) {
    if let Err(error) = app.emit(QUIT_STATUS_EVENT, coordinator.status()) {
        log::warn!("[ExitCoordinator] publishing the quit status failed: {error}");
    }
}

/// Maps a supervisor failure to a stable code the frontend localizes.
fn runtime_unavailable_code(error: &RuntimeUnavailable) -> &'static str {
    match error {
        RuntimeUnavailable::NotConnected => QUIT_ERROR_RUNTIME_NOT_CONNECTED,
        _ => RuntimeStopOutcome::Failed.code(),
    }
}

#[command]
pub fn set_mouse_event_state(window_label: &str, state: bool) -> Result<()> {
    if window_label == "assistant" {
        crate::constants::ON_MOUSE_EVENT.store(state, Ordering::Relaxed);
    }
    Ok(())
}

/// Move window to the left or right bottom corner of the current screen
///
/// # Arguments
/// - `app` - The app handle
/// - `window_label` - The label of the window to move
/// - `direction` - The direction to move ("left" or "right")
///
/// # Example
/// ```js
/// import { invoke } from '@tauri-apps/api/core'
///
/// // Move window to left bottom corner
/// await invoke('move_window_to_screen_edge', { windowLabel: 'main', direction: 'left' });
///
/// // Move window to right bottom corner
/// await invoke('move_window_to_screen_edge', { windowLabel: 'main', direction: 'right' });
/// ```
#[tauri::command]
pub fn move_window_to_screen_edge(
    app: tauri::AppHandle,
    window_label: &str,
    direction: &str,
) -> Result<()> {
    let window = app
        .get_webview_window(window_label)
        .ok_or_else(|| AppError::General {
            message: t!(
                "main.failed_to_find_window_with_label",
                label = window_label
            )
            .to_string(),
        })?;

    if let Ok(Some(monitor)) = window.current_monitor() {
        let monitor_size = monitor.size();
        let monitor_position = monitor.position();

        // Get current window outer size (including title bar and borders)
        let window_size = window.outer_size().map_err(|e| AppError::General {
            message: t!("main.failed_to_get_window_size", error = e.to_string()).to_string(),
        })?;

        // Get current window position to preserve the y-coordinate
        let current_pos = window.outer_position().map_err(|e| AppError::General {
            message: e.to_string(),
        })?;

        // Calculate new x position based on direction
        let new_x = match direction {
            "left" => monitor_position.x + 1,
            "right" => {
                monitor_position.x + monitor_size.width as i32 - window_size.width as i32 - 1
            }
            _ => {
                return Err(AppError::General {
                    message: t!("main.invalid_direction", direction = direction).to_string(),
                })
            }
        };

        // Keep current y-coordinate
        let new_y = current_pos.y;

        let new_position =
            tauri::Position::Physical(tauri::PhysicalPosition { x: new_x, y: new_y });

        window
            .set_position(new_position)
            .map_err(|e| AppError::General {
                message: t!("main.failed_to_set_window_position", error = e.to_string())
                    .to_string(),
            })?;

        log::debug!(
            "Moved window '{}' horizontally to {} at x: {}, y: {}",
            window_label,
            direction,
            new_x,
            new_y
        );
    } else {
        return Err(AppError::General {
            message: t!("main.failed_to_get_current_monitor").to_string(),
        });
    }

    Ok(())
}

#[tauri::command]
pub fn center_window(app: tauri::AppHandle, window_label: &str) -> Result<()> {
    let window = app
        .get_webview_window(window_label)
        .ok_or_else(|| AppError::General {
            message: t!(
                "main.failed_to_find_window_with_label",
                label = window_label
            )
            .to_string(),
        })?;

    if let Ok(Some(monitor)) = window.current_monitor() {
        let monitor_size = monitor.size();
        let monitor_pos = monitor.position();
        let window_size = window.outer_size().map_err(|e| AppError::General {
            message: e.to_string(),
        })?;
        let current_pos = window.outer_position().map_err(|e| AppError::General {
            message: e.to_string(),
        })?;

        let new_x = monitor_pos.x + (monitor_size.width as i32 - window_size.width as i32) / 2;
        let new_y = current_pos.y;

        let new_position =
            tauri::Position::Physical(tauri::PhysicalPosition { x: new_x, y: new_y });
        window
            .set_position(new_position)
            .map_err(|e| AppError::General {
                message: e.to_string(),
            })?;
    } else {
        // Fallback to default center if monitor info is not available
        window.center().map_err(|e| AppError::General {
            message: e.to_string(),
        })?;
    }

    Ok(())
}
