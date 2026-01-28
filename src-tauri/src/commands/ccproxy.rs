use crate::db::MainStore;
use std::sync::Arc;
use tauri::State;

#[tauri::command]
pub async fn get_ccproxy_daily_stats(
    days: i32,
    main_store: State<'_, Arc<std::sync::RwLock<MainStore>>>,
) -> Result<Vec<serde_json::Value>, String> {
    let store = main_store.read().map_err(|e| e.to_string())?;
    store.get_ccproxy_daily_stats(days).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_ccproxy_provider_stats_by_date(
    date: String,
    main_store: State<'_, Arc<std::sync::RwLock<MainStore>>>,
) -> Result<Vec<serde_json::Value>, String> {
    let store = main_store.read().map_err(|e| e.to_string())?;
    store.get_ccproxy_provider_stats_by_date(&date).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_ccproxy_error_stats_by_date(
    date: String,
    main_store: State<'_, Arc<std::sync::RwLock<MainStore>>>,
) -> Result<Vec<serde_json::Value>, String> {
    let store = main_store.read().map_err(|e| e.to_string())?;
    store.get_ccproxy_error_stats_by_date(&date).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_ccproxy_model_usage_stats(
    days: i32,
    main_store: State<'_, Arc<std::sync::RwLock<MainStore>>>,
) -> Result<Vec<serde_json::Value>, String> {
    let store = main_store.read().map_err(|e| e.to_string())?;
    store.get_ccproxy_model_usage_stats(days).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_ccproxy_error_distribution_stats(
    days: i32,
    main_store: State<'_, Arc<std::sync::RwLock<MainStore>>>,
) -> Result<Vec<serde_json::Value>, String> {
    let store = main_store.read().map_err(|e| e.to_string())?;
    store.get_ccproxy_error_distribution_stats(days).map_err(|e| e.to_string())
}
