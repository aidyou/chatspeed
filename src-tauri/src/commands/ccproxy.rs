use crate::db::MainStore;
use std::sync::Arc;
use tauri::State;

#[tauri::command]
pub async fn delete_ccproxy_stats(
    days: i32,
    main_store: State<'_, Arc<MainStore>>,
) -> Result<(), String> {
    let runtime = main_store.db_runtime().map_err(|e| e.to_string())?;
    MainStore::delete_ccproxy_stats_with_runtime(runtime, days)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_ccproxy_daily_stats(
    days: i32,
    main_store: State<'_, Arc<MainStore>>,
) -> Result<Vec<serde_json::Value>, String> {
    let runtime = main_store.db_runtime().map_err(|e| e.to_string())?;
    MainStore::get_ccproxy_daily_stats_with_runtime(runtime, days)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_ccproxy_grouped_stats(
    days: i32,
    main_store: State<'_, Arc<MainStore>>,
) -> Result<Vec<serde_json::Value>, String> {
    let runtime = main_store.db_runtime().map_err(|e| e.to_string())?;
    MainStore::get_ccproxy_grouped_stats_with_runtime(runtime, days)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_ccproxy_grouped_stats_by_date_range(
    start_date: String,
    end_date: String,
    main_store: State<'_, Arc<MainStore>>,
) -> Result<Vec<serde_json::Value>, String> {
    let runtime = main_store.db_runtime().map_err(|e| e.to_string())?;
    MainStore::get_ccproxy_grouped_stats_by_date_range_with_runtime(runtime, &start_date, &end_date)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_ccproxy_today_cost_stats(
    main_store: State<'_, Arc<MainStore>>,
) -> Result<Vec<serde_json::Value>, String> {
    let runtime = main_store.db_runtime().map_err(|e| e.to_string())?;
    MainStore::get_ccproxy_today_cost_stats_with_runtime(runtime)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_ccproxy_provider_stats_by_date(
    date: String,
    main_store: State<'_, Arc<MainStore>>,
) -> Result<Vec<serde_json::Value>, String> {
    let runtime = main_store.db_runtime().map_err(|e| e.to_string())?;
    MainStore::get_ccproxy_provider_stats_by_date_with_runtime(runtime, date)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_ccproxy_error_stats_by_date(
    date: String,
    client_model: Option<String>,
    backend_model: Option<String>,
    main_store: State<'_, Arc<MainStore>>,
) -> Result<Vec<serde_json::Value>, String> {
    let runtime = main_store.db_runtime().map_err(|e| e.to_string())?;
    MainStore::get_ccproxy_error_stats_by_date_with_runtime(
        runtime,
        date,
        client_model,
        backend_model,
    )
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_ccproxy_model_usage_stats(
    days: i32,
    main_store: State<'_, Arc<MainStore>>,
) -> Result<Vec<serde_json::Value>, String> {
    let runtime = main_store.db_runtime().map_err(|e| e.to_string())?;
    MainStore::get_ccproxy_model_usage_stats_with_runtime(runtime, days)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_ccproxy_model_token_usage_stats(
    days: i32,
    main_store: State<'_, Arc<MainStore>>,
) -> Result<Vec<serde_json::Value>, String> {
    let runtime = main_store.db_runtime().map_err(|e| e.to_string())?;
    MainStore::get_ccproxy_model_token_usage_stats_with_runtime(runtime, days)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_ccproxy_error_distribution_stats(
    days: i32,
    main_store: State<'_, Arc<MainStore>>,
) -> Result<Vec<serde_json::Value>, String> {
    let runtime = main_store.db_runtime().map_err(|e| e.to_string())?;
    MainStore::get_ccproxy_error_distribution_stats_with_runtime(runtime, days)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_ccproxy_provider_token_usage_stats(
    days: i32,
    main_store: State<'_, Arc<MainStore>>,
) -> Result<Vec<serde_json::Value>, String> {
    let runtime = main_store.db_runtime().map_err(|e| e.to_string())?;
    MainStore::get_ccproxy_provider_token_usage_stats_with_runtime(runtime, days)
        .await
        .map_err(|e| e.to_string())
}
