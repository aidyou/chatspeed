// ==========================================
// 1. Agent Configuration Commands
// ==========================================

use serde_json::{json, Value};
use std::sync::Arc;
use tauri::State;

use crate::{
    ai::interaction::chat_completion::ChatState,
    builtin_agents::load_default_shell_policy_from_resources,
    db::{Agent, MainStore},
};

fn filter_tool_list_json(raw: Option<String>, blocked_tool: &str) -> Option<String> {
    let tools = raw
        .as_deref()
        .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|tool| tool != blocked_tool)
        .collect::<Vec<_>>();
    Some(serde_json::to_string(&tools).unwrap_or_else(|_| "[]".to_string()))
}

fn sanitize_agent_for_persistence(agent: &mut Agent) {
    let available_tools = agent
        .available_tools
        .as_deref()
        .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .unwrap_or_default();
    let has_bash = available_tools
        .iter()
        .any(|tool| tool == crate::tools::TOOL_BASH);

    if !has_bash {
        agent.auto_approve =
            filter_tool_list_json(agent.auto_approve.clone(), crate::tools::TOOL_BASH);
    }

    if agent.role.as_deref() != Some("child") {
        return;
    }

    agent.planning_prompt = None;
    agent.image_recognition_prompt = None;
    agent.available_tools = filter_tool_list_json(agent.available_tools.clone(), crate::tools::TOOL_BASH);
    agent.auto_approve = filter_tool_list_json(agent.auto_approve.clone(), crate::tools::TOOL_BASH);
    agent.allowed_paths = Some("[]".to_string());
    agent.shell_policy = Some("[]".to_string());
    agent.skill_enabled = Some(false);
    agent.selected_skills = Some("[]".to_string());

    if let Some(models) = agent.models.as_mut() {
        models.plan = None;
        models.vision = None;
        models.utility = None;
    }
}

#[tauri::command]
pub async fn add_agent(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    tsid_generator: State<'_, Arc<crate::libs::tsid::TsidGenerator>>,
    mut agent: Agent,
) -> Result<String, String> {
    agent.id = tsid_generator.generate().map_err(|e| e.to_string())?;
    agent.is_system = Some(false);
    agent.version = Some(agent.version.unwrap_or(0));
    sanitize_agent_for_persistence(&mut agent);
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
    let effective_agent =
        if let Some(existing) = store.get_agent(&agent.id).map_err(|e| e.to_string())? {
            if existing.is_system.unwrap_or(false) {
                let mut updated = agent;
                updated.id = existing.id.clone();
                updated.name = existing.name.clone();
                updated.description = existing.description.clone();
                updated.role = existing.role.clone();
                updated.parent_agent_id = existing.parent_agent_id.clone();
                updated.system_prompt = existing.system_prompt.clone();
                updated.planning_prompt = existing.planning_prompt.clone();
                updated.is_system = existing.is_system;
                updated.version = existing.version;
                updated.sort_index = existing.sort_index;
                updated
            } else {
                let mut updated = agent;
                updated.is_system = Some(false);
                updated.version = existing.version.or(Some(0));
                updated.sort_index = existing.sort_index;
                updated
            }
        } else {
            let mut updated = agent;
            updated.is_system = Some(false);
            updated.version = Some(updated.version.unwrap_or(0));
            updated
        };
    let mut effective_agent = effective_agent;
    sanitize_agent_for_persistence(&mut effective_agent);
    store
        .update_agent(&effective_agent)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn delete_agent(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    id: String,
) -> Result<(), String> {
    let store = state.read().map_err(|e| e.to_string())?;
    if store
        .get_agent(&id)
        .map_err(|e| e.to_string())?
        .is_some_and(|agent| agent.is_system.unwrap_or(false))
    {
        return Err("System agent cannot be deleted".to_string());
    }
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
pub async fn update_agent_order(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    agent_ids: Vec<String>,
) -> Result<(), String> {
    let store = state.read().map_err(|e| e.to_string())?;
    store
        .update_agent_order(agent_ids)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn get_available_tools(chat_state: State<'_, Arc<ChatState>>) -> Result<Value, String> {
    let native_meta = chat_state.tool_manager.get_all_native_tool_metadata().await;
    Ok(json!(native_meta))
}

#[tauri::command]
pub async fn get_default_shell_policy() -> Result<Value, String> {
    Ok(json!(load_default_shell_policy_from_resources()?))
}

#[tauri::command]
pub async fn get_default_image_recognition_prompt() -> Result<String, String> {
    Ok(crate::workflow::react::prompts::DEFAULT_IMAGE_RECOGNITION_PROMPT.to_string())
}
