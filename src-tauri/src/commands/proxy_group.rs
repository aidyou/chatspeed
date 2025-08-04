use rust_i18n::t;
use std::sync::{Arc, Mutex};
use tauri::{command, State};

use crate::db::{MainStore, ProxyGroup};

#[command]
pub fn proxy_group_list(state: State<Arc<Mutex<MainStore>>>) -> Result<Vec<ProxyGroup>, String> {
    let store = state
        .lock()
        .map_err(|e| t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())?;
    Ok(store.config.get_proxy_groups())
}

#[command]
pub fn proxy_group_add(
    state: State<Arc<Mutex<MainStore>>>,
    item: ProxyGroup,
) -> Result<i64, String> {
    let mut store = state
        .lock()
        .map_err(|e| t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())?;
    store.proxy_group_add(&item).map_err(|e| e.to_string())
}

#[command]
pub fn proxy_group_update(
    state: State<Arc<Mutex<MainStore>>>,
    item: ProxyGroup,
) -> Result<(), String> {
    let mut store = state
        .lock()
        .map_err(|e| t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())?;
    store.proxy_group_update(&item).map_err(|e| e.to_string())
}

#[command]
pub fn proxy_group_delete(state: State<Arc<Mutex<MainStore>>>, id: i64) -> Result<(), String> {
    let mut store = state
        .lock()
        .map_err(|e| t!("db.failed_to_lock_main_store", error = e.to_string()).to_string())?;
    store.proxy_group_delete(id).map_err(|e| e.to_string())
}
