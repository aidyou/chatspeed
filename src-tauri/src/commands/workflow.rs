use crate::ai::chat::openai::OpenAIChat;
use crate::ai::interaction::chat_completion::AiChatEnum;
use crate::ai::interaction::chat_completion::ChatState;
use crate::db::{
    Agent, AgentConfig, MainStore, MemoryCandidate, Workflow, WorkflowEfficiencyReport,
    WorkflowMessage, WorkflowSnapshot,
};
use crate::libs::tsid::TsidGenerator;
use crate::workflow::react::child_tasks::get_sub_agent_registry;
use crate::workflow::react::dispatcher::{Dispatcher, DispatcherMetricsSnapshot};
use crate::workflow::react::events::WorkflowEvent;
use crate::workflow::react::gateway::{Gateway, TauriGateway};
use crate::workflow::react::intelligence::IntelligenceManager;
use crate::workflow::react::manager::{ManagedSessionStatus, WorkflowManager};
use crate::workflow::react::memory::MemoryManager;
use crate::workflow::react::orchestrator::{
    list_background_task_ids_for_owner, stop_background_task, BackgroundTask, SubAgentFactory,
    BACKGROUND_TASKS,
};
use crate::workflow::react::replay::{restore_execution_context, RecoveryResult};
use crate::workflow::react::runtime_observation::{
    runtime_observation_metadata, RuntimeObservationType,
};
use crate::workflow::react::signals::SignalType;
use crate::workflow::react::types::{
    ExecutionContext, GatewayPayload, RuntimeState, StepType, SubAgentCompletion, WaitReason,
    WorkflowSignal, WorkflowState,
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

#[cfg(test)]
use rusqlite::params;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowMemoryDiagnostics {
    pub global_memory: Option<String>,
    pub project_memory: Option<String>,
    pub project_key: Option<String>,
    pub global_candidates: Vec<MemoryCandidate>,
    pub project_candidates: Vec<MemoryCandidate>,
}

// ==========================================
// 0. Helper Functions for @mentions
// ==========================================

fn workflow_planning_root(allowed_roots: &[PathBuf]) -> PathBuf {
    allowed_roots
        .first()
        .cloned()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
        .join(".cs")
}

fn reset_workflow_planning_note(allowed_roots: &[PathBuf]) -> Result<(), String> {
    let planning_root = workflow_planning_root(allowed_roots);
    let note_path = planning_root.join("note.md");
    if note_path.exists() {
        std::fs::write(note_path, "").map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn should_treat_as_title_source(message: &WorkflowMessage) -> bool {
    message.role == "user"
        && message
            .step_type
            .as_deref()
            .map_or(true, |step_type| step_type != "observe")
        && !message.message.trim().is_empty()
}

fn spawn_workflow_title_generation_if_missing(
    session_id: String,
    user_query: String,
    state: Arc<std::sync::RwLock<MainStore>>,
    chat_state: Arc<ChatState>,
    gateway: Arc<TauriGateway>,
) -> Result<(), String> {
    if user_query.trim().is_empty() {
        return Ok(());
    }

    let should_generate_title = {
        let store = state.read().map_err(|e| e.to_string())?;
        store
            .get_workflow_snapshot(&session_id)
            .ok()
            .map(|snapshot| {
                snapshot
                    .workflow
                    .title
                    .as_deref()
                    .map_or(true, |title| title.trim().is_empty())
            })
            .unwrap_or(true)
    };

    if !should_generate_title {
        return Ok(());
    }

    let (provider_id, model_name) = {
        let store = state.read().map_err(|e| e.to_string())?;
        let workflow = store
            .get_workflow_snapshot(&session_id)
            .map_err(|e| e.to_string())?
            .workflow;
        let agent_config = workflow
            .agent_config
            .as_deref()
            .and_then(AgentConfig::from_json)
            .unwrap_or_default();
        let model = agent_config
            .models
            .as_ref()
            .and_then(|models| models.act.as_ref().or(models.plan.as_ref()));
        let provider_id = model.map(|model| model.id).unwrap_or(0);
        let model_name = model.map(|model| model.model.clone()).unwrap_or_default();
        (provider_id, model_name)
    };

    let intelligence_manager = IntelligenceManager::new(
        session_id.clone(),
        chat_state,
        provider_id,
        model_name.clone(),
        provider_id,
        model_name,
    );
    tokio::spawn(async move {
        if let Ok(title) = intelligence_manager
            .generate_workflow_title(&user_query)
            .await
        {
            if !title.trim().is_empty() {
                let _ = gateway
                    .send(
                        &session_id,
                        GatewayPayload::WorkflowTitleUpdated {
                            title: title.clone(),
                        },
                    )
                    .await;
            }
        }
    });

    Ok(())
}

fn is_user_authored_workflow_message(message: &WorkflowMessage) -> bool {
    message.role == "user" && message.step_type.as_deref() != Some("observe")
}

fn is_successful_completion_tool_message(message: &WorkflowMessage) -> bool {
    if message.role != "tool" || message.is_error {
        return false;
    }

    let Some(meta) = message.metadata.as_ref() else {
        return false;
    };

    let is_completion_tool = meta
        .get("tool_name")
        .and_then(|v| v.as_str())
        .map(|tool_name| tool_name == crate::tools::TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY)
        .unwrap_or(false);
    if !is_completion_tool {
        return false;
    }

    let execution_status = meta
        .get("execution_status")
        .and_then(|v| v.as_str())
        .unwrap_or("completed");
    let approval_status = meta
        .get("approval_status")
        .and_then(|v| v.as_str())
        .unwrap_or("approved");

    execution_status == "completed" && approval_status != "pending" && approval_status != "rejected"
}

fn latest_successful_completion_index(messages: &[WorkflowMessage]) -> Option<usize> {
    messages
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, message)| {
            if is_successful_completion_tool_message(message) {
                Some(index)
            } else {
                None
            }
        })
}

fn extract_completion_summary_from_tool_message(message: &WorkflowMessage) -> Option<String> {
    if !is_successful_completion_tool_message(message) {
        return None;
    }

    let metadata_summary = message
        .metadata
        .as_ref()
        .and_then(|meta| meta.get("summary"))
        .and_then(|summary| summary.as_str())
        .map(str::trim)
        .filter(|summary| !summary.is_empty())
        .map(str::to_string);
    if metadata_summary.is_some() {
        return metadata_summary;
    }

    let tool_call = message
        .metadata
        .as_ref()
        .and_then(|meta| meta.get("tool_call"))?;
    let args_value = tool_call.get("arguments").or_else(|| {
        tool_call
            .get("function")
            .and_then(|function| function.get("arguments"))
    })?;
    let parsed_args = if let Some(args_text) = args_value.as_str() {
        serde_json::from_str::<serde_json::Value>(args_text).ok()
    } else {
        Some(args_value.clone())
    }?;
    parsed_args
        .get("summary")
        .and_then(|summary| summary.as_str())
        .map(str::trim)
        .filter(|summary| !summary.is_empty())
        .map(str::to_string)
}

fn rollback_workflow_agent_config(
    state: &Arc<std::sync::RwLock<MainStore>>,
    session_id: &str,
    previous_config_json: &str,
) {
    if let Ok(store) = state.read() {
        let _ = store.update_workflow_agent_config(session_id, previous_config_json);
    }
}

fn can_defer_runtime_config_signal_for_completed_session(signal_type: &str) -> bool {
    matches!(
        signal_type,
        "update_allowed_paths"
            | "update_final_audit"
            | "update_auto_compress"
            | "update_model_config"
            | "update_skills_config"
            | "update_approval_level"
            | "update_phase"
            | "remove_auto_approved_tool"
            | "remove_shell_policy_item"
    )
}

fn should_inject_terminal_user_message_into_live_session(
    managed_status: Option<ManagedSessionStatus>,
) -> bool {
    matches!(
        managed_status,
        Some(ManagedSessionStatus::Active | ManagedSessionStatus::Waiting)
    )
}

fn managed_status_blocks_tail_rewind(managed_status: Option<ManagedSessionStatus>) -> bool {
    matches!(
        managed_status,
        Some(
            ManagedSessionStatus::Active
                | ManagedSessionStatus::Waiting
                | ManagedSessionStatus::Stopping
        )
    )
}

async fn inject_runtime_config_signal(
    gateway: &Arc<TauriGateway>,
    workflow_manager: &Arc<WorkflowManager>,
    state: &Arc<std::sync::RwLock<MainStore>>,
    session_id: &str,
    previous_config_json: &str,
    signal: Value,
) -> Result<(), String> {
    if !workflow_manager.has_session(session_id) {
        return Ok(());
    }

    let signal_type = signal
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    if let Err(error) = workflow_manager.validate_signal_routing(session_id, signal_type) {
        if can_defer_runtime_config_signal_for_completed_session(signal_type)
            && workflow_manager.get_session_status(session_id)
                == Some(ManagedSessionStatus::Completed)
        {
            log::info!(
                "[Workflow][session={}][phase=config] Deferred '{}' injection because the live session is completed; updated config will apply on the next resumed turn",
                session_id,
                signal_type
            );
            return Ok(());
        }
        rollback_workflow_agent_config(state, session_id, previous_config_json);
        return Err(format!("Signal rejected: {}", error));
    }

    gateway
        .inject_input(session_id, signal.to_string())
        .await
        .map_err(|error| {
            rollback_workflow_agent_config(state, session_id, previous_config_json);
            format!("Gateway injection failed: {}", error)
        })?;

    Ok(())
}

fn raw_workflow_agent_config_json(store: &MainStore, session_id: &str) -> Result<String, String> {
    store
        .get_workflow(session_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Workflow {} not found", session_id))
        .map(|workflow| workflow.agent_config.unwrap_or_else(|| "{}".to_string()))
}

fn raw_workflow_agent_config(store: &MainStore, session_id: &str) -> Result<AgentConfig, String> {
    Ok(
        AgentConfig::from_json(&raw_workflow_agent_config_json(store, session_id)?)
            .unwrap_or_default(),
    )
}

fn previous_completed_task_context(messages: &[WorkflowMessage]) -> Option<(String, String)> {
    let completion_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter_map(|(index, message)| {
            if is_successful_completion_tool_message(message) {
                Some(index)
            } else {
                None
            }
        })
        .collect();
    let last_completion_index = *completion_indices.last()?;
    let segment_start = completion_indices
        .iter()
        .rev()
        .nth(1)
        .map(|index| index + 1)
        .unwrap_or(0);
    let segment = &messages[segment_start..=last_completion_index];

    let previous_query = segment
        .iter()
        .find(|message| is_user_authored_workflow_message(message))
        .map(|message| message.message.trim().to_string())
        .filter(|value| !value.is_empty())?;

    let previous_summary = segment
        .iter()
        .rev()
        .find_map(extract_completion_summary_from_tool_message)
        .or_else(|| {
            segment
                .iter()
                .rev()
                .find(|message| message.role == "assistant")
                .map(|message| message.message.trim().to_string())
        })
        .filter(|value| !value.is_empty())?;

    Some((previous_query, previous_summary))
}

fn workflow_auto_compress_enabled(config: &Value) -> bool {
    config
        .get("autoCompress")
        .and_then(|value| value.as_bool())
        .or_else(|| {
            config
                .get("auto_compress")
                .and_then(|value| value.as_bool())
        })
        .unwrap_or(true)
}

fn extract_first_json_object(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        return Some(trimmed);
    }

    let fenced = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed)
        .trim();
    let fenced = fenced.strip_suffix("```").unwrap_or(fenced).trim();
    if fenced.starts_with('{') && fenced.ends_with('}') {
        return Some(fenced);
    }

    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    (start < end).then_some(&trimmed[start..=end])
}

