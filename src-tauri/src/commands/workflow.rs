use crate::ai::interaction::chat_completion::ChatState;
use crate::db::{MainStore, Workflow, WorkflowMessage};
use crate::libs::tsid::TsidGenerator;
use crate::workflow::react::gateway::{Gateway, TauriGateway};
use crate::workflow::react::orchestrator::{BackgroundTask, SubAgentFactory, BACKGROUND_TASKS};
use crate::workflow::react::types::StepType;
use chrono::{DateTime, Local};
use glob::glob;
use regex::Regex;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};

// ==========================================
// 0. Helper Functions for @mentions
// ==========================================

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{}B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1}K", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}M", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn get_file_metadata_info(path: &Path) -> String {
    let metadata = match fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return "Unknown metadata".to_string(),
    };
    let size_str = format_size(metadata.len());
    let modified: String = metadata
        .modified()
        .map(|m| {
            let dt: DateTime<Local> = m.into();
            dt.format("%b %d %H:%M").to_string()
        })
        .unwrap_or_else(|_| "Unknown".to_string());

    format!("Size: {}, Modified: {}", size_str, modified)
}

fn inject_at_mentions(prompt: &str, allowed_paths: &[String]) -> (String, String) {
    let mut attached_context = String::new();
    let re = Regex::new(r"@([^\s]+)").unwrap();

    let mut injections = Vec::new();
    let mut handled_patterns = std::collections::HashSet::new();

    for cap in re.captures_iter(prompt) {
        let pattern = &cap[1];
        if handled_patterns.contains(pattern) {
            continue;
        }
        handled_patterns.insert(pattern.to_string());

        let mut target_paths = Vec::new();
        if pattern.contains('*') || pattern.contains('?') {
            for base in allowed_paths {
                let full_pattern = Path::new(base).join(pattern).to_string_lossy().to_string();
                if let Ok(paths) = glob(&full_pattern) {
                    for entry in paths.flatten() {
                        target_paths.push((entry, pattern.to_string()));
                    }
                }
            }
        } else {
            for base in allowed_paths {
                let full_path = Path::new(base).join(pattern);
                if full_path.exists() {
                    target_paths.push((full_path, pattern.to_string()));
                    break;
                }
            }
        }

        for (path, rel_pattern) in target_paths {
            if injections.len() >= 20 {
                break;
            }

            if path.is_file() {
                let metadata = match fs::metadata(&path) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                let size = metadata.len();
                let info = get_file_metadata_info(&path);

                if size > 500 * 1024 {
                    injections.push(format!(
                        "\n<file_content path={:?}>\n[File too large to show full content]\nMetadata: {}\n</file_content>\n<SYSTEM_REMINDER>The user referenced a large file {}. Above are its basic attributes. If you need to analyze the file content, use 'read_file' or 'grep' tools to read specific parts as needed.</SYSTEM_REMINDER>",
                        rel_pattern, info, rel_pattern
                    ));
                } else {
                    match fs::read(&path) {
                        Ok(bytes) => {
                            if let Ok(content) = String::from_utf8(bytes) {
                                injections.push(format!(
                                    "\n<file_content path={:?}>\n{}\n</file_content>\n",
                                    rel_pattern, content
                                ));
                            } else {
                                injections.push(format!(
                                    "\n<file_content path={:?}>\n[Binary File or Invalid Encoding]\nMetadata: {}\n</file_content>\n<SYSTEM_REMINDER>The user referenced a binary file {} that cannot be displayed as text directly.</SYSTEM_REMINDER>",
                                    rel_pattern, info, rel_pattern
                                ));
                            }
                        }
                        Err(_) => {}
                    }
                }
            } else if path.is_dir() {
                let mut entries = Vec::new();

                // Use ignore crate to respect .gitignore
                let mut walker = ignore::WalkBuilder::new(&path);
                walker
                    .max_depth(Some(1))
                    .standard_filters(true)
                    .hidden(false);

                for result in walker.build() {
                    let entry = match result {
                        Ok(e) => e,
                        Err(_) => continue,
                    };

                    if entry.depth() == 0 {
                        continue;
                    }

                    let name = entry.file_name().to_string_lossy().to_string();
                    let name_lower = name.to_lowercase();

                    // Manual filters
                    if name == "node_modules"
                        || name == ".git"
                        || name == "__pycache__"
                        || name_lower.ends_with(".pyc")
                        || name_lower == "thumbs.db"
                        || name_lower == ".ds_store"
                    {
                        continue;
                    }

                    if let Ok(meta) = entry.metadata() {
                        let mtime = meta
                            .modified()
                            .map(|t| {
                                let dt: DateTime<Local> = t.into();
                                dt.format("%b %d %H:%M").to_string()
                            })
                            .unwrap_or_else(|_| "-".into());
                        entries.push((meta.is_dir(), name, meta.len(), mtime));
                    }
                }

                entries.sort_by(|a, b| {
                    b.0.cmp(&a.0)
                        .then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))
                });

                let entry_count = entries.len();
                entries.truncate(50);

                let mut list_str = String::new();
                for (is_dir, name, size, mtime) in entries {
                    let prefix = if is_dir { "d" } else { "-" };
                    list_str.push_str(&format!(
                        "{} {:>8} {} {}\n",
                        prefix,
                        format_size(size),
                        mtime,
                        name
                    ));
                }

                if entry_count > 50 {
                    list_str.push_str(&format!("\n... and {} more items.", entry_count - 50));
                }

                injections.push(format!(
                    "\n<list_dir path={:?}>\n{}\n</list_dir>\n",
                    rel_pattern, list_str
                ));
            }
        }
    }

    if !injections.is_empty() {
        attached_context.push_str("\n--- Attached Context ---\n");
        for inj in injections {
            attached_context.push_str(&inj);
        }
        attached_context.push_str("\n<SYSTEM_REMINDER>Above is the technical context for the files/directories referenced in your prompt. Please use this information to answer the request accurately.</SYSTEM_REMINDER>\n");
    }

    (prompt.to_string(), attached_context)
}

