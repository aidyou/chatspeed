//! ChatHub commands.
//!
//! These commands expose the independent `chat_hubs` table and the single ChatHub
//! page docked inside the Workflow window. They never touch workflow sessions, tasks,
//! messages or approvals: showing or hiding an entry only changes the view layer.
use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};

use crate::chat_hub;
use crate::db::{ChatHub, MainStore};
use crate::error::{AppError, Result};

/// `cs://sync-state` type broadcast after every successful ChatHub mutation so
/// open windows can refresh their entry list.
const CHAT_HUB_SYNC_TYPE: &str = "chat_hubs";

fn broadcast_chat_hubs_changed(app: &AppHandle) -> Result<()> {
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
pub fn get_all_chat_hubs(state: State<Arc<MainStore>>) -> Result<Vec<ChatHub>> {
    state.get_all_chat_hubs().map_err(AppError::from)
}

/// Adds a ChatHub entry at the end of the list.
#[tauri::command]
pub fn add_chat_hub(
    app: AppHandle,
    state: State<Arc<MainStore>>,
    name: String,
    logo: String,
    url: String,
) -> Result<ChatHub> {
    let hub = state.add_chat_hub(&name, &logo, &url)?;
    broadcast_chat_hubs_changed(&app)?;
    Ok(hub)
}

/// Updates the editable fields of an existing ChatHub entry.
#[tauri::command]
pub fn update_chat_hub(
    app: AppHandle,
    state: State<Arc<MainStore>>,
    id: i64,
    name: String,
    logo: String,
    url: String,
) -> Result<ChatHub> {
    let hub = state.update_chat_hub(id, &name, &logo, &url)?;
    broadcast_chat_hubs_changed(&app)?;
    Ok(hub)
}

/// Deletes a ChatHub entry, including preset entries.
#[tauri::command]
pub fn delete_chat_hub(app: AppHandle, state: State<Arc<MainStore>>, id: i64) -> Result<()> {
    state.delete_chat_hub(id)?;
    broadcast_chat_hubs_changed(&app)
}

/// Persists the full ChatHub order after a drag and drop reorder.
#[tauri::command]
pub fn update_chat_hub_order(
    app: AppHandle,
    state: State<Arc<MainStore>>,
    hub_ids: Vec<i64>,
) -> Result<()> {
    state.update_chat_hub_order(hub_ids)?;
    broadcast_chat_hubs_changed(&app)
}

/// Reveals the ChatHub page, docked to the right edge of the Workflow window.
///
/// The work runs on the platform main thread because it creates a real webview. The
/// same page is reused for every entry, so the site keeps its cookies and session
/// while navigating between entries. `top_inset` is the space the frontend chrome
/// occupies; only carriers that stack the page over the workflow UI use it.
#[tauri::command]
pub async fn show_chat_hub_page(
    app: AppHandle,
    url: String,
    width: f64,
    top_inset: f64,
) -> Result<()> {
    chat_hub::run_on_page_thread(&app, move |state, app| {
        state.show(app, &url, width, top_inset)
    })
    .await
}

/// Hides the ChatHub page while keeping its session alive.
#[tauri::command]
pub async fn hide_chat_hub_page(app: AppHandle) -> Result<()> {
    chat_hub::run_on_page_thread(&app, |state, app| state.hide(app)).await
}

/// Applies a new width to the docked page.
#[tauri::command]
pub async fn set_chat_hub_page_width(app: AppHandle, width: f64) -> Result<()> {
    chat_hub::run_on_page_thread(&app, move |state, app| state.set_width(app, width)).await
}

/// Releases the ChatHub page and the browsing session it holds.
#[tauri::command]
pub async fn destroy_chat_hub_page(app: AppHandle) -> Result<()> {
    chat_hub::run_on_page_thread(&app, |state, app| state.destroy(app)).await
}

/// Tells the frontend how it has to make room for the docked page.
///
/// `split` means the platform already lays both webviews out side by side, `reserve`
/// means the page is stacked over the workflow UI and the frontend keeps that space
/// free itself.
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