async fn classify_related_task_summary(
    chat_state: Arc<ChatState>,
    main_store: Arc<std::sync::RwLock<MainStore>>,
    session_id: &str,
    provider_id: i64,
    model_name: &str,
    previous_query: &str,
    previous_summary: &str,
    new_query: &str,
) -> Result<Option<String>, String> {
    if model_name.trim().is_empty()
        || previous_query.trim().is_empty()
        || previous_summary.trim().is_empty()
        || new_query.trim().is_empty()
    {
        return Ok(None);
    }

    let chat_interface = {
        let mut chats_guard = chat_state.chats.lock().await;
        chats_guard
            .entry(crate::ccproxy::ChatProtocol::OpenAI)
            .or_default()
            .entry(format!("{}_planning_relation", session_id))
            .or_insert_with(|| crate::create_chat!(main_store))
            .clone()
    };

    let messages = vec![
        json!({
            "role": "system",
            "content": "You classify whether a new request continues a previously completed task.\nReturn JSON only with shape: {\"relation\":\"same_task|related_task|new_task\",\"summary_for_planner\":\"...\"}.\nUse `same_task` when the new request continues or revises the same task.\nUse `related_task` when it is strongly related and previous completed work should inform the next plan.\nUse `new_task` when prior work should not be carried forward.\nIf relation is `same_task` or `related_task`, write `summary_for_planner` as a concise markdown summary of the previous completed task that is directly useful for planning the new request. Otherwise return an empty string."
        }),
        json!({
            "role": "user",
            "content": format!(
                "<previous_user_query>\n{}\n</previous_user_query>\n<previous_completed_summary>\n{}\n</previous_completed_summary>\n<new_user_query>\n{}\n</new_user_query>",
                previous_query, previous_summary, new_query
            )
        }),
    ];

    let (tx, mut rx) = tokio::sync::mpsc::channel(100);
    chat_interface
        .chat(
            provider_id,
            model_name,
            format!("{}_planning_relation", session_id),
            messages,
            None,
            None,
            move |chunk| {
                let _ = tx.try_send(chunk);
            },
        )
        .await
        .map_err(|e| e.to_string())?;

    let mut response = String::new();
    while let Some(chunk) = rx.recv().await {
        match chunk.r#type {
            crate::ai::traits::chat::MessageType::Text => response.push_str(&chunk.chunk),
            crate::ai::traits::chat::MessageType::Finished => break,
            crate::ai::traits::chat::MessageType::Error => return Err(chunk.chunk.clone()),
            _ => {}
        }
    }

    #[derive(Deserialize)]
    struct TaskRelationResponse {
        relation: String,
        summary_for_planner: String,
    }

    let parsed = extract_first_json_object(&response)
        .and_then(|json_text| serde_json::from_str::<TaskRelationResponse>(json_text).ok());

    let Some(parsed) = parsed else {
        return Ok(None);
    };

    if matches!(parsed.relation.as_str(), "same_task" | "related_task") {
        let summary = parsed.summary_for_planner.trim().to_string();
        if !summary.is_empty() {
            return Ok(Some(summary));
        }
    }

    Ok(None)
}

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

fn normalize_snapshot_after_live_reconciliation(
    store: &MainStore,
    session_id: &str,
    snapshot: &mut WorkflowSnapshot,
    has_live_session: bool,
) -> Result<(), String> {
    if has_live_session {
        return Ok(());
    }

    if snapshot.workflow.status == WorkflowState::Stopping.to_string() {
        log::info!(
            "[Workflow][session={}][phase=snapshot] Persisted stopping state has no live session; normalizing to cancelled",
            session_id
        );
        store
            .update_workflow_status(session_id, &WorkflowState::Cancelled.to_string())
            .map_err(|e| e.to_string())?;
        snapshot.workflow.status = WorkflowState::Cancelled.to_string();
    }

    Ok(())
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
    config.final_review_mode = Some(
        if config.final_audit.unwrap_or(false) {
            "sub_agent_review"
        } else {
            "off"
        }
        .to_string(),
    );
    config.skill_enabled = agent.skill_enabled;
    config.phase = agent.phase.clone();

    if let Some(approve_str) = &agent.auto_approve {
        config.auto_approve = serde_json::from_str(approve_str).ok();
    }

    config.approval_level = agent.approval_level.clone();

    if let Some(tools_str) = &agent.available_tools {
        config.available_tools = serde_json::from_str(tools_str).ok();
    }

    if let Some(skills_str) = &agent.selected_skills {
        config.selected_skills = serde_json::from_str(skills_str).ok();
    }

    config
}

