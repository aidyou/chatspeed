//! ccproxy statistics Tauri commands.
//!
//! The ccproxy statistics tables are owned by the runtime store. These wrappers
//! only translate the Tauri wire into `/control/v1/data-commands/*` calls
//! through the [`RuntimeSupervisor`].

use std::sync::Arc;

use tauri::State;

use crate::runtime_client::RuntimeSupervisor;

/// Deletes ccproxy statistics older than the given day window.
#[tauri::command]
pub async fn delete_ccproxy_stats(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    days: i32,
) -> Result<(), String> {
    crate::runtime_data::delete_ccproxy_stats(supervisor.inner().as_ref(), days).await
}

/// Returns daily ccproxy statistics.
#[tauri::command]
pub async fn get_ccproxy_daily_stats(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    days: i32,
) -> Result<Vec<serde_json::Value>, String> {
    crate::runtime_data::get_ccproxy_daily_stats(supervisor.inner().as_ref(), days).await
}

/// Returns grouped ccproxy statistics.
#[tauri::command]
pub async fn get_ccproxy_grouped_stats(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    days: i32,
) -> Result<Vec<serde_json::Value>, String> {
    crate::runtime_data::get_ccproxy_grouped_stats(supervisor.inner().as_ref(), days).await
}

/// Returns grouped ccproxy statistics for an explicit date range.
#[tauri::command]
pub async fn get_ccproxy_grouped_stats_by_date_range(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    start_date: String,
    end_date: String,
) -> Result<Vec<serde_json::Value>, String> {
    crate::runtime_data::get_ccproxy_grouped_stats_by_date_range(
        supervisor.inner().as_ref(),
        start_date,
        end_date,
    )
    .await
}

/// Returns today's ccproxy cost statistics.
#[tauri::command]
pub async fn get_ccproxy_today_cost_stats(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
) -> Result<Vec<serde_json::Value>, String> {
    crate::runtime_data::get_ccproxy_today_cost_stats(supervisor.inner().as_ref()).await
}

/// Returns provider statistics for one date.
#[tauri::command]
pub async fn get_ccproxy_provider_stats_by_date(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    date: String,
) -> Result<Vec<serde_json::Value>, String> {
    crate::runtime_data::get_ccproxy_provider_stats_by_date(supervisor.inner().as_ref(), date).await
}

/// Returns error statistics for one date.
#[tauri::command]
pub async fn get_ccproxy_error_stats_by_date(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    date: String,
    client_model: Option<String>,
    backend_model: Option<String>,
) -> Result<Vec<serde_json::Value>, String> {
    crate::runtime_data::get_ccproxy_error_stats_by_date(
        supervisor.inner().as_ref(),
        date,
        client_model,
        backend_model,
    )
    .await
}

/// Returns model usage statistics.
#[tauri::command]
pub async fn get_ccproxy_model_usage_stats(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    days: i32,
) -> Result<Vec<serde_json::Value>, String> {
    crate::runtime_data::get_ccproxy_model_usage_stats(supervisor.inner().as_ref(), days).await
}

/// Returns model token usage statistics.
#[tauri::command]
pub async fn get_ccproxy_model_token_usage_stats(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    days: i32,
) -> Result<Vec<serde_json::Value>, String> {
    crate::runtime_data::get_ccproxy_model_token_usage_stats(supervisor.inner().as_ref(), days)
        .await
}

/// Returns error distribution statistics.
#[tauri::command]
pub async fn get_ccproxy_error_distribution_stats(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    days: i32,
) -> Result<Vec<serde_json::Value>, String> {
    crate::runtime_data::get_ccproxy_error_distribution_stats(supervisor.inner().as_ref(), days)
        .await
}

/// Returns provider token usage statistics.
#[tauri::command]
pub async fn get_ccproxy_provider_token_usage_stats(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    days: i32,
) -> Result<Vec<serde_json::Value>, String> {
    crate::runtime_data::get_ccproxy_provider_token_usage_stats(supervisor.inner().as_ref(), days)
        .await
}
