use crate::db::{MainStore, ProxyGroup};
use std::sync::{Arc, RwLock};
use tauri::{command, State};

#[command]
pub fn proxy_group_list(state: State<Arc<RwLock<MainStore>>>) -> Result<Vec<ProxyGroup>, String> {
    let store = state.read().map_err(|e| e.to_string())?;
    Ok(store.config.get_proxy_groups())
}

#[command]
pub fn proxy_group_add(
    state: State<Arc<RwLock<MainStore>>>,
    item: ProxyGroup,
) -> Result<i64, String> {
    let mut store = state.write().map_err(|e| e.to_string())?;
    store.proxy_group_add(&item).map_err(|e| e.to_string())
}

#[command]
pub fn proxy_group_update(
    state: State<Arc<RwLock<MainStore>>>,
    item: ProxyGroup,
) -> Result<(), String> {
    let mut store = state.write().map_err(|e| e.to_string())?;
    store.proxy_group_update(&item).map_err(|e| e.to_string())
}

#[command]
pub fn proxy_group_delete(state: State<Arc<RwLock<MainStore>>>, id: i64) -> Result<(), String> {
    let mut store = state.write().map_err(|e| e.to_string())?;
    store.proxy_group_delete(id).map_err(|e| e.to_string())
}
