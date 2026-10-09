//! ChatHub commands.
//!
//! The `chat_hubs` table CRUD is runtime-owned data reached through
//! `/control/v1/data-commands/*`. The docked ChatHub page commands (webview
//! lifecycle) remain local desktop capabilities and never touch the store.

use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};

use crate::chat_hub;
use crate::db::ChatHub;
use crate::error::{AppError, Result as AppResult};
use crate::runtime_client::RuntimeSupervisor;
use crate::runtime_data::{ChatHubAddBody, ChatHubUpdateBody};

/// `cs://sync-state` type broadcast after every successful ChatHub mutation so
/// open windows can refresh their entry list.
const CHAT_HUB_SYNC_TYPE: &str = "chat_hubs";

fn broadcast_chat_hubs_changed(app: &AppHandle) -> AppResult<()> {
    app.emit(
        "cs://sync-state",
        serde_json::json!({
            "type": CHAT_HUB_SYNC_TYPE,
            "windowLabel": "",
        }),
    )
    .map_err(AppError::from)
}

/// Returns every ChatHub entry ordered by `sort_index`.
#[tauri::command]
pub async fn get_all_chat_hubs(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
) -> Result<Vec<ChatHub>, String> {
    crate::runtime_data::get_all_chat_hubs(supervisor.inner().as_ref()).await
}

/// Adds a ChatHub entry at the end of the list.
#[tauri::command]
pub async fn add_chat_hub(
    app: AppHandle,
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    name: String,
    logo: String,
    url: String,
) -> Result<ChatHub, String> {
    let hub = crate::runtime_data::add_chat_hub(
        supervisor.inner().as_ref(),
        ChatHubAddBody { name, logo, url },
    )
    .await?;
    broadcast_chat_hubs_changed(&app).map_err(|error| error.to_string())?;
    Ok(hub)
}

/// Updates the editable fields of an existing ChatHub entry.
#[tauri::command]
pub async fn update_chat_hub(
    app: AppHandle,
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: i64,
    name: String,
    logo: String,
    url: String,
) -> Result<ChatHub, String> {
    let hub = crate::runtime_data::update_chat_hub(
        supervisor.inner().as_ref(),
        ChatHubUpdateBody {
            id,
            name,
            logo,
            url,
        },
    )
    .await?;
    broadcast_chat_hubs_changed(&app).map_err(|error| error.to_string())?;
    Ok(hub)
}

/// Deletes a ChatHub entry, including preset entries.
#[tauri::command]
pub async fn delete_chat_hub(
    app: AppHandle,
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: i64,
) -> Result<(), String> {
    crate::runtime_data::delete_chat_hub(supervisor.inner().as_ref(), id).await?;
    broadcast_chat_hubs_changed(&app).map_err(|error| error.to_string())
}

/// Persists the full ChatHub order after a drag and drop reorder.
#[tauri::command]
pub async fn update_chat_hub_order(
    app: AppHandle,
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    hub_ids: Vec<i64>,
) -> Result<(), String> {
    crate::runtime_data::update_chat_hub_order(supervisor.inner().as_ref(), hub_ids).await?;
    broadcast_chat_hubs_changed(&app).map_err(|error| error.to_string())
}

/// Reveals one ChatHub tab at the dock rectangle the frontend measured.
///
/// The work runs on the platform main thread because it creates a real webview. Each `tab_id`
/// keeps its own webview (and its browsing session) while another tab is shown, and `bounds` is
/// the dock rectangle the frontend measured in its own layout, in logical pixels. The carrier
/// places the view at that rectangle and never resizes the window itself: how much room the window
/// makes for the dock column is decided once, by [`set_dock_width`].
///
/// `width` and `top_inset` are kept for an older caller that sends no `bounds`: they describe the
/// same right dock, and the carrier turns them into the rectangle. `corner_radius` is the radius
/// the window draws at its bottom right corner, which the carrier clips the dock column with.
#[tauri::command]
pub async fn show_chat_hub_page(
    app: AppHandle,
    url: String,
    width: f64,
    top_inset: f64,
    corner_radius: f64,
    tab_id: Option<String>,
    bounds: Option<crate::native_dock::DockBounds>,
) -> AppResult<()> {
    chat_hub::run_on_page_thread(&app, move |state, app| {
        state.show(app, &url, width, top_inset, corner_radius, tab_id, bounds)
    })
    .await
}

/// Hides every ChatHub tab while keeping their sessions alive.
#[tauri::command]
pub async fn hide_chat_hub_page(app: AppHandle) -> AppResult<()> {
    chat_hub::run_on_page_thread(&app, |state, app| state.hide(app)).await
}

/// Makes room for the right dock in the Workflow window.
///
/// `width` is the space the frontend reserves on its right for the docked views, in logical
/// pixels, and zero when the dock is closed. The window grows by the width the dock does not hold
/// yet, so the workflow UI keeps the size it had, and hands that width back when the dock goes
/// away. The window is capped at the work area of its screen and moved back onto the screen when
/// making room would push it out, so the dock can never move a part of the window out of view.
#[tauri::command]
pub async fn set_dock_width(app: AppHandle, width: f64) -> AppResult<()> {
    crate::dock_window::apply_docked_width(&app, width).await
}

/// Reloads one ChatHub tab's current page, keeping its webview and its session.
///
/// A reload keeps the page the tab is already showing, so a session, a draft or a scroll position
/// survives it; the tab is not recreated.
#[tauri::command]
pub async fn reload_chat_hub_page(app: AppHandle, tab_id: String) -> AppResult<()> {
    chat_hub::run_on_page_thread(&app, move |state, app| state.reload(app, &tab_id)).await
}

/// Closes one ChatHub tab, or every tab when no tab id is given.
///
/// Closing is the only operation that destroys a tab's webview and its browsing session.
#[tauri::command]
pub async fn destroy_chat_hub_page(app: AppHandle, tab_id: Option<String>) -> AppResult<()> {
    chat_hub::run_on_page_thread(&app, move |state, app| state.destroy(app, tab_id)).await
}

/// Tells the frontend how it has to make room for the docked page.
///
/// Every platform reserves the right dock itself, so the mode is always `reserve`: the frontend
/// measures the dock rectangle in its own layout and hands it to the carrier.
#[tauri::command]
pub fn get_chat_hub_view_mode() -> String {
    chat_hub::view_mode().to_string()
}

/// Returns the width limits the docked page accepts.
///
/// The splitter clamps a drag with the same limits as the carrier, so the frontend
/// never asks for a width the page cannot have.
#[tauri::command]
pub fn get_chat_hub_page_limits() -> chat_hub::ChatHubPageLimits {
    chat_hub::ChatHubPageLimits::current()
}
