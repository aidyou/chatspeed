// ==========================================
// 1. Agent Configuration Commands
// ==========================================

use serde_json::{json, Value};
use std::sync::Arc;
use tauri::State;

use crate::{
    ai::interaction::chat_completion::ChatState,
    builtin_agents::load_default_shell_policy_from_resources,
    db::{agent::is_supported_sub_agent_role, Agent, MainStore},
    tools::AgentSandboxConfig,
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

fn filter_git_inspection_tools_for_role(raw: Option<String>, role: Option<&str>) -> Option<String> {
    let tools = raw
        .as_deref()
        .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|tool| {
            role == Some("child")
                || !matches!(
                    tool.as_str(),
                    crate::tools::TOOL_GIT_DIFF | crate::tools::TOOL_GIT_INSPECT
                )
        })
        .collect::<Vec<_>>();
    Some(serde_json::to_string(&tools).unwrap_or_else(|_| "[]".to_string()))
}

fn sanitize_agent_for_persistence(agent: &mut Agent) -> Result<(), String> {
    let available_tools = agent
        .available_tools
        .as_deref()
        .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .unwrap_or_default();
    let has_bash = available_tools
        .iter()
        .any(|tool| tool == crate::tools::TOOL_BASH);
    let role = agent.role.as_deref();

    if !has_bash || role == Some("child") {
        agent.auto_approve =
            filter_tool_list_json(agent.auto_approve.clone(), crate::tools::TOOL_BASH);
        agent.sandbox_config = None;
    } else if let Some(raw_config) = agent.sandbox_config.as_deref() {
        let config = AgentSandboxConfig::from_json(raw_config)
            .ok_or_else(|| "sandbox config is not valid JSON".to_string())?;
        config.validate()?;
        agent.sandbox_config = Some(
            config
                .to_json()
                .ok_or_else(|| "failed to serialize sandbox config".to_string())?,
        );
    }

    let role = agent.role.as_deref();
    if role != Some("child") {
        agent.parent_agent_id = None;
        agent.sub_agent_role = None;
    } else {
        agent.sub_agent_role = agent
            .sub_agent_role
            .take()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
    }
    agent.available_tools =
        filter_git_inspection_tools_for_role(agent.available_tools.clone(), role);
    agent.auto_approve = filter_git_inspection_tools_for_role(agent.auto_approve.clone(), role);

    if role != Some("child") {
        return Ok(());
    }

    agent.planning_prompt = None;
    agent.image_recognition_prompt = None;
    agent.available_tools =
        filter_tool_list_json(agent.available_tools.clone(), crate::tools::TOOL_BASH);
    agent.auto_approve = filter_tool_list_json(agent.auto_approve.clone(), crate::tools::TOOL_BASH);
    agent.allowed_paths = Some("[]".to_string());
    agent.shell_policy = Some("[]".to_string());
    agent.sandbox_config = None;
    agent.skill_enabled = Some(false);
    agent.selected_skills = Some("[]".to_string());

    if let Some(models) = agent.models.as_mut() {
        models.plan = None;
        models.vision = None;
        models.utility = None;
    }
    Ok(())
}

