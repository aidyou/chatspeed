use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_json::Value;
use std::sync::{Arc, OnceLock};
use tauri::{command, AppHandle, State};

use crate::ai::interaction::chat_completion::ChatState;
use crate::ai::interaction::chat_completion::PendingWorkflow;
use crate::db::{Agent, MainStore, Workflow, WorkflowMessage, WorkflowSnapshot};
use crate::error::{AppError, Result};
use crate::libs::tsid::TsidGenerator;
use crate::tools::MCP_TOOL_NAME_SPLIT;
// use crate::workflow::dag::WorkflowEngine;

// Initialize a global TSID generator with lazy initialization to avoid startup panic
static TSID_GENERATOR: OnceLock<TsidGenerator> = OnceLock::new();

/// Gets or initializes the global TSID generator
fn get_tsid_generator() -> &'static TsidGenerator {
    TSID_GENERATOR.get_or_init(|| {
        TsidGenerator::new(1).unwrap_or_else(|e| {
            log::error!("Failed to create TSID generator with node_id 1: {}. Using fallback node_id 0.", e);
            // Fallback to node_id 0 if node_id 1 fails
            TsidGenerator::new(0).unwrap_or_else(|e2| {
                log::error!("CRITICAL: TSID generator creation failed even with node_id 0: {}", e2);
                // Last resort: create with minimal configuration
                // This panic is acceptable here as TSID is critical for workflow IDs
                panic!("Cannot initialize TSID generator: {:?}", e2)
            })
        })
    })
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
) -> Result<Vec<FrontendTool>> {
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

// #[command]
// pub async fn run_dag_workflow(app_handle: AppHandle, workflow_config: &str) -> Result<()> {
//     let engine = WorkflowEngine::new(app_handle).await?;

//     let _ = engine.execute(workflow_config).await?;

//     log::debug!(
//         "Workflow execution result: {:#?}",
//         engine.context.get_last_output().await
//     );
//     Ok(())
// }

/// Adds a new agent
#[command]
pub async fn add_agent(
    main_store: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    agent_payload: super::types::workflow::AgentPayload,
) -> Result<String> {
    let new_id = get_tsid_generator().generate().map_err(|e| AppError::General {
        message: e.to_string(),
    })?;
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
    let store = main_store.read()?;
    store.add_agent(&agent).map_err(AppError::Db)
}

/// Updates an existing agent
#[command]
pub async fn update_agent(
    main_store: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    agent_payload: super::types::workflow::AgentPayload,
) -> Result<()> {
    let agent: Agent = agent_payload.into();
    let store = main_store.read()?;
    store.update_agent(&agent).map_err(AppError::Db)
}

/// Deletes an agent
#[command]
pub async fn delete_agent(
    main_store: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    id: String,
) -> Result<()> {
    let store = main_store.read()?;
    store.delete_agent(&id).map_err(AppError::Db)
}

/// Gets an agent by ID
#[command]
pub async fn get_agent(
    main_store: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    id: String,
) -> Result<Option<Agent>> {
    let store = main_store.read()?;
    store.get_agent(&id).map_err(AppError::Db)
}

/// Gets all agents
#[command]
pub async fn get_all_agents(
    main_store: State<'_, Arc<std::sync::RwLock<MainStore>>>,
) -> Result<Vec<Agent>> {
    let store = main_store.read()?;
    store.get_all_agents().map_err(AppError::Db)
}

/// Executes a chat completion for the workflow engine, acting as a proxy.
/// It forwards tool calls from the LLM back to the frontend for execution.
#[command]
pub async fn workflow_chat_completion(
    app_handle: AppHandle,
    chat_state: State<'_, Arc<ChatState>>,
    payload: super::types::workflow::WorkflowChatPayload,
) -> Result<String> {
    let stream_id = get_tsid_generator().generate().map_err(|e| AppError::General {
        message: e.to_string(),
    })?;

    // Create the pending workflow context
    let pending_workflow = PendingWorkflow {
        app_handle,
        payload,
    };

    // Store it in the dashmap, keyed by the stream_id
    // It will be handled at @src-tauri/src/workflow/helper.rs
    chat_state
        .pending_workflow_chat_completions
        .insert(stream_id.clone(), pending_workflow);

    // Return the stream_id to the frontend
    Ok(stream_id)
}

/// Executes a tool call for the workflow engine using the ChatState tool manager
#[command]
pub async fn workflow_call_tool(
    chat_state: State<'_, Arc<ChatState>>,
    tool_name: String,
    arguments: Option<Value>,
) -> Result<Value> {
    let tool_manager = &chat_state.tool_manager;

    // Parse arguments or use empty object
    let args = arguments.unwrap_or_else(|| json!({}));

    // Execute the tool call using the tool manager
    match tool_manager.tool_call(&tool_name, args).await {
        Ok(result) => Ok(result),
        Err(e) => Err(AppError::Tool(e)),
    }
}

// =================================================
//  Workflow Database Commands
// =================================================

#[command]
pub async fn create_workflow(
    main_store: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    user_query: String,
    agent_id: String,
) -> Result<Workflow> {
    let id = get_tsid_generator().generate().map_err(|e| AppError::General {
        message: e.to_string(),
    })?;
    let store = main_store.read()?;
    store
        .create_workflow(&id, &user_query, &agent_id)
        .map_err(AppError::Db)
}

#[command]
pub async fn add_workflow_message(
    main_store: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    session_id: String,
    role: String,
    message: String,
    metadata: Option<Value>,
) -> Result<WorkflowMessage> {
    let msg = WorkflowMessage {
        id: None,
        session_id,
        role,
        message,
        metadata,
        created_at: None,
    };
    let store = main_store.read()?;
    store.add_workflow_message(&msg).map_err(AppError::Db)
}

#[command]
pub async fn update_workflow_status(
    main_store: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    workflow_id: String,
    status: String,
) -> Result<()> {
    let store = main_store.read()?;
    store
        .update_workflow_status(&workflow_id, &status)
        .map_err(AppError::Db)
}

#[command]
pub async fn get_workflow_snapshot(
    main_store: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    workflow_id: String,
) -> Result<WorkflowSnapshot> {
    let store = main_store.read()?;
    store
        .get_workflow_snapshot(&workflow_id)
        .map_err(AppError::Db)
}

#[command]
pub async fn update_workflow_title(
    main_store: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    workflow_id: String,
    title: String,
) -> Result<()> {
    let store = main_store.read()?;
    store
        .update_workflow_title(&workflow_id, &title)
        .map_err(AppError::Db)
}

#[command]
pub async fn update_workflow_todo_list(
    main_store: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    workflow_id: String,
    todo_list: String,
) -> Result<()> {
    let store = main_store.read()?;
    store
        .update_workflow_todo_list(&workflow_id, &todo_list)
        .map_err(AppError::Db)
}

#[command]
pub async fn delete_workflow(
    main_store: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    workflow_id: String,
) -> Result<()> {
    let store = main_store.read()?;
    store.delete_workflow(&workflow_id).map_err(AppError::Db)
}

#[command]
pub async fn list_workflows(
    main_store: State<'_, Arc<std::sync::RwLock<MainStore>>>,
) -> Result<Vec<Workflow>> {
    let store = main_store.read()?;
    store.list_workflows().map_err(AppError::Db)
}
