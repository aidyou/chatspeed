use crate::ai::interaction::chat_completion::ChatState;
use crate::db::{
    Agent, AgentConfig, MainStore, Workflow, WorkflowEfficiencyReport, WorkflowMessage,
    WorkflowSnapshot,
};
use crate::libs::tsid::TsidGenerator;
use crate::workflow::react::child_tasks::get_sub_agent_registry;
use crate::workflow::react::context::ContextManager;
use crate::workflow::react::dispatcher::{Dispatcher, DispatcherMetricsSnapshot};
use crate::workflow::react::engine::WorkflowExecutor;
use crate::workflow::react::events::WorkflowEvent;
use crate::workflow::react::gateway::{Gateway, TauriGateway};
use crate::workflow::react::intelligence::IntelligenceManager;
use crate::workflow::react::manager::{ManagedSessionStatus, WorkflowManager};
use crate::workflow::react::orchestrator::{
    clear_completed_background_tasks_for_owner, list_background_task_ids_for_owner,
    stop_background_task, BackgroundTask, SubAgentFactory, BACKGROUND_TASKS,
};

use crate::workflow::react::prompts::{
    is_agent_personality_preset, AGENT_PERSONALITY_PRESET_DEFAULT_ID,
    AGENT_PERSONALITY_PRESET_PREFIX,
};
use crate::workflow::react::replay::{
    replay_events_to_execution_context, restore_execution_context, RecoveryResult,
};
use crate::workflow::react::runtime_observation::{
    runtime_observation_metadata, runtime_observation_metadata_with_visibility,
    RuntimeObservationLlmVisibility, RuntimeObservationType, RuntimeObservationUiVisibility,
};
use crate::workflow::react::security::workspace_walk_builder;
#[cfg(test)]
use crate::workflow::react::security::CHATSPEED_IGNORE_FILE;
use crate::workflow::react::signals::{stash_runtime_signal, SignalType};
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

const UI_WORKFLOW_MESSAGE_PAGE_SIZE: usize = 300;

fn serialize_workflow_message_page_cursor(before_message_id: Option<i64>) -> Option<String> {
    before_message_id.map(|message_id| message_id.to_string())
}