#[tauri::command]
pub async fn add_agent(
    state: State<'_, Arc<MainStore>>,
    tsid_generator: State<'_, Arc<crate::libs::tsid::TsidGenerator>>,
    mut agent: Agent,
) -> Result<String, String> {
    agent.id = tsid_generator.generate().map_err(|e| e.to_string())?;
    agent.is_system = Some(false);
    agent.version = Some(agent.version.unwrap_or(0));
    sanitize_agent_for_persistence(&mut agent)?;
    validate_sub_agent_role(&agent)?;
    let store = &*state;
    let id = store.add_agent(&agent).map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
pub async fn update_agent(state: State<'_, Arc<MainStore>>, agent: Agent) -> Result<(), String> {
    let store = &*state;
    let effective_agent =
        if let Some(existing) = store.get_agent(&agent.id).map_err(|e| e.to_string())? {
            if existing.is_system.unwrap_or(false) {
                let mut updated = agent;
                updated.id = existing.id.clone();
                updated.name = existing.name.clone();
                updated.description = existing.description.clone();
                updated.role = existing.role.clone();
                updated.parent_agent_id = existing.parent_agent_id.clone();
                updated.sub_agent_role = existing.sub_agent_role.clone();
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
    sanitize_agent_for_persistence(&mut effective_agent)?;
    validate_sub_agent_role(&effective_agent)?;
    store
        .update_agent(&effective_agent)
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn validate_sub_agent_role(agent: &Agent) -> Result<(), String> {
    if agent.role.as_deref() == Some("child") && agent.parent_agent_id.is_none() {
        return Err("Child agents must belong to a primary agent".to_string());
    }
    if let Some(role) = agent.sub_agent_role.as_deref() {
        if !is_supported_sub_agent_role(role) {
            return Err(format!("Unsupported sub-agent role: {role}"));
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn delete_agent(state: State<'_, Arc<MainStore>>, id: String) -> Result<(), String> {
    let store = &*state;
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
    state: State<'_, Arc<MainStore>>,
    id: String,
) -> Result<Option<Agent>, String> {
    let runtime = {
        let store = &*state;
        store.db_runtime().map_err(|e| e.to_string())?
    };
    MainStore::get_agent_with_runtime(runtime, id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_all_agents(state: State<'_, Arc<MainStore>>) -> Result<Vec<Agent>, String> {
    let runtime = {
        let store = &*state;
        store.db_runtime().map_err(|e| e.to_string())?
    };
    MainStore::get_all_agents_with_runtime(runtime)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_agent_order(
    state: State<'_, Arc<MainStore>>,
    agent_ids: Vec<String>,
) -> Result<(), String> {
    let runtime = {
        let store = &*state;
        store.db_runtime().map_err(|e| e.to_string())?
    };
    MainStore::update_agent_order_with_runtime(runtime, agent_ids)
        .await
        .map_err(|e| e.to_string())
}

fn git_review_tool_metadata() -> Vec<Value> {
    vec![
        json!({
            "id": crate::tools::TOOL_GIT_DIFF,
            "name": crate::tools::TOOL_GIT_DIFF,
            "category": "FileSystem",
            "scope": "workflow",
            "child_only": true
        }),
        json!({
            "id": crate::tools::TOOL_GIT_INSPECT,
            "name": crate::tools::TOOL_GIT_INSPECT,
            "category": "FileSystem",
            "scope": "workflow",
            "child_only": true
        }),
    ]
}

#[tauri::command]
pub async fn get_available_tools(chat_state: State<'_, Arc<ChatState>>) -> Result<Value, String> {
    let mut native_meta = chat_state.tool_manager.get_all_native_tool_metadata().await;
    // Git review tools are instantiated with a session PathGuard only for child workflows.
    // Expose metadata for agent configuration without globally registering executable instances.
    native_meta.extend(git_review_tool_metadata());
    native_meta.sort_by(|left, right| {
        left["id"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["id"].as_str().unwrap_or_default())
    });
    Ok(json!(native_meta))
}

#[cfg(test)]
mod tests {
    use super::{git_review_tool_metadata, sanitize_agent_for_persistence};
    use crate::db::Agent;

    #[test]
    fn git_review_metadata_is_child_only_and_unique() {
        let metadata = git_review_tool_metadata();
        assert_eq!(metadata.len(), 2);
        for id in [crate::tools::TOOL_GIT_DIFF, crate::tools::TOOL_GIT_INSPECT] {
            let matches = metadata
                .iter()
                .filter(|tool| tool["id"].as_str() == Some(id))
                .collect::<Vec<_>>();
            assert_eq!(matches.len(), 1, "{id} metadata should appear once");
            assert_eq!(matches[0]["child_only"].as_bool(), Some(true));
            assert_eq!(matches[0]["scope"].as_str(), Some("workflow"));
        }
    }

    #[test]
    fn primary_agents_cannot_persist_git_review_tools() {
        let mut agent = Agent::new(
            "primary-test".to_string(),
            "Primary Test".to_string(),
            None,
            Some("primary".to_string()),
            None,
            String::new(),
            None,
            None,
            Some(
                serde_json::json!([
                    crate::tools::TOOL_GIT_DIFF,
                    crate::tools::TOOL_GIT_INSPECT,
                    crate::tools::TOOL_READ_FILE,
                ])
                .to_string(),
            ),
            Some(
                serde_json::json!([crate::tools::TOOL_GIT_DIFF, crate::tools::TOOL_GIT_INSPECT,])
                    .to_string(),
            ),
            None,
            Some("[]".to_string()),
            Some("[]".to_string()),
            Some(false),
            Some("default".to_string()),
            Some(true),
            Some("[]".to_string()),
            Some("standard".to_string()),
            Some(false),
            Some(true),
            None,
        );

        sanitize_agent_for_persistence(&mut agent).expect("sanitize primary agent");
        let available_tools = serde_json::from_str::<Vec<String>>(
            agent.available_tools.as_deref().expect("available tools"),
        )
        .expect("available tools json");
        let auto_approve = serde_json::from_str::<Vec<String>>(
            agent.auto_approve.as_deref().expect("auto approve"),
        )
        .expect("auto approve json");
        assert_eq!(available_tools, vec![crate::tools::TOOL_READ_FILE]);
        assert!(auto_approve.is_empty());
    }

    #[test]
    fn no_shell_and_child_agents_cannot_persist_sandbox_config() {
        let sandbox_config = serde_json::json!({
            "executionMode": "sandbox_only",
            "runtimePreference": "docker",
            "defaultProfile": "busybox",
            "profiles": {
                "busybox": {
                    "enabled": true,
                    "image": "busybox:latest",
                    "network": { "mode": "none" },
                    "workspaceAccess": "read_write"
                }
            }
        })
        .to_string();

        let mut no_shell = Agent::new(
            "no-shell".to_string(),
            "No Shell".to_string(),
            None,
            Some("primary".to_string()),
            None,
            String::new(),
            None,
            None,
            Some(serde_json::json!([crate::tools::TOOL_READ_FILE]).to_string()),
            Some(serde_json::json!([crate::tools::TOOL_BASH]).to_string()),
            None,
            Some("[]".to_string()),
            Some("[]".to_string()),
            Some(false),
            Some("default".to_string()),
            Some(true),
            Some("[]".to_string()),
            Some("standard".to_string()),
            Some(false),
            Some(false),
            None,
        );
        no_shell.sandbox_config = Some(sandbox_config.clone());

        sanitize_agent_for_persistence(&mut no_shell).expect("sanitize no-shell agent");
        assert!(no_shell.sandbox_config.is_none());
        assert_eq!(
            serde_json::from_str::<Vec<String>>(no_shell.auto_approve.as_deref().unwrap())
                .expect("auto approve json"),
            Vec::<String>::new()
        );

        no_shell.available_tools = Some(serde_json::json!([crate::tools::TOOL_BASH]).to_string());
        no_shell.sandbox_config = Some(
            serde_json::json!({
                "executionMode": "auto",
                "runtimePreference": "auto",
                "defaultProfile": "busybox",
                "profiles": {
                    "busybox": {
                        "enabled": true,
                        "image": "busybox:latest",
                        "commandPatterns": ["^git(?=\\s|$)"],
                        "network": { "mode": "none" },
                        "workspaceAccess": "read_write"
                    }
                }
            })
            .to_string(),
        );
        assert!(sanitize_agent_for_persistence(&mut no_shell).is_err());

        let mut child = Agent::new(
            "child".to_string(),
            "Child".to_string(),
            None,
            Some("child".to_string()),
            Some("parent".to_string()),
            String::new(),
            Some("planning".to_string()),
            None,
            Some(serde_json::json!([crate::tools::TOOL_BASH]).to_string()),
            Some(serde_json::json!([crate::tools::TOOL_BASH]).to_string()),
            None,
            Some("[{\"pattern\":\"^git status$\",\"decision\":\"allow\"}]".to_string()),
            Some(serde_json::json!(["/tmp"]).to_string()),
            Some(false),
            Some("default".to_string()),
            Some(true),
            Some(serde_json::json!(["help"]).to_string()),
            Some("standard".to_string()),
            Some(false),
            Some(false),
            None,
        );
        child.sandbox_config = Some(sandbox_config);

        sanitize_agent_for_persistence(&mut child).expect("sanitize child agent");
        assert!(child.sandbox_config.is_none());
        assert_eq!(child.available_tools.as_deref(), Some("[]"));
        assert_eq!(child.allowed_paths.as_deref(), Some("[]"));
        assert_eq!(child.shell_policy.as_deref(), Some("[]"));
    }
}

#[tauri::command]
pub async fn get_default_shell_policy() -> Result<Value, String> {
    Ok(json!(load_default_shell_policy_from_resources()?))
}

#[tauri::command]
pub async fn get_default_image_recognition_prompt() -> Result<String, String> {
    Ok(crate::workflow::react::prompts::DEFAULT_IMAGE_RECOGNITION_PROMPT.to_string())
}
