// Agent configuration Tauri commands.
//
// The agent table CRUD is already served over `/control/v1/agents` (see
// `runtime_agent`). Agent ordering and the native tool metadata are runtime-owned
// data reached through `/control/v1/data-commands/*`. Only static resource
// helpers (default shell policy, default image prompt) stay local.

use serde_json::{json, Value};
use std::sync::Arc;
use tauri::State;

use crate::{
    builtin_agents::load_default_shell_policy_from_resources, db::Agent,
    runtime_client::RuntimeSupervisor,
};

#[tauri::command]
pub async fn add_agent(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    agent: Agent,
) -> Result<String, String> {
    crate::runtime_agent::add_agent(supervisor.inner().as_ref(), agent).await
}

#[tauri::command]
pub async fn update_agent(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    agent: Agent,
) -> Result<(), String> {
    crate::runtime_agent::update_agent(supervisor.inner().as_ref(), agent).await
}

#[tauri::command]
pub async fn delete_agent(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: String,
) -> Result<(), String> {
    crate::runtime_agent::delete_agent(supervisor.inner().as_ref(), &id).await
}

#[tauri::command]
pub async fn get_agent(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    id: String,
) -> Result<Option<Agent>, String> {
    crate::runtime_agent::get_agent(supervisor.inner().as_ref(), &id).await
}

#[tauri::command]
pub async fn get_all_agents(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
) -> Result<Vec<Agent>, String> {
    crate::runtime_agent::list_agents(supervisor.inner().as_ref()).await
}

/// Persists the agent sort order.
#[tauri::command]
pub async fn update_agent_order(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
    agent_ids: Vec<String>,
) -> Result<(), String> {
    crate::runtime_data::update_agent_order(supervisor.inner().as_ref(), agent_ids).await
}

/// Returns the native tool metadata the agent configuration UI offers.
#[tauri::command]
pub async fn get_available_tools(
    supervisor: State<'_, Arc<RuntimeSupervisor>>,
) -> Result<Value, String> {
    crate::runtime_data::get_available_tools(supervisor.inner().as_ref()).await
}

#[tauri::command]
pub async fn get_default_shell_policy() -> Result<Value, String> {
    Ok(json!(load_default_shell_policy_from_resources()?))
}

#[tauri::command]
pub async fn get_default_image_recognition_prompt() -> Result<String, String> {
    Ok(crate::workflow::react::prompts::DEFAULT_IMAGE_RECOGNITION_PROMPT.to_string())
}