fn parse_workflow_message_page_cursor(before_message_id: &str) -> Result<i64, String> {
    before_message_id
        .parse::<i64>()
        .map_err(|_| "Invalid workflow message page cursor".to_string())
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

fn try_acquire_workflow_title_generation(session_id: &str, chat_state: &ChatState) -> bool {
    match chat_state
        .workflow_title_generation_in_flight
        .entry(session_id.to_string())
    {
        dashmap::mapref::entry::Entry::Occupied(_) => false,
        dashmap::mapref::entry::Entry::Vacant(entry) => {
            entry.insert(());
            true
        }
    }
}

fn release_workflow_title_generation(session_id: &str, chat_state: &ChatState) {
    chat_state
        .workflow_title_generation_in_flight
        .remove(session_id);
}

fn spawn_workflow_title_generation_if_missing(
    session_id: String,
    user_query: String,
    state: Arc<MainStore>,
    chat_state: Arc<ChatState>,
    gateway: Arc<TauriGateway>,
) -> Result<(), String> {
    if user_query.trim().is_empty() {
        return Ok(());
    }

    if !try_acquire_workflow_title_generation(&session_id, &chat_state) {
        return Ok(());
    }

    let should_generate_title = {
        let store = &*state;
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
        release_workflow_title_generation(&session_id, &chat_state);
        return Ok(());
    }

    let (provider_id, model_name) = {
        let store = &*state;
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
        chat_state.clone(),
        provider_id,
        model_name.clone(),
        provider_id,
        model_name.clone(),
        format!("{session_id}:task:1"),
        session_id.clone(),
        format!("{session_id}:task:1"),
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

        release_workflow_title_generation(&session_id, &chat_state);
    });

    Ok(())
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
        .map(|tool_name| tool_name == crate::tools::TOOL_COMPLETE_WORKFLOW)
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

const NEW_SEGMENT_AFTER_COMPLETION_REMINDER: &str = "<SYSTEM_REMINDER>This is a new context segment following earlier completed work. A segment boundary is not necessarily an objective boundary. Determine the completion-report scope from the user's current request. If the request is independent, complete and summarize only the work and verification for this segment's task. If the user explicitly continues, corrects, refines, or extends the immediately preceding objective, treat the related segments as one continuous objective and summarize the combined outcome, including relevant earlier work and the current changes. Do not include unrelated completed tasks. In either case, this segment requires a new current completion report; never reuse an earlier task's completion report or pending draft as this segment's report.</SYSTEM_REMINDER>";

fn prompt_with_new_segment_completion_scope(clean_prompt: &str) -> String {
    format!(
        "{}\n\n{}",
        clean_prompt, NEW_SEGMENT_AFTER_COMPLETION_REMINDER
    )
}

fn rollback_workflow_agent_config(
    state: &Arc<MainStore>,
    session_id: &str,
    previous_config_json: &str,
) {
    let _ = state.update_workflow_agent_config(session_id, previous_config_json);
}

fn can_defer_runtime_config_signal_for_completed_session(signal_type: &str) -> bool {
    matches!(
        signal_type,
        "update_allowed_paths"
            | "update_final_audit"
            | "update_auto_compress"
            | "update_model_config"
            | "update_skills_config"
            | "update_sandbox_config"
            | "update_personality"
            | "update_approval_level"
            | "update_phase"
            | "update_available_tools"
            | "update_auto_approved_tools"
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
    state: &Arc<MainStore>,
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

fn wait_reason_blocks_manual_clear(wait_reason: Option<&WaitReason>) -> bool {
    matches!(
        wait_reason,
        Some(
            WaitReason::Approval
                | WaitReason::UserInput
                | WaitReason::Confirmation
                | WaitReason::SubAgent
        )
    )
}

fn runtime_state_allows_manual_clear(state: &RuntimeState) -> bool {
    matches!(
        state,
        RuntimeState::Pending
            | RuntimeState::Completed
            | RuntimeState::Failed
            | RuntimeState::Cancelled
    )
}

fn workflow_state_allows_manual_clear(state: &WorkflowState) -> bool {
    matches!(
        state,
        WorkflowState::Pending
            | WorkflowState::Completed
            | WorkflowState::Error
            | WorkflowState::Cancelled
    )
}

fn persist_cancelled_workflow_state(store: &MainStore, session_id: &str) -> Result<(), String> {
    store
        .update_workflow_status(session_id, &WorkflowState::Cancelled.to_string())
        .map_err(|error| error.to_string())?;

    if let Some(mut context) = store
        .get_execution_context(session_id)
        .map_err(|error| error.to_string())?
    {
        context.state = RuntimeState::Cancelled;
        context.wait_reason = None;
        store
            .upsert_execution_context(&context)
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}

fn persist_failed_workflow_state(store: &MainStore, session_id: &str) -> Result<(), String> {
    store
        .update_workflow_status(session_id, &WorkflowState::Error.to_string())
        .map_err(|error| error.to_string())?;

    if let Some(mut context) = store
        .get_execution_context(session_id)
        .map_err(|error| error.to_string())?
    {
        context.state = RuntimeState::Failed;
        context.wait_reason = None;
        store
            .upsert_execution_context(&context)
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}

fn persist_pending_workflow_state(store: &MainStore, session_id: &str) -> Result<(), String> {
    if let Some(mut context) = store
        .get_execution_context(session_id)
        .map_err(|error| error.to_string())?
    {
        context.reset_for_new_context();
        store
            .upsert_execution_context(&context)
            .map_err(|error| error.to_string())?;
    }

    store
        .update_workflow_status(session_id, &WorkflowState::Pending.to_string())
        .map_err(|error| error.to_string())?;

    Ok(())
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
        .unwrap_or(false)
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
        let next_message = message.clone();

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
            // Tool declarations remain in the projection even after a dedicated
            // observation supersedes the pending placeholder. They preserve the
            // durable tool_call_id chain and the original in-batch tool order;
            // the frontend decides whether a declaration needs its own card.
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

#[derive(Debug, Clone, PartialEq)]
struct LargeFileReadChunk {
    offset: usize,
    limit: usize,
    end_line: usize,
}

#[derive(Debug, Clone, PartialEq)]
struct LargeFileReadPlan {
    total_lines: usize,
    average_line_chars: usize,
    max_line_chars: usize,
    recommended_limit: usize,
    estimated_calls: usize,
    chunk_plan: Vec<LargeFileReadChunk>,
}

fn build_large_file_read_plan(path: &Path) -> Option<LargeFileReadPlan> {
    let file = fs::File::open(path).ok()?;
    let reader = BufReader::new(file);

    let mut total_lines = 0usize;
    let mut total_chars = 0usize;
    let mut max_line_chars = 0usize;
    let mut line_char_counts = Vec::new();

    for line in reader.lines() {
        let content = line.ok()?;
        let line_chars = content.chars().count();
        total_lines += 1;
        total_chars += line_chars;
        max_line_chars = max_line_chars.max(line_chars);
        line_char_counts.push(line_chars);
    }

    if total_lines == 0 {
        return None;
    }

    let average_line_chars = total_chars.div_ceil(total_lines);
    let mut chunk_plan = Vec::new();
    let mut chunk_start = 0usize;
    let mut chunk_chars = 0usize;
    let mut chunk_lines = 0usize;

    for (index, line_chars) in line_char_counts.iter().enumerate() {
        let line_number = index + 1;
        let rendered_len = format!("{:>6}\t{}", line_number, "x".repeat(*line_chars))
            .chars()
            .count()
            + 1;

        if chunk_lines > 0 && chunk_chars + rendered_len > crate::tools::READ_FILE_MAX_OUTPUT_CHARS
        {
            chunk_plan.push(LargeFileReadChunk {
                offset: chunk_start,
                limit: chunk_lines,
                end_line: index,
            });
            chunk_start = index;
            chunk_chars = 0;
            chunk_lines = 0;
        }

        chunk_chars += rendered_len;
        chunk_lines += 1;
    }

    if chunk_lines > 0 {
        chunk_plan.push(LargeFileReadChunk {
            offset: chunk_start,
            limit: chunk_lines,
            end_line: total_lines,
        });
    }

    let recommended_limit = chunk_plan
        .first()
        .map(|chunk| chunk.limit)
        .unwrap_or(crate::tools::DEFAULT_READ_FILE_LIMIT);
    let estimated_calls = chunk_plan.len();

    Some(LargeFileReadPlan {
        total_lines,
        average_line_chars,
        max_line_chars,
        recommended_limit,
        estimated_calls,
        chunk_plan,
    })
}

fn format_large_file_read_plan(
    path_label: &str,
    file_size: u64,
    plan: &LargeFileReadPlan,
) -> String {
    let chunk_preview = plan
        .chunk_plan
        .iter()
        .take(6)
        .map(|chunk| format!("offset={},limit={}", chunk.offset, chunk.limit))
        .collect::<Vec<_>>()
        .join("; ");

    let offset_clause = if plan.estimated_calls <= 6 {
        format!("Recommended chunk sequence: {}.", chunk_preview)
    } else {
        format!(
            "Recommended chunk sequence starts with {} and continues with the same exact per-chunk simulation until EOF.",
            chunk_preview
        )
    };

    format!(
        "The user referenced a large file {}. If you only need symbols, keys, or a specific section, prefer 'grep' first. When the task requires reviewing or executing against the whole file, follow the full-file read plan below instead of guessing chunk sizes. This file is {} across {} lines (average {} chars/line, max {} chars/line), and the exact simulated full-read plan is {} call(s). {}",
        path_label,
        format_size(file_size),
        plan.total_lines,
        plan.average_line_chars,
        plan.max_line_chars,
        plan.estimated_calls,
        offset_clause
    )
}

fn format_large_file_read_plan_block(
    path_label: &str,
    file_size: u64,
    plan: &LargeFileReadPlan,
) -> String {
    let chunk_preview = plan
        .chunk_plan
        .iter()
        .take(8)
        .map(|chunk| format!("{}:{}", chunk.offset, chunk.limit))
        .collect::<Vec<_>>()
        .join(",");

    format!(
        "<full_file_read_plan path=\"{}\" recommended_when=\"need_complete_file\" prefer_grep_when=\"targeted_lookup\" file_size_bytes=\"{}\" total_lines=\"{}\" average_line_chars=\"{}\" max_line_chars=\"{}\" first_chunk_limit=\"{}\" estimated_calls=\"{}\" chunk_preview=\"{}\">\nUse `grep` first for targeted lookup. When the task requires whole-file review, execute the exact chunk plan in order. Each chunk in chunk_preview is encoded as offset:limit, and later chunks are based on exact in-memory simulation of the file against the read_file output cap.\n</full_file_read_plan>",
        path_label,
        file_size,
        plan.total_lines,
        plan.average_line_chars,
        plan.max_line_chars,
        plan.recommended_limit,
        plan.estimated_calls,
        chunk_preview
    )
}

const MAX_AT_MENTION_TARGETS: usize = 10;

fn parse_at_mention_capture(capture: &regex::Captures<'_>) -> Option<String> {
    if let Some(quoted) = capture.get(1) {
        let mut value = String::new();
        let mut chars = quoted.as_str().chars();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                if let Some(escaped) = chars.next() {
                    value.push(escaped);
                } else {
                    value.push(ch);
                }
            } else {
                value.push(ch);
            }
        }
        return Some(value);
    }

    capture.get(2).map(|unquoted| unquoted.as_str().to_string())
}

fn parse_at_mentions(prompt: &str) -> Vec<String> {
    let mention_re =
        Regex::new(r#"@\"((?:\\.|[^\"\\])*)\"|@([^\s]+)"#).expect("at-mention regex must compile");

    mention_re
        .captures_iter(prompt)
        .filter_map(|capture| parse_at_mention_capture(&capture))
        .collect()
}

fn canonical_allowed_roots(allowed_paths: &[String]) -> Vec<PathBuf> {
    allowed_paths
        .iter()
        .filter_map(|path| fs::canonicalize(path).ok())
        .collect()
}

fn is_path_within_allowed_roots(path: &Path, allowed_roots: &[PathBuf]) -> Option<PathBuf> {
    let canonical_path = fs::canonicalize(path).ok()?;
    allowed_roots
        .iter()
        .any(|root| canonical_path.starts_with(root))
        .then_some(canonical_path)
}

fn format_at_mention_path(path: &Path, allowed_roots: &[PathBuf]) -> String {
    if let Some(primary_root) = allowed_roots.first() {
        if let Ok(relative) = path.strip_prefix(primary_root) {
            return relative.to_string_lossy().into_owned();
        }
    }
    path.to_string_lossy().into_owned()
}

fn format_at_mention_token(path: &str) -> String {
    if path.chars().any(char::is_whitespace) {
        format!("@\"{}\"", path.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        format!("@{path}")
    }
}

fn resolve_at_mention_targets(pattern: &str, allowed_roots: &[PathBuf]) -> Vec<PathBuf> {
    let pattern_path = Path::new(pattern);
    let candidate_patterns = if pattern_path.is_absolute() {
        vec![pattern_path.to_path_buf()]
    } else {
        allowed_roots
            .first()
            .map(|root| vec![root.join(pattern_path)])
            .unwrap_or_default()
    };
    let mut targets = std::collections::HashSet::new();

    for candidate in candidate_patterns {
        if pattern.contains('*') || pattern.contains('?') {
            let candidate_pattern = candidate.to_string_lossy();
            if let Ok(paths) = glob(&candidate_pattern) {
                for path in paths.flatten() {
                    if let Some(path) = is_path_within_allowed_roots(&path, allowed_roots) {
                        targets.insert(path);
                    }
                }
            }
        } else if let Some(path) = is_path_within_allowed_roots(&candidate, allowed_roots) {
            targets.insert(path);
        }
    }

    let mut targets = targets.into_iter().collect::<Vec<_>>();
    targets.sort();
    targets
}

fn normalize_at_mentions(prompt: &str, allowed_roots: &[PathBuf]) -> String {
    let mention_re =
        Regex::new(r#"@\"((?:\\.|[^\"\\])*)\"|@([^\s]+)"#).expect("at-mention regex must compile");
    let mut normalized_targets: std::collections::HashMap<PathBuf, String> =
        std::collections::HashMap::new();
    mention_re
        .replace_all(prompt, |capture: &regex::Captures<'_>| {
            let Some(pattern) = parse_at_mention_capture(capture) else {
                return capture[0].to_string();
            };
            let targets = resolve_at_mention_targets(&pattern, allowed_roots);
            if targets.is_empty() {
                return capture[0].to_string();
            }
            targets
                .into_iter()
                .filter_map(|path| {
                    if let Some(token) = normalized_targets.get(&path) {
                        return Some(token.clone());
                    }
                    if normalized_targets.len() >= MAX_AT_MENTION_TARGETS {
                        return None;
                    }
                    let display_path = format_at_mention_path(&path, allowed_roots);
                    let token = format_at_mention_token(&display_path);
                    normalized_targets.insert(path, token.clone());
                    Some(token)
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .into_owned()
}

fn inject_at_mentions(prompt: &str, allowed_paths: &[String]) -> (String, String) {
    let mut attached_context = String::new();
    let allowed_roots = canonical_allowed_roots(allowed_paths);

    let mut injections = Vec::new();
    let mut handled_targets = std::collections::HashSet::new();

    for pattern in parse_at_mentions(prompt) {
        for path in resolve_at_mention_targets(&pattern, &allowed_roots) {
            if !handled_targets.insert(path.clone()) {
                continue;
            }
            if injections.len() >= MAX_AT_MENTION_TARGETS {
                break;
            }
            let display_path = format_at_mention_path(&path, &allowed_roots);

            if path.is_file() {
                let metadata = match fs::metadata(&path) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                let size = metadata.len();
                let info = get_file_metadata_info(&path);

                if size > 10 * 1024 {
                    let reminder = if let Some(plan) = build_large_file_read_plan(&path) {
                        format!(
                            "{}\n{}",
                            format_large_file_read_plan_block(&display_path, size, &plan),
                            format_large_file_read_plan(&display_path, size, &plan)
                        )
                    } else {
                        format!(
                            "The user referenced a large file {}. If you only need specific symbols or sections, prefer 'grep' first. When you need to read the complete file, use sequential 'read_file' calls with 'offset' and 'limit' parameters.",
                            display_path
                        )
                    };
                    injections.push(format!(
                        "\n<file_content path={:?}>\n[File too large to show full content]\n{}\n</file_content>\n<SYSTEM_REMINDER>{}</SYSTEM_REMINDER>",
                        display_path, info, reminder
                    ));
                } else {
                    match fs::read(&path) {
                        Ok(bytes) => {
                            if let Ok(content) = String::from_utf8(bytes) {
                                injections.push(format!(
                                    "\n<file_content path={:?}>\n{}\n</file_content>\n",
                                    display_path, content
                                ));
                            } else {
                                injections.push(format!(
                                    "\n<file_content path={:?}>\n[Binary File or Invalid Encoding]\nMetadata: {}\n</file_content>\n<SYSTEM_REMINDER>The user referenced a binary file {} that cannot be displayed as text directly.</SYSTEM_REMINDER>",
                                    display_path, info, display_path
                                ));
                            }
                        }
                        Err(_) => {}
                    }
                }
            } else if path.is_dir() {
                let mut entries = Vec::new();

                // Keep referenced-directory expansion aligned with @ suggestions,
                // the sidebar tree, and native file tools.
                let mut walker = workspace_walk_builder(&path);
                walker.max_depth(Some(1));

                for result in walker.build() {
                    let entry = match result {
                        Ok(e) => e,
                        Err(_) => continue,
                    };

                    if entry.depth() == 0 {
                        continue;
                    }

                    let entry_path = entry.path();
                    if is_path_within_allowed_roots(entry_path, &allowed_roots).is_none() {
                        continue;
                    }

                    let name = entry.file_name().to_string_lossy().to_string();

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
                    display_path, list_str
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

    (
        normalize_at_mentions(prompt, &allowed_roots),
        attached_context,
    )
}

fn allowed_paths_from_workflow_snapshot(snapshot: &WorkflowSnapshot) -> Vec<String> {
    snapshot
        .workflow
        .agent_config
        .as_deref()
        .and_then(AgentConfig::from_json)
        .and_then(|cfg| cfg.allowed_paths)
        .unwrap_or_default()
}

fn inject_at_mentions_into_signal(signal: &str, allowed_paths: &[String]) -> String {
    let mut parsed = match serde_json::from_str::<serde_json::Value>(signal) {
        Ok(value) => value,
        Err(_) => return signal.to_string(),
    };

    let signal_type = parsed
        .get("type")
        .and_then(|value| value.as_str())
        .and_then(SignalType::from_str);
    if !matches!(
        signal_type,
        Some(SignalType::UserMessage | SignalType::LegacyUserInput)
    ) {
        return signal.to_string();
    }

    let Some(content) = parsed
        .get("content")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
    else {
        return signal.to_string();
    };

    let (normalized_content, mention_context) = inject_at_mentions(&content, allowed_paths);
    if normalized_content != content {
        parsed["content"] = json!(normalized_content);
    }
    if mention_context.is_empty() {
        return serde_json::to_string(&parsed).unwrap_or_else(|_| signal.to_string());
    }

    let existing = parsed
        .get("attached_context")
        .and_then(|value| value.as_str())
        .or_else(|| {
            parsed
                .get("attachedContext")
                .and_then(|value| value.as_str())
        })
        .map(|value| value.to_string());
    let combined = combine_attached_context(mention_context, existing);
    parsed["attached_context"] = json!(combined);

    if let Some(object) = parsed.as_object_mut() {
        object.remove("attachedContext");
    }

    serde_json::to_string(&parsed).unwrap_or_else(|_| signal.to_string())
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
    pub auto_approve_plan: Option<bool>,
    pub final_audit: Option<bool>,
    pub inherited_agent_config: Option<String>,
}

fn build_agent_config_from_agent(
    agent: &Agent,
    allowed_paths: Option<&Value>,
    final_audit: Option<bool>,
) -> AgentConfig {
    let mut config = AgentConfig::default();

    config.personality = agent.personality.clone();
    config.models = agent.models.clone();
    config.max_contexts = agent.max_contexts;

    if let Some(policy_str) = &agent.shell_policy {
        config.shell_policy = serde_json::from_str(policy_str).ok();
    }

    config.sandbox_execution_mode = Some(agent.sandbox_execution_mode.clone());
    config.sandbox_scheme_id = agent.sandbox_scheme_id.clone();

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
    config.mcp_tool_exposure = agent
        .mcp_tool_exposure
        .as_deref()
        .and_then(|tools| serde_json::from_str(tools).ok());
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

fn build_workflow_config_for_request(
    agent: &Agent,
    request: &CreateWorkflowRequest,
) -> AgentConfig {
    let mut config =
        build_agent_config_from_agent(agent, request.allowed_paths.as_ref(), request.final_audit);

    if let Some(inherited) = &request.inherited_agent_config {
        if let Some(inherited_config) = validated_inherited_agent_config(inherited) {
            config = merge_inherited_workflow_config(&config, &inherited_config);
        }
    } else {
        config.auto_approve_plan = Some(false);
        config.auto_compress = Some(false);
        config.final_audit = Some(false);
        config.final_review_mode = Some("off".to_string());
        config.phase = Some("standard".to_string());
    }

    if let Some(final_audit) = request.final_audit {
        config.final_audit = Some(final_audit);
        config.final_review_mode = Some(
            if final_audit {
                "sub_agent_review"
            } else {
                "off"
            }
            .to_string(),
        );
    }

    if let Some(auto_approve_plan) = request.auto_approve_plan {
        config.auto_approve_plan = Some(auto_approve_plan);
    }

    fill_missing_agent_config_fields(&mut config, agent);
    enforce_auto_approve_tool_visibility(&mut config);
    config
}

fn merge_unique_tools(tool_lists: impl IntoIterator<Item = Option<Vec<String>>>) -> Vec<String> {
    let mut merged = Vec::new();
    for tools in tool_lists.into_iter().flatten() {
        for tool in tools {
            if !merged.contains(&tool) {
                merged.push(tool);
            }
        }
    }
    merged
}

fn enforce_auto_approve_tool_visibility(config: &mut AgentConfig) {
    let available_tools = config.available_tools.as_ref();
    if let Some(auto_approve) = config.auto_approve.as_mut() {
        auto_approve.retain(|tool| {
            tool != crate::tools::TOOL_BASH
                && available_tools
                    .map(|available| available.contains(tool))
                    .unwrap_or(true)
        });
    }
}

fn merge_shell_allow_rules(
    agent_rules: Option<Vec<crate::tools::ShellPolicyRule>>,
    inherited_rules: Option<Vec<crate::tools::ShellPolicyRule>>,
) -> Option<Vec<crate::tools::ShellPolicyRule>> {
    let mut merged = agent_rules.unwrap_or_default();
    let Some(inherited_rules) = inherited_rules else {
        return Some(merged);
    };

    for rule in inherited_rules {
        if !matches!(rule.decision, crate::tools::ShellDecision::Allow)
            || merged
                .iter()
                .any(|existing| existing.pattern == rule.pattern)
        {
            continue;
        }
        merged.push(rule);
    }

    Some(merged)
}

/// Merges preferences from "create from current workflow" into a newly created
/// workflow. This function is not used for a normal new workflow: that path starts
/// from the current Agent configuration without an inherited payload.
///
/// The current Agent is always the capability/default authority. The inherited
/// snapshot contains only user choices made in the source workflow. In particular,
/// `available_tools` records the tools the user left checked, rather than a second
/// capability definition. The merge filters those checked tools through the current
/// Agent's tool list: removed tools cannot return and newly added Agent tools are not
/// selected until the user chooses them. Auto-approval is then filtered against that
/// resulting visible tool set; shell command policy and sandbox configuration keep
/// their separate inheritance rules below.
fn merge_inherited_workflow_config(
    agent_config: &AgentConfig,
    inherited_config: &AgentConfig,
) -> AgentConfig {
    let mut merged = agent_config.clone();

    // These are workflow preferences and may intentionally carry across sessions.
    merged.allowed_paths = inherited_config
        .allowed_paths
        .clone()
        .or(merged.allowed_paths);
    merged.approval_level = inherited_config
        .approval_level
        .clone()
        .or(merged.approval_level);
    merged.auto_approve_plan = inherited_config
        .auto_approve_plan
        .or(merged.auto_approve_plan);
    merged.auto_compress = inherited_config.auto_compress.or(merged.auto_compress);
    merged.final_audit = inherited_config.final_audit.or(merged.final_audit);
    merged.final_review_mode = inherited_config
        .final_review_mode
        .clone()
        .or(merged.final_review_mode);
    merged.skill_enabled = inherited_config.skill_enabled.or(merged.skill_enabled);
    merged.selected_skills = inherited_config
        .selected_skills
        .clone()
        .or(merged.selected_skills);
    merged.phase = inherited_config.phase.clone().or(merged.phase);
    merged.models = inherited_config.models.clone().or(merged.models);
    merged.personality = resolve_inherited_workflow_personality(agent_config, inherited_config);

    // Agent configuration defines the available tool capabilities. An inherited
    // workflow contributes only the user's checked-tool preference: retain selected
    // tools that the current Agent still exposes, while never restoring removed tools
    // or auto-enabling capabilities added after the preference was chosen.
    let available_tools: Option<Vec<String>> = match (
        agent_config.available_tools.as_ref(),
        inherited_config.available_tools.as_ref(),
    ) {
        (Some(agent_tools), Some(inherited_tools)) => Some(
            agent_tools
                .iter()
                .filter(|tool| inherited_tools.contains(*tool))
                .cloned()
                .collect(),
        ),
        (Some(agent_tools), None) => Some(agent_tools.clone()),
        (None, _) => None,
    };
    merged.available_tools = available_tools.clone();
    let is_tool_allowed = |tool: &str| {
        available_tools.as_ref().map_or(true, |tools| {
            tools.iter().any(|configured| configured == tool)
        })
    };

    merged.mcp_tool_exposure = agent_config.mcp_tool_exposure.clone().map(|tools| {
        tools
            .into_iter()
            .filter(|tool| is_tool_allowed(tool))
            .collect()
    });

    merged.auto_approve = Some(
        merge_unique_tools([
            agent_config.auto_approve.clone(),
            inherited_config.auto_approve.clone(),
        ])
        .into_iter()
        .filter(|tool| tool != crate::tools::TOOL_BASH && is_tool_allowed(tool))
        .collect(),
    );

    // Shell is an Agent capability. If bash is unavailable, do not inherit rules or sandbox state.
    merged.shell_policy = if is_tool_allowed(crate::tools::TOOL_BASH) {
        merge_shell_allow_rules(
            agent_config.shell_policy.clone(),
            inherited_config.shell_policy.clone(),
        )
    } else {
        agent_config.shell_policy.clone()
    };

    // Sandbox settings only inherit when this workflow explicitly overrode the Agent defaults.
    // The scheme itself is resolved again at the next canonical task boundary.
    if is_tool_allowed(crate::tools::TOOL_BASH) && inherited_config.sandbox_override == Some(true) {
        merged.sandbox_override = Some(true);
        merged.sandbox_execution_mode = inherited_config.sandbox_execution_mode.clone();
        merged.sandbox_scheme_id = inherited_config.sandbox_scheme_id.clone();
        merged.sandbox_config = inherited_config.sandbox_config.clone();
    } else {
        merged.sandbox_override = None;
        merged.sandbox_config = agent_config.sandbox_config.clone();
    }

    merged.max_contexts = agent_config.max_contexts;
    merged.sync_legacy_final_audit_flag();
    enforce_auto_approve_tool_visibility(&mut merged);
    merged
}

fn resolve_inherited_workflow_personality(
    agent_config: &AgentConfig,
    inherited_config: &AgentConfig,
) -> Option<String> {
    let inherited = inherited_config.personality.as_deref().map(str::trim);
    match inherited {
        None | Some("") => agent_config.personality.clone(),
        Some(value) if is_agent_personality_preset(value) => Some(value.to_string()),
        Some(value) if value.starts_with(AGENT_PERSONALITY_PRESET_PREFIX) => {
            Some(AGENT_PERSONALITY_PRESET_DEFAULT_ID.to_string())
        }
        Some(value) if agent_config.personality.as_deref().map(str::trim) == Some(value) => {
            Some(value.to_string())
        }
        Some(_) => Some(AGENT_PERSONALITY_PRESET_DEFAULT_ID.to_string()),
    }
}

fn resolve_agent_sandbox_snapshot(
    store: &MainStore,
    agent: &Agent,
    config: &mut AgentConfig,
) -> Result<(), String> {
    let (execution_mode, scheme_id) = if config.sandbox_override == Some(true) {
        (
            config
                .sandbox_execution_mode
                .clone()
                .unwrap_or(crate::tools::ShellExecutionMode::HostOnly),
            config.sandbox_scheme_id.clone(),
        )
    } else {
        (
            agent.sandbox_execution_mode.clone(),
            agent.sandbox_scheme_id.clone(),
        )
    };

    config.sandbox_execution_mode = Some(execution_mode.clone());
    config.sandbox_scheme_id = scheme_id.clone();
    config.sandbox_config = None;

    match execution_mode {
        crate::tools::ShellExecutionMode::HostOnly => Ok(()),
        crate::tools::ShellExecutionMode::Auto | crate::tools::ShellExecutionMode::SandboxOnly => {
            let scheme_id = scheme_id
                .as_deref()
                .ok_or_else(|| "sandbox execution mode requires a scheme reference".to_string())?;
            let scheme = store
                .get_sandbox_scheme(scheme_id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("sandbox scheme {scheme_id} not found"))?;
            if scheme.disabled {
                return Err(format!("sandbox scheme {scheme_id} is disabled"));
            }
            scheme.config.validate()?;
            if execution_mode == crate::tools::ShellExecutionMode::Auto
                && crate::tools::enabled_common_profile(scheme.config.profiles.iter())?.is_none()
            {
                return Err(
                    "auto sandbox scheme requires one enabled common catch-all profile".to_string(),
                );
            }
            let profiles = scheme
                .config
                .profiles
                .into_iter()
                .map(|profile| (profile.id.clone(), profile))
                .collect();
            config.sandbox_config = Some(crate::tools::AgentSandboxConfig {
                scheme_id: Some(scheme.id),
                scheme_revision: scheme.updated_at,
                execution_mode,
                runtime_preference: scheme.config.runtime_preference,
                profiles,
                host_rules: scheme.config.host_rules,
            });
            Ok(())
        }
    }
}

fn reset_workflow_phase_for_new_context(store: &MainStore, session_id: &str) -> Result<(), String> {
    let mut config = raw_workflow_agent_config(store, session_id)?;
    config.phase = Some("standard".to_string());
    store
        .update_workflow_agent_config(session_id, &config.to_json())
        .map_err(|error| error.to_string())
}

fn sync_workflow_agent_config_at_tool_boundary(
    store: &MainStore,
    session_id: &str,
) -> Result<AgentConfig, String> {
    let workflow = store
        .get_workflow(session_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Workflow {} not found", session_id))?;
    let agent = store
        .get_agent(&workflow.agent_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Agent {} not found", workflow.agent_id))?;

    let agent_config = build_agent_config_from_agent(&agent, None, None);
    let mut merged = workflow
        .agent_config
        .as_deref()
        .and_then(validated_inherited_agent_config)
        .map(|inherited| merge_inherited_workflow_config(&agent_config, &inherited))
        .unwrap_or(agent_config);
    fill_missing_agent_config_fields(&mut merged, &agent);
    resolve_agent_sandbox_snapshot(store, &agent, &mut merged)?;
    enforce_auto_approve_tool_visibility(&mut merged);

    let merged_json = merged.to_json();
    if workflow.agent_config.as_deref() != Some(merged_json.as_str()) {
        store
            .update_workflow_agent_config(session_id, &merged_json)
            .map_err(|e| e.to_string())?;
        log::info!(
            "[Workflow][session={}][phase=config] Synchronized Agent tool capabilities at task boundary",
            session_id
        );
    }

    Ok(merged)
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

    if config.personality.is_none() && defaults.personality.is_some() {
        config.personality = defaults.personality;
        changed = true;
    }

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
    if config.sandbox_config.is_none() && defaults.sandbox_config.is_some() {
        config.sandbox_config = defaults.sandbox_config;
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
    if config.mcp_tool_exposure.is_none() && defaults.mcp_tool_exposure.is_some() {
        config.mcp_tool_exposure = defaults.mcp_tool_exposure;
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
    state: State<'_, Arc<MainStore>>,
    chat_state: State<'_, Arc<ChatState>>,
    gateway: State<'_, Arc<TauriGateway>>,
    request: CreateWorkflowRequest,
) -> Result<String, String> {
    let (agent, runtime) = {
        let store = &*state;
        let agent = store
            .get_agent(&request.agent_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Agent {} not found", request.agent_id))?;
        let runtime = store.db_runtime().map_err(|e| e.to_string())?;
        (agent, runtime)
    };

    // Always use TSID for new workflow sessions
    let session_id = tsid_generator.generate().map_err(|e| e.to_string())?;

    log::info!(
        "[Workflow][session={}][phase=create] Creating workflow for agent_id={}",
        session_id,
        request.agent_id
    );

    if agent.role.as_deref() == Some("child") {
        return Err("Child agents cannot be used as top-level workflow agents".to_string());
    }

    let mut config = build_workflow_config_for_request(&agent, &request);
    {
        let store = &*state;
        resolve_agent_sandbox_snapshot(store, &agent, &mut config)?;
    }

    let agent_config_json = config.to_json();

    // Use empty string for user_query if not provided (new workflow creation)
    let user_query = request.user_query.as_deref().unwrap_or("");

    MainStore::create_workflow_with_runtime(
        runtime,
        session_id.clone(),
        user_query.to_string(),
        request.agent_id.clone(),
        Some(agent_config_json),
        None,
    )
    .await
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
pub async fn list_workflows(state: State<'_, Arc<MainStore>>) -> Result<Vec<Workflow>, String> {
    let runtime = state.db_runtime().map_err(|e| e.to_string())?;

    MainStore::list_workflows_with_runtime(runtime)
        .await
        .map_err(|e| e.to_string())
}

fn terminal_workflow_state(runtime_state: &RuntimeState) -> Option<WorkflowState> {
    match runtime_state {
        RuntimeState::Completed => Some(WorkflowState::Completed),
        RuntimeState::Failed => Some(WorkflowState::Error),
        RuntimeState::Cancelled => Some(WorkflowState::Cancelled),
        _ => None,
    }
}

fn terminal_workflow_state_from_event_type(event_type: &str) -> Option<WorkflowState> {
    match event_type {
        "workflow_completed" => Some(WorkflowState::Completed),
        "workflow_failed" => Some(WorkflowState::Error),
        "workflow_cancelled" => Some(WorkflowState::Cancelled),
        _ => None,
    }
}

fn durable_child_terminal_state(
    store: &MainStore,
    child_id: &str,
) -> Result<Option<WorkflowState>, crate::db::StoreError> {
    let snapshot_error = match store.get_execution_context(child_id) {
        Ok(Some(context)) if context.version == ExecutionContext::CURRENT_VERSION => {
            if let Some(terminal_state) = terminal_workflow_state(&context.state) {
                return Ok(Some(terminal_state));
            }
            None
        }
        Ok(Some(context)) => Some(format!(
            "Child workflow snapshot version mismatch: expected {}, got {}",
            ExecutionContext::CURRENT_VERSION,
            context.version
        )),
        Ok(None) => None,
        Err(error) => Some(format!("Child workflow snapshot is unreadable: {error}")),
    };

    if let Some(event_type) = store.latest_workflow_event_type(child_id)? {
        if let Some(terminal_state) = terminal_workflow_state_from_event_type(&event_type) {
            return Ok(Some(terminal_state));
        }
    }

    let events = store.list_workflow_events(child_id)?;
    if events.is_empty() {
        if let Some(error) = snapshot_error {
            return Err(crate::db::StoreError::InvalidData(error));
        }
        return Ok(None);
    }

    let context = replay_events_to_execution_context(child_id, &events).map_err(|error| {
        crate::db::StoreError::InvalidData(format!(
            "Cannot safely reconcile child workflow {child_id}: {error}"
        ))
    })?;
    Ok(terminal_workflow_state(&context.state))
}

fn child_was_detached_by_latest_manual_clear(
    store: &MainStore,
    parent_session_id: &str,
    child_id: &str,
) -> Result<bool, crate::db::StoreError> {
    let snapshot = store.get_workflow_snapshot(parent_session_id)?;
    let marker_index = snapshot
        .messages
        .iter()
        .rposition(ContextManager::is_manual_clear_context_message);
    let Some(marker_index) = marker_index else {
        return Ok(false);
    };
    let marker = &snapshot.messages[marker_index];
    let previous_context = marker
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("previous_execution_context"))
        .cloned()
        .and_then(|value| serde_json::from_value::<ExecutionContext>(value).ok());
    let tracked_by_previous_context = previous_context.is_some_and(|context| {
        context
            .sub_agent_sessions
            .iter()
            .any(|sub_agent_id| sub_agent_id == child_id)
            || context.waiting_on_sub_agent_id.as_deref() == Some(child_id)
            || context
                .pending_sub_agent_completions
                .iter()
                .any(|completion| completion.sub_agent_id == child_id)
    });
    let observed_before_clear = snapshot.messages[..marker_index].iter().any(|message| {
        message
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("sub_agent_id"))
            .and_then(Value::as_str)
            == Some(child_id)
    });
    if !tracked_by_previous_context && !observed_before_clear {
        return Ok(false);
    }

    let current_context = store.get_execution_context(parent_session_id)?;
    Ok(current_context.is_none_or(|context| {
        !context
            .sub_agent_sessions
            .iter()
            .any(|sub_agent_id| sub_agent_id == child_id)
            && context.waiting_on_sub_agent_id.as_deref() != Some(child_id)
            && !context
                .pending_sub_agent_completions
                .iter()
                .any(|completion| completion.sub_agent_id == child_id)
    }))
}

fn reconcile_durable_child_completion(
    store: &MainStore,
    child: &Workflow,
    terminal_status: WorkflowState,
) -> Result<bool, crate::db::StoreError> {
    let Some(child_id) = child.id.as_deref() else {
        return Ok(false);
    };
    let Some(parent_session_id) = child.parent_session_id.as_deref() else {
        return Ok(false);
    };
    let status = match terminal_status {
        WorkflowState::Completed => "completed",
        WorkflowState::Cancelled => "cancelled",
        WorkflowState::Error => "failed",
        _ => return Ok(false),
    };
    let mut parent_context = store
        .get_execution_context(parent_session_id)?
        .unwrap_or_else(|| ExecutionContext {
            session_id: parent_session_id.to_string(),
            state: RuntimeState::Waiting,
            wait_reason: Some(WaitReason::SubAgent),
            queued_user_messages: Vec::new(),
            current_segment_id: 1,
            current_step: 0,
            max_steps: 0,
            pending_tools: Vec::new(),
            last_action_summary: None,
            current_context_tokens: None,
            max_context_tokens: None,
            last_event_id: None,
            version: ExecutionContext::CURRENT_VERSION.to_string(),
            waiting_on_sub_agent_id: Some(child_id.to_string()),
            awaiting_user_tool_call_id: None,
            effective_task_objective: None,
            sub_agent_sessions: vec![child_id.to_string()],
            pending_sub_agent_completions: Vec::new(),
            pending_final_review: None,
            pending_completion_reports: Vec::new(),
            removed_queued_user_message_ids: Vec::new(),
        });
    let existing_completion = parent_context
        .pending_sub_agent_completions
        .iter()
        .find(|completion| completion.sub_agent_id == child_id);
    let already_projected =
        existing_completion.is_some_and(|completion| completion.usage_summary.is_some());
    let should_signal = matches!(&parent_context.state, RuntimeState::Waiting)
        && matches!(
            parent_context.wait_reason.as_ref(),
            Some(WaitReason::SubAgent)
        )
        && parent_context.waiting_on_sub_agent_id.as_deref() == Some(child_id)
        && existing_completion.is_none_or(|completion| !completion.consumed);
    parent_context
        .sub_agent_sessions
        .retain(|sub_agent_id| sub_agent_id != child_id);
    if !should_signal && parent_context.waiting_on_sub_agent_id.as_deref() == Some(child_id) {
        parent_context.waiting_on_sub_agent_id = None;
    }
    let (has_event, is_background, has_background_projection) =
        store.get_child_reconciliation_state(parent_session_id, child_id)?;
    if already_projected
        && has_event
        && (!is_background || has_background_projection)
        && !should_signal
    {
        store.upsert_execution_context(&parent_context)?;
        return Ok(false);
    }

    let task_run_id = store
        .workflow_current_task_run_id(child_id)
        .unwrap_or_else(|_| format!("{child_id}:task:1"));
    let duration_ms = store
        .workflow_started_at_ms(child_id)
        .ok()
        .flatten()
        .map(|started_at_ms| (chrono::Utc::now().timestamp_millis() - started_at_ms).max(0));
    let root_task_run_id = store
        .workflow_current_task_run_id(parent_session_id)
        .unwrap_or_else(|_| format!("{parent_session_id}:task:1"));
    let usage_summary = match store.load_workflow_task_usage(child_id, &task_run_id)? {
        Some(summary) => summary,
        None => {
            let summary = store.summarize_workflow_task_usage(
                child_id,
                &task_run_id,
                parent_session_id,
                &root_task_run_id,
                status,
                duration_ms,
            )?;
            store.upsert_workflow_task_usage(
                child_id,
                &task_run_id,
                parent_session_id,
                &root_task_run_id,
                status,
                None,
                None,
                summary.duration_ms,
                &summary,
            )?;
            summary
        }
    };
    let result = json!({
        "status": status,
        "task_id": child_id,
        "summary": "Sub-agent terminal completion recovered after restart",
        "tool_calls_count": 0,
        "usage_summary": usage_summary,
    });

    if !already_projected {
        parent_context
            .pending_sub_agent_completions
            .retain(|completion| completion.sub_agent_id != child_id);
        parent_context
            .pending_sub_agent_completions
            .push(SubAgentCompletion {
                sub_agent_id: child_id.to_string(),
                parent_session_id: parent_session_id.to_string(),
                status: status.to_string(),
                result: result["result"].as_str().map(str::to_string),
                summary: result["summary"].as_str().map(str::to_string),
                error: result["error"].as_str().map(str::to_string),
                tool_calls_count: 0,
                usage_summary: serde_json::from_value(result["usage_summary"].clone()).ok(),
                completed_at_ms: chrono::Utc::now().timestamp_millis(),
                consumed: false,
            });
    }
    store.upsert_execution_context(&parent_context)?;

    // Re-deliver only while the parent is durably waiting for this child. A missing live channel
    // is normal during cold recovery; the durable completion will be applied when the executor
    // is restored.
    if should_signal {
        if let Err(error) = WorkflowManager::try_send_signal_to_session(
            parent_session_id,
            json!({
                "type": "sub_agent_complete",
                "sub_agent_id": child_id,
                "result": result,
            })
            .to_string(),
        ) {
            if error.contains("signal channel not found") {
                log::trace!(
                    "[Workflow][session={}][phase=reconcile][event=sub_agent_completion] Live recovery channel unavailable; durable completion remains replayable: {}",
                    child_id,
                    error
                );
            } else {
                log::warn!(
                    "[Workflow][session={}][phase=reconcile][event=sub_agent_completion] Live recovery signal failed; durable completion remains replayable: {}",
                    child_id,
                    error
                );
            }
        }
    }

    if !has_event {
        store.append_workflow_event(&WorkflowEvent::sub_agent_completed(
            parent_session_id.to_string(),
            child_id.to_string(),
            status.to_string(),
            result.clone(),
        ))?;
    }

    if is_background && !has_background_projection {
        let segment_id = parent_context.current_segment_id;
        let summary = result["summary"].as_str().unwrap_or_default();
        let mut metadata = runtime_observation_metadata_with_visibility(
            RuntimeObservationType::SubAgentCompletion,
            RuntimeObservationLlmVisibility::Hide,
            RuntimeObservationUiVisibility::Hide,
            json!({
                "sub_agent_id": child_id,
                "execution_mode": "background",
                "result": result,
                "summary": summary,
                "execution_status": status,
            }),
        );
        metadata["sub_agent_id"] = json!(child_id);
        metadata["execution_mode"] = json!("background");
        metadata["result"] = result.clone();
        metadata["summary"] = json!(summary);
        metadata["execution_status"] = json!(status);
        store.add_workflow_message(&WorkflowMessage {
            id: None,
            session_id: parent_session_id.to_string(),
            role: "user".to_string(),
            message: String::new(),
            reasoning: None,
            message_kind: "runtime_observation".to_string(),
            message_subtype: Some("sub_agent_completion".to_string()),
            segment_id,
            source_event_type: Some("sub_agent_completed".to_string()),
            metadata: Some(metadata),
            attached_context: None,
            step_type: Some(StepType::Observe.to_string()),
            step_index: 0,
            is_error: false,
            error_type: None,
            created_at: None,
        })?;
    }
    Ok(true)
}

fn reconcile_child_workflows(
    store: &MainStore,
    child_workflows: Vec<Workflow>,
) -> Result<(), crate::db::StoreError> {
    let mut decisions = Vec::new();

    for child in child_workflows {
        let Some(child_id) = child.id.clone() else {
            continue;
        };
        if BACKGROUND_TASKS.contains_key(&child_id) {
            continue;
        }

        let detached_by_manual_clear = match child.parent_session_id.as_deref() {
            Some(parent_session_id) => {
                child_was_detached_by_latest_manual_clear(store, parent_session_id, &child_id)?
            }
            None => false,
        };
        let terminal_status = durable_child_terminal_state(store, &child_id)?;
        decisions.push((child, child_id, terminal_status, detached_by_manual_clear));
    }

    for (child, child_id, terminal_status, detached_by_manual_clear) in decisions {
        if detached_by_manual_clear {
            let detached_status = terminal_status.clone().unwrap_or(WorkflowState::Cancelled);
            store.update_workflow_status(&child_id, &detached_status.to_string())?;
            if terminal_status.is_none() {
                let mut child_context =
                    store
                        .get_execution_context(&child_id)?
                        .unwrap_or_else(|| ExecutionContext {
                            session_id: child_id.clone(),
                            state: RuntimeState::Cancelled,
                            wait_reason: None,
                            queued_user_messages: Vec::new(),
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
                            awaiting_user_tool_call_id: None,
                            effective_task_objective: None,
                            sub_agent_sessions: Vec::new(),
                            pending_sub_agent_completions: Vec::new(),
                            pending_final_review: None,
                            pending_completion_reports: Vec::new(),
                            removed_queued_user_message_ids: Vec::new(),
                        });
                child_context.state = RuntimeState::Cancelled;
                child_context.wait_reason = None;
                child_context.pending_tools.clear();
                child_context.waiting_on_sub_agent_id = None;
                child_context.last_action_summary =
                    Some("Sub-agent detached by manual context clear.".to_string());
                store.upsert_execution_context(&child_context)?;
            }
            log::info!(
                "[Workflow][session={}][phase=reconcile] Child belongs to a cleared parent context; parent projection was skipped",
                child_id
            );
            continue;
        }
        if let Some(terminal_status) = terminal_status {
            let reconciled =
                reconcile_durable_child_completion(store, &child, terminal_status.clone())?;
            if reconciled {
                log::info!(
                    "[Workflow][session={}][phase=reconcile][event=child_terminal_durable] Recovered durable terminal child state={:?}",
                    child_id,
                    terminal_status
                );
                store.update_workflow_status(&child_id, &terminal_status.to_string())?;
            }
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
                    queued_user_messages: Vec::new(),
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
                    awaiting_user_tool_call_id: None,
                    effective_task_objective: None,
                    sub_agent_sessions: Vec::new(),
                    pending_sub_agent_completions: Vec::new(),
                    pending_final_review: None,
                    pending_completion_reports: Vec::new(),
                    removed_queued_user_message_ids: Vec::new(),
                });
        context.state = RuntimeState::Cancelled;
        context.wait_reason = None;
        context.pending_tools.clear();
        context.waiting_on_sub_agent_id = None;
        context.last_action_summary =
            Some("Sub-agent interrupted by application restart.".to_string());
        store.upsert_execution_context(&context)?;

        if let Some(parent_session_id) = child.parent_session_id.clone() {
            let task_run_id = store
                .workflow_current_task_run_id(&child_id)
                .unwrap_or_else(|_| format!("{child_id}:task:1"));
            let duration_ms =
                store
                    .workflow_started_at_ms(&child_id)
                    .ok()
                    .flatten()
                    .map(|started_at_ms| {
                        (chrono::Utc::now().timestamp_millis() - started_at_ms).max(0)
                    });
            let root_task_run_id = store
                .workflow_current_task_run_id(&parent_session_id)
                .unwrap_or_else(|_| format!("{parent_session_id}:task:1"));
            let usage_summary = store.summarize_workflow_task_usage(
                &child_id,
                &task_run_id,
                &parent_session_id,
                &root_task_run_id,
                "interrupted",
                duration_ms,
            )?;
            store.upsert_workflow_task_usage(
                &child_id,
                &task_run_id,
                &parent_session_id,
                &root_task_run_id,
                "interrupted",
                None,
                None,
                usage_summary.duration_ms,
                &usage_summary,
            )?;
            let usage_summary = Some(usage_summary);
            let event = WorkflowEvent::sub_agent_interrupted(
                parent_session_id.clone(),
                child_id.clone(),
                "application_restart".to_string(),
            );
            store.append_workflow_event(&event)?;

            let message = format!(
                "<SYSTEM_REMINDER>\nSub-agent {} was interrupted by application restart and marked as cancelled.\n</SYSTEM_REMINDER>",
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
                                "tool_calls_count": context.pending_tools.len(),
                                "usage_summary": usage_summary.clone()
                            },
                        }),
                    );
                    metadata["sub_agent_id"] = json!(child_id.clone());
                    metadata["summary"] = json!("Sub-agent interrupted");
                    metadata["result"] = json!({
                        "status": "interrupted",
                        "task_id": child_id.clone(),
                        "error": "Sub-agent interrupted by application restart",
                        "tool_calls_count": context.pending_tools.len(),
                        "usage_summary": usage_summary.clone()
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
                        usage_summary,
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

pub(crate) fn reconcile_interrupted_child_workflows(
    store: &MainStore,
) -> Result<(), crate::db::StoreError> {
    let child_workflows = store.list_child_workflows()?;
    reconcile_child_workflows(store, child_workflows)
}

fn reconcile_child_workflows_for_parent(
    store: &MainStore,
    parent_session_id: &str,
) -> Result<(), crate::db::StoreError> {
    // A current terminal parent snapshot cannot resume or consume a child completion. Avoid
    // scanning every child against the parent's full event history during read-only UI loads.
    // Keep the fallback for missing, stale, or inconsistent snapshots so recovery remains safe.
    if let Some(context) = store.get_execution_context(parent_session_id)? {
        if context.version == ExecutionContext::CURRENT_VERSION
            && terminal_workflow_state(&context.state).is_some()
            && context.waiting_on_sub_agent_id.is_none()
            && context.sub_agent_sessions.is_empty()
            && context.pending_sub_agent_completions.is_empty()
        {
            log::debug!(
                "[Workflow][session={}][phase=reconcile] Skipping child recovery for terminal parent snapshot",
                parent_session_id
            );
            return Ok(());
        }
    }

    let child_workflows = store.list_child_workflows_for_parent(parent_session_id)?;
    reconcile_child_workflows(store, child_workflows)
}

#[tauri::command]
pub async fn delete_workflow(
    state: State<'_, Arc<MainStore>>,
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

    let runtime = {
        let store = &*state;
        store.db_runtime().map_err(|e| e.to_string())?
    };
    MainStore::delete_workflow_with_runtime(runtime, session_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_last_workflow_message(
    state: State<'_, Arc<MainStore>>,
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

    let store = &*state;
    store
        .delete_last_message(&session_id)
        .map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowContextFrameResult {
    pub noop: bool,
    pub segment_id: i32,
    pub marker_message: Option<WorkflowMessage>,
    pub current_context_tokens: usize,
    pub max_context_tokens: usize,
    pub state: WorkflowState,
    pub wait_reason: Option<WaitReason>,
    pub has_live_session: bool,
}

fn newest_persisted_segment_id(messages: &[WorkflowMessage]) -> i32 {
    messages
        .iter()
        .map(|message| message.segment_id)
        .max()
        .unwrap_or(1)
}

fn effective_segment_id_from_snapshot(
    messages: &[WorkflowMessage],
    execution_context_segment_id: Option<i32>,
) -> i32 {
    newest_persisted_segment_id(messages)
        .max(execution_context_segment_id.unwrap_or(1))
        .max(1)
}

async fn hydrate_execution_context_for_snapshot(
    main_store: Arc<MainStore>,
    session_id: &str,
    workflow_status: Option<&str>,
    execution_context: Option<ExecutionContext>,
) -> Option<ExecutionContext> {
    let terminal_runtime_state = workflow_status
        .and_then(|status| status.parse::<WorkflowState>().ok())
        .map(|state| RuntimeState::from(&state))
        .filter(|state| {
            matches!(
                state,
                RuntimeState::Completed | RuntimeState::Failed | RuntimeState::Cancelled
            )
        });
    let mut execution_context = execution_context;
    if let (Some(context), Some(terminal_state)) =
        (execution_context.as_mut(), terminal_runtime_state)
    {
        context.state = terminal_state;
        context.wait_reason = None;
    }

    let needs_recovery = match execution_context.as_ref() {
        Some(ctx) => ctx.current_context_tokens.is_none(),
        None => true,
    };
    if !needs_recovery {
        return execution_context;
    }

    let max_context_tokens = execution_context
        .as_ref()
        .and_then(|ctx| ctx.max_context_tokens)
        .unwrap_or(4096);
    let tsid_generator = Arc::new(TsidGenerator::new(0).ok()?);
    let mut context = ContextManager::new(
        session_id.to_string(),
        main_store,
        max_context_tokens,
        tsid_generator,
    );
    context.load_history().await.ok()?;

    let mut hydrated = execution_context.unwrap_or_else(|| {
        let runtime_state = workflow_status
            .and_then(|status| status.parse::<WorkflowState>().ok())
            .map(|state| RuntimeState::from(&state))
            .unwrap_or(RuntimeState::Pending);
        ExecutionContext {
            session_id: session_id.to_string(),
            state: runtime_state,
            wait_reason: None,
            queued_user_messages: Vec::new(),
            current_segment_id: context.current_segment_id,
            current_step: 0,
            max_steps: 100,
            pending_tools: Vec::new(),
            last_action_summary: None,
            current_context_tokens: None,
            max_context_tokens: Some(max_context_tokens),
            last_event_id: None,
            version: ExecutionContext::CURRENT_VERSION.to_string(),
            waiting_on_sub_agent_id: None,
            awaiting_user_tool_call_id: None,
            effective_task_objective: None,
            sub_agent_sessions: Vec::new(),
            pending_sub_agent_completions: Vec::new(),
            pending_final_review: None,
            pending_completion_reports: Vec::new(),
            removed_queued_user_message_ids: Vec::new(),
        }
    });
    hydrated.current_segment_id = context.current_segment_id;
    hydrated.current_context_tokens = Some(context.current_token_estimate());
    hydrated.max_context_tokens = Some(context.max_tokens);

    Some(hydrated)
}

async fn begin_new_context_frame_for_cold_session(
    main_store: Arc<MainStore>,
    tsid_generator: Arc<TsidGenerator>,
    session_id: &str,
    previous_execution_context: Option<ExecutionContext>,
) -> Result<(i32, Option<WorkflowMessage>), String> {
    if previous_execution_context
        .as_ref()
        .is_some_and(|ctx| !runtime_state_allows_manual_clear(&ctx.state))
    {
        return Err("Cannot clear context unless workflow is stopped".to_string());
    }
    if wait_reason_blocks_manual_clear(
        previous_execution_context
            .as_ref()
            .and_then(|ctx| ctx.wait_reason.as_ref()),
    ) {
        return Err(
            "Cannot clear context while workflow is waiting for user interaction".to_string(),
        );
    }

    let max_context_tokens = previous_execution_context
        .as_ref()
        .and_then(|ctx| ctx.max_context_tokens)
        .unwrap_or(4096);

    let mut context = ContextManager::new(
        session_id.to_string(),
        main_store.clone(),
        max_context_tokens,
        tsid_generator,
    );
    context.load_history().await.map_err(|e| e.to_string())?;
    if context
        .messages
        .last()
        .is_some_and(ContextManager::is_manual_clear_context_message)
    {
        return Ok((context.current_segment_id, context.messages.last().cloned()));
    }
    context
        .begin_manual_clear_context_segment(0)
        .await
        .map_err(|e| e.to_string())?;

    let mut execution_context = previous_execution_context.unwrap_or_else(|| {
        crate::workflow::react::types::ExecutionContext {
            session_id: session_id.to_string(),
            state: RuntimeState::Pending,
            wait_reason: None,
            queued_user_messages: Vec::new(),
            current_segment_id: context.current_segment_id,
            current_step: 0,
            max_steps: 100,
            pending_tools: Vec::new(),
            last_action_summary: None,
            current_context_tokens: None,
            max_context_tokens: Some(max_context_tokens),
            last_event_id: None,
            version: crate::workflow::react::types::ExecutionContext::CURRENT_VERSION.to_string(),
            waiting_on_sub_agent_id: None,
            awaiting_user_tool_call_id: None,
            effective_task_objective: None,
            sub_agent_sessions: Vec::new(),
            pending_sub_agent_completions: Vec::new(),
            pending_final_review: None,
            pending_completion_reports: Vec::new(),
            removed_queued_user_message_ids: Vec::new(),
        }
    });
    execution_context.current_segment_id = context.current_segment_id;
    execution_context.current_context_tokens = Some(context.current_token_estimate());
    execution_context.max_context_tokens = Some(context.max_tokens);

    let store = &*main_store;
    store
        .upsert_execution_context(&execution_context)
        .map_err(|e| e.to_string())?;

    Ok((context.current_segment_id, context.messages.last().cloned()))
}

async fn clear_persisted_workflow_todo_list(
    main_store: &MainStore,
    session_id: &str,
) -> Result<(), String> {
    let runtime = main_store.db_runtime().map_err(|e| e.to_string())?;
    MainStore::update_workflow_todo_list_with_runtime(
        runtime,
        session_id.to_string(),
        "[]".to_string(),
    )
    .await
    .map_err(|e| e.to_string())
}

async fn finalize_manual_clear_context_state(
    main_store: &Arc<MainStore>,
    workflow_manager: &Arc<WorkflowManager>,
    gateway: &Arc<TauriGateway>,
    session_id: &str,
) -> Result<(), String> {
    {
        let store = &*main_store;
        persist_pending_workflow_state(&store, session_id)?;
    }
    clear_persisted_workflow_todo_list(main_store, session_id).await?;

    if let Some(executor) = workflow_manager.get_executor(session_id) {
        let mut guard = executor.lock().await;
        guard.set_state(WorkflowState::Pending);
    }
    workflow_manager.remove_session(session_id);
    BACKGROUND_TASKS.remove(session_id);
    WorkflowManager::unregister_session_signal_tx_with_source(
        session_id,
        "begin_new_context_frame.finalize",
    );
    gateway
        .unregister_session_with_source(session_id, "begin_new_context_frame.finalize")
        .await;

    log::info!(
        "[Workflow][session={}][phase=begin_new_context_frame] Manual clear completed; authoritative state transitioned to pending and stopped executor resources were released",
        session_id
    );
    Ok(())
}

#[tauri::command]
pub async fn workflow_begin_new_context_frame(
    state: State<'_, Arc<MainStore>>,
    chat_state: State<'_, Arc<ChatState>>,
    tsid_generator: State<'_, Arc<TsidGenerator>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    gateway: State<'_, Arc<TauriGateway>>,
    session_id: String,
) -> Result<WorkflowContextFrameResult, String> {
    let main_store = state.inner().clone();
    let chat_state = chat_state.inner().clone();
    let workflow_manager = workflow_manager.inner().clone();
    let gateway = gateway.inner().clone();
    let tsid_generator = tsid_generator.inner().clone();

    let snapshot = {
        let store = &*main_store;
        store
            .get_workflow_snapshot(&session_id)
            .map_err(|e| e.to_string())?
    };

    let execution_context = hydrate_execution_context_for_snapshot(
        main_store.clone(),
        &session_id,
        Some(snapshot.workflow.status.as_str()),
        restore_context_for_signal(main_store.clone(), &session_id),
    )
    .await;
    let effective_segment_id = effective_segment_id_from_snapshot(
        &snapshot.messages,
        execution_context.as_ref().map(|ctx| ctx.current_segment_id),
    );
    if snapshot
        .messages
        .last()
        .is_some_and(ContextManager::is_manual_clear_context_message)
    {
        {
            let store = &*main_store;
            sync_workflow_agent_config_at_tool_boundary(&store, &session_id)?;
            reset_workflow_phase_for_new_context(&store, &session_id)?;
        }
        cleanup_owned_background_resources(&session_id, &chat_state).await;
        finalize_manual_clear_context_state(&main_store, &workflow_manager, &gateway, &session_id)
            .await?;
        return Ok(WorkflowContextFrameResult {
            noop: true,
            segment_id: effective_segment_id,
            marker_message: snapshot.messages.last().cloned(),
            current_context_tokens: execution_context
                .as_ref()
                .and_then(|ctx| ctx.current_context_tokens)
                .unwrap_or(0),
            max_context_tokens: execution_context
                .as_ref()
                .and_then(|ctx| ctx.max_context_tokens)
                .unwrap_or(4096),
            state: WorkflowState::Pending,
            wait_reason: None,
            has_live_session: false,
        });
    }

    if wait_reason_blocks_manual_clear(
        execution_context
            .as_ref()
            .and_then(|ctx| ctx.wait_reason.as_ref()),
    ) {
        return Err(
            "Cannot clear context while workflow is waiting for user interaction".to_string(),
        );
    }
    if execution_context
        .as_ref()
        .is_some_and(|ctx| !runtime_state_allows_manual_clear(&ctx.state))
    {
        return Err("Cannot clear context unless workflow is stopped".to_string());
    }

    if has_reconciled_live_session(&workflow_manager, &session_id, "begin_new_context_frame").await
    {
        let Some(executor) = workflow_manager.get_executor(&session_id) else {
            return Err("Workflow session is live but executor is unavailable".to_string());
        };

        let state = {
            let guard = executor.lock().await;
            guard.state()
        };
        let live_wait_reason = match state {
            WorkflowState::Paused => Some(WaitReason::Confirmation),
            WorkflowState::AwaitingUser => Some(WaitReason::UserInput),
            WorkflowState::AwaitingApproval | WorkflowState::AwaitingAutoApproval => {
                Some(WaitReason::Approval)
            }
            WorkflowState::AwaitingSubAgent => Some(WaitReason::SubAgent),
            _ => None,
        };
        if wait_reason_blocks_manual_clear(live_wait_reason.as_ref()) {
            return Err(
                "Cannot clear context while workflow is waiting for user interaction".to_string(),
            );
        }
        if !workflow_state_allows_manual_clear(&state) {
            return Err("Cannot clear context unless workflow is stopped".to_string());
        }

        {
            let mut guard = executor.lock().await;
            guard
                .begin_manual_clear_context_segment()
                .await
                .map_err(|e| e.to_string())?;
        }
        {
            let store = &*main_store;
            sync_workflow_agent_config_at_tool_boundary(&store, &session_id)?;
            reset_workflow_phase_for_new_context(&store, &session_id)?;
        }

        let (marker_message, segment_id, current_context_tokens, max_context_tokens) = {
            let store = &*main_store;
            let marker_message = store
                .get_workflow_snapshot(&session_id)
                .map_err(|e| e.to_string())?
                .messages
                .last()
                .cloned();
            let current_context = store
                .get_execution_context(&session_id)
                .map_err(|e| e.to_string())?;
            (
                marker_message,
                current_context
                    .as_ref()
                    .map(|ctx| ctx.current_segment_id)
                    .unwrap_or(effective_segment_id.saturating_add(1)),
                current_context
                    .as_ref()
                    .and_then(|ctx| ctx.current_context_tokens)
                    .unwrap_or(0),
                current_context
                    .as_ref()
                    .and_then(|ctx| ctx.max_context_tokens)
                    .unwrap_or(4096),
            )
        };
        cleanup_owned_background_resources(&session_id, &chat_state).await;
        finalize_manual_clear_context_state(&main_store, &workflow_manager, &gateway, &session_id)
            .await?;
        return Ok(WorkflowContextFrameResult {
            noop: false,
            segment_id,
            marker_message,
            current_context_tokens,
            max_context_tokens,
            state: WorkflowState::Pending,
            wait_reason: None,
            has_live_session: false,
        });
    }

    let (segment_id, marker_message) = begin_new_context_frame_for_cold_session(
        main_store.clone(),
        tsid_generator,
        &session_id,
        execution_context,
    )
    .await?;
    {
        let store = &*main_store;
        sync_workflow_agent_config_at_tool_boundary(&store, &session_id)?;
        reset_workflow_phase_for_new_context(&store, &session_id)?;
    }
    let current_context = {
        let store = &*state;
        store
            .get_execution_context(&session_id)
            .map_err(|e| e.to_string())?
    };
    cleanup_owned_background_resources(&session_id, &chat_state).await;
    finalize_manual_clear_context_state(&main_store, &workflow_manager, &gateway, &session_id)
        .await?;
    Ok(WorkflowContextFrameResult {
        noop: false,
        segment_id,
        marker_message,
        current_context_tokens: current_context
            .as_ref()
            .and_then(|ctx| ctx.current_context_tokens)
            .unwrap_or(0),
        max_context_tokens: current_context
            .as_ref()
            .and_then(|ctx| ctx.max_context_tokens)
            .unwrap_or(4096),
        state: WorkflowState::Pending,
        wait_reason: None,
        has_live_session: false,
    })
}

#[tauri::command]
pub async fn get_workflow_snapshot(
    state: State<'_, Arc<MainStore>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
) -> Result<Value, String> {
    let main_store = state.inner().clone();
    let recovery_store = main_store.clone();
    let recovery_session_id = session_id.clone();
    tokio::task::spawn_blocking(move || {
        reconcile_child_workflows_for_parent(recovery_store.as_ref(), &recovery_session_id)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("Failed to join workflow child recovery: {e}"))??;

    let snapshot_store = main_store.clone();
    let snapshot_session_id = session_id.clone();
    let (
        mut snapshot,
        message_window_before_id,
        hidden_earlier_message_count,
        hidden_completed_task_count,
        has_more_in_current_task,
    ) = tokio::task::spawn_blocking(move || -> Result<_, String> {
        let store = &*snapshot_store;
        let mut workflow = store
            .get_workflow_for_ui(&snapshot_session_id)
            .map_err(|e| e.to_string())?;
        let message_page = store
            .get_recent_workflow_message_page(&snapshot_session_id, UI_WORKFLOW_MESSAGE_PAGE_SIZE)
            .map_err(|e| e.to_string())?;
        normalize_workflow_agent_config_in_memory(&store, &mut workflow)?;
        Ok((
            WorkflowSnapshot {
                workflow,
                messages: message_page.messages,
            },
            message_page.before_message_id,
            message_page.hidden_message_count,
            message_page.hidden_completed_task_count,
            message_page.has_more_in_current_task,
        ))
    })
    .await
    .map_err(|e| format!("Failed to join workflow snapshot query: {e}"))??;

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
    let reconciliation_store = main_store.clone();
    let reconciliation_session_id = session_id.clone();
    let (snapshot, execution_context, tail_rewind_kind) =
        tokio::task::spawn_blocking(move || -> Result<_, String> {
            {
                let store = &*reconciliation_store;
                normalize_snapshot_after_live_reconciliation(
                    &store,
                    &reconciliation_session_id,
                    &mut snapshot,
                    has_live_session,
                )?;
            }
            let execution_context = restore_context_for_signal(
                reconciliation_store.clone(),
                &reconciliation_session_id,
            );
            let tail_rewind_kind = {
                let store = &*reconciliation_store;
                store
                    .get_tail_rewind_kind(&reconciliation_session_id)
                    .map_err(|e| e.to_string())?
            };
            Ok((snapshot, execution_context, tail_rewind_kind))
        })
        .await
        .map_err(|e| format!("Failed to join workflow snapshot reconciliation: {e}"))??;
    let merged_messages = merge_ui_workflow_messages(&snapshot.messages);
    let can_rewind_tail = !has_blocking_live_session && tail_rewind_kind.is_some();

    // Convert snapshot to JSON and inject hasLiveSession
    let mut snapshot_json = serde_json::to_value(&snapshot).map_err(|e| e.to_string())?;
    if let Some(obj) = snapshot_json.as_object_mut() {
        obj.insert("messages".to_string(), json!(merged_messages));
        obj.insert(
            "messageWindowBeforeId".to_string(),
            json!(serialize_workflow_message_page_cursor(
                message_window_before_id
            )),
        );
        obj.insert(
            "hiddenEarlierMessageCount".to_string(),
            json!(hidden_earlier_message_count),
        );
        obj.insert(
            "hiddenCompletedTaskCount".to_string(),
            json!(hidden_completed_task_count),
        );
        obj.insert(
            "hasMoreInCurrentTask".to_string(),
            json!(has_more_in_current_task),
        );
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
pub async fn get_earlier_workflow_message_page(
    state: State<'_, Arc<MainStore>>,
    session_id: String,
    before_message_id: String,
) -> Result<Value, String> {
    let before_message_id = parse_workflow_message_page_cursor(&before_message_id)?;
    let page = {
        let store = &*state;
        store
            .get_earlier_workflow_message_page(
                &session_id,
                before_message_id,
                UI_WORKFLOW_MESSAGE_PAGE_SIZE,
            )
            .map_err(|e| e.to_string())?
    };
    let merged_messages = merge_ui_workflow_messages(&page.messages);

    log::debug!(
        "[Workflow][session={}][command=get_earlier_workflow_message_page] loaded_messages={} hidden_messages={}",
        session_id,
        merged_messages.len(),
        page.hidden_message_count
    );

    Ok(json!({
        "messages": merged_messages,
        "beforeMessageId": serialize_workflow_message_page_cursor(page.before_message_id),
        "hiddenEarlierMessageCount": page.hidden_message_count,
        "hasMoreInCurrentTask": page.has_more_in_current_task,
    }))
}

#[tauri::command]
pub async fn get_earlier_workflow_messages(
    state: State<'_, Arc<MainStore>>,
    session_id: String,
    before_message_id: String,
) -> Result<Value, String> {
    let before_message_id = parse_workflow_message_page_cursor(&before_message_id)?;
    let window = {
        let store = &*state;
        store
            .get_workflow_message_window(&session_id, Some(before_message_id), 1)
            .map_err(|e| e.to_string())?
    };
    let merged_messages = merge_ui_workflow_messages(&window.messages);

    log::debug!(
        "[Workflow][session={}][command=get_earlier_workflow_messages] loaded_messages={} hidden_completed_tasks={}",
        session_id,
        merged_messages.len(),
        window.hidden_completed_task_count
    );

    Ok(json!({
        "messages": merged_messages,
        "beforeMessageId": serialize_workflow_message_page_cursor(window.before_message_id),
        "hiddenCompletedTaskCount": window.hidden_completed_task_count,
    }))
}

#[tauri::command]
pub async fn get_workflow_agent_config(
    state: State<'_, Arc<MainStore>>,
    session_id: String,
) -> Result<Value, String> {
    let store = &*state;
    let config_json = raw_workflow_agent_config_json(&store, &session_id)?;
    serde_json::from_str::<Value>(&config_json).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn add_workflow_message(
    state: State<'_, Arc<MainStore>>,
    chat_state: State<'_, Arc<ChatState>>,
    gateway: State<'_, Arc<TauriGateway>>,
    message: WorkflowMessage,
) -> Result<i64, String> {
    let runtime = {
        let store = &*state;
        store.db_runtime().map_err(|e| e.to_string())?
    };
    let res = MainStore::add_workflow_message_with_runtime(runtime, message.clone())
        .await
        .map_err(|e| e.to_string())?;

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
    state: State<'_, Arc<MainStore>>,
    session_id: String,
    title: String,
) -> Result<(), String> {
    let runtime = {
        let store = &*state;
        store.db_runtime().map_err(|e| e.to_string())?
    };
    MainStore::update_workflow_title_with_runtime(runtime, session_id, title)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_workflow_title_and_query(
    state: State<'_, Arc<MainStore>>,
    session_id: String,
    title: String,
    user_query: String,
) -> Result<(), String> {
    let runtime = {
        let store = &*state;
        store.db_runtime().map_err(|e| e.to_string())?
    };
    MainStore::update_workflow_title_and_query_with_runtime(runtime, session_id, title, user_query)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_workflow_query(
    state: State<'_, Arc<MainStore>>,
    session_id: String,
    user_query: String,
) -> Result<(), String> {
    let runtime = {
        let store = &*state;
        store.db_runtime().map_err(|e| e.to_string())?
    };
    MainStore::update_workflow_query_with_runtime(runtime, session_id, user_query)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_workflow_status(
    state: State<'_, Arc<MainStore>>,
    session_id: String,
    status: String,
) -> Result<(), String> {
    let runtime = {
        let store = &*state;
        store.db_runtime().map_err(|e| e.to_string())?
    };
    MainStore::update_workflow_status_with_runtime(runtime, session_id, status)
        .await
        .map_err(|e| e.to_string())
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
    begin_new_segment: bool,
) -> Result<(), crate::workflow::react::error::WorkflowEngineError> {
    if begin_new_segment {
        executor.begin_new_context_segment().await?;
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
            prompt_with_new_segment_completion_scope(clean_prompt)
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
    gateway: &Arc<TauriGateway>,
    workflow_manager: &Arc<WorkflowManager>,
    main_store: &Arc<MainStore>,
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
                let _ = persist_cancelled_workflow_state(
                    main_store_for_spawn.as_ref(),
                    &session_id_for_spawn,
                );
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
            if let Err(error) =
                persist_failed_workflow_state(main_store_for_spawn.as_ref(), &session_id_for_spawn)
            {
                log::error!(
                    "[Workflow][session={}][phase=run_loop][event=failed_state_persist] Could not persist failed workflow state after completed-session resume: {}",
                    session_id_for_spawn,
                    error
                );
            }
            let _ = manager_for_spawn
                .update_session_status(&session_id_for_spawn, ManagedSessionStatus::Failed);
            let _ = gateway_for_spawn
                .send(
                    &session_id_for_spawn,
                    crate::workflow::react::types::GatewayPayload::State {
                        state: WorkflowState::Error,
                        wait_reason: None,
                    },
                )
                .await;
            let _ = gateway_for_spawn
                .send(
                    &session_id_for_spawn,
                    crate::workflow::react::types::GatewayPayload::Message {
                        message_id: None,
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

    clear_completed_background_tasks_for_owner(root_session_id);
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

fn should_run_terminal_manual_compression(
    signal: Option<&WorkflowSignal>,
    snapshot_status: &str,
) -> bool {
    matches!(signal, Some(WorkflowSignal::ManualCompress))
        && compat_is_terminal_snapshot_status(snapshot_status)
}

fn should_bypass_manager_for_terminal_manual_compression(
    signal: Option<&WorkflowSignal>,
    snapshot_status: &str,
    managed_status: Option<&ManagedSessionStatus>,
) -> bool {
    should_run_terminal_manual_compression(signal, snapshot_status)
        && !matches!(
            managed_status,
            Some(
                ManagedSessionStatus::Active
                    | ManagedSessionStatus::Waiting
                    | ManagedSessionStatus::Stopping
            )
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

fn compat_is_awaiting_user_snapshot_status(status: &str) -> bool {
    matches!(
        compat_snapshot_workflow_state(status),
        Some(WorkflowState::AwaitingUser)
    )
}

fn restore_context_for_signal(store: Arc<MainStore>, session_id: &str) -> Option<ExecutionContext> {
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
        || compat_is_awaiting_user_snapshot_status(snapshot_status)
        || (ctx.is_none() && compat_is_resumable_snapshot_status_for_user_message(snapshot_status))
}

fn should_reinject_user_message_after_recovery(effective_wait_reason: Option<&WaitReason>) -> bool {
    matches!(
        effective_wait_reason,
        Some(WaitReason::UserInput | WaitReason::Approval)
    )
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
    state: State<'_, Arc<MainStore>>,
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

    if initial_prompt.is_some() {
        let store = &*main_store_arc;
        let workflow = store
            .get_workflow(&session_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Workflow {} not found", session_id))?;
        if workflow.status.parse::<WorkflowState>().ok() == Some(WorkflowState::Completed) {
            sync_workflow_agent_config_at_tool_boundary(&store, &session_id)?;
        }
    }

    let (
        raw_prompt,
        clean_prompt,
        attached_context,
        initial_message_metadata,
        allowed_paths,
        workflow_status,
    ) = {
        let store = &*main_store_arc;
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
        )
    };

    if initial_prompt.is_some() {
        let _ = spawn_workflow_title_generation_if_missing(
            session_id.clone(),
            raw_prompt.clone(),
            main_store_arc.clone(),
            chat_state_arc.clone(),
            gateway_arc.clone(),
        );
    }

    let mut agent_config = {
        let store = &*main_store_arc;
        store
            .get_agent(&agent_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Agent {} not found", agent_id))?
    };
    // Load agent_config JSON for easier access to overrides like 'phase'
    let agent_config_json: Value = {
        let store = &*main_store_arc;
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

    if initial_prompt.is_some() {
        if try_resume_completed_live_session(
            &session_id,
            &raw_prompt,
            &clean_prompt,
            &attached_context,
            initial_message_metadata.clone(),
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
                let _ = persist_cancelled_workflow_state(
                    main_store_for_spawn.as_ref(),
                    &session_id_for_spawn,
                );
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
            if let Err(error) =
                persist_failed_workflow_state(main_store_for_spawn.as_ref(), &session_id_for_spawn)
            {
                log::error!(
                    "[Workflow][session={}][phase=run_loop][event=failed_state_persist] Could not persist failed workflow state: {}",
                    session_id_for_spawn,
                    error
                );
            }
            let _ = manager_for_spawn
                .update_session_status(&session_id_for_spawn, ManagedSessionStatus::Failed);

            let _ = gateway_for_spawn
                .send(
                    &session_id_for_spawn,
                    crate::workflow::react::types::GatewayPayload::State {
                        state: WorkflowState::Error,
                        wait_reason: None,
                    },
                )
                .await;
            let _ = gateway_for_spawn
                .send(
                    &session_id_for_spawn,
                    crate::workflow::react::types::GatewayPayload::Message {
                        message_id: None,
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

async fn run_terminal_manual_compression(
    app: &AppHandle,
    main_store: Arc<MainStore>,
    chat_state: Arc<ChatState>,
    tsid_generator: Arc<TsidGenerator>,
    gateway: Arc<TauriGateway>,
    factory: Arc<dyn SubAgentFactory>,
    session_id: &str,
    workflow_snapshot: &WorkflowSnapshot,
) -> Result<bool, String> {
    let mut agent_config = main_store
        .get_agent(&workflow_snapshot.workflow.agent_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Agent {} not found", workflow_snapshot.workflow.agent_id))?;
    let agent_config_value = workflow_snapshot
        .workflow
        .agent_config
        .as_ref()
        .and_then(|config| serde_json::from_str::<Value>(config).ok())
        .unwrap_or_else(|| json!({}));
    if let Some(config) = workflow_snapshot.workflow.agent_config.as_deref() {
        agent_config.merge_config(config);
    }
    let allowed_paths = workflow_snapshot
        .workflow
        .agent_config
        .as_deref()
        .and_then(AgentConfig::from_json)
        .and_then(|config| config.allowed_paths)
        .unwrap_or_default()
        .into_iter()
        .map(|path| {
            let path = PathBuf::from(path);
            if path.is_absolute() {
                path
            } else {
                std::env::current_dir().unwrap_or_default().join(path)
            }
        })
        .collect::<Vec<_>>();
    let policy = match agent_config_value
        .get("phase")
        .and_then(|value| value.as_str())
        .and_then(|phase| {
            use std::str::FromStr;
            crate::workflow::react::policy::ExecutionPhase::from_str(phase).ok()
        }) {
        Some(crate::workflow::react::policy::ExecutionPhase::Planning) => {
            crate::workflow::react::policy::ExecutionPolicy::planning_strict()
        }
        Some(crate::workflow::react::policy::ExecutionPhase::Implementation) => {
            crate::workflow::react::policy::ExecutionPolicy::implementation()
        }
        Some(crate::workflow::react::policy::ExecutionPhase::Standard) | None => {
            crate::workflow::react::policy::ExecutionPolicy::standard()
        }
    };
    let auto_compress_enabled = workflow_auto_compress_enabled(&agent_config_value);
    let global_tool_manager = chat_state.tool_manager.clone();
    let mut executor = WorkflowExecutor::new(
        session_id.to_string(),
        main_store,
        chat_state,
        gateway as Arc<dyn Gateway>,
        factory,
        agent_config,
        allowed_paths,
        app.path().app_data_dir().unwrap_or_default(),
        None,
        None,
        tsid_generator,
        global_tool_manager,
        auto_compress_enabled,
        policy,
    );

    executor
        .run_terminal_manual_compression()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn workflow_approve_plan(
    app: AppHandle,
    main_store: State<'_, Arc<MainStore>>,
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
    state: State<'_, Arc<MainStore>>,
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
        let store = &*main_store_arc;
        store
            .get_workflow_snapshot(&session_id)
            .map_err(|e| e.to_string())?
    };
    let signal = inject_at_mentions_into_signal(
        &signal,
        &allowed_paths_from_workflow_snapshot(&workflow_snapshot),
    );
    let recovery_context = restore_context_for_signal(main_store_arc.clone(), &session_id);
    let workflow_signal = WorkflowSignal::parse(&signal);

    // Phase 1: reconcile manager state before choosing the manual-compression path.
    let has_live_session =
        has_reconciled_live_session(&workflow_manager_arc, &session_id, "signal").await;
    let reconciled_managed_status = workflow_manager_arc.get_session_status(&session_id);

    // A stopped workflow is compressed through the terminal-only path before manager validation.
    // A still-live executor continues through the existing signal path to avoid concurrent compression.
    if should_bypass_manager_for_terminal_manual_compression(
        workflow_signal.as_ref(),
        &workflow_snapshot.workflow.status,
        reconciled_managed_status.as_ref(),
    ) {
        log::info!(
            "[Workflow][session={}][phase=signal] Routing manual compression through the terminal-only path before manager validation",
            session_id
        );
        let applied = run_terminal_manual_compression(
            &app,
            main_store_arc.clone(),
            chat_state.inner().clone(),
            tsid_generator.inner().clone(),
            gateway_arc.clone(),
            factory.inner().clone(),
            &session_id,
            &workflow_snapshot,
        )
        .await?;
        return Ok(if applied {
            "Terminal workflow compressed without resuming execution".to_string()
        } else {
            "Terminal workflow has no new safe segment to compress".to_string()
        });
    }

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
                let should_reinject_after_recovery =
                    should_reinject_user_message_after_recovery(effective_wait_reason.as_ref())
                        || (recovery_context.is_none()
                            && matches!(
                                compat_snapshot_workflow_state(&workflow_snapshot.workflow.status),
                                Some(
                                    WorkflowState::AwaitingUser
                                        | WorkflowState::AwaitingApproval
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
                    if should_reinject_after_recovery {
                        None
                    } else {
                        val["content"].as_str().map(|content| content.to_string())
                    },
                    signal_json_metadata(&signal),
                    signal_json_attached_context(&signal),
                    None,
                )
                .await?;

                if should_reinject_after_recovery {
                    let mut retries = 5;
                    let mut last_error = None;
                    while retries > 0 {
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                        match gateway_arc.inject_input(&session_id, signal.clone()).await {
                            Ok(_) => {
                                log::info!(
                                    "[Workflow] user_message injected successfully after recovery wait"
                                );
                                break;
                            }
                            Err(e) => {
                                last_error = Some(e);
                                retries -= 1;
                                log::info!(
                                    "[Workflow] Failed to inject user_message during recovery wait, retries left: {}",
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

                    return Ok("Workflow resumed and user message reinjected".to_string());
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
                workflow_signal.as_ref(),
                Some(
                    WorkflowSignal::Continue
                        | WorkflowSignal::Stop
                        | WorkflowSignal::ManualCompress
                )
            ) {
                let is_manual_compress = matches!(
                    workflow_signal.as_ref(),
                    Some(WorkflowSignal::ManualCompress)
                );
                let can_resume = is_manual_compress
                    || effective_wait_reason == Some(WaitReason::Confirmation)
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
                    "[Workflow] Session {} is resuming to process {} signal",
                    session_id,
                    val["type"]
                );

                if is_manual_compress {
                    stash_runtime_signal(
                        &session_id,
                        serde_json::json!({
                            "type": "manual_compress",
                            "resume_only": true,
                        })
                        .to_string(),
                    );
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

                if is_manual_compress {
                    return Ok("Workflow resumed and manual compression triggered".to_string());
                }

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
    state: State<'_, Arc<MainStore>>,
    chat_state: State<'_, Arc<ChatState>>,
    gateway: State<'_, Arc<TauriGateway>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
) -> Result<(), String> {
    let previous_status = {
        let store = &**state.inner();
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
                let store = &**state.inner();
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
            let _ = persist_cancelled_workflow_state(state.inner().as_ref(), &session_id);
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
            let _ = state
                .inner()
                .update_workflow_status(&session_id, &previous_status);
            Err(e.to_string())
        }
    }
}

#[tauri::command]
pub async fn workflow_get_tasks(
    state: State<'_, Arc<MainStore>>,
    session_id: String,
) -> Result<Vec<Value>, String> {
    let store = &*state;
    store
        .get_todo_list_for_workflow(&session_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn update_workflow_todo_list(
    state: State<'_, Arc<MainStore>>,
    session_id: String,
    todo_list: String,
) -> Result<(), String> {
    let runtime = {
        let store = &*state;
        store.db_runtime().map_err(|e| e.to_string())?
    };
    MainStore::update_workflow_todo_list_with_runtime(runtime, session_id, todo_list)
        .await
        .map_err(|e| e.to_string())
}

#[derive(Debug, serde::Serialize)]
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

        let walker = workspace_walk_builder(&base).build();

        for result in walker {
            let entry = match result {
                Ok(e) => e,
                Err(_) => continue,
            };

            let path = entry.path().to_path_buf();
            let name = entry.file_name().to_string_lossy().to_string();

            let relative_path = path.strip_prefix(&base).unwrap_or(&path);
            let rel_path = relative_path.to_string_lossy().to_string();
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
    state: State<'_, Arc<MainStore>>,
    gateway: State<'_, Arc<TauriGateway>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
    allowed_paths: Value,
) -> Result<(), String> {
    let runtime_paths: Vec<String> =
        serde_json::from_value(allowed_paths.clone()).map_err(|e| e.to_string())?;
    let previous_config_json = {
        let store = &*state;
        raw_workflow_agent_config_json(&store, &session_id)?
    };

    {
        let store = &*state;
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
    state: State<'_, Arc<MainStore>>,
    gateway: State<'_, Arc<TauriGateway>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
    final_audit: bool,
) -> Result<(), String> {
    let previous_config_json = {
        let store = &*state;
        raw_workflow_agent_config_json(&store, &session_id)?
    };
    {
        let store = &*state;
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
    state: State<'_, Arc<MainStore>>,
    gateway: State<'_, Arc<TauriGateway>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
    auto_compress: bool,
) -> Result<(), String> {
    let previous_config_json = {
        let store = &*state;
        raw_workflow_agent_config_json(&store, &session_id)?
    };
    {
        let store = &*state;
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
pub async fn update_workflow_personality(
    state: State<'_, Arc<MainStore>>,
    gateway: State<'_, Arc<TauriGateway>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
    personality: String,
) -> Result<(), String> {
    let previous_config_json = {
        let store = &*state;
        raw_workflow_agent_config_json(&store, &session_id)?
    };
    let personality = personality.trim().to_string();

    {
        let store = &*state;
        let workflow = store
            .get_workflow(&session_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Workflow {session_id} not found"))?;
        let agent = store
            .get_agent(&workflow.agent_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Agent {} not found", workflow.agent_id))?;

        let is_custom_personality =
            !personality.is_empty() && !personality.starts_with(AGENT_PERSONALITY_PRESET_PREFIX);
        if personality.starts_with(AGENT_PERSONALITY_PRESET_PREFIX)
            && !is_agent_personality_preset(&personality)
        {
            return Err("Unknown execution style preset".to_string());
        }
        if is_custom_personality
            && agent.personality.as_deref().map(str::trim) != Some(&personality)
        {
            return Err(
                "The selected custom execution style is no longer available for this Agent"
                    .to_string(),
            );
        }

        let mut config = raw_workflow_agent_config(&store, &session_id)?;
        config.personality = (!personality.is_empty()).then_some(personality.clone());
        store
            .update_workflow_agent_config(&session_id, &config.to_json())
            .map_err(|error| error.to_string())?;
    }

    inject_runtime_config_signal(
        gateway.inner(),
        workflow_manager.inner(),
        state.inner(),
        &session_id,
        &previous_config_json,
        serde_json::json!({
            "type": "update_personality",
            "personality": (!personality.is_empty()).then_some(personality)
        }),
    )
    .await
}

#[tauri::command]
pub async fn update_workflow_model_config(
    state: State<'_, Arc<MainStore>>,
    gateway: State<'_, Arc<TauriGateway>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
    configs: Value,
) -> Result<(), String> {
    let previous_config_json = {
        let store = &*state;
        raw_workflow_agent_config_json(&store, &session_id)?
    };
    {
        let store = &*state;
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
    state: State<'_, Arc<MainStore>>,
    gateway: State<'_, Arc<TauriGateway>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
    skill_enabled: bool,
    selected_skills: Vec<String>,
) -> Result<(), String> {
    let previous_config_json = {
        let store = &*state;
        raw_workflow_agent_config_json(&store, &session_id)?
    };
    {
        let store = &*state;
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
    state: State<'_, Arc<MainStore>>,
    gateway: State<'_, Arc<TauriGateway>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
    approval_level: String,
) -> Result<(), String> {
    let previous_config_json = {
        let store = &*state;
        raw_workflow_agent_config_json(&store, &session_id)?
    };
    {
        let store = &*state;
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
    state: State<'_, Arc<MainStore>>,
    gateway: State<'_, Arc<TauriGateway>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
    phase: String,
) -> Result<(), String> {
    let previous_config_json = {
        let store = &*state;
        raw_workflow_agent_config_json(&store, &session_id)?
    };
    {
        let store = &*state;
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
pub async fn update_workflow_sandbox_config(
    state: State<'_, Arc<MainStore>>,
    gateway: State<'_, Arc<TauriGateway>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
    execution_mode: crate::tools::ShellExecutionMode,
    sandbox_scheme_id: Option<String>,
) -> Result<(), String> {
    let previous_config_json = {
        let store = &*state;
        raw_workflow_agent_config_json(&store, &session_id)?
    };

    let sandbox_config = {
        let store = &*state;
        let workflow = store
            .get_workflow(&session_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Workflow {session_id} not found"))?;
        let agent = store
            .get_agent(&workflow.agent_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Agent {} not found", workflow.agent_id))?;
        let mut config = raw_workflow_agent_config(&store, &session_id)?;
        config.sandbox_override = Some(true);
        config.sandbox_execution_mode = Some(execution_mode.clone());
        config.sandbox_scheme_id = sandbox_scheme_id;
        resolve_agent_sandbox_snapshot(&store, &agent, &mut config)?;
        let sandbox_config = config.sandbox_config.clone();
        store
            .update_workflow_agent_config(&session_id, &config.to_json())
            .map_err(|error| error.to_string())?;
        sandbox_config
    };

    inject_runtime_config_signal(
        gateway.inner(),
        workflow_manager.inner(),
        state.inner(),
        &session_id,
        &previous_config_json,
        serde_json::json!({
            "type": "update_sandbox_config",
            "execution_mode": execution_mode,
            "sandbox_scheme_id": sandbox_config
                .as_ref()
                .and_then(|config| config.scheme_id.clone()),
            "sandbox_config": sandbox_config
        }),
    )
    .await
}

#[tauri::command]
pub async fn update_workflow_agent_config(
    state: State<'_, Arc<MainStore>>,
    gateway: State<'_, Arc<TauriGateway>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
    agent_config: String,
) -> Result<(), String> {
    let previous_config_json = {
        let store = &*state;
        raw_workflow_agent_config_json(&store, &session_id)?
    };

    let mut normalized_config = AgentConfig::from_json(&agent_config)
        .ok_or_else(|| "Invalid agent config JSON".to_string())?;
    {
        let store = &*state;
        let workflow = store
            .get_workflow(&session_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Workflow {session_id} not found"))?;
        let agent = store
            .get_agent(&workflow.agent_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("Agent {} not found", workflow.agent_id))?;

        if let (Some(selected_tools), Some(agent_tools_json)) = (
            normalized_config.available_tools.as_mut(),
            agent.available_tools.as_deref(),
        ) {
            let agent_tools = serde_json::from_str::<Vec<String>>(agent_tools_json)
                .map_err(|error| format!("Invalid Agent available tools: {error}"))?;
            selected_tools.retain(|tool| agent_tools.contains(tool));
        }
    }
    enforce_auto_approve_tool_visibility(&mut normalized_config);
    let normalized_config_json = normalized_config.to_json();
    let signal_agent_config =
        serde_json::to_value(&normalized_config).map_err(|e| e.to_string())?;

    {
        let store = &*state;
        store
            .update_workflow_agent_config(&session_id, &normalized_config_json)
            .map_err(|e| e.to_string())?;
    }

    if let Some(available_tools) = signal_agent_config.get("availableTools").cloned() {
        inject_runtime_config_signal(
            gateway.inner(),
            workflow_manager.inner(),
            state.inner(),
            &session_id,
            &previous_config_json,
            serde_json::json!({
                "type": "update_available_tools",
                "available_tools": available_tools
            }),
        )
        .await?;
    }

    if let Some(auto_approve) = signal_agent_config.get("autoApprove").cloned() {
        inject_runtime_config_signal(
            gateway.inner(),
            workflow_manager.inner(),
            state.inner(),
            &session_id,
            &previous_config_json,
            serde_json::json!({
                "type": "update_auto_approved_tools",
                "auto_approve": auto_approve
            }),
        )
        .await?;
    }

    Ok(())
}

#[tauri::command]
pub async fn update_workflow_agent_id(
    state: State<'_, Arc<MainStore>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
    agent_id: String,
) -> Result<String, String> {
    let store = &*state;

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
    state: State<'_, Arc<MainStore>>,
    session_id: String,
) -> Result<Vec<String>, String> {
    let store = &*state;
    let mut config = store
        .get_workflow(&session_id)
        .map_err(|e| e.to_string())?
        .and_then(|workflow| workflow.agent_config)
        .and_then(|s| AgentConfig::from_json(&s))
        .unwrap_or_default();
    enforce_auto_approve_tool_visibility(&mut config);

    Ok(config.auto_approve.unwrap_or_default())
}

#[tauri::command]
pub async fn remove_auto_approved_tool(
    state: State<'_, Arc<MainStore>>,
    gateway: State<'_, Arc<TauriGateway>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
    tool_name: String,
) -> Result<(), String> {
    let previous_config_json = {
        let store = &*state;
        raw_workflow_agent_config_json(&store, &session_id)?
    };

    // Update database first
    {
        let store = &*state;

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
    state: State<'_, Arc<MainStore>>,
    gateway: State<'_, Arc<TauriGateway>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    session_id: String,
    pattern: String,
) -> Result<(), String> {
    let previous_config_json = {
        let store = &*state;
        raw_workflow_agent_config_json(&store, &session_id)?
    };

    // Update database first
    {
        let store = &*state;

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
    state: State<'_, Arc<MainStore>>,
    session_id: String,
) -> Result<Vec<crate::workflow::react::events::WorkflowEventRecord>, String> {
    let store = &*state;
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
    state: State<'_, Arc<MainStore>>,
    session_id: String,
) -> Result<WorkflowEfficiencyReport, String> {
    let store = &*state;
    store
        .get_workflow_efficiency_report(&session_id)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    struct TestStore {
        _temp_dir: tempfile::TempDir,
        store: MainStore,
    }

    impl std::ops::Deref for TestStore {
        type Target = MainStore;

        fn deref(&self) -> &Self::Target {
            &self.store
        }
    }

    #[test]
    fn workflow_auto_compression_defaults_to_disabled() {
        assert!(!workflow_auto_compress_enabled(&json!({})));
        assert!(!workflow_auto_compress_enabled(
            &json!({ "autoCompress": null })
        ));
        assert!(!workflow_auto_compress_enabled(
            &json!({ "auto_compress": null })
        ));
        assert!(workflow_auto_compress_enabled(
            &json!({ "autoCompress": true })
        ));
        assert!(workflow_auto_compress_enabled(
            &json!({ "auto_compress": true })
        ));
    }

    #[test]
    fn completed_session_runtime_config_signals_are_deferred() {
        for signal_type in [
            "update_available_tools",
            "update_auto_approved_tools",
            "update_final_audit",
            "update_auto_compress",
            "update_approval_level",
            "update_phase",
        ] {
            assert!(
                can_defer_runtime_config_signal_for_completed_session(signal_type),
                "completed sessions should defer {signal_type}"
            );
        }
    }

    fn create_test_store() -> TestStore {
        let dir = tempdir().expect("failed to create temp dir");
        let db_path = dir.path().join("workflow_commands_test.db");
        let store = MainStore::new(db_path).expect("failed to create MainStore");
        TestStore {
            _temp_dir: dir,
            store,
        }
    }

    #[tokio::test]
    async fn search_workspace_files_allows_csignore_whitelisted_gitignored_directory() {
        let root = tempdir().expect("failed to create temp dir");
        let root_path = root
            .path()
            .canonicalize()
            .expect("failed to canonicalize root");
        let dev_data = root_path.join("dev_data");
        std::fs::create_dir(root_path.join(".git")).expect("failed to mark git root");
        std::fs::create_dir(&dev_data).expect("failed to create dev_data");
        std::fs::write(root_path.join(".gitignore"), "dev_data/\n")
            .expect("failed to write .gitignore");
        std::fs::write(
            root_path.join(CHATSPEED_IGNORE_FILE),
            "!dev_data/\n!dev_data/**\n",
        )
        .expect("failed to write .csignore");
        std::fs::write(dev_data.join("fixture.txt"), "fixture").expect("failed to write fixture");

        let results = search_workspace_files(
            vec![root_path.to_string_lossy().to_string()],
            "dev_data".to_string(),
        )
        .await
        .expect("search should succeed");

        assert!(
            results
                .iter()
                .any(|file| file.relative_path == "dev_data" && file.is_directory),
            "dev_data directory should be available in @ file suggestions: {results:?}"
        );
        assert!(
            results
                .iter()
                .any(|file| file.relative_path == "dev_data/fixture.txt"),
            "files inside dev_data should be available in @ file suggestions: {results:?}"
        );
    }

    #[tokio::test]
    async fn search_workspace_files_respects_nested_gitignore_when_csignore_is_unspecified() {
        let root = tempdir().expect("failed to create temp dir");
        let root_path = root
            .path()
            .canonicalize()
            .expect("failed to canonicalize root");
        let nested = root_path.join("src-tauri");
        let target = nested.join("target");
        std::fs::create_dir(root_path.join(".git")).expect("failed to mark git root");
        std::fs::create_dir_all(&target).expect("failed to create nested target");
        std::fs::write(
            root_path.join(CHATSPEED_IGNORE_FILE),
            "!dev_data/\n!dev_data/**\n",
        )
        .expect("failed to write .csignore");
        std::fs::write(nested.join(".gitignore"), "/target/\n")
            .expect("failed to write nested .gitignore");
        std::fs::write(target.join("artifact.txt"), "artifact")
            .expect("failed to write ignored artifact");

        let results = search_workspace_files(
            vec![root_path.to_string_lossy().to_string()],
            "target".to_string(),
        )
        .await
        .expect("search should succeed");

        assert!(
            results
                .iter()
                .all(|file| !file.relative_path.starts_with("src-tauri/target")),
            "paths unspecified by .csignore must fall back to nested .gitignore rules: {results:?}"
        );
    }

    fn seed_agent(store: &MainStore, agent_id: &str) {
        let agent_id = agent_id.to_string();
        store
            .db_runtime()
            .expect("failed to obtain database runtime")
            .write_blocking(move |conn| {
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
                )?;
                Ok(())
            })
            .expect("failed to seed agent");
    }

    #[test]
    fn workflow_message_page_cursor_round_trips_large_i64_ids_as_strings() {
        let message_id = 873149882892816384_i64;
        let cursor = serialize_workflow_message_page_cursor(Some(message_id));

        assert_eq!(cursor.as_deref(), Some("873149882892816384"));
        assert_eq!(
            parse_workflow_message_page_cursor(cursor.as_deref().expect("cursor should exist")),
            Ok(message_id)
        );
        assert!(parse_workflow_message_page_cursor("873149882892816400").is_ok());
        assert!(parse_workflow_message_page_cursor("not-a-cursor").is_err());
    }

    #[test]
    fn new_segment_reminder_defines_completion_scope_for_independent_and_continuous_tasks() {
        let prompt = prompt_with_new_segment_completion_scope("Correct the previous version tag");

        assert!(prompt.starts_with("Correct the previous version tag"));
        assert!(prompt.contains("A segment boundary is not necessarily an objective boundary"));
        assert!(prompt.contains("If the request is independent"));
        assert!(prompt.contains("summarize only the work and verification for this segment's task"));
        assert!(prompt.contains("explicitly continues, corrects, refines, or extends"));
        assert!(prompt.contains("summarize the combined outcome"));
        assert!(prompt.contains("Do not include unrelated completed tasks"));
        assert!(prompt.contains("requires a new current completion report"));
        assert!(prompt.contains("never reuse an earlier task's completion report or pending draft"));
    }

    #[test]
    fn reconcile_interrupted_child_persists_usage_under_parent_current_task_run() {
        let store = create_test_store();
        seed_agent(&store, "agent-test");
        store
            .create_workflow("parent-usage", "Parent task", "agent-test", None, None)
            .expect("failed to create parent workflow");
        store
            .create_workflow(
                "child-usage",
                "Child task",
                "agent-test",
                None,
                Some("parent-usage"),
            )
            .expect("failed to create child workflow");

        reconcile_interrupted_child_workflows(&store).expect("failed to reconcile child workflows");

        let summary = store
            .load_workflow_task_usage("child-usage", "child-usage:task:1")
            .expect("failed to load child usage summary")
            .expect("reconciled child usage summary should be durable");
        assert_eq!(summary.terminal_status, "interrupted");
        assert!(
            summary.is_partial,
            "restart reconciliation without attributed stats must remain partial"
        );

        let root_task_run_id = store
            .workflow_current_task_run_id("parent-usage")
            .expect("failed to resolve parent task run");
        store
            .db_runtime()
            .expect("failed to obtain database runtime")
            .read_blocking(move |conn| {
                let root: (String, String) = conn.query_row(
                    "SELECT root_session_id, root_task_run_id FROM workflow_task_usage
                     WHERE session_id = 'child-usage' AND task_run_id = 'child-usage:task:1'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
                assert_eq!(root, ("parent-usage".to_string(), root_task_run_id));
                Ok(())
            })
            .expect("failed to verify child usage root attribution");
    }

    #[test]
    fn reconcile_child_workflows_for_parent_ignores_other_parent_history() {
        let store = create_test_store();
        seed_agent(&store, "agent-test");
        for parent_id in ["current-parent", "history-parent"] {
            store
                .create_workflow(parent_id, "Parent task", "agent-test", None, None)
                .expect("failed to create parent workflow");
        }
        for (child_id, parent_id) in [
            ("current-child", "current-parent"),
            ("history-child", "history-parent"),
        ] {
            store
                .create_workflow(child_id, "Child task", "agent-test", None, Some(parent_id))
                .expect("failed to create child workflow");
            let mut context = ExecutionContext::new(child_id.to_string());
            context.state = RuntimeState::Completed;
            store
                .upsert_execution_context(&context)
                .expect("failed to persist completed child state");
        }

        reconcile_child_workflows_for_parent(&store, "current-parent")
            .expect("failed to recover current parent children");

        let current_child = store
            .get_workflow("current-child")
            .expect("failed to load current child")
            .expect("current child should exist");
        assert_eq!(current_child.status, "completed");
        let history_child = store
            .get_workflow("history-child")
            .expect("failed to load historical child")
            .expect("historical child should exist");
        assert_eq!(history_child.status, "pending");
        assert!(store
            .get_execution_context("current-parent")
            .expect("failed to load current parent context")
            .expect("current parent completion projection should be durable")
            .pending_sub_agent_completions
            .iter()
            .any(|completion| completion.sub_agent_id == "current-child"));
        assert!(store
            .get_execution_context("history-parent")
            .expect("failed to load historical parent context")
            .is_none());
    }

    #[test]
    fn reconcile_child_workflows_for_parent_skips_terminal_parent_snapshot() {
        let store = create_test_store();
        seed_agent(&store, "agent-test");
        store
            .create_workflow("terminal-parent", "Parent task", "agent-test", None, None)
            .expect("failed to create parent workflow");
        store
            .create_workflow(
                "terminal-parent-child",
                "Child task",
                "agent-test",
                None,
                Some("terminal-parent"),
            )
            .expect("failed to create child workflow");

        let mut parent_context = ExecutionContext::new("terminal-parent".to_string());
        parent_context.state = RuntimeState::Cancelled;
        store
            .upsert_execution_context(&parent_context)
            .expect("failed to persist terminal parent snapshot");

        reconcile_child_workflows_for_parent(&store, "terminal-parent")
            .expect("terminal parent reconciliation should be skipped safely");

        let child = store
            .get_workflow("terminal-parent-child")
            .expect("failed to load child workflow")
            .expect("child workflow should exist");
        assert_eq!(child.status, "pending");
        assert!(store
            .list_workflow_events("terminal-parent")
            .expect("failed to load parent events")
            .is_empty());
    }

    #[test]
    fn reconcile_terminal_child_redelivers_signal_after_prior_parent_projection() {
        let store = create_test_store();
        seed_agent(&store, "agent-test");
        store
            .create_workflow("signal-parent", "Parent task", "agent-test", None, None)
            .expect("failed to create parent workflow");
        store
            .create_workflow(
                "signal-child",
                "Child task",
                "agent-test",
                None,
                Some("signal-parent"),
            )
            .expect("failed to create child workflow");
        let mut child_context = ExecutionContext::new("signal-child".to_string());
        child_context.state = RuntimeState::Completed;
        store
            .upsert_execution_context(&child_context)
            .expect("failed to persist completed child state");

        // Simulate a failure after the parent completion row was written but before the event or
        // live signal. The first reconciliation establishes the durable completion with no live
        // parent channel; the second must still re-deliver once the parent is running.
        reconcile_interrupted_child_workflows(&store)
            .expect("failed to create durable parent completion projection");
        let parent = store
            .get_execution_context("signal-parent")
            .expect("failed to load parent context")
            .expect("parent completion projection should be durable");
        assert!(parent
            .pending_sub_agent_completions
            .iter()
            .any(|completion| completion.sub_agent_id == "signal-child"));

        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        WorkflowManager::register_session_signal_tx("signal-parent".to_string(), tx);
        reconcile_interrupted_child_workflows(&store)
            .expect("failed to re-deliver terminal child completion");
        let signal: Value = serde_json::from_str(
            &rx.try_recv()
                .expect("live parent must receive recovered child completion"),
        )
        .expect("recovered signal should be valid JSON");
        assert_eq!(signal["type"], "sub_agent_complete");
        assert_eq!(signal["sub_agent_id"], "signal-child");
        assert!(signal["result"]["usage_summary"].is_object());
        WorkflowManager::unregister_session_signal_tx("signal-parent");
    }

    #[test]
    fn reconcile_terminal_parent_does_not_receive_redundant_child_signal() {
        let store = create_test_store();
        seed_agent(&store, "agent-test");
        store
            .create_workflow("completed-parent", "Parent task", "agent-test", None, None)
            .expect("failed to create parent workflow");
        store
            .create_workflow(
                "completed-child",
                "Child task",
                "agent-test",
                None,
                Some("completed-parent"),
            )
            .expect("failed to create child workflow");

        let mut parent_context = ExecutionContext::new("completed-parent".to_string());
        parent_context.state = RuntimeState::Completed;
        store
            .upsert_execution_context(&parent_context)
            .expect("failed to persist completed parent state");
        let mut child_context = ExecutionContext::new("completed-child".to_string());
        child_context.state = RuntimeState::Completed;
        store
            .upsert_execution_context(&child_context)
            .expect("failed to persist completed child state");

        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        WorkflowManager::register_session_signal_tx("completed-parent".to_string(), tx);
        reconcile_interrupted_child_workflows(&store)
            .expect("failed to reconcile completed parent child");
        reconcile_interrupted_child_workflows(&store)
            .expect("repeated reconciliation should converge");

        assert!(
            rx.try_recv().is_err(),
            "terminal parent must not receive a redundant child completion signal"
        );
        let completion_events = store
            .list_workflow_events("completed-parent")
            .expect("failed to load parent completion events")
            .into_iter()
            .filter(|event| {
                event.event_type == "sub_agent_completed"
                    && event.event_data["sub_agent_id"].as_str() == Some("completed-child")
            })
            .count();
        assert_eq!(completion_events, 1);
        WorkflowManager::unregister_session_signal_tx("completed-parent");
    }

    #[test]
    fn reconcile_terminal_background_child_restores_hidden_completion_projection() {
        let store = create_test_store();
        seed_agent(&store, "agent-test");
        store
            .create_workflow("background-parent", "Parent task", "agent-test", None, None)
            .expect("failed to create parent workflow");
        store
            .create_workflow(
                "background-child",
                "Child task",
                "agent-test",
                None,
                Some("background-parent"),
            )
            .expect("failed to create child workflow");
        store
            .append_workflow_event(&WorkflowEvent::sub_agent_started(
                "background-parent".to_string(),
                "background-child".to_string(),
                "background".to_string(),
            ))
            .expect("failed to persist background start event");
        let mut child_context = ExecutionContext::new("background-child".to_string());
        child_context.state = RuntimeState::Completed;
        store
            .upsert_execution_context(&child_context)
            .expect("failed to persist completed child state");

        reconcile_interrupted_child_workflows(&store).expect("failed to recover background child");
        reconcile_interrupted_child_workflows(&store).expect("recovery must be idempotent");

        let hidden_projections = store
            .get_workflow_snapshot("background-parent")
            .expect("failed to load recovered parent messages")
            .messages
            .into_iter()
            .filter(|message| {
                message.source_event_type.as_deref() == Some("sub_agent_completed")
                    && message.metadata.as_ref().is_some_and(|metadata| {
                        metadata["sub_agent_id"].as_str() == Some("background-child")
                            && metadata["execution_mode"].as_str() == Some("background")
                            && metadata["result"]["usage_summary"].is_object()
                    })
            })
            .count();
        assert_eq!(hidden_projections, 1);
    }

    #[test]
    fn reconcile_interrupted_child_preserves_terminal_snapshot() {
        let store = create_test_store();
        seed_agent(&store, "agent-test");
        store
            .create_workflow("parent-session", "Parent task", "agent-test", None, None)
            .expect("failed to create parent workflow");
        store
            .create_workflow(
                "subagent_completed_snapshot",
                "Child task",
                "agent-test",
                None,
                Some("parent-session"),
            )
            .expect("failed to create child workflow");

        let mut context = ExecutionContext::new("subagent_completed_snapshot".to_string());
        context.state = RuntimeState::Completed;
        store
            .upsert_execution_context(&context)
            .expect("failed to persist completed child snapshot");

        reconcile_interrupted_child_workflows(&store).expect("failed to reconcile child workflows");

        let child = store
            .get_workflow("subagent_completed_snapshot")
            .expect("failed to load child workflow")
            .expect("child workflow should exist");
        assert_eq!(child.status, "completed");

        let parent = store
            .get_execution_context("parent-session")
            .expect("failed to load parent context")
            .expect("recovery must persist the parent completion projection");
        assert!(parent
            .pending_sub_agent_completions
            .iter()
            .any(|completion| {
                completion.sub_agent_id == "subagent_completed_snapshot"
                    && completion.status == "completed"
                    && completion.usage_summary.is_some()
            }));

        let completion_events = store
            .list_workflow_events("parent-session")
            .expect("failed to load parent events")
            .into_iter()
            .filter(|event| event.event_type == "sub_agent_completed")
            .count();
        assert_eq!(completion_events, 1);
    }

    #[test]
    fn reconcile_interrupted_child_preserves_terminal_event_reconciliation() {
        for (suffix, event, expected_status) in [
            (
                "completed",
                WorkflowEvent::workflow_completed(
                    "subagent_event_completed".to_string(),
                    Some("done".to_string()),
                ),
                "completed",
            ),
            (
                "failed",
                WorkflowEvent::workflow_failed(
                    "subagent_event_failed".to_string(),
                    "failed".to_string(),
                ),
                "error",
            ),
            (
                "cancelled",
                WorkflowEvent::workflow_cancelled("subagent_event_cancelled".to_string()),
                "cancelled",
            ),
        ] {
            let store = create_test_store();
            seed_agent(&store, "agent-test");
            let parent_id = format!("parent-{suffix}");
            let child_id = format!("subagent_event_{suffix}");
            store
                .create_workflow(&parent_id, "Parent task", "agent-test", None, None)
                .expect("failed to create parent workflow");
            store
                .create_workflow(
                    &child_id,
                    "Child task",
                    "agent-test",
                    None,
                    Some(&parent_id),
                )
                .expect("failed to create child workflow");
            store
                .append_workflow_event(&event)
                .expect("failed to append terminal child event");

            reconcile_interrupted_child_workflows(&store)
                .expect("failed to reconcile child workflows");

            let child = store
                .get_workflow(&child_id)
                .expect("failed to load child workflow")
                .expect("child workflow should exist");
            assert_eq!(child.status, expected_status);

            let completion_events = store
                .list_workflow_events(&parent_id)
                .expect("failed to load parent events")
                .into_iter()
                .filter(|record| {
                    matches!(
                        record.event_type.as_str(),
                        "sub_agent_completed" | "sub_agent_failed" | "sub_agent_interrupted"
                    )
                })
                .count();
            assert_eq!(completion_events, 1);
            let parent_context = store
                .get_execution_context(&parent_id)
                .expect("failed to load parent context")
                .expect("terminal child recovery must persist parent context");
            assert!(parent_context
                .pending_sub_agent_completions
                .iter()
                .any(|completion| {
                    completion.sub_agent_id == child_id && completion.usage_summary.is_some()
                }));
            assert!(!parent_context
                .sub_agent_sessions
                .iter()
                .any(|sub_agent_id| sub_agent_id == &child_id));
        }
    }

    #[test]
    fn reconcile_interrupted_child_uses_latest_terminal_event_without_replay() {
        let store = create_test_store();
        seed_agent(&store, "agent-test");
        store
            .create_workflow(
                "parent-terminal-tail",
                "Parent task",
                "agent-test",
                None,
                None,
            )
            .expect("failed to create parent workflow");
        store
            .create_workflow(
                "subagent-terminal-tail",
                "Child task",
                "agent-test",
                None,
                Some("parent-terminal-tail"),
            )
            .expect("failed to create child workflow");
        store
            .append_workflow_event(&WorkflowEvent::new(
                crate::workflow::react::events::WorkflowEventType::StateChanged,
                "subagent-terminal-tail".to_string(),
                json!({}),
            ))
            .expect("failed to append malformed child event");
        store
            .append_workflow_event(&WorkflowEvent::workflow_completed(
                "subagent-terminal-tail".to_string(),
                Some("done".to_string()),
            ))
            .expect("failed to append terminal child event");

        reconcile_interrupted_child_workflows(&store)
            .expect("latest terminal event should bypass malformed prefix replay");

        let child = store
            .get_workflow("subagent-terminal-tail")
            .expect("failed to load child workflow")
            .expect("child workflow should exist");
        assert_eq!(child.status, "completed");
    }

    #[test]
    fn reconcile_interrupted_child_uses_terminal_event_for_outdated_snapshot() {
        let store = create_test_store();
        seed_agent(&store, "agent-test");
        store
            .create_workflow("parent-outdated", "Parent task", "agent-test", None, None)
            .expect("failed to create parent workflow");
        store
            .create_workflow(
                "subagent_outdated_snapshot",
                "Child task",
                "agent-test",
                None,
                Some("parent-outdated"),
            )
            .expect("failed to create child workflow");

        let mut context = ExecutionContext::new("subagent_outdated_snapshot".to_string());
        context.state = RuntimeState::Completed;
        context.version = "0.0.1".to_string();
        store
            .upsert_execution_context(&context)
            .expect("failed to persist outdated child snapshot");
        store
            .append_workflow_event(&WorkflowEvent::workflow_failed(
                "subagent_outdated_snapshot".to_string(),
                "terminal failure".to_string(),
            ))
            .expect("failed to append terminal child event");

        reconcile_interrupted_child_workflows(&store).expect("failed to reconcile child workflows");

        let child = store
            .get_workflow("subagent_outdated_snapshot")
            .expect("failed to load child workflow")
            .expect("child workflow should exist");
        assert_eq!(child.status, "error");
        assert!(store
            .list_workflow_events("parent-outdated")
            .expect("failed to load parent events")
            .iter()
            .all(|event| event.event_type != "sub_agent_interrupted"));
    }

    #[test]
    fn reconcile_interrupted_child_fails_safely_on_invalid_event_replay() {
        let store = create_test_store();
        seed_agent(&store, "agent-test");
        store
            .create_workflow("parent-invalid", "Parent task", "agent-test", None, None)
            .expect("failed to create parent workflow");
        store
            .create_workflow(
                "subagent_invalid_replay",
                "Child task",
                "agent-test",
                None,
                Some("parent-invalid"),
            )
            .expect("failed to create child workflow");
        store
            .append_workflow_event(&WorkflowEvent::workflow_completed(
                "subagent_invalid_replay".to_string(),
                Some("done".to_string()),
            ))
            .expect("failed to append terminal child event");
        store
            .append_workflow_event(&WorkflowEvent::new(
                crate::workflow::react::events::WorkflowEventType::StateChanged,
                "subagent_invalid_replay".to_string(),
                json!({}),
            ))
            .expect("failed to append malformed child event");

        let error = reconcile_interrupted_child_workflows(&store)
            .expect_err("invalid event replay must fail safely");
        assert!(error.to_string().contains("Cannot safely reconcile"));

        let child = store
            .get_workflow("subagent_invalid_replay")
            .expect("failed to load child workflow")
            .expect("child workflow should exist");
        assert_eq!(child.status, "pending");
        assert!(store
            .list_workflow_events("parent-invalid")
            .expect("failed to load parent events")
            .iter()
            .all(|event| event.event_type != "sub_agent_interrupted"));
        let parent_messages = store
            .get_workflow_snapshot("parent-invalid")
            .expect("failed to load parent snapshot")
            .messages;
        assert!(parent_messages.iter().all(|message| {
            message.source_event_type.as_deref() != Some("sub_agent_interrupted")
        }));
    }

    #[test]
    fn reconcile_interrupted_children_validate_all_before_writing() {
        let store = create_test_store();
        seed_agent(&store, "agent-test");
        for parent_id in ["parent-stale-first", "parent-invalid-second"] {
            store
                .create_workflow(parent_id, "Parent task", "agent-test", None, None)
                .expect("failed to create parent workflow");
        }
        store
            .create_workflow(
                "subagent_stale_first",
                "Stale child",
                "agent-test",
                None,
                Some("parent-stale-first"),
            )
            .expect("failed to create stale child workflow");
        store
            .create_workflow(
                "subagent_invalid_second",
                "Invalid child",
                "agent-test",
                None,
                Some("parent-invalid-second"),
            )
            .expect("failed to create invalid child workflow");
        store
            .append_workflow_event(&WorkflowEvent::new(
                crate::workflow::react::events::WorkflowEventType::StateChanged,
                "subagent_invalid_second".to_string(),
                json!({}),
            ))
            .expect("failed to append malformed child event");

        let error = reconcile_interrupted_child_workflows(&store)
            .expect_err("validation failure must abort before writes");
        assert!(error.to_string().contains("Cannot safely reconcile"));

        for child_id in ["subagent_stale_first", "subagent_invalid_second"] {
            let child = store
                .get_workflow(child_id)
                .expect("failed to load child workflow")
                .expect("child workflow should exist");
            assert_eq!(child.status, "pending");
            assert!(store
                .get_execution_context(child_id)
                .expect("failed to load child context")
                .is_none());
        }
        for parent_id in ["parent-stale-first", "parent-invalid-second"] {
            assert!(store
                .list_workflow_events(parent_id)
                .expect("failed to load parent events")
                .iter()
                .all(|event| event.event_type != "sub_agent_interrupted"));
            let parent_messages = store
                .get_workflow_snapshot(parent_id)
                .expect("failed to load parent snapshot")
                .messages;
            assert!(parent_messages.iter().all(|message| {
                message.source_event_type.as_deref() != Some("sub_agent_interrupted")
            }));
        }
    }

    #[test]
    fn test_build_large_file_read_plan_generates_full_read_strategy() {
        let root = tempdir().unwrap();
        let path = root.path().join("large.txt");
        let content = (0..240)
            .map(|i| format!("fn line_{}() {{ return {}; }}", i, i))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, content).unwrap();

        let plan = build_large_file_read_plan(&path).expect("plan should exist");

        assert_eq!(plan.total_lines, 240);
        assert!(plan.recommended_limit > 0);
        assert!(plan.recommended_limit <= crate::tools::DEFAULT_READ_FILE_LIMIT);
        assert!(!plan.chunk_plan.is_empty());
        assert_eq!(plan.chunk_plan[0].offset, 0);
        assert!(plan.chunk_plan[0].limit > 0);
        assert!(plan.estimated_calls >= 1);
    }

    #[test]
    fn test_inject_at_mentions_deduplicates_canonical_targets() {
        let root = tempdir().unwrap();
        let file_path = root.path().join("sample.txt");
        std::fs::write(&file_path, "unique mention content").unwrap();
        let prompt = format!(
            "Review @sample.txt and @sample.txt and @\"{}\"",
            file_path.display()
        );

        let (normalized_prompt, attached) =
            inject_at_mentions(&prompt, &[root.path().to_string_lossy().to_string()]);

        assert_eq!(
            normalized_prompt,
            "Review @sample.txt and @sample.txt and @sample.txt"
        );
        assert_eq!(attached.matches("unique mention content").count(), 1);
        assert_eq!(
            attached
                .matches("<file_content path=\"sample.txt\">")
                .count(),
            1
        );
    }

    #[test]
    fn test_inject_at_mentions_limits_wildcard_expansion_to_ten_targets() {
        let root = tempdir().unwrap();
        for index in 0..25 {
            std::fs::write(
                root.path().join(format!("file-{index:02}.txt")),
                index.to_string(),
            )
            .unwrap();
        }

        let (normalized_prompt, attached) = inject_at_mentions(
            "Review @*.txt",
            &[root.path().to_string_lossy().to_string()],
        );

        assert_eq!(
            normalized_prompt.matches('@').count(),
            MAX_AT_MENTION_TARGETS
        );
        assert_eq!(
            attached.matches("<file_content path=").count(),
            MAX_AT_MENTION_TARGETS
        );
    }

    #[test]
    fn test_inject_at_mentions_prefers_primary_root_for_ambiguous_relative_paths() {
        let primary_root = tempdir().unwrap();
        let secondary_root = tempdir().unwrap();
        let primary_docs = primary_root.path().join("docs");
        let secondary_docs = secondary_root.path().join("docs");
        std::fs::create_dir(&primary_docs).unwrap();
        std::fs::create_dir(&secondary_docs).unwrap();
        std::fs::write(primary_docs.join("primary.txt"), "primary docs").unwrap();
        std::fs::write(secondary_docs.join("secondary.txt"), "secondary docs").unwrap();

        let secondary_docs_path = std::fs::canonicalize(&secondary_docs).unwrap();
        let secondary_display_path = secondary_docs_path.to_string_lossy();
        let prompt = format!(
            "Write one file to @docs and explicitly inspect @\"{}\"",
            secondary_display_path
        );

        let (normalized_prompt, attached) = inject_at_mentions(
            &prompt,
            &[
                primary_root.path().to_string_lossy().to_string(),
                secondary_root.path().to_string_lossy().to_string(),
            ],
        );

        assert_eq!(
            normalized_prompt,
            format!(
                "Write one file to @docs and explicitly inspect @{}",
                secondary_display_path
            )
        );
        assert!(attached.contains("<list_dir path=\"docs\">"));
        assert!(attached.contains("primary.txt"));
        assert!(attached.contains(&format!("<list_dir path=\"{}\">", secondary_display_path)));
        assert!(attached.contains("secondary.txt"));
    }

    #[test]
    fn test_inject_at_mentions_formats_secondary_root_as_canonical_absolute_path() {
        let primary_root = tempdir().unwrap();
        let secondary_root = tempdir().unwrap();
        let secondary_file = secondary_root.path().join("secondary file.txt");
        std::fs::write(&secondary_file, "secondary content").unwrap();
        let prompt = format!("Review @\"{}\"", secondary_file.display());

        let (normalized_prompt, attached) = inject_at_mentions(
            &prompt,
            &[
                primary_root.path().to_string_lossy().to_string(),
                secondary_root.path().to_string_lossy().to_string(),
            ],
        );
        let canonical_file = std::fs::canonicalize(&secondary_file).unwrap();
        let display_path = canonical_file.to_string_lossy();

        assert_eq!(normalized_prompt, format!("Review @\"{display_path}\""));
        assert!(attached.contains(&format!("<file_content path=\"{display_path}\">")));
        assert!(attached.contains("secondary content"));
    }

    #[test]
    fn test_inject_at_mentions_supports_quoted_authorized_paths() {
        let primary_parent = tempdir().unwrap();
        let primary_root = primary_parent.path().join("primary root ");
        let secondary_root = tempdir().unwrap();
        let primary_file = primary_root.join(" leading and trailing ");
        let primary_dir = primary_root.join("folder name");
        let secondary_file = secondary_root.path().join("secondary file.txt");
        std::fs::create_dir(&primary_root).unwrap();
        std::fs::write(&primary_file, "primary content").unwrap();
        std::fs::create_dir(&primary_dir).unwrap();
        std::fs::write(primary_dir.join("child.txt"), "child content").unwrap();
        std::fs::write(&secondary_file, "secondary content").unwrap();

        let prompt = format!(
            r#"Review @" leading and trailing " and @"folder name" plus @"{}""#,
            secondary_file.display()
        );
        let (_, attached) = inject_at_mentions(
            &prompt,
            &[
                primary_root.to_string_lossy().to_string(),
                secondary_root.path().to_string_lossy().to_string(),
            ],
        );

        assert!(attached.contains("primary content"));
        assert!(attached.contains("child.txt"));
        assert!(attached.contains("secondary content"));
    }

    #[test]
    fn test_inject_at_mentions_rejects_outside_absolute_paths() {
        let allowed_root = tempdir().unwrap();
        let outside_root = tempdir().unwrap();
        let outside_file = outside_root.path().join("outside.txt");
        std::fs::write(&outside_file, "outside content").unwrap();

        let prompt = format!(r#"Review @"{}""#, outside_file.display());
        let (_, attached) = inject_at_mentions(
            &prompt,
            &[allowed_root.path().to_string_lossy().to_string()],
        );

        assert!(attached.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn test_inject_at_mentions_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let allowed_root = tempdir().unwrap();
        let outside_root = tempdir().unwrap();
        let outside_file = outside_root.path().join("outside.txt");
        let escaped_link = allowed_root.path().join("escaped-link.txt");
        std::fs::write(&outside_file, "outside content").unwrap();
        symlink(&outside_file, &escaped_link).unwrap();

        let (_, attached) = inject_at_mentions(
            "Review @escaped-link.txt",
            &[allowed_root.path().to_string_lossy().to_string()],
        );

        assert!(attached.is_empty());
    }

    #[test]
    fn test_inject_at_mentions_large_file_includes_full_read_plan_and_grep_guidance() {
        let root = tempdir().unwrap();
        let large_path = root.path().join("large.ts");
        let content = (0..900)
            .map(|i| format!("export const value_{} = '{}';", i, "x".repeat(24)))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&large_path, content).unwrap();

        let (_, attached) =
            inject_at_mentions("@large.ts", &[root.path().to_string_lossy().to_string()]);

        assert!(attached.contains(
            "If you only need symbols, keys, or a specific section, prefer 'grep' first."
        ));
        assert!(attached.contains("<full_file_read_plan "));
        assert!(attached.contains("recommended_when=\"need_complete_file\""));
        assert!(attached.contains("prefer_grep_when=\"targeted_lookup\""));
        assert!(attached.contains("first_chunk_limit=\""));
        assert!(attached.contains("chunk_preview=\"0:"));
        assert!(attached
            .contains("When the task requires reviewing or executing against the whole file"));
        assert!(attached.contains("exact simulated full-read plan"));
        assert!(attached.contains("Recommended chunk sequence"));
    }

    #[test]
    fn test_inject_at_mentions_into_signal_adds_attached_context_for_user_messages() {
        let root = tempdir().unwrap();
        let file_path = root.path().join("sample.txt");
        std::fs::write(&file_path, "alpha\nbeta\ngamma").unwrap();

        let signal = json!({
            "type": "user_message",
            "content": "Please review @sample.txt",
            "attached_context": "existing context"
        })
        .to_string();

        let enriched =
            inject_at_mentions_into_signal(&signal, &[root.path().to_string_lossy().to_string()]);
        let parsed: serde_json::Value = serde_json::from_str(&enriched).unwrap();
        let attached_context = parsed["attached_context"].as_str().unwrap_or_default();

        assert_eq!(parsed["content"], "Please review @sample.txt");
        assert!(attached_context.contains("existing context"));
        assert!(
            attached_context.contains("<file_content path=\"sample.txt\">")
                || attached_context.contains("<file_content path=\"sample.txt\"")
        );
        assert!(attached_context.contains("alpha"));
        assert!(attached_context.contains("beta"));
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
    fn test_should_reinject_user_message_after_recovery() {
        assert!(should_reinject_user_message_after_recovery(Some(
            &WaitReason::UserInput
        )));
        assert!(should_reinject_user_message_after_recovery(Some(
            &WaitReason::Approval
        )));
        assert!(!should_reinject_user_message_after_recovery(Some(
            &WaitReason::Confirmation
        )));
        assert!(!should_reinject_user_message_after_recovery(None));
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
    fn test_runtime_state_allows_manual_clear() {
        assert!(runtime_state_allows_manual_clear(&RuntimeState::Pending));
        assert!(runtime_state_allows_manual_clear(&RuntimeState::Completed));
        assert!(runtime_state_allows_manual_clear(&RuntimeState::Failed));
        assert!(runtime_state_allows_manual_clear(&RuntimeState::Cancelled));
        assert!(!runtime_state_allows_manual_clear(&RuntimeState::Running));
        assert!(!runtime_state_allows_manual_clear(&RuntimeState::Stopping));
        assert!(!runtime_state_allows_manual_clear(&RuntimeState::Waiting));
    }

    #[test]
    fn test_workflow_state_allows_manual_clear() {
        assert!(workflow_state_allows_manual_clear(&WorkflowState::Pending));
        assert!(workflow_state_allows_manual_clear(
            &WorkflowState::Completed
        ));
        assert!(workflow_state_allows_manual_clear(&WorkflowState::Error));
        assert!(workflow_state_allows_manual_clear(
            &WorkflowState::Cancelled
        ));
        assert!(!workflow_state_allows_manual_clear(
            &WorkflowState::Thinking
        ));
        assert!(!workflow_state_allows_manual_clear(
            &WorkflowState::Executing
        ));
        assert!(!workflow_state_allows_manual_clear(
            &WorkflowState::Auditing
        ));
        assert!(!workflow_state_allows_manual_clear(
            &WorkflowState::Stopping
        ));
        assert!(!workflow_state_allows_manual_clear(&WorkflowState::Paused));
        assert!(!workflow_state_allows_manual_clear(
            &WorkflowState::AwaitingApproval
        ));
        assert!(!workflow_state_allows_manual_clear(
            &WorkflowState::AwaitingAutoApproval
        ));
        assert!(!workflow_state_allows_manual_clear(
            &WorkflowState::AwaitingUser
        ));
        assert!(!workflow_state_allows_manual_clear(
            &WorkflowState::AwaitingSubAgent
        ));
    }

    #[test]
    fn test_persist_cancelled_workflow_state_updates_execution_context() {
        let store = create_test_store();
        let session_id = "cancelled-context-state";
        seed_agent(&store, "agent-test");
        store
            .create_workflow(session_id, "Initial query", "agent-test", None, None)
            .expect("failed to create workflow");

        let mut context = ExecutionContext::new(session_id.to_string());
        context.state = RuntimeState::Running;
        context.wait_reason = Some(WaitReason::Approval);
        store
            .upsert_execution_context(&context)
            .expect("failed to persist execution context");

        persist_cancelled_workflow_state(&store, session_id)
            .expect("failed to persist cancelled workflow state");

        let snapshot = store
            .get_workflow_snapshot(session_id)
            .expect("failed to load workflow snapshot");
        let execution_context = store
            .get_execution_context(session_id)
            .expect("failed to load execution context")
            .expect("execution context should exist");
        assert_eq!(snapshot.workflow.status, "cancelled");
        assert_eq!(execution_context.state, RuntimeState::Cancelled);
        assert_eq!(execution_context.wait_reason, None);
    }

    #[test]
    fn test_persist_failed_workflow_state_updates_execution_context() {
        let store = create_test_store();
        let session_id = "failed-context-state";
        seed_agent(&store, "agent-test");
        store
            .create_workflow(session_id, "Initial query", "agent-test", None, None)
            .expect("failed to create workflow");

        let mut context = ExecutionContext::new(session_id.to_string());
        context.state = RuntimeState::Running;
        context.wait_reason = Some(WaitReason::Approval);
        store
            .upsert_execution_context(&context)
            .expect("failed to persist execution context");

        persist_failed_workflow_state(&store, session_id)
            .expect("failed to persist failed workflow state");

        let snapshot = store
            .get_workflow_snapshot(session_id)
            .expect("failed to load workflow snapshot");
        let execution_context = store
            .get_execution_context(session_id)
            .expect("failed to load execution context")
            .expect("execution context should exist");
        assert_eq!(snapshot.workflow.status, "error");
        assert_eq!(execution_context.state, RuntimeState::Failed);
        assert_eq!(execution_context.wait_reason, None);
    }

    #[test]
    fn latest_manual_clear_detaches_previous_child_from_recovery() {
        let store = create_test_store();
        let session_id = "manual-clear-detached-child";
        seed_agent(&store, "agent-test");
        store
            .create_workflow(session_id, "Initial query", "agent-test", None, None)
            .expect("failed to create workflow");
        let mut previous_context = ExecutionContext::new(session_id.to_string());
        previous_context.current_segment_id = 1;
        store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "tool".to_string(),
                message: String::new(),
                reasoning: None,
                message_kind: "message".to_string(),
                message_subtype: None,
                segment_id: 1,
                source_event_type: None,
                metadata: Some(json!({
                    "tool_name": crate::tools::TOOL_SUB_AGENT_RUN,
                    "sub_agent_id": "old-child",
                    "execution_mode": "background"
                })),
                attached_context: None,
                step_type: Some(StepType::Observe.to_string()),
                step_index: 1,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add old child observation");
        store
            .add_workflow_message(&WorkflowMessage {
                id: None,
                session_id: session_id.to_string(),
                role: "system".to_string(),
                message: String::new(),
                reasoning: None,
                message_kind: "summary".to_string(),
                message_subtype: Some("manual_clear_context".to_string()),
                segment_id: 2,
                source_event_type: None,
                metadata: Some(json!({
                    "previous_execution_context": previous_context
                })),
                attached_context: None,
                step_type: None,
                step_index: 0,
                is_error: false,
                error_type: None,
                created_at: None,
            })
            .expect("failed to add manual clear marker");
        let mut current_context = ExecutionContext::new(session_id.to_string());
        current_context.current_segment_id = 2;
        store
            .upsert_execution_context(&current_context)
            .expect("failed to persist current context");

        assert!(
            child_was_detached_by_latest_manual_clear(&store, session_id, "old-child")
                .expect("manual clear ownership check should succeed")
        );
        assert!(
            !child_was_detached_by_latest_manual_clear(&store, session_id, "new-child")
                .expect("new child ownership check should succeed")
        );

        store
            .create_workflow(
                "old-child",
                "Old background child",
                "agent-test",
                None,
                Some(session_id),
            )
            .expect("failed to create old child workflow");
        let mut child_context = ExecutionContext::new("old-child".to_string());
        child_context.state = RuntimeState::Completed;
        store
            .upsert_execution_context(&child_context)
            .expect("failed to persist terminal child context");
        store
            .update_workflow_status("old-child", &WorkflowState::Completed.to_string())
            .expect("failed to persist terminal child status");

        reconcile_interrupted_child_workflows(&store)
            .expect("detached child reconciliation should succeed");

        let parent_context = store
            .get_execution_context(session_id)
            .expect("failed to reload parent context")
            .expect("parent context should remain available");
        assert!(parent_context.sub_agent_sessions.is_empty());
        assert!(parent_context.pending_sub_agent_completions.is_empty());
        let snapshot = store
            .get_workflow_snapshot(session_id)
            .expect("failed to load parent snapshot");
        assert!(snapshot
            .messages
            .iter()
            .skip_while(|message| { !ContextManager::is_manual_clear_context_message(message) })
            .all(|message| {
                message
                    .metadata
                    .as_ref()
                    .and_then(|metadata| metadata.get("sub_agent_id"))
                    .and_then(Value::as_str)
                    != Some("old-child")
            }));
    }

    #[test]
    fn reset_workflow_phase_for_new_context_preserves_user_preferences() {
        let store = create_test_store();
        let session_id = "manual-clear-phase-reset";
        seed_agent(&store, "agent-test");
        let config = AgentConfig {
            phase: Some("implementation".to_string()),
            approval_level: Some("smart".to_string()),
            allowed_paths: Some(vec!["/preserved".to_string()]),
            available_tools: Some(vec!["read_file".to_string()]),
            models: Some(crate::db::agent::AgentModels::default()),
            ..AgentConfig::default()
        };
        store
            .create_workflow(
                session_id,
                "Initial query",
                "agent-test",
                Some(config.to_json()),
                None,
            )
            .expect("failed to create workflow");

        reset_workflow_phase_for_new_context(&store, session_id)
            .expect("phase reset should succeed");

        let persisted =
            raw_workflow_agent_config(&store, session_id).expect("failed to load reset config");
        assert_eq!(persisted.phase.as_deref(), Some("standard"));
        assert_eq!(persisted.approval_level.as_deref(), Some("smart"));
        assert_eq!(
            persisted.allowed_paths,
            Some(vec!["/preserved".to_string()])
        );
        assert_eq!(
            persisted.available_tools,
            Some(vec!["read_file".to_string()])
        );
        assert!(persisted.models.is_some());
    }

    #[test]
    fn test_persist_pending_workflow_state_updates_execution_context() {
        let store = create_test_store();
        let session_id = "manual-clear-pending-state";
        seed_agent(&store, "agent-test");
        store
            .create_workflow(session_id, "Initial query", "agent-test", None, None)
            .expect("failed to create workflow");
        store
            .update_workflow_status(session_id, &WorkflowState::Cancelled.to_string())
            .expect("failed to seed cancelled workflow status");

        let mut context = ExecutionContext::new(session_id.to_string());
        context.state = RuntimeState::Cancelled;
        context.wait_reason = Some(WaitReason::SubAgent);
        context.current_segment_id = 2;
        context.current_step = 12;
        context.current_context_tokens = Some(64);
        context.waiting_on_sub_agent_id = Some("subagent-old".to_string());
        context.sub_agent_sessions.push("subagent-old".to_string());
        context.pending_final_review = Some(crate::workflow::react::types::PendingFinalReview {
            sub_agent_id: "reviewer-old".to_string(),
            completion_summary: "old summary".to_string(),
        });
        context
            .removed_queued_user_message_ids
            .push("old-queue".to_string());
        store
            .upsert_execution_context(&context)
            .expect("failed to persist execution context");

        persist_pending_workflow_state(&store, session_id)
            .expect("failed to persist pending workflow state");

        let snapshot = store
            .get_workflow_snapshot(session_id)
            .expect("failed to load workflow snapshot");
        let execution_context = store
            .get_execution_context(session_id)
            .expect("failed to load execution context")
            .expect("execution context should exist");
        assert_eq!(snapshot.workflow.status, "pending");
        assert_eq!(execution_context.state, RuntimeState::Pending);
        assert_eq!(execution_context.wait_reason, None);
        assert_eq!(execution_context.current_segment_id, 2);
        assert_eq!(execution_context.current_context_tokens, Some(64));
        assert_eq!(execution_context.current_step, 0);
        assert_eq!(execution_context.waiting_on_sub_agent_id, None);
        assert!(execution_context.sub_agent_sessions.is_empty());
        assert!(execution_context.pending_sub_agent_completions.is_empty());
        assert_eq!(execution_context.pending_final_review, None);
        assert!(execution_context.pending_completion_reports.is_empty());
        assert!(execution_context.queued_user_messages.is_empty());
        assert!(execution_context.removed_queued_user_message_ids.is_empty());
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
    fn terminal_manual_compression_bypasses_manager_only_after_session_stops() {
        let manual_compress = WorkflowSignal::ManualCompress;

        for status in ["completed", "error", "cancelled"] {
            assert!(should_run_terminal_manual_compression(
                Some(&manual_compress),
                status
            ));
        }
        assert!(!should_run_terminal_manual_compression(
            Some(&manual_compress),
            "executing"
        ));
        assert!(!should_run_terminal_manual_compression(
            Some(&WorkflowSignal::Continue),
            "completed"
        ));

        assert!(should_bypass_manager_for_terminal_manual_compression(
            Some(&manual_compress),
            "completed",
            Some(&ManagedSessionStatus::Completed),
        ));
        assert!(should_bypass_manager_for_terminal_manual_compression(
            Some(&manual_compress),
            "completed",
            None,
        ));
        for managed_status in [
            ManagedSessionStatus::Active,
            ManagedSessionStatus::Waiting,
            ManagedSessionStatus::Stopping,
        ] {
            assert!(!should_bypass_manager_for_terminal_manual_compression(
                Some(&manual_compress),
                "completed",
                Some(&managed_status),
            ));
        }
    }

    #[test]
    fn test_can_resume_user_message_from_recovery_uses_awaiting_user_status() {
        let mut stale_context = ExecutionContext::new("session-6".to_string());
        stale_context.state = RuntimeState::Running;

        assert!(can_resume_user_message_from_recovery(
            Some(&stale_context),
            "awaiting_user"
        ));
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
    fn inherited_workflow_config_applies_user_tool_preferences_to_current_agent_capabilities() {
        let agent_config = AgentConfig {
            available_tools: Some(vec![
                "read_file".to_string(),
                "server__MCP__new_tool".to_string(),
            ]),
            mcp_tool_exposure: Some(vec![
                "server__MCP__new_tool".to_string(),
                "server__MCP__disabled_tool".to_string(),
            ]),
            auto_approve: Some(vec!["read_file".to_string()]),
            max_contexts: Some(128_000),
            ..AgentConfig::default()
        };
        let inherited_config = AgentConfig {
            available_tools: Some(vec![
                "read_file".to_string(),
                "server__MCP__removed_tool".to_string(),
                crate::tools::TOOL_BASH.to_string(),
            ]),
            mcp_tool_exposure: Some(Vec::new()),
            auto_approve: Some(vec![
                "server__MCP__new_tool".to_string(),
                "server__MCP__removed_tool".to_string(),
                crate::tools::TOOL_BASH.to_string(),
            ]),
            max_contexts: Some(32_000),
            ..AgentConfig::default()
        };

        let merged = merge_inherited_workflow_config(&agent_config, &inherited_config);

        assert_eq!(merged.available_tools, Some(vec!["read_file".to_string()]));
        assert_eq!(merged.mcp_tool_exposure, Some(Vec::new()));
        assert_eq!(merged.auto_approve, Some(vec!["read_file".to_string()]));
        assert_eq!(merged.max_contexts, Some(128_000));
        assert!(merged.shell_policy.is_none());
    }

    #[test]
    fn inherited_workflow_personality_respects_agent_defaults_and_stale_preferences() {
        let agent_config = AgentConfig {
            personality: Some("Current custom style".to_string()),
            ..AgentConfig::default()
        };

        let no_preference = merge_inherited_workflow_config(&agent_config, &AgentConfig::default());
        assert_eq!(
            no_preference.personality.as_deref(),
            Some("Current custom style")
        );

        let matching_custom = merge_inherited_workflow_config(
            &agent_config,
            &AgentConfig {
                personality: Some("Current custom style".to_string()),
                ..AgentConfig::default()
            },
        );
        assert_eq!(
            matching_custom.personality.as_deref(),
            Some("Current custom style")
        );

        let stale_custom = merge_inherited_workflow_config(
            &agent_config,
            &AgentConfig {
                personality: Some("Removed custom style".to_string()),
                ..AgentConfig::default()
            },
        );
        assert_eq!(
            stale_custom.personality.as_deref(),
            Some(crate::workflow::react::prompts::AGENT_PERSONALITY_PRESET_DEFAULT_ID)
        );
    }

    #[test]
    fn task_boundary_sync_refreshes_visible_and_auto_approved_tools_together() {
        let store = create_test_store();
        let session_id = "task-boundary-tool-sync";
        seed_agent(&store, "agent-test");

        store
            .db_runtime()
            .expect("failed to obtain database runtime")
            .write_blocking(|conn| {
                conn.execute(
                    "UPDATE agents
                 SET available_tools = ?1, auto_approve = ?2, mcp_tool_exposure = ?3,
                     shell_policy = ?4
                 WHERE id = ?5",
                    params![
                        serde_json::to_string(&vec![
                            "read_file",
                            "server__MCP__new_tool",
                            crate::tools::TOOL_BASH
                        ])
                        .expect("serialize Agent available tools"),
                        serde_json::to_string(&vec![
                            "server__MCP__new_tool",
                            "server__MCP__removed_tool"
                        ])
                        .expect("serialize Agent auto approve"),
                        serde_json::to_string(&vec![
                            "server__MCP__new_tool",
                            "server__MCP__removed_tool"
                        ])
                        .expect("serialize Agent MCP exposure"),
                        serde_json::to_string(&vec![crate::tools::ShellPolicyRule {
                            pattern: "^git status$".to_string(),
                            decision: crate::tools::ShellDecision::Review(
                                "Review repository state".to_string(),
                            ),
                            description: None,
                        }])
                        .expect("serialize Agent shell policy"),
                        "agent-test"
                    ],
                )?;
                Ok(())
            })
            .expect("failed to update Agent tool config");

        let stale_config = AgentConfig {
            allowed_paths: Some(vec!["/workflow".to_string()]),
            available_tools: Some(vec![
                "read_file".to_string(),
                "server__MCP__removed_tool".to_string(),
            ]),
            auto_approve: Some(vec![
                "read_file".to_string(),
                "server__MCP__removed_tool".to_string(),
            ]),
            mcp_tool_exposure: Some(vec!["server__MCP__removed_tool".to_string()]),
            shell_policy: Some(vec![crate::tools::ShellPolicyRule {
                pattern: "^git diff$".to_string(),
                decision: crate::tools::ShellDecision::Allow,
                description: None,
            }]),
            ..AgentConfig::default()
        };
        store
            .create_workflow(
                session_id,
                "Initial query",
                "agent-test",
                Some(stale_config.to_json()),
                None,
            )
            .expect("failed to create workflow");

        let synced = sync_workflow_agent_config_at_tool_boundary(&store, session_id)
            .expect("failed to synchronize Agent tool config");

        assert_eq!(
            synced.available_tools,
            Some(vec![
                "read_file".to_string(),
                "server__MCP__new_tool".to_string(),
                crate::tools::TOOL_BASH.to_string(),
            ])
        );
        assert_eq!(
            synced.auto_approve,
            Some(vec![
                "server__MCP__new_tool".to_string(),
                "read_file".to_string(),
            ])
        );
        assert_eq!(
            synced.mcp_tool_exposure,
            Some(vec!["server__MCP__new_tool".to_string()])
        );
        assert_eq!(synced.allowed_paths, Some(vec!["/workflow".to_string()]));
        let shell_policy = synced
            .shell_policy
            .as_ref()
            .expect("missing synchronized shell policy");
        assert_eq!(shell_policy.len(), 2);
        assert_eq!(shell_policy[0].pattern, "^git status$");
        assert!(matches!(
            shell_policy[0].decision,
            crate::tools::ShellDecision::Review(_)
        ));
        assert_eq!(shell_policy[1].pattern, "^git diff$");
        assert!(matches!(
            shell_policy[1].decision,
            crate::tools::ShellDecision::Allow
        ));

        let persisted = store
            .get_workflow(session_id)
            .expect("failed to load workflow")
            .and_then(|workflow| workflow.agent_config)
            .and_then(|config| AgentConfig::from_json(&config))
            .expect("missing persisted Agent config");
        assert_eq!(persisted.available_tools, synced.available_tools);
        assert_eq!(persisted.auto_approve, synced.auto_approve);
    }

    #[test]
    fn auto_approved_tools_are_filtered_by_visible_tool_capabilities() {
        let mut config = AgentConfig {
            available_tools: Some(vec!["read_file".to_string()]),
            auto_approve: Some(vec![
                "read_file".to_string(),
                "write_file".to_string(),
                crate::tools::TOOL_BASH.to_string(),
            ]),
            ..AgentConfig::default()
        };

        enforce_auto_approve_tool_visibility(&mut config);

        assert_eq!(config.auto_approve, Some(vec!["read_file".to_string()]));
    }

    #[test]
    fn inherited_workflow_shell_allows_extend_agent_policy_without_overriding_it() {
        let agent_rule = crate::tools::ShellPolicyRule {
            pattern: "^git status$".to_string(),
            decision: crate::tools::ShellDecision::Review("Agent review".to_string()),
            description: None,
        };
        let agent_config = AgentConfig {
            available_tools: Some(vec![crate::tools::TOOL_BASH.to_string()]),
            shell_policy: Some(vec![agent_rule.clone()]),
            ..AgentConfig::default()
        };
        let inherited_config = AgentConfig {
            shell_policy: Some(vec![
                crate::tools::ShellPolicyRule {
                    pattern: "^git status$".to_string(),
                    decision: crate::tools::ShellDecision::Allow,
                    description: None,
                },
                crate::tools::ShellPolicyRule {
                    pattern: "^git diff$".to_string(),
                    decision: crate::tools::ShellDecision::Allow,
                    description: None,
                },
                crate::tools::ShellPolicyRule {
                    pattern: "^rm ".to_string(),
                    decision: crate::tools::ShellDecision::Deny("Workflow deny".to_string()),
                    description: None,
                },
            ]),
            ..AgentConfig::default()
        };

        let merged = merge_inherited_workflow_config(&agent_config, &inherited_config);

        let shell_policy = merged
            .shell_policy
            .expect("shell policy should be inherited");
        assert_eq!(shell_policy.len(), 2);
        assert_eq!(shell_policy[0].pattern, agent_rule.pattern);
        assert!(matches!(
            shell_policy[0].decision,
            crate::tools::ShellDecision::Review(_)
        ));
        assert_eq!(shell_policy[1].pattern, "^git diff$");
        assert!(matches!(
            shell_policy[1].decision,
            crate::tools::ShellDecision::Allow
        ));
    }

    #[test]
    fn inherited_workflow_sandbox_override_wins_over_agent_defaults() {
        let agent_config = AgentConfig {
            available_tools: Some(vec![crate::tools::TOOL_BASH.to_string()]),
            sandbox_execution_mode: Some(crate::tools::ShellExecutionMode::Auto),
            sandbox_scheme_id: Some("agent-scheme".to_string()),
            ..AgentConfig::default()
        };
        let inherited_config = AgentConfig {
            sandbox_override: Some(true),
            sandbox_execution_mode: Some(crate::tools::ShellExecutionMode::SandboxOnly),
            sandbox_scheme_id: Some("workflow-scheme".to_string()),
            ..AgentConfig::default()
        };

        let merged = merge_inherited_workflow_config(&agent_config, &inherited_config);

        assert_eq!(merged.sandbox_override, Some(true));
        assert_eq!(
            merged.sandbox_execution_mode,
            Some(crate::tools::ShellExecutionMode::SandboxOnly)
        );
        assert_eq!(merged.sandbox_scheme_id.as_deref(), Some("workflow-scheme"));
    }

    #[test]
    fn inherited_workflow_without_sandbox_override_uses_agent_defaults() {
        let agent_config = AgentConfig {
            available_tools: Some(vec![crate::tools::TOOL_BASH.to_string()]),
            sandbox_execution_mode: Some(crate::tools::ShellExecutionMode::Auto),
            sandbox_scheme_id: Some("agent-scheme".to_string()),
            ..AgentConfig::default()
        };
        let inherited_config = AgentConfig {
            sandbox_execution_mode: Some(crate::tools::ShellExecutionMode::SandboxOnly),
            sandbox_scheme_id: Some("stale-workflow-scheme".to_string()),
            ..AgentConfig::default()
        };

        let merged = merge_inherited_workflow_config(&agent_config, &inherited_config);

        assert_eq!(merged.sandbox_override, None);
        assert_eq!(
            merged.sandbox_execution_mode,
            Some(crate::tools::ShellExecutionMode::Auto)
        );
        assert_eq!(merged.sandbox_scheme_id.as_deref(), Some("agent-scheme"));
    }

    #[test]
    fn inherited_workflow_config_preserves_none_and_empty_tool_allowlist_semantics() {
        let inherited_config = AgentConfig {
            auto_approve: Some(vec!["workflow_only".to_string()]),
            ..AgentConfig::default()
        };
        let unrestricted_agent = AgentConfig {
            available_tools: None,
            mcp_tool_exposure: Some(vec!["server__MCP__tool".to_string()]),
            auto_approve: Some(vec!["agent_only".to_string()]),
            ..AgentConfig::default()
        };
        let empty_agent = AgentConfig {
            available_tools: Some(Vec::new()),
            mcp_tool_exposure: Some(vec!["server__MCP__tool".to_string()]),
            auto_approve: Some(vec!["agent_only".to_string()]),
            ..AgentConfig::default()
        };

        let unrestricted = merge_inherited_workflow_config(&unrestricted_agent, &inherited_config);
        assert_eq!(
            unrestricted.mcp_tool_exposure,
            Some(vec!["server__MCP__tool".to_string()])
        );
        assert_eq!(
            unrestricted.auto_approve,
            Some(vec!["agent_only".to_string(), "workflow_only".to_string()])
        );

        let empty = merge_inherited_workflow_config(&empty_agent, &inherited_config);
        assert_eq!(empty.mcp_tool_exposure, Some(Vec::new()));
        assert_eq!(empty.auto_approve, Some(Vec::new()));
    }

    #[test]
    fn inherited_workflow_shell_rules_are_ignored_when_agent_disables_bash() {
        let agent_rule = crate::tools::ShellPolicyRule {
            pattern: "^git status$".to_string(),
            decision: crate::tools::ShellDecision::Review("Agent review".to_string()),
            description: None,
        };
        let agent_config = AgentConfig {
            available_tools: Some(vec!["read_file".to_string()]),
            shell_policy: Some(vec![agent_rule]),
            ..AgentConfig::default()
        };
        let inherited_config = AgentConfig {
            shell_policy: Some(vec![crate::tools::ShellPolicyRule {
                pattern: "^git diff$".to_string(),
                decision: crate::tools::ShellDecision::Allow,
                description: None,
            }]),
            ..AgentConfig::default()
        };

        let merged = merge_inherited_workflow_config(&agent_config, &inherited_config);
        let shell_policy = merged
            .shell_policy
            .expect("agent shell policy should remain");
        assert_eq!(shell_policy.len(), 1);
        assert_eq!(shell_policy[0].pattern, "^git status$");
    }

    #[test]
    fn fresh_workflow_creation_disables_runtime_options_without_changing_agent_defaults() {
        let agent = Agent::new(
            "agent-test".to_string(),
            "Agent Test".to_string(),
            None,
            Some("primary".to_string()),
            None,
            "You are a test agent.".to_string(),
            None,
            None,
            Some("[]".to_string()),
            Some("[]".to_string()),
            None,
            Some("[]".to_string()),
            Some("[]".to_string()),
            Some(true),
            Some("default".to_string()),
            Some(true),
            Some("[]".to_string()),
            Some("planning".to_string()),
            Some(false),
            Some(false),
            Some(128_000),
        );
        let request = CreateWorkflowRequest {
            user_query: None,
            agent_id: agent.id.clone(),
            allowed_paths: None,
            auto_approve_plan: None,
            final_audit: None,
            inherited_agent_config: None,
        };

        let config = build_workflow_config_for_request(&agent, &request);

        assert_eq!(config.phase.as_deref(), Some("standard"));
        assert_eq!(config.auto_approve_plan, Some(false));
        assert_eq!(config.auto_compress, Some(false));
        assert_eq!(config.final_audit, Some(false));
        assert_eq!(config.final_review_mode.as_deref(), Some("off"));
    }

    #[test]
    fn workflow_creation_request_cannot_override_agent_tool_capabilities() {
        let mut agent = Agent::new(
            "agent-test".to_string(),
            "Agent Test".to_string(),
            None,
            Some("primary".to_string()),
            None,
            "You are a test agent.".to_string(),
            None,
            None,
            Some(
                serde_json::to_string(&vec![
                    "read_file".to_string(),
                    "server__MCP__new_tool".to_string(),
                ])
                .expect("serialize available tools"),
            ),
            Some(serde_json::to_string(&vec!["read_file"]).expect("serialize auto approve")),
            None,
            Some("[]".to_string()),
            Some("[\"/agent\"]".to_string()),
            Some(false),
            Some("default".to_string()),
            Some(true),
            Some("[]".to_string()),
            Some("standard".to_string()),
            Some(false),
            Some(false),
            Some(128_000),
        );
        agent.mcp_tool_exposure = Some(
            serde_json::to_string(&vec!["server__MCP__new_tool"]).expect("serialize MCP exposure"),
        );

        let request = CreateWorkflowRequest {
            user_query: Some("test".to_string()),
            agent_id: agent.id.clone(),
            allowed_paths: Some(json!(["/workflow"])),
            auto_approve_plan: Some(true),
            final_audit: Some(false),
            inherited_agent_config: Some(
                serde_json::to_string(&AgentConfig {
                    available_tools: Some(vec![
                        "server__MCP__removed_tool".to_string(),
                        crate::tools::TOOL_BASH.to_string(),
                    ]),
                    mcp_tool_exposure: Some(Vec::new()),
                    auto_approve: Some(vec![
                        "server__MCP__new_tool".to_string(),
                        "server__MCP__removed_tool".to_string(),
                    ]),
                    shell_policy: Some(vec![crate::tools::ShellPolicyRule {
                        pattern: "^git diff$".to_string(),
                        decision: crate::tools::ShellDecision::Allow,
                        description: None,
                    }]),
                    allowed_paths: Some(vec!["/inherited".to_string()]),
                    approval_level: Some("smart".to_string()),
                    auto_approve_plan: Some(false),
                    final_audit: Some(true),
                    final_review_mode: Some("sub_agent_review".to_string()),
                    ..AgentConfig::default()
                })
                .expect("serialize inherited config"),
            ),
        };

        let persisted =
            AgentConfig::from_json(&build_workflow_config_for_request(&agent, &request).to_json())
                .expect("persisted config should deserialize");

        assert_eq!(
            persisted.available_tools,
            Some(vec![
                "read_file".to_string(),
                "server__MCP__new_tool".to_string(),
            ])
        );
        assert_eq!(
            persisted.mcp_tool_exposure,
            Some(vec!["server__MCP__new_tool".to_string()])
        );
        assert_eq!(
            persisted.auto_approve,
            Some(vec![
                "read_file".to_string(),
                "server__MCP__new_tool".to_string()
            ])
        );
        assert!(persisted.shell_policy.is_some_and(|rules| rules.is_empty()));
        assert_eq!(
            persisted.allowed_paths,
            Some(vec!["/inherited".to_string()])
        );
        assert_eq!(persisted.auto_approve_plan, Some(true));
        assert_eq!(persisted.final_audit, Some(false));
        assert_eq!(persisted.final_review_mode.as_deref(), Some("off"));
        assert_eq!(persisted.approval_level, Some("smart".to_string()));
    }

    #[tokio::test]
    async fn test_manual_clear_context_clears_persisted_todo_list() {
        let store = create_test_store();
        let main_store = Arc::new(store);
        let session_id = "manual-clear-todos";
        seed_agent(&main_store, "agent-test");
        main_store
            .create_workflow(session_id, "Initial query", "agent-test", None, None)
            .expect("failed to create workflow");
        main_store
            .update_workflow_todo_list(
                session_id,
                r#"[{"subject":"unfinished","status":"in_progress"}]"#,
            )
            .expect("failed to seed todo list");

        clear_persisted_workflow_todo_list(&main_store, session_id)
            .await
            .expect("todo list clear should succeed");

        let todos = main_store
            .get_todo_list_for_workflow(session_id)
            .expect("failed to read todo list");
        assert!(todos.is_empty());
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

    #[test]
    fn test_hydrate_execution_context_for_snapshot_recovers_tokens_without_snapshot() {
        let runtime = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
        let TestStore { _temp_dir, store } = create_test_store();
        let store = Arc::new(store);
        let session_id = "snapshot-hydrate-context";
        {
            let store_guard = store.as_ref();
            seed_agent(&store_guard, "agent-test");
            store_guard
                .create_workflow(session_id, "Initial query", "agent-test", None, None)
                .expect("failed to create workflow");
            store_guard
                .add_workflow_message(&WorkflowMessage {
                    id: None,
                    session_id: session_id.to_string(),
                    role: "user".to_string(),
                    message: "Please inspect the failing workflow and explain the root cause"
                        .to_string(),
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
                })
                .expect("failed to add workflow message");
        }

        let hydrated = runtime.block_on(hydrate_execution_context_for_snapshot(
            store,
            session_id,
            Some("pending"),
            None,
        ));

        let hydrated = hydrated.expect("expected execution context to be recovered");
        assert_eq!(hydrated.current_segment_id, 1);
        assert_eq!(hydrated.max_context_tokens, Some(4096));
        assert!(
            hydrated
                .current_context_tokens
                .is_some_and(|tokens| tokens > 0),
            "recovered snapshot should expose a positive current_context_tokens value"
        );
    }

    #[test]
    fn test_hydrate_execution_context_prefers_terminal_workflow_status() {
        let runtime = tokio::runtime::Runtime::new().expect("failed to create tokio runtime");
        let TestStore { _temp_dir, store } = create_test_store();
        let store = Arc::new(store);
        let mut context = ExecutionContext::new("cancelled-snapshot".to_string());
        context.state = RuntimeState::Running;
        context.wait_reason = Some(WaitReason::Approval);
        context.current_context_tokens = Some(128);

        let hydrated = runtime.block_on(hydrate_execution_context_for_snapshot(
            store,
            "cancelled-snapshot",
            Some("cancelled"),
            Some(context),
        ));

        let hydrated = hydrated.expect("expected execution context to be preserved");
        assert_eq!(hydrated.state, RuntimeState::Cancelled);
        assert_eq!(hydrated.wait_reason, None);
        assert_eq!(hydrated.current_context_tokens, Some(128));
    }
}
