// ==========================================
// 1. Agent Configuration Commands
// ==========================================

use serde_json::{json, Value};
use std::sync::Arc;
use tauri::State;

use crate::{
    ai::interaction::chat_completion::ChatState,
    db::{Agent, MainStore},
};

#[tauri::command]
pub async fn add_agent(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    tsid_generator: State<'_, Arc<crate::libs::tsid::TsidGenerator>>,
    mut agent: Agent,
) -> Result<String, String> {
    agent.id = tsid_generator.generate().map_err(|e| e.to_string())?;
    let store = state.read().map_err(|e| e.to_string())?;
    let id = store.add_agent(&agent).map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
pub async fn update_agent(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    agent: Agent,
) -> Result<(), String> {
    let store = state.read().map_err(|e| e.to_string())?;
    store.update_agent(&agent).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn delete_agent(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    id: String,
) -> Result<(), String> {
    let store = state.read().map_err(|e| e.to_string())?;
    store.delete_agent(&id).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn get_agent(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    id: String,
) -> Result<Option<Agent>, String> {
    let store = state.read().map_err(|e| e.to_string())?;
    let agent = store.get_agent(&id).map_err(|e| e.to_string())?;
    Ok(agent)
}

#[tauri::command]
pub async fn get_all_agents(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
) -> Result<Vec<Agent>, String> {
    let store = state.read().map_err(|e| e.to_string())?;
    let agents = store.get_all_agents().map_err(|e| e.to_string())?;
    Ok(agents)
}

#[tauri::command]
pub async fn get_available_tools(chat_state: State<'_, Arc<ChatState>>) -> Result<Value, String> {
    let native_meta = chat_state.tool_manager.get_all_native_tool_metadata().await;
    Ok(json!(native_meta))
}
