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

/// Reveals the ChatHub page, docked to the right edge of the Workflow window.
///
/// The work runs on the platform main thread because it creates a real webview. The
/// same page is reused for every entry, so the site keeps its cookies and session
/// while navigating between entries. `corner_radius` is the radius of the rounded window
/// border the page gives back at the corners it covers, and `top_inset` is the space the
/// frontend chrome occupies, which only a carrier that stacks the page over the workflow UI
/// needs.
#[tauri::command]
pub async fn show_chat_hub_page(
    app: AppHandle,
    url: String,
    width: f64,
    top_inset: f64,
    corner_radius: f64,
) -> AppResult<()> {
    chat_hub::run_on_page_thread(&app, move |state, app| {
        state.show(app, &url, width, top_inset, corner_radius)
    })
    .await
}

/// Hides the ChatHub page while keeping its session alive.
#[tauri::command]
pub async fn hide_chat_hub_page(app: AppHandle) -> AppResult<()> {
    chat_hub::run_on_page_thread(&app, |state, app| state.hide(app)).await
}

/// Applies a new width to the docked page.
#[tauri::command]
pub async fn set_chat_hub_page_width(app: AppHandle, width: f64) -> AppResult<()> {
    chat_hub::run_on_page_thread(&app, move |state, app| state.set_width(app, width)).await
}

/// Releases the ChatHub page and the browsing session it holds.
#[tauri::command]
pub async fn destroy_chat_hub_page(app: AppHandle) -> AppResult<()> {
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
