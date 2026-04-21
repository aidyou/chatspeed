use crate::ai::interaction::chat_completion::ChatState;
use crate::db::{Agent, AgentConfig, MainStore, Workflow, WorkflowMessage};
use crate::libs::tsid::TsidGenerator;
use crate::workflow::react::child_tasks::get_sub_agent_registry;
use crate::workflow::react::dispatcher::{Dispatcher, DispatcherMetricsSnapshot};
use crate::workflow::react::events::WorkflowEvent;
use crate::workflow::react::gateway::{Gateway, TauriGateway};
use crate::workflow::react::manager::{ManagedSessionStatus, WorkflowManager};
use crate::workflow::react::orchestrator::{
    list_background_task_ids_for_owner, stop_background_task, BackgroundTask, SubAgentFactory,
    BACKGROUND_TASKS,
};
use crate::workflow::react::replay::{restore_execution_context, RecoveryResult};
use crate::workflow::react::signals::SignalType;
use crate::workflow::react::types::{
    ExecutionContext, RuntimeState, StepType, SubAgentCompletion, WaitReason, WorkflowSignal,
    WorkflowState,
};
use chrono::{DateTime, Local};
use glob::glob;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};

// ==========================================
// 0. Helper Functions for @mentions
// ==========================================

fn merge_ui_workflow_messages(messages: &[WorkflowMessage]) -> Vec<WorkflowMessage> {
    let mut latest_tool_message_index = std::collections::HashMap::<String, usize>::new();

    for (idx, message) in messages.iter().enumerate() {
        let Some(meta) = message.metadata.as_ref() else {
            continue;
        };
        let Some(tool_call_id) = meta.get("tool_call_id").and_then(|v| v.as_str()) else {
            continue;
        };

        if message.role == "tool"
            || (message.role == "user" && message.step_type.as_deref() == Some("observe"))
        {
            latest_tool_message_index.insert(tool_call_id.to_string(), idx);
        }
    }

    let mut merged = Vec::with_capacity(messages.len());

    for (idx, message) in messages.iter().enumerate() {
        let mut next_message = message.clone();

        if let Some(meta) = next_message.metadata.as_ref() {
            if let Some(tool_call_id) = meta.get("tool_call_id").and_then(|v| v.as_str()) {
                if (next_message.role == "tool"
                    || (next_message.role == "user"
                        && next_message.step_type.as_deref() == Some("observe")))
                    && latest_tool_message_index
                        .get(tool_call_id)
                        .is_some_and(|latest_idx| *latest_idx != idx)
                {
                    continue;
                }
            }
        }

        if next_message.role == "assistant" {
            if let Some(meta) = next_message.metadata.as_mut() {
                if let Some(tool_calls) = meta.get_mut("tool_calls").and_then(|v| v.as_array_mut())
                {
                    tool_calls.retain(|call| {
                        let call_id = call
                            .get("id")
                            .and_then(|v| v.as_str())
                            .or_else(|| call.get("tool_call_id").and_then(|v| v.as_str()));

                        match call_id {
                            Some(id) => !latest_tool_message_index.contains_key(id),
                            None => true,
                        }
                    });

                    if tool_calls.is_empty() {
                        meta.as_object_mut().map(|obj| obj.remove("tool_calls"));
                    }
                }
            }

            let has_text = !next_message.message.trim().is_empty()
                || next_message
                    .reasoning
                    .as_ref()
                    .is_some_and(|reasoning| !reasoning.trim().is_empty());
            let has_tool_calls = next_message
                .metadata
                .as_ref()
                .and_then(|meta| meta.get("tool_calls"))
                .and_then(|v| v.as_array())
                .is_some_and(|calls| !calls.is_empty());

            if !has_text && !has_tool_calls {
                continue;
            }
        }

        merged.push(next_message);
    }

    merged
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{}B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1}K", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}M", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Efficiently count lines using BufReader streaming (no full content load)
/// Returns None if file cannot be read or is binary
fn count_lines(path: &Path) -> Option<u64> {
    let file = fs::File::open(path).ok()?;
    let reader = BufReader::new(file);
    // BufReader::lines() handles various line endings (\n, \r\n)
    Some(reader.lines().count() as u64)
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

    let lines_str = count_lines(path)
        .map(|n| format!("{} lines", n))
        .unwrap_or_else(|| "Unknown lines".to_string());

    format!(
        "Size: {}, Lines: {}, Modified: {}",
        size_str, lines_str, modified
    )
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

                if size > 10 * 1024 {
                    injections.push(format!(
                        "\n<file_content path={:?}>\n[File too large to show full content]\n{}\n</file_content>\n<SYSTEM_REMINDER>The user referenced a large file {}. Use 'read_file' tool with 'offset' and 'limit' parameters to read specific line ranges (e.g., offset=1, limit=100 for lines 1-100). Use 'grep' tool to search for specific patterns.</SYSTEM_REMINDER>",
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

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkflowRequest {
    pub user_query: Option<String>, // Allow empty for new workflow creation
    pub agent_id: String,
    pub allowed_paths: Option<Value>,
    pub final_audit: Option<bool>,
    pub inherited_agent_config: Option<String>,
}

fn build_agent_config_from_agent(
    agent: &Agent,
    allowed_paths: Option<&Value>,
    final_audit: Option<bool>,
) -> AgentConfig {
    let mut config = AgentConfig::default();

    config.models = agent.models.clone();
    config.max_contexts = agent.max_contexts;

    if let Some(policy_str) = &agent.shell_policy {
        config.shell_policy = serde_json::from_str(policy_str).ok();
    }

    if let Some(val) = allowed_paths {
        config.allowed_paths = serde_json::from_value::<Vec<String>>(val.clone()).ok();
    } else if let Some(paths_str) = &agent.allowed_paths {
        config.allowed_paths = serde_json::from_str(paths_str).ok();
    }

    config.final_audit = Some(final_audit.unwrap_or(agent.final_audit.unwrap_or(false)));

    if let Some(approve_str) = &agent.auto_approve {
        config.auto_approve = serde_json::from_str(approve_str).ok();
    }

    config.approval_level = agent.approval_level.clone();

    if let Some(tools_str) = &agent.available_tools {
        config.available_tools = serde_json::from_str(tools_str).ok();
    }

    config
}

fn agent_shell_policy_value(agent: &Agent) -> Option<Value> {
    agent
        .shell_policy
        .as_deref()
        .and_then(|policy| serde_json::from_str::<Value>(policy).ok())
}

fn agent_config_to_json_with_agent_shell_policy(
    config: &AgentConfig,
    agent: &Agent,
) -> Result<String, String> {
    let mut value = serde_json::to_value(config).map_err(|e| e.to_string())?;
    if let Some(policy) = agent_shell_policy_value(agent) {
        if let Some(obj) = value.as_object_mut() {
            obj.insert("shellPolicy".to_string(), policy);
        }
    }
    serde_json::to_string(&value).map_err(|e| e.to_string())
}

fn fill_missing_agent_config_fields(config: &mut AgentConfig, agent: &Agent) -> bool {
    let defaults = build_agent_config_from_agent(agent, None, None);
    let mut changed = false;

    if config.allowed_paths.is_none() && defaults.allowed_paths.is_some() {
        config.allowed_paths = defaults.allowed_paths;
        changed = true;
    }
    if config.shell_policy.is_none() && defaults.shell_policy.is_some() {
        config.shell_policy = defaults.shell_policy;
        changed = true;
    }
    if config.auto_approve.is_none() && defaults.auto_approve.is_some() {
        config.auto_approve = defaults.auto_approve;
        changed = true;
    }
    if config.available_tools.is_none() && defaults.available_tools.is_some() {
        config.available_tools = defaults.available_tools;
        changed = true;
    }
    if config.models.is_none() && defaults.models.is_some() {
        config.models = defaults.models;
        changed = true;
    }
    if config.max_contexts.is_none() && defaults.max_contexts.is_some() {
        config.max_contexts = defaults.max_contexts;
        changed = true;
    }

    changed
}

fn normalize_workflow_agent_config(
    store: &MainStore,
    workflow: &mut Workflow,
) -> Result<(), String> {
    let agent = store
        .get_agent(&workflow.agent_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Agent {} not found", workflow.agent_id))?;

    let mut config = workflow
        .agent_config
        .as_deref()
        .and_then(AgentConfig::from_json)
        .unwrap_or_default();

    let shell_policy_missing =
        config.shell_policy.is_none() && agent_shell_policy_value(&agent).is_some();
    let missing_fields_filled = fill_missing_agent_config_fields(&mut config, &agent);

    if missing_fields_filled || shell_policy_missing {
        let normalized_config = agent_config_to_json_with_agent_shell_policy(&config, &agent)?;
        store
            .update_workflow_agent_config(
                workflow.id.as_deref().unwrap_or_default(),
                &normalized_config,
            )
            .map_err(|e| e.to_string())?;
        workflow.agent_config = Some(normalized_config);
    }

    Ok(())
}

#[tauri::command]
pub async fn create_workflow(
    tsid_generator: State<'_, Arc<TsidGenerator>>,
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    request: CreateWorkflowRequest,
) -> Result<String, String> {
    let store = state.read().map_err(|e| e.to_string())?;

    // Always use TSID for new workflow sessions
    let session_id = tsid_generator.generate().map_err(|e| e.to_string())?;

    log::info!(
        "[Workflow][session={}][phase=create] Creating workflow for agent_id={}",
        session_id,
        request.agent_id
    );

    // Get agent to construct initial agent_config
    let agent = store
        .get_agent(&request.agent_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Agent {} not found", request.agent_id))?;

    if agent.role.as_deref() == Some("child") {
        return Err("Child agents cannot be used as top-level workflow agents".to_string());
    }

    // Build agent config from agent defaults.
    let mut config =
        build_agent_config_from_agent(&agent, request.allowed_paths.as_ref(), request.final_audit);

    // Merge inherited config if provided.
    if let Some(inherited) = &request.inherited_agent_config {
        if let Some(inherited_config) = AgentConfig::from_json(inherited) {
            // Validate and merge models
            if let Some(models) = &inherited_config.models {
                let mut validated_models = models.clone();
                // Preserve per-role model parameters from the inherited config.
                // `temperature < 0` and `max_tokens <= 0` are valid sentinels in this
                // codebase for "off/unset", so do not erase them during merge.
                for model in [&mut validated_models.plan, &mut validated_models.act] {
                    if let Some(m) = model {
                        if let Some(temp) = m.temperature {
                            m.temperature = Some(if temp < 0.0 {
                                -0.1
                            } else {
                                temp.clamp(0.0, 2.0)
                            });
                        }
                        if let Some(ctx) = m.context_size {
                            m.context_size = Some(ctx.clamp(1024, 2_000_000));
                        }
                        if let Some(max_tokens) = m.max_tokens {
                            m.max_tokens = Some(max_tokens.max(0));
                        }
                    }
                }
                config.models = Some(validated_models);
            }

            // Merge other fields from inherited config
            config.merge_from(&inherited_config);
        }
    }

    let agent_config_json = config.to_json();

    // Use empty string for user_query if not provided (new workflow creation)
    let user_query = request.user_query.as_deref().unwrap_or("");

    store
        .create_workflow(
            &session_id,
            user_query,
            &request.agent_id,
            Some(agent_config_json),
            None,
        )
        .map_err(|e| e.to_string())?;

    // Generate and store session key for proxy authentication
    let session_key = format!("sk-{}", uuid::Uuid::new_v4());
    chat_state
        .workflow_keys
        .insert(session_id.clone(), session_key);

    log::info!(
        "[Workflow][session={}][phase=create] Workflow created successfully, agent_id={}",
        session_id,
        request.agent_id
    );

    Ok(session_id)
}

#[tauri::command]
pub async fn list_workflows(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
) -> Result<Vec<Workflow>, String> {
    {
        let store = state.write().map_err(|e| e.to_string())?;
        reconcile_interrupted_child_workflows(&store).map_err(|e| e.to_string())?;
    }

    let store = state.read().map_err(|e| e.to_string())?;
    let list = store.list_workflows().map_err(|e| e.to_string())?;
    Ok(list)
}

fn reconcile_interrupted_child_workflows(store: &MainStore) -> Result<(), crate::db::StoreError> {
    let child_workflows = store.list_nonterminal_child_workflows()?;
    for child in child_workflows {
        let Some(child_id) = child.id.clone() else {
            continue;
        };

        if BACKGROUND_TASKS.contains_key(&child_id) {
            continue;
        }

        log::info!(
            "[Workflow][session={}][phase=reconcile][event=child_interrupted] Marking stale child task as cancelled",
            child_id
        );

        store.update_workflow_status(&child_id, &WorkflowState::Cancelled.to_string())?;

        let mut context =
            store
                .get_execution_context(&child_id)?
                .unwrap_or_else(|| ExecutionContext {
                    session_id: child_id.clone(),
                    state: RuntimeState::Cancelled,
                    wait_reason: None,
                    current_step: 0,
                    max_steps: 0,
                    pending_tools: Vec::new(),
                    last_action_summary: None,
                    current_context_tokens: None,
                    max_context_tokens: None,
                    last_event_id: None,
                    version: ExecutionContext::CURRENT_VERSION.to_string(),
                    waiting_on_sub_agent_id: None,
                    sub_agent_sessions: Vec::new(),
                    pending_sub_agent_completions: Vec::new(),
                });
        context.state = RuntimeState::Cancelled;
        context.wait_reason = None;
        context.pending_tools.clear();
        context.waiting_on_sub_agent_id = None;
        context.last_action_summary =
            Some("Sub-agent interrupted by application restart.".to_string());
        store.upsert_execution_context(&context)?;

        if let Some(parent_session_id) = child.parent_session_id.clone() {
            let event = WorkflowEvent::sub_agent_interrupted(
                parent_session_id.clone(),
                child_id.clone(),
                "application_restart".to_string(),
            );
            store.append_workflow_event(&event)?;

            let message = format!(
                "<SYSTEM_REMINDER>\nSub-agent {} was interrupted by application restart and marked as cancelled. If you still need to continue that delegated work, call the sub-agent again with a fresh task.\n</SYSTEM_REMINDER>",
                child_id
            );
            let parent_message = WorkflowMessage {
                id: None,
                session_id: parent_session_id.clone(),
                role: "user".to_string(),
                message: message.clone(),
                reasoning: None,
                metadata: Some(json!({
                    "message_kind": "runtime_observation",
                    "observation_type": "sub_agent_interrupted",
                    "sub_agent_id": child_id,
                    "summary": "Sub-agent interrupted",
                    "result": {
                        "status": "interrupted",
                        "task_id": child_id,
                        "error": "Sub-agent interrupted by application restart",
                        "tool_calls_count": context.pending_tools.len()
                    },
                    "execution_status": "interrupted",
                    "is_error": true,
                    "error_type": "SubAgentInterrupted"
                })),
                attached_context: None,
                step_type: Some(StepType::Observe.to_string()),
                step_index: 0,
                is_error: true,
                error_type: Some("SubAgentInterrupted".to_string()),
                created_at: None,
            };
            let _ = store.add_workflow_message(&parent_message)?;

            if let Some(mut parent_context) = store.get_execution_context(&parent_session_id)? {
                parent_context
                    .sub_agent_sessions
                    .retain(|existing| existing != &child_id);
                if parent_context.waiting_on_sub_agent_id.as_deref() == Some(&child_id) {
                    parent_context.waiting_on_sub_agent_id = None;
                    parent_context.wait_reason = None;
                    parent_context.state = RuntimeState::Pending;
                }
                parent_context
                    .pending_sub_agent_completions
                    .retain(|existing| existing.sub_agent_id != child_id);
                parent_context
                    .pending_sub_agent_completions
                    .push(SubAgentCompletion {
                        sub_agent_id: child_id.clone(),
                        parent_session_id: parent_session_id.clone(),
                        status: "interrupted".to_string(),
                        summary: None,
                        error: Some(message.clone()),
                        tool_calls_count: 0,
                        completed_at_ms: chrono::Utc::now().timestamp_millis(),
                        consumed: true,
                    });
                store.upsert_execution_context(&parent_context)?;
                store.update_workflow_status(
                    &parent_session_id,
                    &WorkflowState::Pending.to_string(),
                )?;
            }
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn delete_workflow(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    gateway: State<'_, Arc<TauriGateway>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
) -> Result<(), String> {
    cleanup_workflow_resources(
        &session_id,
        chat_state.inner(),
        gateway.inner(),
        workflow_manager.inner(),
    )
    .await;

    let store = state.read().map_err(|e| e.to_string())?;
    store
        .delete_workflow(&session_id)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn get_workflow_snapshot(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
) -> Result<Value, String> {
    let main_store = state.inner().clone();
    let snapshot = {
        let store = main_store.read().map_err(|e| e.to_string())?;
        let mut snapshot = store
            .get_workflow_snapshot(&session_id)
            .map_err(|e| e.to_string())?;
        normalize_workflow_agent_config(&store, &mut snapshot.workflow)?;
        snapshot
    };
    let merged_messages = merge_ui_workflow_messages(&snapshot.messages);
    let execution_context = restore_context_for_signal(main_store, &session_id);

    // Phase 0-3 UI State Reconciliation: Add hasLiveSession field.
    // Reconcile terminal executors first so the frontend does not keep seeing
    // zombie runtime sessions after a turn has already finished.
    let has_live_session =
        has_reconciled_live_session(&workflow_manager.inner().clone(), &session_id, "snapshot")
            .await;

    // Convert snapshot to JSON and inject hasLiveSession
    let mut snapshot_json = serde_json::to_value(&snapshot).map_err(|e| e.to_string())?;
    if let Some(obj) = snapshot_json.as_object_mut() {
        obj.insert("messages".to_string(), json!(merged_messages));
        obj.insert("hasLiveSession".to_string(), json!(has_live_session));
        obj.insert("executionContext".to_string(), json!(execution_context));
    }

    log::debug!(
        "[Workflow][session={}][command=get_workflow_snapshot] hasLiveSession={}",
        session_id,
        has_live_session
    );

    Ok(snapshot_json)
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
pub async fn update_workflow_title_and_query(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    session_id: String,
    title: String,
    user_query: String,
) -> Result<(), String> {
    let store = state.read().map_err(|e| e.to_string())?;
    store
        .update_workflow_title_and_query(&session_id, &title, &user_query)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn update_workflow_query(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    session_id: String,
    user_query: String,
) -> Result<(), String> {
    let store = state.read().map_err(|e| e.to_string())?;
    store
        .update_workflow_query(&session_id, &user_query)
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

async fn has_reconciled_live_session(
    workflow_manager: &Arc<WorkflowManager>,
    session_id: &str,
    phase: &str,
) -> bool {
    if !workflow_manager.has_session(session_id) {
        return false;
    }

    if let Some(status) = workflow_manager.get_session_status(session_id) {
        if matches!(
            status,
            ManagedSessionStatus::Completed
                | ManagedSessionStatus::Failed
                | ManagedSessionStatus::Cancelled
        ) {
            log::warn!(
                "[Workflow][session={}][phase={}] Found stale live session in terminal managed status {:?}, removing before command handling",
                session_id,
                phase,
                status
            );
            workflow_manager.remove_session(session_id);
            return false;
        }
    }

    let Some(executor) = workflow_manager.get_executor(session_id) else {
        log::warn!(
            "[Workflow][session={}][phase={}] Manager had session entry without executor, removing stale entry",
            session_id,
            phase
        );
        workflow_manager.remove_session(session_id);
        return false;
    };

    let Ok(executor_guard) = executor.try_lock() else {
        log::debug!(
            "[Workflow][session={}][phase={}] Session exists and executor is busy; treating as live",
            session_id,
            phase
        );
        return true;
    };

    let executor_state = executor_guard.state();
    let managed_status = match executor_state {
        WorkflowState::Paused
        | WorkflowState::AwaitingUser
        | WorkflowState::AwaitingApproval
        | WorkflowState::AwaitingAutoApproval
        | WorkflowState::AwaitingSubAgent => ManagedSessionStatus::Waiting,
        WorkflowState::Completed => ManagedSessionStatus::Completed,
        WorkflowState::Error => ManagedSessionStatus::Failed,
        WorkflowState::Cancelled => ManagedSessionStatus::Cancelled,
        WorkflowState::Pending
        | WorkflowState::Thinking
        | WorkflowState::Executing
        | WorkflowState::Auditing => ManagedSessionStatus::Active,
    };
    let _ = workflow_manager.update_session_status(session_id, managed_status);

    if matches!(
        executor_state,
        WorkflowState::Completed | WorkflowState::Error | WorkflowState::Cancelled
    ) {
        log::warn!(
            "[Workflow][session={}][phase={}] Found stale live session in terminal state={}, removing before command handling",
            session_id,
            phase,
            executor_state
        );
        workflow_manager.remove_session(session_id);
        return false;
    }

    true
}

fn is_stale_gateway_injection_error(
    error: &crate::workflow::react::error::WorkflowEngineError,
) -> bool {
    matches!(
        error,
        crate::workflow::react::error::WorkflowEngineError::GatewayInputChannelClosed
            | crate::workflow::react::error::WorkflowEngineError::GatewayInputChannelMissing
    )
}

async fn interrupt_openai_session(chat_state: &Arc<ChatState>, session_id: &str) {
    let mut chats = chat_state.chats.lock().await;
    if let Some(protocol_chats) = chats.get_mut(&crate::ccproxy::ChatProtocol::OpenAI) {
        if let Some(chat) = protocol_chats.get_mut(session_id) {
            chat.set_stop_flag(true).await;
            log::info!(
                "[Workflow][session={}][phase=cleanup] Set chat stop_flag=true",
                session_id
            );
        }
    }
}

async fn cleanup_owned_background_resources(root_session_id: &str, chat_state: &Arc<ChatState>) {
    let registry = get_sub_agent_registry();
    let mut visited = std::collections::HashSet::new();
    let mut pending_task_ids = registry.list_sub_agents_for_parent(root_session_id);
    pending_task_ids.extend(list_background_task_ids_for_owner(root_session_id));

    while let Some(task_id) = pending_task_ids.pop() {
        if !visited.insert(task_id.clone()) {
            continue;
        }

        pending_task_ids.extend(registry.list_sub_agents_for_parent(&task_id));
        pending_task_ids.extend(list_background_task_ids_for_owner(&task_id));

        interrupt_openai_session(chat_state, &task_id).await;
        if stop_background_task(&task_id, Some(chat_state)).await {
            WorkflowManager::unregister_session_signal_tx(&task_id);
            log::info!(
                "[Workflow][session={}][phase=cleanup] Reclaimed owned background task {}",
                root_session_id,
                task_id
            );
        }
    }
}

async fn cleanup_workflow_resources(
    session_id: &str,
    chat_state: &Arc<ChatState>,
    gateway: &Arc<TauriGateway>,
    workflow_manager: &Arc<WorkflowManager>,
) {
    interrupt_openai_session(chat_state, session_id).await;
    cleanup_owned_background_resources(session_id, chat_state).await;
    if workflow_manager.has_session(session_id) {
        let _ = gateway
            .inject_input(session_id, "{\"type\": \"stop\"}".to_string())
            .await;
    }

    if let Some(executor) = workflow_manager.get_executor(session_id) {
        let mut guard = executor.lock().await;
        guard.set_state(WorkflowState::Cancelled);
    }

    if workflow_manager.has_session(session_id) {
        let _ = workflow_manager.update_session_status(session_id, ManagedSessionStatus::Cancelled);
        workflow_manager.remove_session(session_id);
    }
    BACKGROUND_TASKS.remove(session_id);
    WorkflowManager::unregister_session_signal_tx(session_id);
    gateway.unregister_session(session_id).await;
}

fn legacy_wait_reason_from_status(status: &str) -> Option<WaitReason> {
    match status.to_lowercase().as_str() {
        "paused" => Some(WaitReason::Confirmation),
        "awaiting_user" => Some(WaitReason::UserInput),
        "awaiting_approval" | "awaitingapproval" | "awaiting_auto_approval" => {
            Some(WaitReason::Approval)
        }
        "awaiting_sub_agent" => Some(WaitReason::SubAgent),
        _ => None,
    }
}

fn restore_context_for_signal(
    store: Arc<std::sync::RwLock<MainStore>>,
    session_id: &str,
) -> Option<ExecutionContext> {
    match restore_execution_context(store, session_id) {
        RecoveryResult::SnapshotHit { context } | RecoveryResult::ReplayFallback { context } => {
            Some(context)
        }
        RecoveryResult::SafeFailed { error, .. } => {
            if error.is_empty_replay_history() {
                log::info!(
                    "[Workflow][session={}][phase=signal] Recovery context not available yet: workflow has no execution snapshot or events",
                    session_id
                );
            } else {
                log::warn!(
                    "[Workflow][session={}][phase=signal] Recovery context unavailable: {}",
                    session_id,
                    error
                );
            }
            None
        }
    }
}

fn is_resumable_from_context_for_user_message(ctx: Option<&ExecutionContext>) -> bool {
    match ctx {
        Some(context) => match context.state {
            RuntimeState::Pending
            | RuntimeState::Completed
            | RuntimeState::Failed
            | RuntimeState::Cancelled => true,
            RuntimeState::Waiting => context.wait_reason == Some(WaitReason::UserInput),
            RuntimeState::Running => false,
        },
        None => false,
    }
}

#[tauri::command]
pub async fn workflow_start(
    app: tauri::AppHandle,
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    tsid_generator: State<'_, Arc<TsidGenerator>>,
    gateway: State<'_, Arc<TauriGateway>>,
    factory: State<'_, Arc<dyn SubAgentFactory>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
    agent_id: String,
    initial_prompt: Option<String>,
    planning_mode: Option<bool>,
) -> Result<String, String> {
    log::info!(
        "[Workflow][session={}][phase=start] Starting workflow, agent_id={}, planning_mode={}",
        session_id,
        agent_id,
        planning_mode.unwrap_or(false)
    );

    let main_store_arc = state.inner().clone();
    let chat_state_arc = chat_state.inner().clone();
    let tsid_generator = tsid_generator.inner().clone();
    let gateway_arc = gateway.inner().clone();
    let factory = factory.inner().clone();
    let workflow_manager_arc = workflow_manager.inner().clone();
    let app_data_dir = app.path().app_data_dir().unwrap_or_default();
    let planning_mode = planning_mode.unwrap_or(false);

    // Reject only truly live sessions. A terminal executor may still exist in
    // memory for a short window while cleanup races the next user action.
    if has_reconciled_live_session(&workflow_manager_arc, &session_id, "start").await {
        log::warn!(
            "[Workflow][session={}][phase=start] Session already exists in WorkflowManager, rejecting duplicate start",
            session_id
        );
        return Err(format!("Session already exists: {}", session_id));
    }

    let (clean_prompt, attached_context, allowed_paths) = {
        let store = main_store_arc.read().map_err(|e| e.to_string())?;
        let snapshot = store
            .get_workflow_snapshot(&session_id)
            .map_err(|e| e.to_string())?;
        let wf = snapshot.workflow;

        let agent_cfg = wf
            .agent_config
            .as_ref()
            .and_then(|s| AgentConfig::from_json(s))
            .unwrap_or_default();

        let paths: Vec<String> = agent_cfg.allowed_paths.unwrap_or_default();

        let prompt = initial_prompt
            .clone()
            .unwrap_or_else(|| wf.user_query.clone());

        // [Bug Fix] If this is the first real message (initial_prompt is Some)
        // and the DB record has an empty user_query, update it now.
        if initial_prompt.is_some() && wf.user_query.is_empty() {
            log::info!(
                "[Workflow] First message detected for session {}, updating user_query",
                session_id
            );
            let _ = store.update_workflow_query(&session_id, &prompt);
        }

        let (p, att) = if initial_prompt.is_some() {
            inject_at_mentions(&prompt, &paths)
        } else {
            (prompt, String::new())
        };

        (p, att, paths)
    };

    let mut agent_config = {
        let store = main_store_arc.read().map_err(|e| e.to_string())?;
        store
            .get_agent(&agent_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Agent {} not found", agent_id))?
    };

    // Load agent_config JSON for easier access to overrides like 'phase'
    let agent_config_json: Value = {
        let store = main_store_arc.read().map_err(|e| e.to_string())?;
        if let Ok(snapshot) = store.get_workflow_snapshot(&session_id) {
            snapshot
                .workflow
                .agent_config
                .as_ref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(json!({}))
        } else {
            json!({})
        }
    };

    // Load agent_config from workflow record if available and merge into agent_config struct
    if let Some(config_str) = agent_config_json.as_str() {
        agent_config.merge_config(config_str);
    } else if !agent_config_json.is_null() {
        if let Ok(config_str) = serde_json::to_string(&agent_config_json) {
            agent_config.merge_config(&config_str);
        }
    }

    let (signal_tx, signal_rx) = tokio::sync::mpsc::channel(100);
    gateway_arc
        .register_session_tx(session_id.clone(), signal_tx.clone())
        .await;
    WorkflowManager::register_session_signal_tx(session_id.clone(), signal_tx);

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

    // Build execution policy with phase from overrides or default
    let mut policy = if planning_mode {
        crate::workflow::react::policy::ExecutionPolicy::planning()
    } else {
        // Try to load phase from overrides
        if let Some(p_str) = agent_config_json.get("phase").and_then(|v| v.as_str()) {
            use std::str::FromStr;
            if let Ok(p) = crate::workflow::react::policy::ExecutionPhase::from_str(p_str) {
                match p {
                    crate::workflow::react::policy::ExecutionPhase::Planning => {
                        crate::workflow::react::policy::ExecutionPolicy::planning()
                    }
                    crate::workflow::react::policy::ExecutionPhase::Implementation => {
                        crate::workflow::react::policy::ExecutionPolicy::implementation()
                    }
                    crate::workflow::react::policy::ExecutionPhase::Standard => {
                        crate::workflow::react::policy::ExecutionPolicy::standard()
                    }
                }
            } else {
                crate::workflow::react::policy::ExecutionPolicy::standard()
            }
        } else {
            crate::workflow::react::policy::ExecutionPolicy::standard()
        }
    };

    // Apply approval level from merged agent config
    if let Some(ref approval_level_str) = agent_config.approval_level {
        use std::str::FromStr;
        if let Ok(level) =
            crate::workflow::react::policy::ApprovalLevel::from_str(approval_level_str)
        {
            policy.approval_level = level.clone();
            log::info!(
                "[Workflow] Session {} using approval level: {:?}",
                session_id,
                level
            );
        }
    }

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
                policy,
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
                policy,
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

            // Check if message already exists to avoid duplicates
            let messages = executor.messages();
            let is_duplicate = messages
                .iter()
                .any(|m| m.role == "user" && m.message == clean_prompt);

            if !is_duplicate {
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
            } else {
                log::warn!("[Workflow] Skipping duplicate message on resume");
            }
        }
    }

    // Register to WorkflowManager FIRST (primary registry)
    if let Err(e) = workflow_manager_arc.register_session(
        session_id.clone(),
        shared_executor.clone(),
        ManagedSessionStatus::Active,
    ) {
        log::error!(
            "[Workflow][session={}][phase=start] Failed to register to WorkflowManager: {}",
            session_id,
            e
        );
        return Err(format!("Failed to register workflow session: {}", e));
    }

    // BACKGROUND_TASKS as compatibility layer (secondary)
    BACKGROUND_TASKS.insert(
        session_id.clone(),
        BackgroundTask::SubAgent {
            owner_session_id: None,
            executor: shared_executor.clone(),
        },
    );

    log::info!(
        "[Workflow][session={}][phase=start] Executor registered to WorkflowManager (primary) and BACKGROUND_TASKS (compat), spawning run_loop",
        session_id
    );

    let session_id_for_spawn = session_id.clone();
    let gateway_for_spawn = gateway_arc.clone();
    let manager_for_spawn = workflow_manager_arc.clone();
    tokio::spawn(async move {
        let mut guard = shared_executor.lock().await;
        if let Err(e) = guard.run_loop().await {
            if let crate::workflow::react::error::WorkflowEngineError::Cancelled(_) = e {
                let _ = manager_for_spawn
                    .update_session_status(&session_id_for_spawn, ManagedSessionStatus::Cancelled);
                log::info!(
                    "[Workflow][session={}][phase=run_loop][event=cancelled] Workflow session was cancelled by user",
                    session_id_for_spawn
                );
                BACKGROUND_TASKS.remove(&session_id_for_spawn);
                WorkflowManager::unregister_session_signal_tx(&session_id_for_spawn);
                gateway_for_spawn
                    .unregister_session(&session_id_for_spawn)
                    .await;
                manager_for_spawn.remove_session(&session_id_for_spawn);
                return;
            }

            log::error!(
                "[Workflow][session={}][phase=run_loop][event=crash] Workflow error: {:?}",
                session_id_for_spawn,
                e
            );
            let _ = manager_for_spawn
                .update_session_status(&session_id_for_spawn, ManagedSessionStatus::Failed);

            // Notify UI about the fatal crash
            let _ = gateway_for_spawn
                .send(
                    &session_id_for_spawn,
                    crate::workflow::react::types::GatewayPayload::State {
                        state: crate::workflow::react::types::WorkflowState::Error,
                        wait_reason: None,
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
        if matches!(guard.state(), WorkflowState::Completed) {
            let _ = manager_for_spawn
                .update_session_status(&session_id_for_spawn, ManagedSessionStatus::Completed);
        }
        BACKGROUND_TASKS.remove(&session_id_for_spawn);
        WorkflowManager::unregister_session_signal_tx(&session_id_for_spawn);
        gateway_for_spawn
            .unregister_session(&session_id_for_spawn)
            .await;
        manager_for_spawn.remove_session(&session_id_for_spawn);
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
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
    agent_id: String,
    plan: String,
) -> Result<(), String> {
    let main_store_arc = main_store.inner().clone();
    let chat_state_arc = chat_state.inner().clone();
    let tsid_generator_arc = tsid_generator.inner().clone();
    let gateway_arc = gateway.inner().clone();
    let factory_arc = factory.inner().clone();
    let workflow_manager_arc = workflow_manager.inner().clone();
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

    let mut agent_config = {
        let store = main_store_arc.read().map_err(|e| e.to_string())?;
        store
            .get_agent(&agent_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Agent {} not found", agent_id))?
    };

    // Load agent_config from workflow record if available and merge into agent_config struct
    {
        let store = main_store_arc.read().map_err(|e| e.to_string())?;
        if let Ok(snapshot) = store.get_workflow_snapshot(&session_id) {
            if let Some(config_str) = snapshot.workflow.agent_config {
                agent_config.merge_config(&config_str);
            }
        }
    }

    let (signal_tx, signal_rx) = tokio::sync::mpsc::channel(100);
    gateway_arc
        .register_session_tx(session_id.clone(), signal_tx.clone())
        .await;
    WorkflowManager::register_session_signal_tx(session_id.clone(), signal_tx);

    let allowed_paths = {
        let store = main_store_arc.read().map_err(|e| e.to_string())?;
        let snapshot = store
            .get_workflow_snapshot(&session_id)
            .map_err(|e| e.to_string())?;
        let wf = snapshot.workflow;

        let agent_cfg = wf
            .agent_config
            .as_ref()
            .and_then(|s| AgentConfig::from_json(s))
            .unwrap_or_default();

        let paths: Vec<String> = agent_cfg.allowed_paths.unwrap_or_default();

        paths
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

    // Register to WorkflowManager FIRST (primary registry)
    if let Err(e) = workflow_manager_arc.register_session(
        session_id.clone(),
        shared_executor.clone(),
        ManagedSessionStatus::Active,
    ) {
        log::error!(
            "[Workflow][session={}][phase=approve_plan] Failed to register to WorkflowManager: {}",
            session_id,
            e
        );
        return Err(format!("Failed to register workflow session: {}", e));
    }

    // BACKGROUND_TASKS as compatibility layer (secondary)
    BACKGROUND_TASKS.insert(
        session_id.clone(),
        BackgroundTask::SubAgent {
            owner_session_id: None,
            executor: shared_executor.clone(),
        },
    );

    let session_id_for_spawn = session_id.clone();
    let gateway_for_spawn = gateway_arc.clone();
    let manager_for_spawn = workflow_manager_arc.clone();
    tokio::spawn(async move {
        let mut guard = shared_executor.lock().await;
        if let Err(e) = guard.run_loop().await {
            if let crate::workflow::react::error::WorkflowEngineError::Cancelled(_) = e {
                let _ = manager_for_spawn
                    .update_session_status(&session_id_for_spawn, ManagedSessionStatus::Cancelled);
                log::info!(
                    "[Workflow][session={}][phase=run_loop][event=cancelled] Workflow session was cancelled by user",
                    session_id_for_spawn
                );
                manager_for_spawn.remove_session(&session_id_for_spawn);
                BACKGROUND_TASKS.remove(&session_id_for_spawn);
                WorkflowManager::unregister_session_signal_tx(&session_id_for_spawn);
                gateway_for_spawn
                    .unregister_session(&session_id_for_spawn)
                    .await;
                return;
            }

            log::error!(
                "[Workflow][session={}][phase=run_loop][event=crash] Workflow error: {:?}",
                session_id_for_spawn,
                e
            );
            let _ = manager_for_spawn
                .update_session_status(&session_id_for_spawn, ManagedSessionStatus::Failed);

            // Notify UI about the fatal crash
            let _ = gateway_for_spawn
                .send(
                    &session_id_for_spawn,
                    crate::workflow::react::types::GatewayPayload::State {
                        state: crate::workflow::react::types::WorkflowState::Error,
                        wait_reason: None,
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
        if matches!(guard.state(), WorkflowState::Completed) {
            let _ = manager_for_spawn
                .update_session_status(&session_id_for_spawn, ManagedSessionStatus::Completed);
        }
        manager_for_spawn.remove_session(&session_id_for_spawn);
        BACKGROUND_TASKS.remove(&session_id_for_spawn);
        WorkflowManager::unregister_session_signal_tx(&session_id_for_spawn);
        gateway_for_spawn
            .unregister_session(&session_id_for_spawn)
            .await;
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
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
    signal: String,
) -> Result<String, String> {
    let signal_type = serde_json::from_str::<serde_json::Value>(&signal)
        .ok()
        .and_then(|value| {
            let raw_type = value["type"].as_str()?;
            Some(
                SignalType::from_str(raw_type)
                    .map(|signal_type| signal_type.as_str().to_string())
                    .unwrap_or_else(|| raw_type.to_string()),
            )
        })
        .unwrap_or_else(|| "unknown".to_string());

    log::info!(
        "[Workflow][session={}][phase=signal] Signal received, type={}",
        session_id,
        signal_type
    );

    let workflow_manager_arc = workflow_manager.inner().clone();
    let gateway_arc = gateway.inner().clone();
    let main_store_arc = state.inner().clone();
    let workflow_snapshot = {
        let store = main_store_arc.read().map_err(|e| e.to_string())?;
        store
            .get_workflow_snapshot(&session_id)
            .map_err(|e| e.to_string())?
    };
    let recovery_context = restore_context_for_signal(main_store_arc, &session_id);

    // Phase 1: Check manager first (primary path)
    let has_live_session =
        has_reconciled_live_session(&workflow_manager_arc, &session_id, "signal").await;
    let should_enter_recovery = if has_live_session {
        log::info!(
            "[WorkflowManager][session={}][event=session_lookup_hit] Session exists in manager",
            session_id
        );

        let signal_type_enum = serde_json::from_str::<serde_json::Value>(&signal)
            .ok()
            .and_then(|value| value["type"].as_str().map(SignalType::from_str))
            .flatten();

        let snapshot_status_lower = workflow_snapshot.workflow.status.to_lowercase();
        let snapshot_is_terminal = matches!(
            snapshot_status_lower.as_str(),
            "completed" | "cancelled" | "error" | "failed"
        );
        let should_force_recovery_for_terminal_user_message = snapshot_is_terminal
            && matches!(
                signal_type_enum,
                Some(SignalType::UserMessage | SignalType::LegacyUserInput)
            );

        if should_force_recovery_for_terminal_user_message {
            log::warn!(
                "[Workflow][session={}][phase=signal] Live session still registered but snapshot is terminal (status={}). Treating user message as resume request and entering recovery.",
                session_id,
                workflow_snapshot.workflow.status
            );
            workflow_manager_arc.remove_session(&session_id);
            gateway_arc.unregister_session(&session_id).await;
            true
        } else {
            // Route signal through manager
            if let Err(e) = workflow_manager_arc.route_signal(&session_id, &signal_type) {
                let allow_recovery_for_terminal_user_message = matches!(
                    signal_type_enum,
                    Some(SignalType::UserMessage | SignalType::LegacyUserInput)
                );
                if allow_recovery_for_terminal_user_message {
                    log::warn!(
                        "[Workflow][session={}][phase=signal] Live session rejected user message with '{}'. Treating as stale terminal session and entering recovery.",
                        session_id,
                        e
                    );
                    workflow_manager_arc.remove_session(&session_id);
                    gateway_arc.unregister_session(&session_id).await;
                    true
                } else {
                    log::warn!(
                        "[WorkflowManager][session={}][event=signal_rejected] Signal '{}' rejected: {}",
                        session_id,
                        signal_type,
                        e
                    );
                    return Err(format!("Signal rejected: {}", e));
                }
            } else {
                log::info!(
                    "[WorkflowManager][session={}][event=signal_routed] Signal '{}' routed successfully",
                    session_id,
                    signal_type
                );

                // Now inject through gateway
                match gateway_arc.inject_input(&session_id, signal.clone()).await {
                    Ok(_) => {
                        log::info!(
                            "[Workflow][session={}][phase=signal] Signal injected successfully, type={}",
                            session_id,
                            signal_type
                        );
                        return Ok("Signal injected".to_string());
                    }
                    Err(e) => {
                        if is_stale_gateway_injection_error(&e) {
                            log::warn!(
                                "[Workflow][session={}][phase=signal] Gateway injection hit stale live session: {}. Removing stale session and entering recovery.",
                                session_id,
                                e
                            );
                            workflow_manager_arc.remove_session(&session_id);
                            gateway_arc.unregister_session(&session_id).await;
                            true
                        } else {
                            log::warn!(
                                "[Workflow][session={}][phase=signal] Gateway injection failed despite active session: {}",
                                session_id,
                                e
                            );
                            return Err(format!("Gateway injection failed: {}", e));
                        }
                    }
                }
            }
        }
    } else {
        log::info!(
            "[WorkflowManager][session={}][event=session_lookup_miss] Session not found in manager, entering recovery",
            session_id
        );
        true
    };

    if should_enter_recovery {
        // Session not in manager - enter recovery logic
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&signal) {
            let signal_type = val["type"].as_str().unwrap_or("unknown");
            let signal_type_enum = SignalType::from_str(signal_type);
            let workflow_signal = WorkflowSignal::parse(&signal);
            let effective_wait_reason = recovery_context
                .as_ref()
                .and_then(|ctx| ctx.wait_reason.clone())
                .or_else(|| legacy_wait_reason_from_status(&workflow_snapshot.workflow.status));

            // Handle user_message signal (Phase 3 unified signal type)
            // Also support legacy user_input for backward compatibility
            if matches!(
                signal_type_enum,
                Some(SignalType::UserMessage | SignalType::LegacyUserInput)
            ) {
                if let Some(content) = val["content"].as_str() {
                    let status_lower = workflow_snapshot.workflow.status.to_lowercase();
                    let is_resumable =
                        is_resumable_from_context_for_user_message(recovery_context.as_ref())
                            || matches!(
                                status_lower.as_str(),
                                "pending"
                                    | "completed"
                                    | "cancelled"
                                    | "error"
                                    | "failed"
                                    | "awaiting_user"
                            );

                    if !is_resumable {
                        log::warn!(
                            "[Workflow][session={}][phase=signal] Cannot resume with user message: runtime_state={:?}, wait_reason={:?}, status={}",
                            session_id,
                            recovery_context.as_ref().map(|ctx| &ctx.state),
                            effective_wait_reason,
                            workflow_snapshot.workflow.status
                        );
                        return Err(format!(
                            "Cannot resume: workflow is in '{}' state",
                            workflow_snapshot.workflow.status
                        ));
                    }

                    log::info!(
                        "[Workflow][session={}][phase=signal] Resuming orphan session with user message, runtime_state={:?}, wait_reason={:?}, status={}",
                        session_id,
                        recovery_context.as_ref().map(|ctx| &ctx.state),
                        effective_wait_reason,
                        workflow_snapshot.workflow.status
                    );

                    workflow_start(
                        app,
                        state,
                        chat_state,
                        tsid_generator,
                        gateway,
                        factory,
                        workflow_manager,
                        session_id,
                        workflow_snapshot.workflow.agent_id.clone(),
                        Some(content.to_string()),
                        None,
                    )
                    .await?;

                    return Ok("Workflow resumed with input".to_string());
                }
            } else if signal_type == "rebroadcast_pending"
                || signal_type == "request_confirm_broadcast"
            {
                let status_lower = workflow_snapshot.workflow.status.to_lowercase();
                let can_rebroadcast = effective_wait_reason == Some(WaitReason::Approval)
                    || status_lower == "awaiting_approval"
                    || status_lower == "awaitingapproval";
                if !can_rebroadcast {
                    log::info!(
                        "[Workflow][session={}][phase=signal] Session is not awaiting approval, skipping rebroadcast_pending. runtime_state={:?}, wait_reason={:?}, status={}",
                        session_id,
                        recovery_context.as_ref().map(|ctx| &ctx.state),
                        effective_wait_reason,
                        workflow_snapshot.workflow.status
                    );
                    return Ok("Workflow is not awaiting approval".to_string());
                }

                log::info!(
                    "[Workflow] Session {} requesting rebroadcast pending. Resuming workflow.",
                    session_id
                );

                workflow_start(
                    app,
                    state,
                    chat_state,
                    tsid_generator,
                    gateway,
                    factory,
                    workflow_manager,
                    session_id.clone(),
                    workflow_snapshot.workflow.agent_id.clone(),
                    None,
                    None,
                )
                .await?;

                return Ok("Workflow resumed and rebroadcast pending triggered".to_string());
            } else if matches!(
                workflow_signal,
                Some(WorkflowSignal::Continue | WorkflowSignal::Stop)
            ) {
                let status_lower = workflow_snapshot.workflow.status.to_lowercase();
                let can_resume = effective_wait_reason == Some(WaitReason::Confirmation)
                    || status_lower == "paused";
                if !can_resume {
                    log::info!(
                        "[Workflow][session={}][phase=signal] Session is not confirmation-waiting, ignoring {}. runtime_state={:?}, wait_reason={:?}, status={}",
                        session_id,
                        signal_type,
                        recovery_context.as_ref().map(|ctx| &ctx.state),
                        effective_wait_reason,
                        workflow_snapshot.workflow.status
                    );
                    return Ok(format!(
                        "Workflow is not paused, ignoring {} signal",
                        signal_type
                    ));
                }

                log::info!(
                    "[Workflow] Session {} is paused, resuming to process {} signal",
                    session_id,
                    val["type"]
                );

                workflow_start(
                    app,
                    state,
                    chat_state,
                    tsid_generator,
                    gateway,
                    factory,
                    workflow_manager,
                    session_id.clone(),
                    workflow_snapshot.workflow.agent_id.clone(),
                    None,
                    None,
                )
                .await?;

                let mut retries = 5;
                let mut last_error = None;
                while retries > 0 {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    match gateway_arc.inject_input(&session_id, signal.clone()).await {
                        Ok(_) => {
                            log::info!(
                                "[Workflow] {} signal injected successfully after retry",
                                signal_type
                            );
                            break;
                        }
                        Err(e) => {
                            last_error = Some(e);
                            retries -= 1;
                            log::warn!(
                                "[Workflow] Failed to inject {} signal, retries left: {}",
                                signal_type,
                                retries
                            );
                        }
                    }
                }
                if retries == 0 {
                    let err_msg = last_error
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| "Unknown error".into());
                    return Err(format!(
                        "Failed to inject {} after resuming: {}",
                        signal_type, err_msg
                    ));
                }

                return Ok(format!("Workflow resumed and {} processed", signal_type));
            } else if matches!(
                workflow_signal,
                Some(WorkflowSignal::ApprovalDecision { .. })
            ) {
                let status_lower = workflow_snapshot.workflow.status.to_lowercase();
                let can_resume = effective_wait_reason == Some(WaitReason::Approval)
                    || status_lower == "awaiting_approval"
                    || status_lower == "awaitingapproval";
                if !can_resume {
                    return Err(format!(
                        "Cannot process approval: Workflow is in '{}' state, not awaiting approval.",
                        workflow_snapshot.workflow.status
                    ));
                }

                log::info!(
                    "[Workflow] Session {} is awaiting approval but not active. Auto-resuming workflow to process approval.",
                    session_id
                );

                workflow_start(
                    app,
                    state,
                    chat_state,
                    tsid_generator,
                    gateway,
                    factory,
                    workflow_manager,
                    session_id.clone(),
                    workflow_snapshot.workflow.agent_id.clone(),
                    None,
                    None,
                )
                .await?;

                let mut retries = 5;
                let mut last_error = None;
                while retries > 0 {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    match gateway_arc.inject_input(&session_id, signal.clone()).await {
                        Ok(_) => {
                            log::info!(
                                "[Workflow] Approval signal injected successfully after retry"
                            );
                            break;
                        }
                        Err(e) => {
                            last_error = Some(e);
                            retries -= 1;
                            log::warn!(
                                "[Workflow] Failed to inject approval signal, retries left: {}",
                                retries
                            );
                        }
                    }
                }
                if retries == 0 {
                    let err_msg = last_error
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| "Unknown error".into());
                    return Err(format!(
                        "Failed to inject approval after resuming: {}",
                        err_msg
                    ));
                }

                return Ok("Workflow resumed and approval processed".to_string());
            } else if matches!(
                workflow_signal,
                Some(WorkflowSignal::SubAgentComplete { .. })
            ) {
                if effective_wait_reason != Some(WaitReason::SubAgent) {
                    return Err(format!(
                        "Cannot process sub-agent completion: Workflow is in '{}' state.",
                        workflow_snapshot.workflow.status
                    ));
                }

                workflow_start(
                    app,
                    state,
                    chat_state,
                    tsid_generator,
                    gateway,
                    factory,
                    workflow_manager,
                    session_id.clone(),
                    workflow_snapshot.workflow.agent_id.clone(),
                    None,
                    None,
                )
                .await?;

                let mut retries = 5;
                let mut last_error = None;
                while retries > 0 {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    match gateway_arc.inject_input(&session_id, signal.clone()).await {
                        Ok(_) => break,
                        Err(e) => {
                            last_error = Some(e);
                            retries -= 1;
                            log::warn!(
                                "[Workflow] Failed to inject sub_agent_complete signal, retries left: {}",
                                retries
                            );
                        }
                    }
                }
                if retries == 0 {
                    let err_msg = last_error
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| "Unknown error".into());
                    return Err(format!(
                        "Failed to inject sub_agent_complete after resuming: {}",
                        err_msg
                    ));
                }

                return Ok("Workflow resumed and sub-agent completion processed".to_string());
            }
        }
    }

    Err(format!(
        "Failed to send signal: No active session for {}",
        session_id
    ))
}

#[tauri::command]
pub async fn workflow_stop(
    chat_state: State<'_, Arc<ChatState>>,
    gateway: State<'_, Arc<TauriGateway>>,
    session_id: String,
) -> Result<(), String> {
    interrupt_openai_session(chat_state.inner(), &session_id).await;

    // Keep stop as a runtime signal only.
    // Terminal persistence and session cleanup should happen on the executor's
    // normal shutdown path after it processes the stop signal.
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
    pub root_path: String,
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
                root_path: root_str.clone(),
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

    // Get current config
    let snapshot = store
        .get_workflow_snapshot(&session_id)
        .map_err(|e| e.to_string())?;

    let mut config = snapshot
        .workflow
        .agent_config
        .and_then(|s| AgentConfig::from_json(&s))
        .unwrap_or_default();

    // Update allowed_paths
    config.allowed_paths = serde_json::from_value(allowed_paths).ok();

    // Save back
    let new_config_str = config.to_json();
    store
        .update_workflow_agent_config(&session_id, &new_config_str)
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

    // Get current config
    let snapshot = store
        .get_workflow_snapshot(&session_id)
        .map_err(|e| e.to_string())?;

    let mut config = snapshot
        .workflow
        .agent_config
        .and_then(|s| AgentConfig::from_json(&s))
        .unwrap_or_default();

    // Update final_audit
    config.final_audit = Some(final_audit);

    // Save back
    let new_config_str = config.to_json();
    store
        .update_workflow_agent_config(&session_id, &new_config_str)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn update_workflow_approval_level(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    session_id: String,
    approval_level: String,
) -> Result<(), String> {
    let store = state.read().map_err(|e| e.to_string())?;

    // Get current config
    let snapshot = store
        .get_workflow_snapshot(&session_id)
        .map_err(|e| e.to_string())?;

    let mut config = snapshot
        .workflow
        .agent_config
        .and_then(|s| AgentConfig::from_json(&s))
        .unwrap_or_default();

    // Update approval_level
    config.approval_level = Some(approval_level);

    // Save back
    let new_config_str = config.to_json();
    store
        .update_workflow_agent_config(&session_id, &new_config_str)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn update_workflow_agent_config(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    session_id: String,
    agent_config: String,
) -> Result<(), String> {
    let store = state.read().map_err(|e| e.to_string())?;
    store
        .update_workflow_agent_config(&session_id, &agent_config)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn update_workflow_agent_id(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
    agent_id: String,
) -> Result<String, String> {
    let store = state.read().map_err(|e| e.to_string())?;

    let agent = store
        .get_agent(&agent_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Agent {} not found", agent_id))?;

    if agent.role.as_deref() == Some("child") {
        return Err("Child agents cannot be used as top-level workflow agents".to_string());
    }

    let snapshot = store
        .get_workflow_snapshot(&session_id)
        .map_err(|e| e.to_string())?;

    if workflow_manager.has_session(&session_id) {
        return Err("Cannot change workflow agent after the session has started".to_string());
    }

    if !snapshot.messages.is_empty() || !snapshot.workflow.user_query.trim().is_empty() {
        return Err("Cannot change workflow agent after the workflow has user input".to_string());
    }

    log::info!(
        "[Workflow][session={}][phase=update_agent] Updating workflow agent_id to {}",
        session_id,
        agent_id
    );

    store
        .update_workflow_agent_id(&session_id, &agent_id)
        .map_err(|e| e.to_string())?;

    let current_config = snapshot
        .workflow
        .agent_config
        .as_deref()
        .and_then(AgentConfig::from_json)
        .unwrap_or_default();
    let mut agent_config = build_agent_config_from_agent(&agent, None, None);
    agent_config.approval_level = current_config.approval_level;
    agent_config.final_audit = current_config.final_audit;
    let agent_config_json = agent_config_to_json_with_agent_shell_policy(&agent_config, &agent)?;
    store
        .update_workflow_agent_config(&session_id, &agent_config_json)
        .map_err(|e| e.to_string())?;

    Ok(agent_config_json)
}

#[tauri::command]
pub async fn get_auto_approved_tools(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    session_id: String,
) -> Result<Vec<String>, String> {
    let store = state.read().map_err(|e| e.to_string())?;

    // Get workflow snapshot
    let snapshot = store
        .get_workflow_snapshot(&session_id)
        .map_err(|e| e.to_string())?;

    // Parse agent_config to extract auto_approve list
    let auto_approve = snapshot
        .workflow
        .agent_config
        .and_then(|s| AgentConfig::from_json(&s))
        .and_then(|config| config.auto_approve)
        .unwrap_or_default();

    Ok(auto_approve)
}

#[tauri::command]
pub async fn remove_auto_approved_tool(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    gateway: State<'_, Arc<TauriGateway>>,
    session_id: String,
    tool_name: String,
) -> Result<(), String> {
    // Update database first
    {
        let store = state.read().map_err(|e| e.to_string())?;

        // Get current config
        let snapshot = store
            .get_workflow_snapshot(&session_id)
            .map_err(|e| e.to_string())?;

        let mut config = snapshot
            .workflow
            .agent_config
            .and_then(|s| AgentConfig::from_json(&s))
            .unwrap_or_default();

        // Remove tool from auto_approve list
        if let Some(ref mut tools) = config.auto_approve {
            tools.retain(|t| t != &tool_name);
        }

        // Save updated config to database
        let config_str = config.to_json();
        store
            .update_workflow_agent_config(&session_id, &config_str)
            .map_err(|e| e.to_string())?;
    }

    // Signal running engine to update runtime auto_approve HashSet (after releasing store lock)
    let _ = gateway
        .inject_input(
            &session_id,
            serde_json::json!({
                "type": "remove_auto_approved_tool",
                "tool_name": tool_name
            })
            .to_string(),
        )
        .await;

    Ok(())
}

#[tauri::command]
pub async fn remove_shell_policy_item(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    gateway: State<'_, Arc<TauriGateway>>,
    session_id: String,
    pattern: String,
) -> Result<(), String> {
    // Update database first
    {
        let store = state.read().map_err(|e| e.to_string())?;

        // Get current config
        let snapshot = store
            .get_workflow_snapshot(&session_id)
            .map_err(|e| e.to_string())?;

        let mut config = snapshot
            .workflow
            .agent_config
            .and_then(|s| AgentConfig::from_json(&s))
            .unwrap_or_default();

        // Remove item from shell_policy list
        if let Some(ref mut policy) = config.shell_policy {
            policy.retain(|item| item.pattern != pattern);
        }

        // Save updated config to database
        let config_str = config.to_json();
        store
            .update_workflow_agent_config(&session_id, &config_str)
            .map_err(|e| e.to_string())?;
    }

    // Signal running engine to update runtime shell_policy
    let _ = gateway
        .inject_input(
            &session_id,
            serde_json::json!({
                "type": "remove_shell_policy_item",
                "pattern": pattern
            })
            .to_string(),
        )
        .await;

    Ok(())
}

#[tauri::command]
pub async fn get_workflow_events(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    session_id: String,
) -> Result<Vec<crate::workflow::react::events::WorkflowEventRecord>, String> {
    let store = state.read().map_err(|e| e.to_string())?;
    store
        .list_workflow_events(&session_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_workflow_dispatcher_metrics(
    session_id: String,
) -> Result<DispatcherMetricsSnapshot, String> {
    Dispatcher::metrics_for_session(&session_id)
        .ok_or_else(|| format!("No dispatcher metrics found for session {}", session_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_legacy_wait_reason_from_status() {
        assert_eq!(
            legacy_wait_reason_from_status("awaiting_approval"),
            Some(WaitReason::Approval)
        );
        assert_eq!(
            legacy_wait_reason_from_status("awaiting_user"),
            Some(WaitReason::UserInput)
        );
        assert_eq!(
            legacy_wait_reason_from_status("paused"),
            Some(WaitReason::Confirmation)
        );
        assert_eq!(
            legacy_wait_reason_from_status("awaiting_sub_agent"),
            Some(WaitReason::SubAgent)
        );
        assert_eq!(legacy_wait_reason_from_status("thinking"), None);
    }

    #[test]
    fn test_is_resumable_from_context_for_user_message() {
        let mut waiting_for_user = ExecutionContext::new("session-1".to_string());
        waiting_for_user.state = RuntimeState::Waiting;
        waiting_for_user.wait_reason = Some(WaitReason::UserInput);
        assert!(is_resumable_from_context_for_user_message(Some(
            &waiting_for_user
        )));

        let mut waiting_for_approval = ExecutionContext::new("session-2".to_string());
        waiting_for_approval.state = RuntimeState::Waiting;
        waiting_for_approval.wait_reason = Some(WaitReason::Approval);
        assert!(!is_resumable_from_context_for_user_message(Some(
            &waiting_for_approval
        )));

        let mut completed = ExecutionContext::new("session-3".to_string());
        completed.state = RuntimeState::Completed;
        assert!(is_resumable_from_context_for_user_message(Some(&completed)));

        let mut running = ExecutionContext::new("session-4".to_string());
        running.state = RuntimeState::Running;
        assert!(!is_resumable_from_context_for_user_message(Some(&running)));
        assert!(!is_resumable_from_context_for_user_message(None));
    }
}
