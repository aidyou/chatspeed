use crate::db::{MainStore, ProxyGroup};
use std::sync::{Arc, RwLock};
use tauri::{command, State};

use crate::error::{AppError, Result};

#[command]
pub fn proxy_group_list(state: State<Arc<RwLock<MainStore>>>) -> Result<Vec<ProxyGroup>> {
    let store = state.read()?;
    Ok(store.config.get_proxy_groups())
}

#[command]
pub fn proxy_group_add(state: State<Arc<RwLock<MainStore>>>, item: ProxyGroup) -> Result<i64> {
    let mut store = state.write()?;
    store.proxy_group_add(&item).map_err(AppError::Db)
}

#[command]
pub fn proxy_group_update(state: State<Arc<RwLock<MainStore>>>, item: ProxyGroup) -> Result<()> {
    let mut store = state.write()?;
    store.proxy_group_update(&item).map_err(AppError::Db)
}

#[command]
pub fn proxy_group_delete(state: State<Arc<RwLock<MainStore>>>, id: i64) -> Result<()> {
    let mut store = state.write()?;
    store.proxy_group_delete(id).map_err(AppError::Db)
}
