use crate::ai::interaction::chat_completion::ChatState;
use crate::db::{Agent, MainStore, Workflow, WorkflowMessage};
use crate::libs::tsid::TsidGenerator;
use crate::workflow::react::executor::WorkflowExecutor;
use crate::workflow::react::gateway::{Gateway, TauriGateway};
use crate::workflow::react::orchestrator::{BackgroundTask, SubAgentFactory, BACKGROUND_TASKS};
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};

// ==========================================
// 1. Agent Configuration Commands
// ==========================================

#[tauri::command]
pub async fn add_agent(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    agent: Agent,
) -> Result<String, String> {
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
    // Core Native Tools + Adapted MCP Tools (Dynamic Metadata from Global Manager)
    // The unified manager now returns everything in a single list.
    let native_meta = chat_state.tool_manager.get_all_native_tool_metadata().await;
    Ok(json!(native_meta))
}

// ==========================================
// 2. Workflow Session Persistence Commands
// ==========================================

#[tauri::command]
pub async fn create_workflow(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    workflow: Workflow,
) -> Result<String, String> {
    log::info!(
        "Creating workflow: id={}, agent_id={}, allowed_paths={:?}",
        workflow.id,
        workflow.agent_id,
        workflow.allowed_paths
    );
    let store = state.read().map_err(|e| e.to_string())?;
    let created = store
        .create_workflow(
            &workflow.id,
            &workflow.user_query,
            &workflow.agent_id,
            workflow.allowed_paths,
        )
        .map_err(|e| e.to_string())?;
    Ok(created.id)
}

#[tauri::command]
pub async fn list_workflows(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
) -> Result<Vec<Workflow>, String> {
    let store = state.read().map_err(|e| e.to_string())?;
    let list = store.list_workflows().map_err(|e| e.to_string())?;
    Ok(list)
}

