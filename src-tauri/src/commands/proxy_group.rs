use crate::db::{MainStore, ProxyGroup};
use serde_json::Value;
use std::sync::Arc;
use tauri::{command, State};

use crate::error::{AppError, Result};

#[command]
pub fn proxy_group_list(state: State<Arc<MainStore>>) -> Result<Vec<ProxyGroup>> {
    let store = &*state;
    Ok(store.config.get_proxy_groups())
}

#[command]
pub fn proxy_group_add(state: State<Arc<MainStore>>, item: ProxyGroup) -> Result<i64> {
    let store = &*state;
    store.proxy_group_add(&item).map_err(AppError::Db)
}

#[command]
pub fn proxy_group_update(state: State<Arc<MainStore>>, item: ProxyGroup) -> Result<()> {
    let store = &*state;
    store.proxy_group_update(&item).map_err(AppError::Db)
}

#[command]
pub fn proxy_group_batch_update(
    state: State<Arc<MainStore>>,
    ids: Vec<i64>,
    prompt_injection: Option<String>,
    prompt_text: Option<String>,
    tool_filter: Option<String>,
    injection_position: Option<String>,
    injection_condition: Option<String>,
    prompt_replace: Option<Value>,
) -> Result<()> {
    let store = &*state;
    store
        .proxy_group_batch_update(
            ids,
            prompt_injection,
            prompt_text,
            tool_filter,
            injection_position,
            injection_condition,
            prompt_replace,
        )
        .map_err(AppError::Db)
}

#[command]
pub fn proxy_group_delete(state: State<Arc<MainStore>>, id: i64) -> Result<()> {
    let store = &*state;
    store.proxy_group_delete(id).map_err(AppError::Db)
}

#[command]
pub fn set_active_proxy_group(state: State<Arc<MainStore>>, name: String) -> Result<()> {
    let store = &*state;
    store
        .set_config(
            crate::constants::CFG_ACTIVE_PROXY_GROUP,
            &serde_json::json!(name),
        )
        .map_err(AppError::Db)
}

#[command]
pub fn get_active_proxy_group(state: State<Arc<MainStore>>) -> Result<String> {
    let store = &*state;
    Ok(store
        .config
        .get_setting(crate::constants::CFG_ACTIVE_PROXY_GROUP)
        .and_then(|value| value.as_str().map(ToString::to_string))
        .unwrap_or_else(|| "default".to_string()))
}