fn validated_inherited_agent_config(inherited: &str) -> Option<AgentConfig> {
    let mut inherited_config = AgentConfig::from_json(inherited)?;
    inherited_config.sync_legacy_final_audit_flag();

    if let Some(models) = &inherited_config.models {
        let mut validated_models = models.clone();
        for model in [
            &mut validated_models.plan,
            &mut validated_models.act,
            &mut validated_models.vision,
            &mut validated_models.utility,
        ] {
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
        inherited_config.models = Some(validated_models);
    }

    Some(inherited_config)
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

    let merge_missing_model_slots =
        |config_models: &mut Option<crate::db::agent::AgentModels>,
         default_models: &Option<crate::db::agent::AgentModels>| {
            let Some(default_models) = default_models.as_ref() else {
                return false;
            };

            let Some(existing_models) = config_models.as_mut() else {
                *config_models = Some(default_models.clone());
                return true;
            };

            let mut models_changed = false;
            if existing_models.plan.is_none() && default_models.plan.is_some() {
                existing_models.plan = default_models.plan.clone();
                models_changed = true;
            }
            if existing_models.act.is_none() && default_models.act.is_some() {
                existing_models.act = default_models.act.clone();
                models_changed = true;
            }
            if existing_models.vision.is_none() && default_models.vision.is_some() {
                existing_models.vision = default_models.vision.clone();
                models_changed = true;
            }
            if existing_models.utility.is_none() && default_models.utility.is_some() {
                existing_models.utility = default_models.utility.clone();
                models_changed = true;
            }
            models_changed
        };

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
    if config.final_audit.is_none() && defaults.final_audit.is_some() {
        config.final_audit = defaults.final_audit;
        changed = true;
    }
    if config.final_review_mode.is_none() && defaults.final_review_mode.is_some() {
        config.final_review_mode = defaults.final_review_mode;
        changed = true;
    }
    if config.skill_enabled.is_none() && defaults.skill_enabled.is_some() {
        config.skill_enabled = defaults.skill_enabled;
        changed = true;
    }
    if config.selected_skills.is_none() && defaults.selected_skills.is_some() {
        config.selected_skills = defaults.selected_skills;
        changed = true;
    }
    if config.phase.is_none() && defaults.phase.is_some() {
        config.phase = defaults.phase;
        changed = true;
    }
    if merge_missing_model_slots(&mut config.models, &defaults.models) {
        changed = true;
    }
    if config.max_contexts.is_none() && defaults.max_contexts.is_some() {
        config.max_contexts = defaults.max_contexts;
        changed = true;
    }

    changed
}

fn normalize_workflow_agent_config_inner(
    store: &MainStore,
    workflow: &mut Workflow,
    persist: bool,
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
    config.sync_legacy_final_audit_flag();

    let shell_policy_missing =
        config.shell_policy.is_none() && agent_shell_policy_value(&agent).is_some();
    let missing_fields_filled = fill_missing_agent_config_fields(&mut config, &agent);

    if missing_fields_filled || shell_policy_missing {
        let normalized_config = agent_config_to_json_with_agent_shell_policy(&config, &agent)?;
        if persist {
            store
                .update_workflow_agent_config(
                    workflow.id.as_deref().unwrap_or_default(),
                    &normalized_config,
                )
                .map_err(|e| e.to_string())?;
        }
        workflow.agent_config = Some(normalized_config);
    }

    Ok(())
}

/// Normalize agent config in memory only, without DB write.
/// Use this for read-only operations like `get_workflow_snapshot`
/// to avoid silently updating `updated_at` and changing sort order.
fn normalize_workflow_agent_config_in_memory(
    store: &MainStore,
    workflow: &mut Workflow,
) -> Result<(), String> {
    normalize_workflow_agent_config_inner(store, workflow, false)
}

#[tauri::command]
pub async fn create_workflow(
    tsid_generator: State<'_, Arc<TsidGenerator>>,
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    gateway: State<'_, Arc<TauriGateway>>,
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
    // Workflow/session overrides take precedence over agent defaults here.
    if let Some(inherited) = &request.inherited_agent_config {
        if let Some(inherited_config) = validated_inherited_agent_config(inherited) {
            config.apply_overrides(&inherited_config);
        }
    }

    fill_missing_agent_config_fields(&mut config, &agent);

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

    let _ = spawn_workflow_title_generation_if_missing(
        session_id.clone(),
        user_query.to_string(),
        state.inner().clone(),
        chat_state.inner().clone(),
        gateway.inner().clone(),
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
                    current_segment_id: 1,
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
                    pending_final_review: None,
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
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: Some("sub_agent_interrupted".to_string()),
                metadata: Some({
                    let mut metadata = runtime_observation_metadata(
                        RuntimeObservationType::SubAgentInterrupted,
                        json!({
                            "sub_agent_id": child_id.clone(),
                            "summary": "Sub-agent interrupted",
                            "result": {
                                "status": "interrupted",
                                "task_id": child_id.clone(),
                                "error": "Sub-agent interrupted by application restart",
                                "tool_calls_count": context.pending_tools.len()
                            },
                        }),
                    );
                    metadata["sub_agent_id"] = json!(child_id.clone());
                    metadata["summary"] = json!("Sub-agent interrupted");
                    metadata["result"] = json!({
                        "status": "interrupted",
                        "task_id": child_id.clone(),
                        "error": "Sub-agent interrupted by application restart",
                        "tool_calls_count": context.pending_tools.len()
                    });
                    metadata["execution_status"] = json!("interrupted");
                    metadata["is_error"] = json!(true);
                    metadata["error_type"] = json!("SubAgentInterrupted");
                    metadata
                }),
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
                        result: None,
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
pub async fn delete_last_workflow_message(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    gateway: State<'_, Arc<TauriGateway>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
) -> Result<bool, String> {
    cleanup_workflow_resources(
        &session_id,
        chat_state.inner(),
        gateway.inner(),
        workflow_manager.inner(),
    )
    .await;

    let store = state.read().map_err(|e| e.to_string())?;
    store
        .delete_last_message(&session_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_workflow_snapshot(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
) -> Result<Value, String> {
    let main_store = state.inner().clone();
    let mut snapshot = {
        let store = main_store.read().map_err(|e| e.to_string())?;
        let mut snapshot = store
            .get_workflow_snapshot(&session_id)
            .map_err(|e| e.to_string())?;
        normalize_workflow_agent_config_in_memory(&store, &mut snapshot.workflow)?;
        snapshot
    };

    // Phase 0-3 UI State Reconciliation: Add hasLiveSession field.
    // Reconcile terminal executors first so the frontend does not keep seeing
    // zombie runtime sessions after a turn has already finished.
    let workflow_manager_arc = workflow_manager.inner().clone();
    let has_live_session =
        has_reconciled_live_session(&workflow_manager_arc, &session_id, "snapshot").await;
    let has_blocking_live_session = if has_live_session {
        managed_status_blocks_tail_rewind(workflow_manager_arc.get_session_status(&session_id))
    } else {
        false
    };
    {
        let store = main_store.read().map_err(|e| e.to_string())?;
        normalize_snapshot_after_live_reconciliation(
            &store,
            &session_id,
            &mut snapshot,
            has_live_session,
        )?;
    }
    let merged_messages = merge_ui_workflow_messages(&snapshot.messages);
    let execution_context = restore_context_for_signal(main_store.clone(), &session_id);
    let tail_rewind_kind = {
        let store = main_store.read().map_err(|e| e.to_string())?;
        store
            .get_tail_rewind_kind(&session_id)
            .map_err(|e| e.to_string())?
    };
    let can_rewind_tail = !has_blocking_live_session && tail_rewind_kind.is_some();

    // Convert snapshot to JSON and inject hasLiveSession
    let mut snapshot_json = serde_json::to_value(&snapshot).map_err(|e| e.to_string())?;
    if let Some(obj) = snapshot_json.as_object_mut() {
        obj.insert("messages".to_string(), json!(merged_messages));
        obj.insert("hasLiveSession".to_string(), json!(has_live_session));
        obj.insert(
            "hasBlockingLiveSession".to_string(),
            json!(has_blocking_live_session),
        );
        obj.insert("executionContext".to_string(), json!(execution_context));
        obj.insert("canRewindTail".to_string(), json!(can_rewind_tail));
        obj.insert("tailRewindKind".to_string(), json!(tail_rewind_kind));
    }

    log::debug!(
        "[Workflow][session={}][command=get_workflow_snapshot] hasLiveSession={}",
        session_id,
        has_live_session
    );

    Ok(snapshot_json)
}

#[tauri::command]
pub async fn get_workflow_agent_config(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    session_id: String,
) -> Result<Value, String> {
    let store = state.read().map_err(|e| e.to_string())?;
    let config_json = raw_workflow_agent_config_json(&store, &session_id)?;
    serde_json::from_str::<Value>(&config_json).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn add_workflow_message(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    gateway: State<'_, Arc<TauriGateway>>,
    message: WorkflowMessage,
) -> Result<i64, String> {
    let store = state.read().map_err(|e| e.to_string())?;
    let res = store
        .add_workflow_message(&message)
        .map_err(|e| e.to_string())?;
    drop(store);

    if should_treat_as_title_source(&message) {
        let _ = spawn_workflow_title_generation_if_missing(
            message.session_id.clone(),
            message.message.clone(),
            state.inner().clone(),
            chat_state.inner().clone(),
            gateway.inner().clone(),
        );
    }

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

    let managed_status = workflow_manager.get_session_status(session_id);
    if let Some(status) = managed_status.clone() {
        if matches!(
            status,
            ManagedSessionStatus::Failed | ManagedSessionStatus::Cancelled
        ) {
            log::info!(
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
        log::info!(
            "[Workflow][session={}][phase={}] Manager had session entry without executor, removing stale entry",
            session_id,
            phase
        );
        workflow_manager.remove_session(session_id);
        return false;
    };

    let Ok(executor_guard) = executor.try_lock() else {
        if phase != "stop" {
            log::debug!(
                "[Workflow][session={}][phase={}] Session exists and executor is busy; treating as live",
                session_id,
                phase
            );
        }
        return true;
    };

    let executor_state = executor_guard.state();
    if !matches!(managed_status, Some(ManagedSessionStatus::Stopping)) {
        let _ = workflow_manager
            .reconcile_session_status_from_workflow_state(session_id, &executor_state);
    }

    if matches!(
        executor_state,
        WorkflowState::Error | WorkflowState::Cancelled
    ) {
        log::info!(
            "[Workflow][session={}][phase={}] Found stale live session in terminal state={}, removing before command handling",
            session_id,
            phase,
            executor_state
        );
        workflow_manager.remove_session(session_id);
        WorkflowManager::unregister_session_signal_tx_with_source(
            session_id,
            "has_reconciled_live_session.terminal_state_recovery",
        );
        return false;
    }

    true
}

async fn append_initial_prompt_to_executor(
    executor: &mut dyn crate::workflow::react::engine::ReActExecutor,
    raw_prompt: &str,
    clean_prompt: &str,
    attached_context: &str,
    message_metadata: Option<Value>,
    related_task_summary: Option<&str>,
    begin_new_segment: bool,
) -> Result<(), crate::workflow::react::error::WorkflowEngineError> {
    const NEW_TASK_AFTER_COMPLETION_REMINDER: &str = "<SYSTEM_REMINDER>This is a new task following earlier completed work. Refer to previously completed tasks only if they are directly relevant to the current request. If they are not relevant, prioritize the current task and avoid being distracted by older context.</SYSTEM_REMINDER>";

    if begin_new_segment {
        executor.begin_new_context_segment().await?;
    }

    if let Some(previous_summary) = related_task_summary {
        executor
            .add_message_and_notify(
                "system".into(),
                format!(
                    "# Related Previous Task Context\n<previous_task_summary>\n{}\n</previous_task_summary>",
                    previous_summary
                ),
                None,
                None,
                None,
                false,
                None,
                Some(json!({
                    "type": "summary",
                    "subtype": "related_previous_task"
                })),
            )
            .await?;
    }

    let att_opt = if attached_context.is_empty() {
        None
    } else {
        Some(attached_context.to_string())
    };

    let messages = executor.messages();
    let has_previous_completed_work = latest_successful_completion_index(&messages).is_some();
    let active_segment_start = latest_successful_completion_index(&messages)
        .map(|index| index + 1)
        .unwrap_or(0);
    let is_duplicate = messages.iter().skip(active_segment_start).any(|m| {
        m.role == "user"
            && m.step_type.as_deref() != Some("observe")
            && (m.message == clean_prompt || m.message == raw_prompt)
    });

    if !is_duplicate {
        let user_content = if begin_new_segment && has_previous_completed_work {
            format!("{}\n\n{}", clean_prompt, NEW_TASK_AFTER_COMPLETION_REMINDER)
        } else {
            clean_prompt.to_string()
        };

        executor
            .add_message_and_notify(
                "user".into(),
                user_content,
                att_opt,
                None,
                None,
                false,
                None,
                message_metadata,
            )
            .await?;
    } else {
        log::info!("[Workflow] Skipping duplicate message on resume");
    }

    Ok(())
}

async fn try_resume_completed_live_session(
    session_id: &str,
    raw_prompt: &str,
    clean_prompt: &str,
    attached_context: &str,
    message_metadata: Option<Value>,
    related_task_summary: Option<&str>,
    gateway: &Arc<TauriGateway>,
    workflow_manager: &Arc<WorkflowManager>,
    main_store: &Arc<std::sync::RwLock<MainStore>>,
) -> Result<bool, String> {
    if !workflow_manager.transition_session_status_if_current(
        session_id,
        ManagedSessionStatus::Completed,
        ManagedSessionStatus::Active,
    ) {
        return Ok(false);
    }

    let Some(shared_executor) = workflow_manager.get_executor(session_id) else {
        workflow_manager.remove_session(session_id);
        WorkflowManager::unregister_session_signal_tx_with_source(
            session_id,
            "workflow_start.resume_completed.missing_executor",
        );
        gateway
            .unregister_session_with_source(
                session_id,
                "workflow_start.resume_completed.missing_executor",
            )
            .await;
        return Ok(false);
    };

    {
        let mut executor = shared_executor.lock().await;
        if executor.state() != WorkflowState::Completed {
            let _ = workflow_manager
                .reconcile_session_status_from_workflow_state(session_id, &executor.state());
            return Ok(false);
        }

        executor
            .prepare_completed_resume()
            .await
            .map_err(|e| e.to_string())?;

        append_initial_prompt_to_executor(
            &mut *executor,
            raw_prompt,
            clean_prompt,
            attached_context,
            message_metadata,
            related_task_summary,
            true,
        )
        .await
        .map_err(|e| e.to_string())?;
    }

    let (signal_tx, signal_rx) = tokio::sync::mpsc::channel(100);
    {
        let mut executor = shared_executor.lock().await;
        executor.attach_signal_rx(signal_rx);
    }
    gateway
        .register_session_tx_with_source(
            session_id.to_string(),
            signal_tx.clone(),
            "workflow_start.resume_completed",
        )
        .await;
    WorkflowManager::register_session_signal_tx_with_source(
        session_id.to_string(),
        signal_tx,
        "workflow_start.resume_completed",
    );

    let session_id_for_spawn = session_id.to_string();
    let gateway_for_spawn = gateway.clone();
    let manager_for_spawn = workflow_manager.clone();
    let main_store_for_spawn = main_store.clone();
    tokio::spawn(async move {
        let mut guard = shared_executor.lock().await;
        if let Err(e) = guard.run_loop().await {
            if let crate::workflow::react::error::WorkflowEngineError::Cancelled(_) = e {
                if let Ok(store) = main_store_for_spawn.read() {
                    let _ = store.update_workflow_status(
                        &session_id_for_spawn,
                        &WorkflowState::Cancelled.to_string(),
                    );
                }
                let _ = manager_for_spawn
                    .update_session_status(&session_id_for_spawn, ManagedSessionStatus::Cancelled);
                let _ = gateway_for_spawn
                    .send(
                        &session_id_for_spawn,
                        crate::workflow::react::types::GatewayPayload::State {
                            state: WorkflowState::Cancelled,
                            wait_reason: None,
                        },
                    )
                    .await;
                BACKGROUND_TASKS.remove(&session_id_for_spawn);
                WorkflowManager::unregister_session_signal_tx_with_source(
                    &session_id_for_spawn,
                    "workflow_start.resume_completed.run_loop.cancelled",
                );
                gateway_for_spawn
                    .unregister_session_with_source(
                        &session_id_for_spawn,
                        "workflow_start.resume_completed.run_loop.cancelled",
                    )
                    .await;
                manager_for_spawn.remove_session(&session_id_for_spawn);
                return;
            }

            log::error!(
                "[Workflow][session={}][phase=run_loop][event=crash] Workflow error after completed-session resume: {:?}",
                session_id_for_spawn,
                e
            );
            let _ = manager_for_spawn
                .update_session_status(&session_id_for_spawn, ManagedSessionStatus::Failed);
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
            if let Some(completed_at_ms) =
                manager_for_spawn.get_session_updated_at_ms(&session_id_for_spawn)
            {
                schedule_completed_session_cleanup(
                    session_id_for_spawn.clone(),
                    completed_at_ms,
                    gateway_for_spawn.clone(),
                    manager_for_spawn.clone(),
                );
            }
        } else {
            BACKGROUND_TASKS.remove(&session_id_for_spawn);
            WorkflowManager::unregister_session_signal_tx_with_source(
                &session_id_for_spawn,
                "workflow_start.resume_completed.run_loop.finalize",
            );
            gateway_for_spawn
                .unregister_session_with_source(
                    &session_id_for_spawn,
                    "workflow_start.resume_completed.run_loop.finalize",
                )
                .await;
            manager_for_spawn.remove_session(&session_id_for_spawn);
        }
    });

    Ok(true)
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
            WorkflowManager::unregister_session_signal_tx_with_source(
                &task_id,
                "cleanup_owned_background_resources.reclaim_sub_agent",
            );
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
    WorkflowManager::unregister_session_signal_tx_with_source(
        session_id,
        "cleanup_workflow_resources.finalize",
    );
    gateway
        .unregister_session_with_source(session_id, "cleanup_workflow_resources.finalize")
        .await;
}

const COMPLETED_SESSION_CLEANUP_DELAY_SECS: u64 = 600;

fn schedule_completed_session_cleanup(
    session_id: String,
    completed_at_ms: i64,
    gateway: Arc<TauriGateway>,
    workflow_manager: Arc<WorkflowManager>,
) {
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(
            COMPLETED_SESSION_CLEANUP_DELAY_SECS,
        ))
        .await;

        let should_cleanup = matches!(
            workflow_manager.get_session_status(&session_id),
            Some(ManagedSessionStatus::Completed)
        ) && workflow_manager
            .get_session_updated_at_ms(&session_id)
            .is_some_and(|current_updated_at| current_updated_at == completed_at_ms);

        if !should_cleanup {
            return;
        }

        log::info!(
            "[Workflow][session={}][phase=cleanup] Completed-session grace period elapsed; removing inactive completed session",
            session_id
        );

        let removed =
            workflow_manager.remove_session_if_updated_matches(&session_id, completed_at_ms);
        if removed.is_some() {
            BACKGROUND_TASKS.remove(&session_id);
            WorkflowManager::unregister_session_signal_tx_with_source(
                &session_id,
                "completed_session_cleanup_timer",
            );
            gateway
                .unregister_session_with_source(&session_id, "completed_session_cleanup_timer")
                .await;
        }
    });
}

fn compat_wait_reason_from_snapshot_status(status: &str) -> Option<WaitReason> {
    compat_snapshot_workflow_state(status).and_then(wait_reason_for_workflow_state)
}

fn compat_snapshot_workflow_state(status: &str) -> Option<WorkflowState> {
    use std::str::FromStr;

    match status {
        "failed" => Some(WorkflowState::Error),
        "awaitingapproval" => Some(WorkflowState::AwaitingApproval),
        other => WorkflowState::from_str(other).ok(),
    }
}

fn wait_reason_for_workflow_state(state: WorkflowState) -> Option<WaitReason> {
    match state {
        WorkflowState::Paused => Some(WaitReason::Confirmation),
        WorkflowState::AwaitingUser => Some(WaitReason::UserInput),
        WorkflowState::AwaitingApproval | WorkflowState::AwaitingAutoApproval => {
            Some(WaitReason::Approval)
        }
        WorkflowState::AwaitingSubAgent => Some(WaitReason::SubAgent),
        _ => None,
    }
}

fn compat_is_terminal_snapshot_status(status: &str) -> bool {
    matches!(
        compat_snapshot_workflow_state(status),
        Some(WorkflowState::Completed | WorkflowState::Error | WorkflowState::Cancelled)
    )
}

fn compat_is_resumable_snapshot_status_for_user_message(status: &str) -> bool {
    matches!(
        compat_snapshot_workflow_state(status),
        Some(
            WorkflowState::Pending
                | WorkflowState::Completed
                | WorkflowState::Cancelled
                | WorkflowState::Error
                | WorkflowState::AwaitingUser
        )
    )
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
                log::info!(
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
            RuntimeState::Stopping => false,
            RuntimeState::Running => false,
        },
        None => false,
    }
}

fn can_resume_user_message_from_recovery(
    ctx: Option<&ExecutionContext>,
    snapshot_status: &str,
) -> bool {
    compat_is_terminal_snapshot_status(snapshot_status)
        || is_resumable_from_context_for_user_message(ctx)
        || (ctx.is_none() && compat_is_resumable_snapshot_status_for_user_message(snapshot_status))
}

fn signal_json_content(signal: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(signal)
        .ok()
        .and_then(|value| value["content"].as_str().map(|content| content.to_string()))
}

fn signal_json_attached_context(signal: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(signal)
        .ok()
        .and_then(|value| {
            value["attached_context"]
                .as_str()
                .map(|content| content.to_string())
                .or_else(|| {
                    value["attachedContext"]
                        .as_str()
                        .map(|content| content.to_string())
                })
        })
}

fn signal_json_metadata(signal: &str) -> Option<Value> {
    serde_json::from_str::<serde_json::Value>(signal)
        .ok()
        .and_then(|value| value.get("metadata").cloned())
}

fn combine_attached_context(base: String, extra: Option<String>) -> String {
    let extra = extra.unwrap_or_default();
    if base.is_empty() {
        return extra;
    }
    if extra.is_empty() {
        return base;
    }
    format!("{}\n\n{}", base, extra)
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
    initial_metadata: Option<Value>,
    initial_attached_context: Option<String>,
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

    let (
        raw_prompt,
        clean_prompt,
        attached_context,
        initial_message_metadata,
        allowed_paths,
        workflow_status,
        existing_messages,
    ) = {
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
            (prompt.clone(), String::new())
        };

        (
            prompt,
            p,
            combine_attached_context(att, initial_attached_context.clone()),
            initial_metadata.clone(),
            paths,
            wf.status.clone(),
            snapshot.messages,
        )
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

    let is_terminal_workflow = compat_is_terminal_snapshot_status(&workflow_status);

    let related_task_summary = if planning_mode && initial_prompt.is_some() && is_terminal_workflow
    {
        let previous_context = previous_completed_task_context(&existing_messages);
        let planning_model = agent_config
            .models
            .as_ref()
            .and_then(|models| models.plan.as_ref().or(models.act.as_ref()));
        if let (Some((previous_query, previous_summary)), Some(model)) =
            (previous_context, planning_model)
        {
            classify_related_task_summary(
                chat_state_arc.clone(),
                main_store_arc.clone(),
                &session_id,
                model.id,
                &model.model,
                &previous_query,
                &previous_summary,
                &clean_prompt,
            )
            .await?
        } else {
            None
        }
    } else {
        None
    };

    if initial_prompt.is_some() {
        if try_resume_completed_live_session(
            &session_id,
            &raw_prompt,
            &clean_prompt,
            &attached_context,
            initial_message_metadata.clone(),
            related_task_summary.as_deref(),
            &gateway_arc,
            &workflow_manager_arc,
            &main_store_arc,
        )
        .await?
        {
            log::info!(
                "[Workflow][session={}][phase=start] Resumed completed live session without rebuilding executor",
                session_id
            );
            return Ok(session_id);
        }
    }

    // Reject only truly live sessions. A completed executor may still exist in
    // memory for a short window and can be resumed above without rebuilding.
    if has_reconciled_live_session(&workflow_manager_arc, &session_id, "start").await {
        if matches!(
            workflow_manager_arc.get_session_status(&session_id),
            Some(ManagedSessionStatus::Stopping)
        ) {
            log::info!(
                "[Workflow][session={}][phase=start] Session is still stopping, rejecting restart",
                session_id
            );
            return Err(format!("Session is stopping: {}", session_id));
        }
        log::info!(
            "[Workflow][session={}][phase=start] Session already exists in WorkflowManager, rejecting duplicate start",
            session_id
        );
        return Err(format!("Session already exists: {}", session_id));
    }

    let (signal_tx, signal_rx) = tokio::sync::mpsc::channel(100);
    gateway_arc
        .register_session_tx_with_source(session_id.clone(), signal_tx.clone(), "workflow_start")
        .await;
    WorkflowManager::register_session_signal_tx_with_source(
        session_id.clone(),
        signal_tx,
        "workflow_start",
    );

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

    // Persisted phase is authoritative for existing workflows. This prevents a stale
    // frontend planning toggle from re-entering strict planning after a plan was approved.
    let persisted_phase = agent_config_json
        .get("phase")
        .and_then(|v| v.as_str())
        .and_then(|p_str| {
            use std::str::FromStr;
            crate::workflow::react::policy::ExecutionPhase::from_str(p_str).ok()
        });
    let mut policy = match persisted_phase {
        Some(crate::workflow::react::policy::ExecutionPhase::Planning) => {
            crate::workflow::react::policy::ExecutionPolicy::planning_strict()
        }
        Some(crate::workflow::react::policy::ExecutionPhase::Implementation) => {
            crate::workflow::react::policy::ExecutionPolicy::implementation()
        }
        Some(crate::workflow::react::policy::ExecutionPhase::Standard) => {
            crate::workflow::react::policy::ExecutionPolicy::standard()
        }
        None => {
            if planning_mode {
                crate::workflow::react::policy::ExecutionPolicy::planning_strict()
            } else {
                crate::workflow::react::policy::ExecutionPolicy::standard()
            }
        }
    };
    let auto_compress_enabled = workflow_auto_compress_enabled(&agent_config_json);

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

    if planning_mode {
        reset_workflow_planning_note(&allowed_roots)?;
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
                auto_compress_enabled,
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
                auto_compress_enabled,
                policy,
            ),
        ))
    };

    {
        let mut executor = shared_executor.lock().await;
        executor.init().await.map_err(|e| e.to_string())?;

        if initial_prompt.is_some() {
            append_initial_prompt_to_executor(
                &mut *executor,
                &raw_prompt,
                &clean_prompt,
                &attached_context,
                initial_message_metadata.clone(),
                related_task_summary.as_deref(),
                is_terminal_workflow,
            )
            .await
            .map_err(|e| e.to_string())?;
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
            output_accessible: false,
        },
    );

    log::info!(
        "[Workflow][session={}][phase=start] Executor registered to WorkflowManager (primary) and BACKGROUND_TASKS (compat), spawning run_loop",
        session_id
    );

    let session_id_for_spawn = session_id.clone();
    let gateway_for_spawn = gateway_arc.clone();
    let manager_for_spawn = workflow_manager_arc.clone();
    let main_store_for_spawn = main_store_arc.clone();
    tokio::spawn(async move {
        let mut guard = shared_executor.lock().await;
        if let Err(e) = guard.run_loop().await {
            if let crate::workflow::react::error::WorkflowEngineError::Cancelled(_) = e {
                if let Ok(store) = main_store_for_spawn.read() {
                    let _ = store.update_workflow_status(
                        &session_id_for_spawn,
                        &WorkflowState::Cancelled.to_string(),
                    );
                }
                let _ = manager_for_spawn
                    .update_session_status(&session_id_for_spawn, ManagedSessionStatus::Cancelled);
                log::info!(
                    "[Workflow][session={}][phase=run_loop][event=cancelled] Workflow session was cancelled by user",
                    session_id_for_spawn
                );
                let _ = gateway_for_spawn
                    .send(
                        &session_id_for_spawn,
                        crate::workflow::react::types::GatewayPayload::State {
                            state: WorkflowState::Cancelled,
                            wait_reason: None,
                        },
                    )
                    .await;
                BACKGROUND_TASKS.remove(&session_id_for_spawn);
                WorkflowManager::unregister_session_signal_tx_with_source(
                    &session_id_for_spawn,
                    "workflow_start.run_loop.cancelled",
                );
                gateway_for_spawn
                    .unregister_session_with_source(
                        &session_id_for_spawn,
                        "workflow_start.run_loop.cancelled",
                    )
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
            if let Some(completed_at_ms) =
                manager_for_spawn.get_session_updated_at_ms(&session_id_for_spawn)
            {
                schedule_completed_session_cleanup(
                    session_id_for_spawn.clone(),
                    completed_at_ms,
                    gateway_for_spawn.clone(),
                    manager_for_spawn.clone(),
                );
            }
        } else {
            BACKGROUND_TASKS.remove(&session_id_for_spawn);
            WorkflowManager::unregister_session_signal_tx_with_source(
                &session_id_for_spawn,
                "workflow_start.run_loop.finalize",
            );
            gateway_for_spawn
                .unregister_session_with_source(
                    &session_id_for_spawn,
                    "workflow_start.run_loop.finalize",
                )
                .await;
            manager_for_spawn.remove_session(&session_id_for_spawn);
        }
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
    log::info!(
        "[Workflow][session={}][phase=approve_plan] Legacy workflow_approve_plan called for agent_id={}; routing through workflow_signal approval",
        session_id,
        agent_id
    );

    let submit_plan_tool_call_id =
        restore_context_for_signal(main_store.inner().clone(), &session_id)
            .and_then(|context| {
                context
                    .pending_tools
                    .into_iter()
                    .find(|tool| tool.tool_name == crate::tools::TOOL_SUBMIT_PLAN)
                    .map(|tool| tool.tool_call_id)
            })
            .ok_or_else(|| {
                format!(
                "Cannot approve plan: no pending structured submit_plan approval for workflow {}",
                session_id
            )
            })?;

    let signal = json!({
        "type": SignalType::Approval.as_str(),
        "id": submit_plan_tool_call_id,
        "approved": true,
        "approve_all": false,
        "metadata": {
            "source": "workflow_approve_plan_compat",
            "legacy_plan_length": plan.len()
        }
    })
    .to_string();

    workflow_signal(
        app,
        main_store,
        chat_state,
        tsid_generator,
        gateway,
        factory,
        workflow_manager,
        session_id,
        signal,
    )
    .await
    .map(|_| ())
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

        let snapshot_is_terminal =
            compat_is_terminal_snapshot_status(&workflow_snapshot.workflow.status);
        let should_force_recovery_for_terminal_user_message = snapshot_is_terminal
            && matches!(
                signal_type_enum,
                Some(SignalType::UserMessage | SignalType::LegacyUserInput)
            );

        if should_force_recovery_for_terminal_user_message {
            let managed_status_for_terminal_user_message =
                workflow_manager_arc.get_session_status(&session_id);
            if should_inject_terminal_user_message_into_live_session(
                managed_status_for_terminal_user_message.clone(),
            ) {
                log::info!(
                    "[Workflow][session={}][phase=signal] Snapshot is terminal but live session status is {:?}; injecting user message into the hot executor instead of rebuilding",
                    session_id,
                    managed_status_for_terminal_user_message
                );
                match gateway_arc.inject_input(&session_id, signal.clone()).await {
                    Ok(_) => {
                        return Ok("Signal injected".to_string());
                    }
                    Err(e) => {
                        if is_stale_gateway_injection_error(&e) {
                            log::info!(
                                "[Workflow][session={}][phase=signal] Hot terminal-session injection hit stale gateway state: {}. Falling back to completed-session recovery.",
                                session_id,
                                e
                            );
                            workflow_manager_arc.remove_session(&session_id);
                            WorkflowManager::unregister_session_signal_tx_with_source(
                                &session_id,
                                "workflow_signal.recovery.terminal_user_message_stale_injection",
                            );
                            gateway_arc
                                .unregister_session_with_source(
                                    &session_id,
                                    "workflow_signal.recovery.terminal_user_message_stale_injection",
                                )
                                .await;
                        } else {
                            return Err(format!("Gateway injection failed: {}", e));
                        }
                    }
                }
            }

            log::info!(
                "[Workflow][session={}][phase=signal] Live session is completed and snapshot is terminal (status={}). Treating user message as completed-session resume request.",
                session_id,
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
                session_id.clone(),
                workflow_snapshot.workflow.agent_id.clone(),
                signal_json_content(&signal),
                signal_json_metadata(&signal),
                signal_json_attached_context(&signal),
                None,
            )
            .await?;
            return Ok("Workflow resumed with input".to_string());
        } else {
            // Route signal through manager
            if let Err(e) = workflow_manager_arc.validate_signal_routing(&session_id, &signal_type)
            {
                let is_stopping_session = matches!(
                    workflow_manager_arc.get_session_status(&session_id),
                    Some(ManagedSessionStatus::Stopping)
                );
                if is_stopping_session {
                    log::info!(
                        "[Workflow][session={}][phase=signal] Signal '{}' rejected because session is stopping",
                        session_id,
                        signal_type
                    );
                    return Err(format!("Signal rejected: {}", e));
                }
                let allow_recovery_for_terminal_user_message = matches!(
                    signal_type_enum,
                    Some(SignalType::UserMessage | SignalType::LegacyUserInput)
                );
                if allow_recovery_for_terminal_user_message {
                    log::info!(
                        "[Workflow][session={}][phase=signal] Live session rejected user message with '{}'. Treating as stale terminal session and entering recovery.",
                        session_id,
                        e
                    );
                    workflow_manager_arc.remove_session(&session_id);
                    WorkflowManager::unregister_session_signal_tx_with_source(
                        &session_id,
                        "workflow_signal.recovery.signal_rejected_user_message",
                    );
                    gateway_arc
                        .unregister_session_with_source(
                            &session_id,
                            "workflow_signal.recovery.signal_rejected_user_message",
                        )
                        .await;
                    true
                } else {
                    log::info!(
                        "[WorkflowManager][session={}][event=signal_rejected] Signal '{}' rejected: {}",
                        session_id,
                        signal_type,
                        e
                    );
                    return Err(format!("Signal rejected: {}", e));
                }
            } else {
                log::info!(
                    "[Workflow][session={}][phase=signal][event=signal_injection_start] Signal '{}' validated; injecting through gateway",
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
                            log::info!(
                                "[Workflow][session={}][phase=signal] Gateway injection hit stale live session: {}. Removing stale session and entering recovery.",
                                session_id,
                                e
                            );
                            workflow_manager_arc.remove_session(&session_id);
                            WorkflowManager::unregister_session_signal_tx_with_source(
                                &session_id,
                                "workflow_signal.recovery.gateway_injection_stale",
                            );
                            gateway_arc
                                .unregister_session_with_source(
                                    &session_id,
                                    "workflow_signal.recovery.gateway_injection_stale",
                                )
                                .await;
                            true
                        } else {
                            log::info!(
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
                .or_else(|| {
                    compat_wait_reason_from_snapshot_status(&workflow_snapshot.workflow.status)
                });

            // Handle user_message signal (Phase 3 unified signal type)
            // Also support legacy user_input for backward compatibility
            if matches!(
                signal_type_enum,
                Some(SignalType::UserMessage | SignalType::LegacyUserInput)
            ) {
                let can_queue_during_approval = effective_wait_reason == Some(WaitReason::Approval)
                    || (recovery_context.is_none()
                        && matches!(
                            compat_snapshot_workflow_state(&workflow_snapshot.workflow.status),
                            Some(
                                WorkflowState::AwaitingApproval
                                    | WorkflowState::AwaitingAutoApproval
                            )
                        ));

                // When recovery_context is None (for example a brand-new workflow with no
                // persisted execution history yet), fall back to the durable workflow status.
                let is_resumable = can_resume_user_message_from_recovery(
                    recovery_context.as_ref(),
                    &workflow_snapshot.workflow.status,
                );

                if !is_resumable {
                    log::info!(
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
                    session_id.clone(),
                    workflow_snapshot.workflow.agent_id.clone(),
                    if can_queue_during_approval {
                        None
                    } else {
                        val["content"].as_str().map(|content| content.to_string())
                    },
                    signal_json_metadata(&signal),
                    signal_json_attached_context(&signal),
                    None,
                )
                .await?;

                if can_queue_during_approval {
                    let mut retries = 5;
                    let mut last_error = None;
                    while retries > 0 {
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                        match gateway_arc.inject_input(&session_id, signal.clone()).await {
                            Ok(_) => {
                                log::info!(
                                    "[Workflow] user_message injected successfully after approval-wait recovery"
                                );
                                break;
                            }
                            Err(e) => {
                                last_error = Some(e);
                                retries -= 1;
                                log::info!(
                                    "[Workflow] Failed to inject user_message during approval-wait recovery, retries left: {}",
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
                            "Failed to inject user_message after resuming: {}",
                            err_msg
                        ));
                    }

                    return Ok("Workflow resumed and user message queued".to_string());
                }

                return Ok("Workflow resumed with input".to_string());
            } else if signal_type == "rebroadcast_pending"
                || signal_type == "request_confirm_broadcast"
            {
                let can_rebroadcast = effective_wait_reason == Some(WaitReason::Approval)
                    || (recovery_context.is_none()
                        && matches!(
                            compat_snapshot_workflow_state(&workflow_snapshot.workflow.status),
                            Some(
                                WorkflowState::AwaitingApproval
                                    | WorkflowState::AwaitingAutoApproval
                            )
                        ));
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
                    None,
                    None,
                )
                .await?;

                return Ok("Workflow resumed and rebroadcast pending triggered".to_string());
            } else if matches!(
                workflow_signal,
                Some(WorkflowSignal::Continue | WorkflowSignal::Stop)
            ) {
                let can_resume = effective_wait_reason == Some(WaitReason::Confirmation)
                    || (recovery_context.is_none()
                        && matches!(
                            compat_snapshot_workflow_state(&workflow_snapshot.workflow.status),
                            Some(WorkflowState::Paused)
                        ));
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
                            log::info!(
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
                let can_resume = effective_wait_reason == Some(WaitReason::Approval)
                    || (recovery_context.is_none()
                        && matches!(
                            compat_snapshot_workflow_state(&workflow_snapshot.workflow.status),
                            Some(
                                WorkflowState::AwaitingApproval
                                    | WorkflowState::AwaitingAutoApproval
                            )
                        ));
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
                            log::info!(
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
                            log::info!(
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
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    gateway: State<'_, Arc<TauriGateway>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
) -> Result<(), String> {
    let previous_status = {
        let store = state.inner().read().map_err(|e| e.to_string())?;
        store
            .get_workflow_snapshot(&session_id)
            .map(|snapshot| snapshot.workflow.status)
            .unwrap_or_else(|_| WorkflowState::Cancelled.to_string())
    };

    interrupt_openai_session(chat_state.inner(), &session_id).await;
    cleanup_owned_background_resources(&session_id, chat_state.inner()).await;

    // Keep stop as a runtime signal only.
    // Terminal persistence and session cleanup should happen on the executor's
    // normal shutdown path after it processes the stop signal.
    let gateway_arc = gateway.inner().clone();
    let workflow_manager = workflow_manager.inner().clone();
    match gateway_arc
        .inject_input(&session_id, "{\"type\": \"stop\"}".to_string())
        .await
    {
        Ok(_) => {
            let _ =
                workflow_manager.update_session_status(&session_id, ManagedSessionStatus::Stopping);
            let _ = gateway_arc
                .send(
                    &session_id,
                    crate::workflow::react::types::GatewayPayload::State {
                        state: WorkflowState::Stopping,
                        wait_reason: None,
                    },
                )
                .await;
            {
                let store = state.inner().read().map_err(|e| e.to_string())?;
                store
                    .update_workflow_status(&session_id, &WorkflowState::Stopping.to_string())
                    .map_err(|e| e.to_string())?;
            }
            // Do not return until the live session is no longer considered active.
            // Frontend-only stop flags are lost on reload, so stop must reconcile
            // manager/gateway state before the command completes.
            for _ in 0..50 {
                if !has_reconciled_live_session(&workflow_manager, &session_id, "stop").await {
                    return Ok(());
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            }

            if !workflow_manager.has_session(&session_id) {
                return Ok(());
            }

            log::warn!(
                "[Workflow][session={}][phase=stop] Stop signal acknowledged but live session still exists after wait window; leaving session in stopping state until executor exits cleanly",
                session_id
            );
            Ok(())
        }
        Err(e) if workflow_manager.has_session(&session_id) => {
            if let Some(executor) = workflow_manager.get_executor(&session_id) {
                let mut guard = executor.lock().await;
                guard.set_state(WorkflowState::Cancelled);
            }
            if let Ok(store) = state.inner().read() {
                let _ = store
                    .update_workflow_status(&session_id, &WorkflowState::Cancelled.to_string());
            }
            let _ = workflow_manager
                .update_session_status(&session_id, ManagedSessionStatus::Cancelled);
            let _ = gateway_arc
                .send(
                    &session_id,
                    crate::workflow::react::types::GatewayPayload::State {
                        state: WorkflowState::Cancelled,
                        wait_reason: None,
                    },
                )
                .await;
            workflow_manager.remove_session(&session_id);
            WorkflowManager::unregister_session_signal_tx_with_source(
                &session_id,
                "workflow_stop.gateway_injection_failed",
            );
            gateway_arc
                .unregister_session_with_source(
                    &session_id,
                    "workflow_stop.gateway_injection_failed",
                )
                .await;
            Err(e.to_string())
        }
        Err(e) => {
            if let Ok(store) = state.inner().read() {
                let _ = store.update_workflow_status(&session_id, &previous_status);
            }
            Err(e.to_string())
        }
    }
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

fn is_obviously_ignored_name(name: &str) -> bool {
    let name_lower = name.to_ascii_lowercase();
    matches!(
        name,
        ".git"
            | ".svn"
            | ".hg"
            | "node_modules"
            | "__pycache__"
            | ".turbo"
            | ".next"
            | ".nuxt"
            | ".svelte-kit"
            | "dist"
            | "coverage"
    ) || name_lower.ends_with(".pyc")
        || matches!(
            name_lower.as_str(),
            ".ds_store" | "thumbs.db" | "desktop.ini"
        )
}

fn path_contains_obviously_ignored_component(path: &Path) -> bool {
    path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .is_some_and(is_obviously_ignored_name)
    })
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

        let mut builder = ignore::WalkBuilder::new(&base);
        builder.standard_filters(true).hidden(false);
        builder.filter_entry(|entry| {
            entry
                .path()
                .file_name()
                .and_then(|name| name.to_str())
                .is_none_or(|name| !is_obviously_ignored_name(name))
        });
        let walker = builder.build();

        for result in walker {
            let entry = match result {
                Ok(e) => e,
                Err(_) => continue,
            };

            let path = entry.path().to_path_buf();
            let name = entry.file_name().to_string_lossy().to_string();

            if path_contains_obviously_ignored_component(&path) {
                continue;
            }

            let rel_path = path
                .strip_prefix(&base)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            let name_lower = name.to_lowercase();

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
                    // Common programming languages
                    "c" | "cpp" | "css" | "go" | "h" | "hpp" | "htm" | "html" | "java" | "js"
                    | "jsx" | "json" | "kotlin" | "less" | "lua" | "perl" | "php" | "py" | "rs"
                    | "ruby" | "scala" | "sass" | "scss" | "sh" | "sql" | "swift" | "toml"
                    | "ts" | "tsx" | "vue" | "xml" | "yaml" | "yml" => {
                        score += 5;
                    }
                    // Less common programming languages
                    "bash" | "cc" | "cxx" | "groovy" | "hxx" | "ini" | "mjs" | "mm" | "plsql"
                    | "ps1" | "r" | "stylus" | "zsh" => {
                        score += 3;
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
    log::info!("get_system_skills returned {} skills", skills.len());
    Ok(skills)
}

#[tauri::command]
pub async fn update_workflow_allowed_paths(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    gateway: State<'_, Arc<TauriGateway>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
    allowed_paths: Value,
) -> Result<(), String> {
    let runtime_paths: Vec<String> =
        serde_json::from_value(allowed_paths.clone()).map_err(|e| e.to_string())?;
    let previous_config_json = {
        let store = state.read().map_err(|e| e.to_string())?;
        raw_workflow_agent_config_json(&store, &session_id)?
    };

    {
        let store = state.read().map_err(|e| e.to_string())?;
        let mut config = raw_workflow_agent_config(&store, &session_id)?;

        config.allowed_paths = Some(runtime_paths.clone());

        let new_config_str = config.to_json();
        store
            .update_workflow_agent_config(&session_id, &new_config_str)
            .map_err(|e| e.to_string())?;
    }

    inject_runtime_config_signal(
        gateway.inner(),
        workflow_manager.inner(),
        state.inner(),
        &session_id,
        &previous_config_json,
        serde_json::json!({
            "type": "update_allowed_paths",
            "paths": runtime_paths
        }),
    )
    .await?;

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
    gateway: State<'_, Arc<TauriGateway>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
    final_audit: bool,
) -> Result<(), String> {
    let previous_config_json = {
        let store = state.read().map_err(|e| e.to_string())?;
        raw_workflow_agent_config_json(&store, &session_id)?
    };
    {
        let store = state.read().map_err(|e| e.to_string())?;
        let mut config = raw_workflow_agent_config(&store, &session_id)?;

        config.final_audit = Some(final_audit);
        config.final_review_mode = Some(if final_audit {
            "sub_agent_review".to_string()
        } else {
            "off".to_string()
        });

        let new_config_str = config.to_json();
        store
            .update_workflow_agent_config(&session_id, &new_config_str)
            .map_err(|e| e.to_string())?;
    }

    inject_runtime_config_signal(
        gateway.inner(),
        workflow_manager.inner(),
        state.inner(),
        &session_id,
        &previous_config_json,
        serde_json::json!({
            "type": "update_final_audit",
            "final_audit": final_audit
        }),
    )
    .await?;

    Ok(())
}

#[tauri::command]
pub async fn update_workflow_auto_compress(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    gateway: State<'_, Arc<TauriGateway>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
    auto_compress: bool,
) -> Result<(), String> {
    let previous_config_json = {
        let store = state.read().map_err(|e| e.to_string())?;
        raw_workflow_agent_config_json(&store, &session_id)?
    };
    {
        let store = state.read().map_err(|e| e.to_string())?;
        let mut config = raw_workflow_agent_config(&store, &session_id)?;

        config.auto_compress = Some(auto_compress);

        let new_config_str = config.to_json();
        store
            .update_workflow_agent_config(&session_id, &new_config_str)
            .map_err(|e| e.to_string())?;
    }

    inject_runtime_config_signal(
        gateway.inner(),
        workflow_manager.inner(),
        state.inner(),
        &session_id,
        &previous_config_json,
        serde_json::json!({
            "type": "update_auto_compress",
            "auto_compress": auto_compress
        }),
    )
    .await?;

    Ok(())
}

#[tauri::command]
pub async fn update_workflow_model_config(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    gateway: State<'_, Arc<TauriGateway>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
    configs: Value,
) -> Result<(), String> {
    let previous_config_json = {
        let store = state.read().map_err(|e| e.to_string())?;
        raw_workflow_agent_config_json(&store, &session_id)?
    };
    {
        let store = state.read().map_err(|e| e.to_string())?;
        let mut agent_config =
            serde_json::from_str::<Value>(&raw_workflow_agent_config_json(&store, &session_id)?)
                .unwrap_or(json!({}));

        agent_config["models"] = configs.clone();

        let config_str = serde_json::to_string(&agent_config).map_err(|e| e.to_string())?;
        store
            .update_workflow_agent_config(&session_id, &config_str)
            .map_err(|e| e.to_string())?;
    }

    inject_runtime_config_signal(
        gateway.inner(),
        workflow_manager.inner(),
        state.inner(),
        &session_id,
        &previous_config_json,
        serde_json::json!({
            "type": "update_model_config",
            "configs": configs
        }),
    )
    .await?;

    Ok(())
}

#[tauri::command]
pub async fn update_workflow_skills_config(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    gateway: State<'_, Arc<TauriGateway>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
    skill_enabled: bool,
    selected_skills: Vec<String>,
) -> Result<(), String> {
    let previous_config_json = {
        let store = state.read().map_err(|e| e.to_string())?;
        raw_workflow_agent_config_json(&store, &session_id)?
    };
    {
        let store = state.read().map_err(|e| e.to_string())?;
        let mut config = raw_workflow_agent_config(&store, &session_id)?;

        config.skill_enabled = Some(skill_enabled);
        config.selected_skills = Some(selected_skills.clone());

        let new_config_str = config.to_json();
        store
            .update_workflow_agent_config(&session_id, &new_config_str)
            .map_err(|e| e.to_string())?;
    }

    inject_runtime_config_signal(
        gateway.inner(),
        workflow_manager.inner(),
        state.inner(),
        &session_id,
        &previous_config_json,
        serde_json::json!({
            "type": "update_skills_config",
            "skill_enabled": skill_enabled,
            "selected_skills": selected_skills
        }),
    )
    .await?;

    Ok(())
}

#[tauri::command]
pub async fn update_workflow_approval_level(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    gateway: State<'_, Arc<TauriGateway>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
    approval_level: String,
) -> Result<(), String> {
    let previous_config_json = {
        let store = state.read().map_err(|e| e.to_string())?;
        raw_workflow_agent_config_json(&store, &session_id)?
    };
    {
        let store = state.read().map_err(|e| e.to_string())?;
        let mut config = raw_workflow_agent_config(&store, &session_id)?;

        config.approval_level = Some(approval_level.clone());

        let new_config_str = config.to_json();
        store
            .update_workflow_agent_config(&session_id, &new_config_str)
            .map_err(|e| e.to_string())?;
    }

    inject_runtime_config_signal(
        gateway.inner(),
        workflow_manager.inner(),
        state.inner(),
        &session_id,
        &previous_config_json,
        serde_json::json!({
            "type": "update_approval_level",
            "approval_level": approval_level
        }),
    )
    .await?;

    Ok(())
}

#[tauri::command]
pub async fn update_workflow_phase(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    gateway: State<'_, Arc<TauriGateway>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
    phase: String,
) -> Result<(), String> {
    let previous_config_json = {
        let store = state.read().map_err(|e| e.to_string())?;
        raw_workflow_agent_config_json(&store, &session_id)?
    };
    {
        let store = state.read().map_err(|e| e.to_string())?;
        let mut config = raw_workflow_agent_config(&store, &session_id)?;

        config.phase = Some(phase.clone());

        let new_config_str = config.to_json();
        store
            .update_workflow_agent_config(&session_id, &new_config_str)
            .map_err(|e| e.to_string())?;
    }

    inject_runtime_config_signal(
        gateway.inner(),
        workflow_manager.inner(),
        state.inner(),
        &session_id,
        &previous_config_json,
        serde_json::json!({
            "type": "update_phase",
            "phase": phase
        }),
    )
    .await?;

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
    agent_config.final_audit = current_config.final_audit;
    agent_config.final_review_mode = current_config.final_review_mode.clone().or_else(|| {
        Some(if current_config.final_audit.unwrap_or(false) {
            "sub_agent_review".to_string()
        } else {
            "off".to_string()
        })
    });
    agent_config.phase = current_config.phase;
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
    let auto_approve = store
        .get_workflow(&session_id)
        .map_err(|e| e.to_string())?
        .and_then(|workflow| workflow.agent_config)
        .and_then(|s| AgentConfig::from_json(&s))
        .and_then(|config| config.auto_approve)
        .unwrap_or_default();

    Ok(auto_approve)
}

#[tauri::command]
pub async fn remove_auto_approved_tool(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    gateway: State<'_, Arc<TauriGateway>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
    tool_name: String,
) -> Result<(), String> {
    let previous_config_json = {
        let store = state.read().map_err(|e| e.to_string())?;
        raw_workflow_agent_config_json(&store, &session_id)?
    };

    // Update database first
    {
        let store = state.read().map_err(|e| e.to_string())?;

        let mut config = raw_workflow_agent_config(&store, &session_id)?;

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

    inject_runtime_config_signal(
        gateway.inner(),
        workflow_manager.inner(),
        state.inner(),
        &session_id,
        &previous_config_json,
        serde_json::json!({
            "type": "remove_auto_approved_tool",
            "tool_name": tool_name
        }),
    )
    .await?;

    Ok(())
}

#[tauri::command]
pub async fn remove_shell_policy_item(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    gateway: State<'_, Arc<TauriGateway>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
    pattern: String,
) -> Result<(), String> {
    let previous_config_json = {
        let store = state.read().map_err(|e| e.to_string())?;
        raw_workflow_agent_config_json(&store, &session_id)?
    };

    // Update database first
    {
        let store = state.read().map_err(|e| e.to_string())?;

        let mut config = raw_workflow_agent_config(&store, &session_id)?;

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

    inject_runtime_config_signal(
        gateway.inner(),
        workflow_manager.inner(),
        state.inner(),
        &session_id,
        &previous_config_json,
        serde_json::json!({
            "type": "remove_shell_policy_item",
            "pattern": pattern
        }),
    )
    .await?;

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

#[tauri::command]
pub async fn get_workflow_efficiency_report(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    session_id: String,
) -> Result<WorkflowEfficiencyReport, String> {
    let store = state.read().map_err(|e| e.to_string())?;
    store
        .get_workflow_efficiency_report(&session_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_workflow_memory_diagnostics(
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    session_id: String,
) -> Result<WorkflowMemoryDiagnostics, String> {
    let store = state.read().map_err(|e| e.to_string())?;
    let snapshot = store
        .get_workflow_snapshot(&session_id)
        .map_err(|e| e.to_string())?;

    let project_root = snapshot
        .workflow
        .agent_config
        .as_deref()
        .and_then(AgentConfig::from_json)
        .and_then(|config| config.allowed_paths)
        .and_then(|paths| paths.first().cloned())
        .map(PathBuf::from);

    let memory_manager = MemoryManager::new(project_root);
    let project_key = memory_manager.project_key().map(str::to_string);
    let global_candidates = store
        .list_memory_candidates(Some("global"), None, None, 100)
        .map_err(|e| e.to_string())?;
    let project_candidates = store
        .list_memory_candidates(Some("project"), project_key.as_deref(), None, 100)
        .map_err(|e| e.to_string())?;

    Ok(WorkflowMemoryDiagnostics {
        global_memory: memory_manager
            .read(crate::workflow::react::memory::MemoryScope::Global)
            .map_err(|e| e.to_string())?,
        project_memory: memory_manager
            .read(crate::workflow::react::memory::MemoryScope::Project)
            .map_err(|e| e.to_string())?,
        project_key,
        global_candidates,
        project_candidates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    fn create_test_store() -> MainStore {
        let dir = tempdir().expect("failed to create temp dir");
        let db_path = dir.path().join("workflow_commands_test.db");
        MainStore::new(db_path).expect("failed to create MainStore")
    }

    fn seed_agent(store: &MainStore, agent_id: &str) {
        let conn = store
            .conn
            .lock()
            .expect("failed to lock db connection for agent seed");
        conn.execute(
            "INSERT INTO agents (id, name, system_prompt, agent_type, max_contexts)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                agent_id,
                "Agent Test",
                "You are a test agent.",
                "autonomous",
                20
            ],
        )
        .expect("failed to seed agent");
    }

    #[test]
    fn test_compat_wait_reason_from_snapshot_status() {
        assert_eq!(
            compat_wait_reason_from_snapshot_status("awaiting_approval"),
            Some(WaitReason::Approval)
        );
        assert_eq!(
            compat_wait_reason_from_snapshot_status("awaiting_user"),
            Some(WaitReason::UserInput)
        );
        assert_eq!(
            compat_wait_reason_from_snapshot_status("paused"),
            Some(WaitReason::Confirmation)
        );
        assert_eq!(
            compat_wait_reason_from_snapshot_status("awaiting_sub_agent"),
            Some(WaitReason::SubAgent)
        );
        assert_eq!(compat_wait_reason_from_snapshot_status("thinking"), None);
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

    #[test]
    fn test_can_resume_user_message_from_recovery_prefers_terminal_snapshot() {
        let mut running = ExecutionContext::new("session-5".to_string());
        running.state = RuntimeState::Running;

        assert!(can_resume_user_message_from_recovery(
            Some(&running),
            "cancelled"
        ));
        assert!(can_resume_user_message_from_recovery(
            Some(&running),
            "completed"
        ));
        assert!(!can_resume_user_message_from_recovery(
            Some(&running),
            "executing"
        ));
    }

    #[test]
    fn test_previous_completed_task_context_prefers_structured_tool_summary() {
        let messages = vec![
            WorkflowMessage {
                id: Some(1),
                session_id: "session-1".to_string(),
                role: "user".to_string(),
                message: "Fix the captcha modal".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: None,
                attached_context: None,
                step_type: Some("think".to_string()),
                step_index: 1,
                is_error: false,
                error_type: None,
                created_at: None,
            },
            WorkflowMessage {
                id: Some(2),
                session_id: "session-1".to_string(),
                role: "assistant".to_string(),
                message: "Stale assistant recap that should not win".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: None,
                attached_context: None,
                step_type: Some("think".to_string()),
                step_index: 2,
                is_error: false,
                error_type: None,
                created_at: None,
            },
            WorkflowMessage {
                id: Some(3),
                session_id: "session-1".to_string(),
                role: "tool".to_string(),
                message: "".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(json!({
                    "tool_name": crate::tools::TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY,
                    "execution_status": "completed",
                    "approval_status": "approved",
                    "summary": "Structured completion summary",
                    "tool_call": {
                        "function": {
                            "name": crate::tools::TOOL_COMPLETE_WORKFLOW_WITH_SUMMARY,
                            "arguments": "{\"summary\":\"Structured completion summary\"}"
                        }
                    }
                })),
                attached_context: None,
                step_type: Some("observe".to_string()),
                step_index: 3,
                is_error: false,
                error_type: None,
                created_at: None,
            },
        ];

        let result = previous_completed_task_context(&messages).expect("expected previous task");
        assert_eq!(result.0, "Fix the captcha modal");
        assert_eq!(result.1, "Structured completion summary");
    }

    #[test]
    fn test_should_inject_terminal_user_message_into_live_session() {
        assert!(should_inject_terminal_user_message_into_live_session(Some(
            ManagedSessionStatus::Active
        )));
        assert!(should_inject_terminal_user_message_into_live_session(Some(
            ManagedSessionStatus::Waiting
        )));
        assert!(!should_inject_terminal_user_message_into_live_session(
            Some(ManagedSessionStatus::Completed)
        ));
        assert!(!should_inject_terminal_user_message_into_live_session(
            Some(ManagedSessionStatus::Stopping)
        ));
        assert!(!should_inject_terminal_user_message_into_live_session(
            Some(ManagedSessionStatus::Failed)
        ));
        assert!(!should_inject_terminal_user_message_into_live_session(None));
    }

    #[test]
    fn test_managed_status_blocks_tail_rewind() {
        assert!(managed_status_blocks_tail_rewind(Some(
            ManagedSessionStatus::Active
        )));
        assert!(managed_status_blocks_tail_rewind(Some(
            ManagedSessionStatus::Waiting
        )));
        assert!(managed_status_blocks_tail_rewind(Some(
            ManagedSessionStatus::Stopping
        )));
        assert!(!managed_status_blocks_tail_rewind(Some(
            ManagedSessionStatus::Completed
        )));
        assert!(!managed_status_blocks_tail_rewind(Some(
            ManagedSessionStatus::Failed
        )));
        assert!(!managed_status_blocks_tail_rewind(Some(
            ManagedSessionStatus::Cancelled
        )));
        assert!(!managed_status_blocks_tail_rewind(None));
    }

    #[test]
    fn test_normalize_snapshot_does_not_interrupt_waiting_approval_workflow() {
        let store = create_test_store();
        let session_id = "snapshot-awaiting-approval";
        seed_agent(&store, "agent-test");
        store
            .create_workflow(session_id, "Initial query", "agent-test", None, None)
            .expect("failed to create workflow");
        store
            .update_workflow_status(session_id, "awaiting_approval")
            .expect("failed to update workflow status");
        let message = store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "tool".to_string(),
                message: "Plan awaiting approval".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(json!({
                    "tool_call_id": "submit_plan_1",
                    "tool_name": crate::tools::TOOL_SUBMIT_PLAN,
                    "approval_status": "approved",
                    "execution_status": "approval_submitted",
                    "summary": "Pending execution after approval"
                })),
                attached_context: None,
                step_type: Some("observe".to_string()),
                step_index: 1,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add workflow message");

        let mut snapshot = store
            .get_workflow_snapshot(session_id)
            .expect("failed to load workflow snapshot");
        normalize_snapshot_after_live_reconciliation(&store, session_id, &mut snapshot, false)
            .expect("normalization should succeed");

        assert_eq!(snapshot.messages.len(), 1);
        let normalized = &snapshot.messages[0];
        assert_eq!(normalized.id, message.id);
        assert!(!normalized.is_error);
        assert_eq!(normalized.error_type, None);
        assert_eq!(
            normalized
                .metadata
                .as_ref()
                .and_then(|meta| meta.get("execution_status"))
                .and_then(|value| value.as_str()),
            Some("approval_submitted")
        );
    }

    #[test]
    fn test_normalize_snapshot_keeps_running_workflow_messages_unchanged_without_live_session() {
        let store = create_test_store();
        let session_id = "snapshot-executing-orphan";
        seed_agent(&store, "agent-test");
        store
            .create_workflow(session_id, "Initial query", "agent-test", None, None)
            .expect("failed to create workflow");
        store
            .update_workflow_status(session_id, "executing")
            .expect("failed to update workflow status");
        store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "tool".to_string(),
                message: "Still running".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(json!({
                    "tool_call_id": "tool_1",
                    "tool_name": "edit_file",
                    "approval_status": "approved",
                    "execution_status": "running",
                    "summary": "Running"
                })),
                attached_context: None,
                step_type: Some("observe".to_string()),
                step_index: 1,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add workflow message");

        let mut snapshot = store
            .get_workflow_snapshot(session_id)
            .expect("failed to load workflow snapshot");
        normalize_snapshot_after_live_reconciliation(&store, session_id, &mut snapshot, false)
            .expect("normalization should succeed");

        let normalized = &snapshot.messages[0];
        assert!(!normalized.is_error);
        assert_eq!(normalized.error_type, None);
        assert_eq!(
            normalized
                .metadata
                .as_ref()
                .and_then(|meta| meta.get("execution_status"))
                .and_then(|value| value.as_str()),
            Some("running")
        );
    }

    #[test]
    fn test_store_identifies_rewindable_ask_user_tail_after_restart() {
        let store = create_test_store();
        let session_id = "snapshot-awaiting-user-rewind";
        seed_agent(&store, "agent-test");
        store
            .create_workflow(session_id, "Initial query", "agent-test", None, None)
            .expect("failed to create workflow");
        store
            .update_workflow_status(session_id, "awaiting_user")
            .expect("failed to update workflow status");
        store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "tool".to_string(),
                message: "[{\"title\":\"Choose\",\"options\":[\"A\",\"B\"]}]".to_string(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(json!({
                    "tool_call_id": "ask_user_1",
                    "tool_name": crate::tools::TOOL_ASK_USER,
                    "display_type": "choice",
                    "execution_status": "completed"
                })),
                attached_context: None,
                step_type: Some("observe".to_string()),
                step_index: 2,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add ask_user message");
        let wait_event_id = store
            .append_workflow_event(&WorkflowEvent::wait_entered(
                session_id.to_string(),
                "user_input".to_string(),
                Vec::new(),
            ))
            .expect("failed to append ask_user wait");

        let mut context = ExecutionContext::new(session_id.to_string());
        context.state = RuntimeState::Waiting;
        context.wait_reason = Some(WaitReason::UserInput);
        context.last_event_id = Some(wait_event_id);
        store
            .upsert_execution_context(&context)
            .expect("failed to persist awaiting-user snapshot");

        assert_eq!(
            store
                .get_tail_rewind_kind(session_id)
                .expect("failed to inspect tail rewind kind"),
            Some("ask_user_wait")
        );
    }

    #[test]
    fn test_reset_workflow_planning_note_is_lazy_for_missing_workspace() {
        let root = tempdir().unwrap();
        let allowed_roots = vec![root.path().to_path_buf()];

        reset_workflow_planning_note(&allowed_roots).unwrap();

        assert!(!root.path().join(".cs").exists());
        assert!(!root.path().join(".cs").join("note.md").exists());
    }

    #[test]
    fn test_reset_workflow_planning_note_clears_existing_note() {
        let root = tempdir().unwrap();
        let planning_root = root.path().join(".cs");
        std::fs::create_dir_all(&planning_root).unwrap();
        let note_path = planning_root.join("note.md");
        std::fs::write(&note_path, "stale plan").unwrap();

        reset_workflow_planning_note(&[root.path().to_path_buf()]).unwrap();

        assert_eq!(std::fs::read_to_string(note_path).unwrap(), "");
    }
}