// ==========================================
// Workflow Session Persistence Commands
// ==========================================

#[tauri::command]
pub async fn create_workflow(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    workflow: Workflow,
) -> Result<String, String> {
    let store = state.read().map_err(|e| e.to_string())?;

    // Get agent's final_audit config to inherit
    let final_audit = store
        .get_agent(&workflow.agent_id)
        .map_err(|e| e.to_string())?
        .and_then(|agent| agent.final_audit)
        .unwrap_or(false);

    let created = store
        .create_workflow(
            &workflow.id,
            &workflow.user_query,
            &workflow.agent_id,
            workflow.allowed_paths,
            Some(final_audit),
        )
        .map_err(|e| e.to_string())?;

    // Generate and store session key for proxy authentication
    let session_key = format!("sk-{}", uuid::Uuid::new_v4());
    chat_state
        .workflow_keys
        .insert(created.id.clone(), session_key);

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
pub async fn add_workflow_message(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    message: WorkflowMessage,
) -> Result<i64, String> {
    let store = state.read().map_err(|e| e.to_string())?;
    let res = store
        .add_workflow_message(&message)
        .map_err(|e| e.to_string())?;
    Ok(res.id.unwrap_or(0))
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
    app: tauri::AppHandle,
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    tsid_generator: State<'_, Arc<TsidGenerator>>,
    gateway: State<'_, Arc<TauriGateway>>,
    factory: State<'_, Arc<dyn SubAgentFactory>>,
    session_id: String,
    agent_id: String,
    initial_prompt: Option<String>,
    planning_mode: Option<bool>,
) -> Result<String, String> {
    let main_store_arc = state.inner().clone();
    let chat_state_arc = chat_state.inner().clone();
    let tsid_generator = tsid_generator.inner().clone();
    let gateway_arc = gateway.inner().clone();
    let factory = factory.inner().clone();
    let app_data_dir = app.path().app_data_dir().unwrap_or_default();
    let planning_mode = planning_mode.unwrap_or(false);

    let (clean_prompt, attached_context, allowed_paths) = {
        let store = main_store_arc.read().map_err(|e| e.to_string())?;
        let snapshot = store
            .get_workflow_snapshot(&session_id)
            .map_err(|e| e.to_string())?;
        let wf = snapshot.workflow;

        let paths: Vec<String> = match wf.allowed_paths {
            Some(v) => {
                if v.is_array() {
                    serde_json::from_value(v).unwrap_or_default()
                } else if let Some(s) = v.as_str() {
                    // Handle string-encoded JSON array
                    serde_json::from_str(s).unwrap_or_default()
                } else {
                    Vec::new()
                }
            }
            None => Vec::new(),
        };

        let prompt = initial_prompt
            .clone()
            .unwrap_or_else(|| wf.user_query.clone());

        let (p, att) = if initial_prompt.is_some() {
            inject_at_mentions(&prompt, &paths)
        } else {
            (prompt, String::new())
        };

        (p, att, paths)
    };

    let agent_config = {
        let store = main_store_arc.read().map_err(|e| e.to_string())?;
        store
            .get_agent(&agent_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Agent {} not found", agent_id))?
    };

    let (signal_tx, signal_rx) = tokio::sync::mpsc::channel(100);
    gateway_arc
        .register_session_tx(session_id.clone(), signal_tx)
        .await;

    let global_tool_manager = chat_state_arc.tool_manager.clone();

    let allowed_roots: Vec<PathBuf> = allowed_paths
        .into_iter()
        .map(|p| {
            let path = PathBuf::from(p);
            if path.is_absolute() {
                path
            } else {
                std::env::current_dir().unwrap_or_default().join(path)
            }
        })
        .collect();

    let shared_executor: Arc<
        tokio::sync::Mutex<dyn crate::workflow::react::engine::ReActExecutor>,
    > = if planning_mode {
        Arc::new(tokio::sync::Mutex::new(
            crate::workflow::react::planners::PlanningExecutor::new(
                session_id.clone(),
                main_store_arc.clone(),
                chat_state_arc,
                gateway_arc.clone() as Arc<dyn Gateway>,
                factory,
                agent_config,
                allowed_roots,
                app_data_dir,
                None,
                Some(signal_rx),
                tsid_generator,
                global_tool_manager,
                crate::workflow::react::policy::ExecutionPolicy::planning(),
            ),
        ))
    } else {
        Arc::new(tokio::sync::Mutex::new(
            crate::workflow::react::runners::ExecutionExecutor::new(
                session_id.clone(),
                main_store_arc.clone(),
                chat_state_arc,
                gateway_arc.clone() as Arc<dyn Gateway>,
                factory,
                agent_config,
                allowed_roots,
                app_data_dir,
                None,
                Some(signal_rx),
                tsid_generator,
                global_tool_manager,
                crate::workflow::react::policy::ExecutionPolicy::standard(),
            ),
        ))
    };

    {
        let mut executor = shared_executor.lock().await;
        executor.init().await.map_err(|e| e.to_string())?;

        if initial_prompt.is_some() {
            let att_opt = if attached_context.is_empty() {
                None
            } else {
                Some(attached_context)
            };
            executor
                .add_message_and_notify(
                    "user".into(),
                    clean_prompt,
                    att_opt,
                    None,
                    None,
                    false,
                    None,
                    None,
                )
                .await
                .map_err(|e| e.to_string())?;
        }
    }

    BACKGROUND_TASKS.insert(
        session_id.clone(),
        BackgroundTask::SubAgent(shared_executor.clone()),
    );

    let session_id_for_spawn = session_id.clone();
    let gateway_for_spawn = gateway_arc.clone();
    tokio::spawn(async move {
        let mut guard = shared_executor.lock().await;
        if let Err(e) = guard.run_loop().await {
            log::error!(
                "Workflow error in session {}: {:?}",
                session_id_for_spawn,
                e
            );
            // Notify UI about the fatal crash
            let _ = gateway_for_spawn
                .send(
                    &session_id_for_spawn,
                    crate::workflow::react::types::GatewayPayload::State {
                        state: crate::workflow::react::types::WorkflowState::Error,
                    },
                )
                .await;

            let _ = gateway_for_spawn
                .send(
                    &session_id_for_spawn,
                    crate::workflow::react::types::GatewayPayload::Message {
                        role: "assistant".to_string(),
                        content: format!(
                            "Critical Error: {}\n<SYSTEM_REMINDER>A fatal error occurred in the execution engine. If this error is related to invalid tool arguments, please correct your parameters and retry. If it is a system-level issue, please inform the user about the failure.</SYSTEM_REMINDER>",
                            e
                        ),
                        reasoning: None,
                        step_type: None,
                        step_index: 0,
                        is_error: true,
                        error_type: Some("engine".to_string()),
                        metadata: None,
                    },
                )
                .await;
        }
        BACKGROUND_TASKS.remove(&session_id_for_spawn);
    });

    Ok(session_id)
}

#[tauri::command]
pub async fn workflow_approve_plan(
    app: AppHandle,
    main_store: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    tsid_generator: State<'_, Arc<TsidGenerator>>,
    gateway: State<'_, Arc<TauriGateway>>,
    factory: State<'_, Arc<dyn SubAgentFactory>>,
    session_id: String,
    agent_id: String,
    plan: String,
) -> Result<(), String> {
    let main_store_arc = main_store.inner().clone();
    let chat_state_arc = chat_state.inner().clone();
    let tsid_generator_arc = tsid_generator.inner().clone();
    let gateway_arc = gateway.inner().clone();
    let factory_arc = factory.inner().clone();
    let app_data_dir = app.path().app_data_dir().unwrap_or_default();

    let mut context = crate::workflow::react::context::ContextManager::new(
        session_id.clone(),
        main_store_arc.clone(),
        128000,
        tsid_generator_arc.clone(),
    );

    context
        .prune_for_execution(plan.clone())
        .await
        .map_err(|e| e.to_string())?;

    {
        let store = main_store_arc.read().map_err(|e| e.to_string())?;
        store
            .update_workflow_status(
                &session_id,
                &crate::workflow::react::types::WorkflowState::Thinking.to_string(),
            )
            .map_err(|e| e.to_string())?;
    }

    let planning_dir = app_data_dir.join("planning").join(&session_id);
    if planning_dir.exists() {
        let _ = std::fs::remove_dir_all(&planning_dir);
        let _ = std::fs::create_dir_all(&planning_dir);
    }

    let agent_config = {
        let store = main_store_arc.read().map_err(|e| e.to_string())?;
        store
            .get_agent(&agent_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Agent {} not found", agent_id))?
    };

    let (signal_tx, signal_rx) = tokio::sync::mpsc::channel(100);
    gateway_arc
        .register_session_tx(session_id.clone(), signal_tx)
        .await;

    let allowed_paths = {
        let store = main_store_arc.read().map_err(|e| e.to_string())?;
        let snapshot = store
            .get_workflow_snapshot(&session_id)
            .map_err(|e| e.to_string())?;
        let wf = snapshot.workflow;
        match wf.allowed_paths {
            Some(v) => {
                if v.is_array() {
                    serde_json::from_value::<Vec<String>>(v).unwrap_or_default()
                } else if let Some(s) = v.as_str() {
                    serde_json::from_str(s).unwrap_or_default()
                } else {
                    Vec::new()
                }
            }
            None => Vec::new(),
        }
    };

    let global_tool_manager = chat_state_arc.tool_manager.clone();

    let allowed_roots: Vec<PathBuf> = allowed_paths
        .into_iter()
        .map(|p| {
            let path = PathBuf::from(p);
            if path.is_absolute() {
                path
            } else {
                std::env::current_dir().unwrap_or_default().join(path)
            }
        })
        .collect();

    // Transition to Execution/Implementation
    let executor_obj = crate::workflow::react::runners::ExecutionExecutor::new(
        session_id.clone(),
        main_store_arc.clone(),
        chat_state_arc,
        gateway_arc.clone() as Arc<dyn Gateway>,
        factory_arc,
        agent_config,
        allowed_roots,
        app_data_dir,
        None,
        Some(signal_rx),
        tsid_generator_arc,
        global_tool_manager,
        crate::workflow::react::policy::ExecutionPolicy::implementation(),
    );

    // [Bug Fix] Correctly re-register tools for Implementation phase
    {
        // Tools are registered during executor.init() which is called below
    }

    let shared_executor: Arc<
        tokio::sync::Mutex<dyn crate::workflow::react::engine::ReActExecutor>,
    > = Arc::new(tokio::sync::Mutex::new(executor_obj));

    {
        let mut executor = shared_executor.lock().await;
        executor.init().await.map_err(|e| e.to_string())?;

        // Prune history and set the approved plan as anchor
        let mut executor_guard = executor;
        // The implementation runner handles pruning during its own internal initialization if needed,
        // but here we force the transition context

        // Use the plan
        executor_guard.add_message_and_notify(
            "user".into(),
            format!("The plan has been approved. Please proceed with execution.\n\nApproved Plan:\n{}", plan),
            None,
            None,
            Some(StepType::Observe),
            false,
            None,
            None,
        ).await.map_err(|e| e.to_string())?;

        // Re-sync pruned history to the UI
        for msg in executor_guard.messages() {
            let _ = gateway_arc
                .send(
                    &session_id,
                    crate::workflow::react::types::GatewayPayload::Message {
                        role: msg.role.clone(),
                        content: msg.message.clone(),
                        reasoning: msg.reasoning.clone(),
                        step_type: msg.step_type.as_ref().and_then(|s| s.parse().ok()),
                        step_index: msg.step_index,
                        is_error: msg.is_error,
                        error_type: msg.error_type.clone(),
                        metadata: msg.metadata.clone(),
                    },
                )
                .await;
        }
    }

    BACKGROUND_TASKS.insert(
        session_id.clone(),
        BackgroundTask::SubAgent(shared_executor.clone()),
    );

    let session_id_for_spawn = session_id.clone();
    let gateway_for_spawn = gateway_arc.clone();
    tokio::spawn(async move {
        let mut guard = shared_executor.lock().await;
        if let Err(e) = guard.run_loop().await {
            log::error!(
                "Workflow error in session {}: {:?}",
                session_id_for_spawn,
                e
            );
            // Notify UI about the fatal crash
            let _ = gateway_for_spawn
                .send(
                    &session_id_for_spawn,
                    crate::workflow::react::types::GatewayPayload::State {
                        state: crate::workflow::react::types::WorkflowState::Error,
                    },
                )
                .await;

            let _ = gateway_for_spawn
                .send(
                    &session_id_for_spawn,
                    crate::workflow::react::types::GatewayPayload::Message {
                        role: "assistant".to_string(),
                        content: format!(
                            "Critical Error: {}\n<SYSTEM_REMINDER>A fatal error occurred in the execution engine. If this error is related to invalid tool arguments, please correct your parameters and retry. If it is a system-level issue, please inform the user about the failure.</SYSTEM_REMINDER>",
                            e
                        ),
                        reasoning: None,
                        step_type: None,
                        step_index: 0,
                        is_error: true,
                        error_type: Some("engine".to_string()),
                        metadata: None,
                    },
                )
                .await;
        }
        BACKGROUND_TASKS.remove(&session_id_for_spawn);
    });

    Ok(())
}

#[tauri::command]
pub async fn workflow_signal(
    app: tauri::AppHandle,
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    tsid_generator: State<'_, Arc<TsidGenerator>>,
    gateway: State<'_, Arc<TauriGateway>>,
    factory: State<'_, Arc<dyn SubAgentFactory>>,
    session_id: String,
    signal: String,
) -> Result<String, String> {
    let gateway_arc = gateway.inner().clone();

    // Try to inject signal into existing running loop
    match gateway_arc.inject_input(&session_id, signal.clone()).await {
        Ok(_) => Ok("Signal injected".to_string()),
        Err(_) => {
            // If injection fails, the engine is likely not running (e.g. after a restart)
            // If the signal is user_input, we can resume the workflow by starting it again
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&signal) {
                if val["type"] == "user_input" {
                    if let Some(content) = val["content"].as_str() {
                        log::info!("[Workflow] No active channel for session {}, attempting to resume with new input", session_id);

                        // Find the agent ID for this session from DB
                        let agent_id = {
                            let store = state.read().map_err(|e| e.to_string())?;
                            let snapshot = store
                                .get_workflow_snapshot(&session_id)
                                .map_err(|e| e.to_string())?;
                            snapshot.workflow.agent_id
                        };

                        // Use our robust workflow_start logic to resume
                        workflow_start(
                            app,
                            state,
                            chat_state,
                            tsid_generator,
                            gateway,
                            factory,
                            session_id,
                            agent_id,
                            Some(content.to_string()),
                            None,
                        )
                        .await?;

                        return Ok("Workflow resumed with input".to_string());
                    }
                }
            }
            Err(format!(
                "Failed to send signal: No active session for {}",
                session_id
            ))
        }
    }
}

#[tauri::command]
pub async fn workflow_stop(
    gateway: State<'_, Arc<TauriGateway>>,
    session_id: String,
) -> Result<(), String> {
    let gateway_arc = gateway.inner().clone();
    gateway_arc
        .inject_input(&session_id, "{\"type\": \"stop\"}".to_string())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn workflow_get_tasks(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    session_id: String,
) -> Result<Vec<Value>, String> {
    let store = state.read().map_err(|e| e.to_string())?;
    store
        .get_todo_list_for_workflow(&session_id)
        .map_err(|e| e.to_string())
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
        .map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
pub struct WorkspaceFile {
    pub name: String,
    pub relative_path: String,
    pub path: String,
    pub is_directory: bool,
    pub score: i32,
}

#[tauri::command]
pub async fn search_workspace_files(
    paths: Vec<String>,
    query: String,
) -> Result<Vec<WorkspaceFile>, String> {
    let mut results = vec![];
    let query_lower = query.to_lowercase();

    for root_str in paths {
        let base = PathBuf::from(&root_str);
        if !base.exists() {
            continue;
        }

        let walker = ignore::WalkBuilder::new(&base)
            .standard_filters(true)
            .hidden(false)
            .build();

        for result in walker {
            let entry = match result {
                Ok(e) => e,
                Err(_) => continue,
            };

            let path = entry.path().to_path_buf();
            let name = entry.file_name().to_string_lossy().to_string();
            let name_lower = name.to_lowercase();

            // Manual filters for common unwanted items
            if name == "node_modules"
                || name == ".git"
                || name == "__pycache__"
                || name_lower.ends_with(".pyc")
                || name_lower == "thumbs.db"
                || name_lower == ".ds_store"
            {
                continue;
            }

            let rel_path = path
                .strip_prefix(&base)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();

            let mut score = 0;
            if !query_lower.is_empty() {
                if name_lower == query_lower {
                    score += 100;
                } else if name_lower.starts_with(&query_lower) {
                    score += 50;
                } else if name_lower.contains(&query_lower) {
                    score += 20;
                } else if rel_path.to_lowercase().contains(&query_lower) {
                    score += 10;
                } else {
                    continue;
                }
            } else {
                score = 1;
            }

            let depth = path
                .components()
                .count()
                .saturating_sub(base.components().count());
            score -= (depth as i32) * 2;

            if let Some(ext) = path
                .extension()
                .and_then(|s| s.to_str().map(|s| s.to_lowercase()))
            {
                match ext.as_str() {
                    "bash" | "c" | "cpp" | "css" | "go" | "groovy" | "h" | "hpp" | "htm"
                    | "html" | "ini" | "java" | "js" | "jsx" | "json" | "kotlin" | "less"
                    | "lua" | "perl" | "php" | "plsql" | "py" | "r" | "rs" | "ruby" | "scala"
                    | "sass" | "scss" | "sh" | "sql" | "stylus" | "swift" | "toml" | "ts"
                    | "tsx" | "vue" | "xml" | "yaml" | "yml" => {
                        score += 5;
                    }
                    "md" | "txt" | "csv" | "tsv" | "log" | "rst" | "readme" => {
                        score += 5;
                    }
                    "dockerfile" | "dockerignore" | "gitignore" | "gitattributes" | "npmrc"
                    | "yarnrc" | "babelrc" | "eslintrc" | "prettierrc" | "webpack.config"
                    | "vite.config" | "rollup.config" | "tsconfig" | "jsconfig" | "makefile"
                    | "cmake" | "gradle" => {
                        score += 3;
                    }
                    _ => {}
                }
            }

            let is_dir = path.is_dir();
            if is_dir {
                score += 5;
            }

            results.push(WorkspaceFile {
                name,
                relative_path: rel_path,
                path: path.to_string_lossy().to_string(),
                is_directory: is_dir,
                score,
            });

            if results.len() > 1000 {
                break;
            }
        }
    }

    results.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.relative_path.cmp(&b.relative_path))
    });
    results.truncate(50);
    Ok(results)
}

use crate::workflow::react::skills::{SkillManifest, SkillScanner};

#[tauri::command]
pub async fn get_system_skills(app: AppHandle) -> Result<Vec<SkillManifest>, String> {
    let app_data_dir = app.path().app_data_dir().unwrap_or_default();
    let scanner = SkillScanner::new(app_data_dir);
    let skills_map = scanner.scan().map_err(|e| e.to_string())?;
    let mut skills: Vec<SkillManifest> = skills_map.into_values().collect();
    skills.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(skills)
}

#[tauri::command]
pub async fn update_workflow_allowed_paths(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    session_id: String,
    allowed_paths: Value,
) -> Result<(), String> {
    let store = state.read().map_err(|e| e.to_string())?;
    // Ensure we store it as a proper JSON string in DB
    let paths_json = if allowed_paths.is_string() {
        allowed_paths.as_str().unwrap_or("[]").to_string()
    } else {
        serde_json::to_string(&allowed_paths).unwrap_or_else(|_| "[]".to_string())
    };
    store
        .update_workflow_allowed_paths(&session_id, &paths_json)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn get_workflow_session_key(
    chat_state: State<'_, Arc<ChatState>>,
    workflow_id: String,
) -> Result<String, String> {
    chat_state
        .workflow_keys
        .get(&workflow_id)
        .map(|v| v.clone())
        .ok_or_else(|| format!("Session key for workflow {} not found", workflow_id))
}

#[tauri::command]
pub async fn update_workflow_final_audit(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    session_id: String,
    final_audit: bool,
) -> Result<(), String> {
    let store = state.read().map_err(|e| e.to_string())?;
    store
        .update_workflow_final_audit(&session_id, final_audit)
        .map_err(|e| e.to_string())?;
    Ok(())
}
