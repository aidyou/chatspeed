use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{command, AppHandle, State};

use crate::ai::interaction::chat_completion::ChatState;
use crate::db::{Agent, MainStore};
use crate::libs::tsid::TsidGenerator;
use crate::tools::MCP_TOOL_NAME_SPLIT;
use crate::workflow::dag::WorkflowEngine;

// Initialize a global TSID generator
lazy_static::lazy_static! {
    static ref TSID_GENERATOR: TsidGenerator = TsidGenerator::new(1).expect("Failed to create TSID generator");
}

/// A simplified tool definition for the frontend
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FrontendTool {
    pub id: String,
    pub name: String,
    pub description: String,
    pub category: String, // "Web", "MCP", etc.
}

/// Gets all available tools for the frontend
#[command]
pub async fn get_available_tools(
    chat_state: State<'_, Arc<ChatState>>,
) -> Result<Vec<FrontendTool>, String> {
    let mut frontend_tools = Vec::new();
    let tool_manager = &chat_state.tool_manager;

    // 1. Get Native Tools
    let native_tools = tool_manager.get_native_tools().await;
    for tool in native_tools {
        frontend_tools.push(FrontendTool {
            id: tool.name().to_string(),
            name: tool.name().to_string(),
            description: tool.description().to_string(),
            category: tool.category().to_string(),
        });
    }

    // 2. Get MCP Tools
    let mcp_tools = tool_manager.get_all_mcp_tools().await;
    for (server_name, tools_vec) in mcp_tools.iter() {
        for tool_decl in tools_vec {
            if tool_decl.disabled {
                continue;
            }
            frontend_tools.push(FrontendTool {
                // The unique ID for an MCP tool is its composite name
                id: format!("{}{}{}", server_name, MCP_TOOL_NAME_SPLIT, tool_decl.name),
                // For display, we can just use the short name
                name: tool_decl.name.clone(),
                description: tool_decl.description.clone(),
                category: "MCP".to_string(),
            });
        }
    }

    Ok(frontend_tools)
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AgentPayload {
    pub id: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub system_prompt: String,
    pub agent_type: String,
    pub planning_prompt: Option<String>,
    pub available_tools: Option<String>,
    pub auto_approve: Option<String>,
    pub plan_model: Option<String>,
    pub act_model: Option<String>,
    pub vision_model: Option<String>,
    pub max_contexts: Option<i32>,
}

impl From<AgentPayload> for Agent {
    fn from(payload: AgentPayload) -> Self {
        Agent::new(
            payload.id.unwrap_or_default(),
            payload.name,
            payload.description,
            payload.system_prompt,
            payload.agent_type,
            payload.planning_prompt,
            payload.available_tools,
            payload.auto_approve,
            payload.plan_model,
            payload.act_model,
            payload.vision_model,
            payload.max_contexts,
        )
    }
}

#[command]
pub async fn run_dag_workflow(app_handle: AppHandle, workflow_config: &str) -> Result<(), String> {
    let engine = WorkflowEngine::new(app_handle)
        .await
        .map_err(|e| e.to_string())?;

    let _ = engine
        .execute(workflow_config)
        .await
        .map_err(|e| e.to_string())?;

    println!(
        "Workflow execution result: {:#?}",
        engine.context.get_last_output().await
    );
    Ok(())
}

/// Adds a new agent
#[command]
pub async fn add_agent(
    main_store: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    agent_payload: AgentPayload,
) -> Result<String, String> {
    let new_id = TSID_GENERATOR.generate().map_err(|e| e.to_string())?;
    let agent = Agent::new(
        new_id,
        agent_payload.name,
        agent_payload.description,
        agent_payload.system_prompt,
        agent_payload.agent_type,
        agent_payload.planning_prompt,
        agent_payload.available_tools,
        agent_payload.auto_approve,
        agent_payload.plan_model,
        agent_payload.act_model,
        agent_payload.vision_model,
        agent_payload.max_contexts,
    );
    let store = main_store.read().map_err(|e| e.to_string())?;
    store.add_agent(&agent).map_err(|e| e.to_string())
}

/// Updates an existing agent
#[command]
pub async fn update_agent(
    main_store: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    agent_payload: AgentPayload,
) -> Result<(), String> {
    let agent: Agent = agent_payload.into();
    let store = main_store.read().map_err(|e| e.to_string())?;
    store.update_agent(&agent).map_err(|e| e.to_string())
}

/// Deletes an agent
#[command]
pub async fn delete_agent(
    main_store: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    id: String,
) -> Result<(), String> {
    let store = main_store.read().map_err(|e| e.to_string())?;
    store.delete_agent(&id).map_err(|e| e.to_string())
}

/// Gets an agent by ID
#[command]
pub async fn get_agent(
    main_store: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    id: String,
) -> Result<Option<Agent>, String> {
    let store = main_store.read().map_err(|e| e.to_string())?;
    store.get_agent(&id).map_err(|e| e.to_string())
}

/// Gets all agents
#[command]
pub async fn get_all_agents(
    main_store: State<'_, Arc<std::sync::RwLock<MainStore>>>,
) -> Result<Vec<Agent>, String> {
    let store = main_store.read().map_err(|e| e.to_string())?;
    store.get_all_agents().map_err(|e| e.to_string())
}