#[tauri::command]
pub async fn delete_workflow(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    session_id: String,
) -> Result<(), String> {
    let store = state.read().map_err(|e| e.to_string())?;
    store
        .delete_workflow(&session_id)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn get_workflow_snapshot(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    session_id: String,
) -> Result<Value, String> {
    let store = state.read().map_err(|e| e.to_string())?;
    let snapshot = store
        .get_workflow_snapshot(&session_id)
        .map_err(|e| e.to_string())?;
    Ok(json!(snapshot))
}

#[tauri::command]
pub async fn update_workflow_title(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    session_id: String,
    title: String,
) -> Result<(), String> {
    let store = state.read().map_err(|e| e.to_string())?;
    store
        .update_workflow_title(&session_id, &title)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn update_workflow_status(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    session_id: String,
    status: String,
) -> Result<(), String> {
    let store = state.read().map_err(|e| e.to_string())?;
    store
        .update_workflow_status(&session_id, &status)
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ==========================================
// 3. ReAct Runtime Control Commands
// ==========================================

#[tauri::command]
pub async fn workflow_start(
    app: AppHandle,
    main_store: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    tsid_generator: State<'_, Arc<TsidGenerator>>,
    gateway: State<'_, Arc<TauriGateway>>,
    factory: State<'_, Arc<dyn SubAgentFactory>>,
    session_id: String,
    agent_id: String,
    initial_prompt: Option<String>,
) -> Result<String, String> {
    log::debug!(
        "[Workflow Command] workflow_start: session_id={}",
        session_id
    );

    let main_store = main_store.inner().clone();
    let chat_state_arc = chat_state.inner().clone();
    let tsid_generator = tsid_generator.inner().clone();
    let gateway = gateway.inner().clone();
    let factory = factory.inner().clone();
    let app_data_dir = app.path().app_data_dir().unwrap_or_default();

    let agent_config = {
        let store = main_store.read().map_err(|e| e.to_string())?;
        store
            .get_agent(&agent_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Agent {} not found", agent_id))?
    };

    let (signal_tx, signal_rx) = tokio::sync::mpsc::channel(100);
    gateway
        .register_session_tx(session_id.clone(), signal_tx)
        .await;

    let global_tool_manager = chat_state_arc.tool_manager.clone();

    let mut executor = WorkflowExecutor::new(
        session_id.clone(),
        main_store,
        chat_state_arc,
        gateway as Arc<dyn Gateway>,
        factory,
        agent_config,
        vec![std::env::current_dir().unwrap_or_default()],
        app_data_dir,
        None,
        Some(signal_rx),
        tsid_generator,
        global_tool_manager,
    );

    executor.init().await.map_err(|e| e.to_string())?;

    if let Some(prompt) = initial_prompt {
        executor
            .add_message_and_notify("user".into(), prompt, None, None, false, None, None)
            .await
            .map_err(|e| e.to_string())?;
    }

    // Register for external control
    let shared_executor = Arc::new(tokio::sync::Mutex::new(executor));
    BACKGROUND_TASKS.insert(
        session_id.clone(),
        BackgroundTask::SubAgent(shared_executor.clone()),
    );

    let session_id_for_spawn = session_id.clone();
    tokio::spawn(async move {
        log::debug!(
            "[Workflow Engine] Entering run_loop for session {}",
            session_id_for_spawn
        );
        let mut guard = shared_executor.lock().await;
        if let Err(e) = guard.run_loop().await {
            log::error!(
                "Workflow execution error in session {}: {:?}",
                session_id_for_spawn,
                e
            );
        }
        log::debug!(
            "[Workflow Engine] Exited run_loop for session {}",
            session_id_for_spawn
        );
        BACKGROUND_TASKS.remove(&session_id_for_spawn);
    });

    Ok(session_id)
}

#[tauri::command]
pub async fn workflow_signal(
    gateway: State<'_, Arc<TauriGateway>>,
    session_id: String,
    signal: String,
) -> Result<(), String> {
    gateway
        .inject_input(&session_id, signal)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workflow_stop(
    gateway: State<'_, Arc<TauriGateway>>,
    session_id: String,
) -> Result<(), String> {
    log::info!("Stopping workflow session: {}", session_id);

    // 1. Inject an asynchronous signal. This is non-blocking and doesn't require executor lock.
    let _ = gateway.inject_input(&session_id, "{\"type\": \"stop\"}".to_string()).await;

    // 2. Mark as cancelled in BACKGROUND_TASKS if it's still there (best effort)
    // We don't wait for the lock here to avoid hanging the UI.
    // The executor's run_loop will see the signal and stop itself.

    Ok(())
}

#[tauri::command]
pub async fn workflow_get_tasks() -> Result<Value, String> {
    let mut tasks = Vec::new();
    for entry in BACKGROUND_TASKS.iter() {
        let (id, task) = entry.pair();
        let task_type = match task {
            BackgroundTask::SubAgent(_) => "Agent",
            BackgroundTask::ShellCommand { .. } => "Shell",
        };
        tasks.push(json!({ "id": id, "type": task_type }));
    }
    Ok(json!(tasks))
}

// --- Legacy placeholders ---

#[tauri::command]
pub async fn add_workflow_message(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    message: WorkflowMessage,
) -> Result<WorkflowMessage, String> {
    let store = state.read().map_err(|e| e.to_string())?;
    let msg = store
        .add_workflow_message(&message)
        .map_err(|e| e.to_string())?;
    Ok(msg)
}

#[tauri::command]
pub async fn get_workflow_session_key() -> Result<String, String> {
    Ok("".into())
}

#[tauri::command]
pub async fn update_workflow_todo_list(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    session_id: String,
    todo_list: String,
) -> Result<(), String> {
    let store = state.read().map_err(|e| e.to_string())?;
    store
        .update_workflow_todo_list(&session_id, &todo_list)
        .map_err(|e| e.to_string())?;
    Ok(())
}

use crate::workflow::react::skills::{SkillManifest, SkillScanner};

#[tauri::command]
pub async fn get_system_skills(app: AppHandle) -> Result<Vec<SkillManifest>, String> {
    let app_data_dir = app.path().app_data_dir().unwrap_or_default();
    let scanner = SkillScanner::new(app_data_dir);
    let skills_map = scanner.scan().map_err(|e| e.to_string())?;
    let mut skills: Vec<SkillManifest> = skills_map.into_values().collect();
    // Sort by name for consistent UI
    skills.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(skills)
}

#[tauri::command]
pub async fn update_workflow_allowed_paths(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    session_id: String,
    allowed_paths: String,
) -> Result<(), String> {
    let store = state.read().map_err(|e| e.to_string())?;
    store
        .update_workflow_allowed_paths(&session_id, &allowed_paths)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn workflow_chat_completion() -> Result<(), String> {
    Ok(())
}

#[tauri::command]
pub async fn workflow_call_tool() -> Result<(), String> {
    Ok(())
}
