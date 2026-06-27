use crate::ai::interaction::chat_completion::ChatState;
use crate::commands::workflow::workflow_start;
use crate::db::{
    MainStore, WorkflowAutomation, WorkflowAutomationRunInsert, WorkflowAutomationUpsert,
};
use crate::libs::tsid::TsidGenerator;
use crate::workflow::automation::types::{
    DailyScheduleConfig, IntervalScheduleConfig, OnceScheduleConfig, WorkflowAutomationRequest,
    WorkflowAutomationRunNowResult, WorkflowAutomationShellConfig,
};
use crate::workflow::react::gateway::TauriGateway;
use crate::workflow::react::manager::WorkflowManager;
use crate::workflow::react::orchestrator::SubAgentFactory;
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveTime, TimeZone, Timelike};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, State};
use tokio::process::Command;

pub fn normalize_datetime_for_db(datetime: DateTime<Local>) -> String {
    datetime.format("%Y-%m-%d %H:%M:%S").to_string()
}

const SHELL_PROMPT_OUTPUT_LIMIT: usize = 8_000;
const SHELL_ATTACHED_CONTEXT_LIMIT: usize = 16_000;

#[derive(Debug, Clone)]
struct PreWorkflowShellResult {
    command: String,
    stdout: String,
    stderr: String,
}

fn parse_date(value: &Option<String>) -> Result<Option<NaiveDate>, String> {
    value
        .as_deref()
        .map(|raw| NaiveDate::parse_from_str(raw, "%Y-%m-%d").map_err(|e| e.to_string()))
        .transpose()
}

fn parse_time(value: &str) -> Result<NaiveTime, String> {
    NaiveTime::parse_from_str(value, "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(value, "%H:%M:%S"))
        .map_err(|e| e.to_string())
}

fn local_datetime(date: NaiveDate, time: NaiveTime) -> Result<DateTime<Local>, String> {
    let naive = date.and_time(time);
    Local
        .from_local_datetime(&naive)
        .single()
        .or_else(|| Local.from_local_datetime(&naive).earliest())
        .ok_or_else(|| format!("Invalid local datetime: {}", naive))
}

fn matches_weekday(date: NaiveDate, weekdays: &[u32]) -> bool {
    if weekdays.is_empty() {
        return true;
    }
    let weekday = date.weekday().number_from_monday();
    weekdays.contains(&weekday)
}

fn within_effective_range(
    date: NaiveDate,
    start_date: Option<NaiveDate>,
    end_date: Option<NaiveDate>,
) -> bool {
    if let Some(start) = start_date {
        if date < start {
            return false;
        }
    }
    if let Some(end) = end_date {
        if date > end {
            return false;
        }
    }
    true
}

fn next_daily_run_from(
    config: DailyScheduleConfig,
    now: DateTime<Local>,
) -> Result<Option<String>, String> {
    let time = parse_time(&config.time)?;
    let start_date = parse_date(&config.start_date)?;
    let end_date = parse_date(&config.end_date)?;
    let mut date = start_date.unwrap_or_else(|| now.date_naive());

    if date < now.date_naive() {
        date = now.date_naive();
    }

    for offset in 0..=370 {
        let candidate_date = date
            .checked_add_signed(Duration::days(offset))
            .ok_or_else(|| "Date calculation overflow".to_string())?;
        if !within_effective_range(candidate_date, start_date, end_date) {
            continue;
        }
        if !matches_weekday(candidate_date, &config.weekdays) {
            continue;
        }
        let candidate = local_datetime(candidate_date, time)?;
        if candidate > now {
            return Ok(Some(normalize_datetime_for_db(candidate)));
        }
    }

    Ok(None)
}

fn next_interval_run_from(
    config: IntervalScheduleConfig,
    now: DateTime<Local>,
) -> Result<Option<String>, String> {
    let interval_minutes = i64::from(config.interval_minutes.max(5));
    let start_date = parse_date(&config.start_date)?;
    let end_date = parse_date(&config.end_date)?;
    let anchor_time = config
        .anchor_time
        .as_deref()
        .map(parse_time)
        .transpose()?
        .unwrap_or_else(|| now.time().with_second(0).unwrap_or_else(|| now.time()));
    let mut candidate =
        local_datetime(start_date.unwrap_or_else(|| now.date_naive()), anchor_time)?;

    while candidate <= now {
        candidate += Duration::minutes(interval_minutes);
    }

    for _ in 0..10000 {
        let date = candidate.date_naive();
        if within_effective_range(date, start_date, end_date)
            && matches_weekday(date, &config.weekdays)
        {
            return Ok(Some(normalize_datetime_for_db(candidate)));
        }
        candidate += Duration::minutes(interval_minutes);
    }

    Ok(None)
}

fn next_once_run_from(
    config: OnceScheduleConfig,
    now: DateTime<Local>,
) -> Result<Option<String>, String> {
    let run_at = match DateTime::parse_from_rfc3339(&config.run_at)
        .map(|dt| dt.with_timezone(&Local))
    {
        Ok(value) => value,
        Err(_) => {
            let naive = chrono::NaiveDateTime::parse_from_str(&config.run_at, "%Y-%m-%d %H:%M:%S")
                .or_else(|_| {
                    chrono::NaiveDateTime::parse_from_str(&config.run_at, "%Y-%m-%d %H:%M")
                })
                .map_err(|e| e.to_string())?;
            Local
                .from_local_datetime(&naive)
                .single()
                .or_else(|| Local.from_local_datetime(&naive).earliest())
                .ok_or_else(|| format!("Invalid local datetime: {}", naive))?
        }
    };

    if run_at > now {
        Ok(Some(normalize_datetime_for_db(run_at)))
    } else {
        Ok(None)
    }
}

pub fn compute_next_run_at(
    schedule_kind: &str,
    schedule_config: &Value,
) -> Result<Option<String>, String> {
    compute_next_run_at_from(schedule_kind, schedule_config, Local::now())
}

fn compute_next_run_at_from(
    schedule_kind: &str,
    schedule_config: &Value,
    now: DateTime<Local>,
) -> Result<Option<String>, String> {
    match schedule_kind {
        "daily" => next_daily_run_from(
            serde_json::from_value(schedule_config.clone()).map_err(|e| e.to_string())?,
            now,
        ),
        "interval" => {
            next_interval_run_from(normalize_interval_schedule_config(schedule_config)?, now)
        }
        "once" => next_once_run_from(
            serde_json::from_value(schedule_config.clone()).map_err(|e| e.to_string())?,
            now,
        ),
        other => Err(format!("Unsupported schedule kind: {}", other)),
    }
}

fn normalize_interval_schedule_config(
    schedule_config: &Value,
) -> Result<IntervalScheduleConfig, String> {
    let mut normalized = schedule_config.clone();
    if let Some(object) = normalized.as_object_mut() {
        if !object.contains_key("interval_minutes") {
            if let Some(interval_hours) = object
                .remove("interval_hours")
                .and_then(|value| value.as_u64())
            {
                object.insert(
                    "interval_minutes".to_string(),
                    Value::from(interval_hours.saturating_mul(60)),
                );
            }
        }
    }

    serde_json::from_value(normalized).map_err(|e| e.to_string())
}

fn validate_request(request: &WorkflowAutomationRequest) -> Result<(), String> {
    if request.title.trim().is_empty() {
        return Err("Automation title is required".to_string());
    }
    if request.agent_id.trim().is_empty() {
        return Err("Automation agent is required".to_string());
    }
    if request.prompt.as_deref().unwrap_or("").trim().is_empty()
        && request
            .prompt_file_path
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
    {
        return Err("Automation prompt or prompt file is required".to_string());
    }
    validate_shell_config(request.shell_config.as_ref())?;
    compute_next_run_at(&request.schedule_kind, &request.schedule_config)?;
    Ok(())
}

fn normalized_continuous_context(request: &WorkflowAutomationRequest) -> bool {
    request.schedule_kind != "once" && request.continuous_context
}

fn build_agent_config(request: &WorkflowAutomationRequest) -> Result<String, String> {
    let mut value = request.agent_config.clone().unwrap_or_else(|| json!({}));
    if !value.is_object() {
        value = json!({});
    }
    if let Some(object) = value.as_object_mut() {
        object.insert("allowedPaths".to_string(), json!(request.allowed_paths));
        object.insert("finalAudit".to_string(), json!(request.self_review));
        object.insert("approvalLevel".to_string(), json!("full"));
    }
    serde_json::to_string(&value).map_err(|e| e.to_string())
}

fn request_to_upsert(
    mut request: WorkflowAutomationRequest,
    id: String,
    existing_workflow_session_id: Option<String>,
) -> Result<WorkflowAutomationUpsert, String> {
    if let Some(prompt_from_file) = read_prompt_file_if_exists(request.prompt_file_path.as_deref())?
    {
        request.prompt = Some(prompt_from_file);
    }
    validate_request(&request)?;
    let next_run_at = if request.enabled {
        compute_next_run_at(&request.schedule_kind, &request.schedule_config)?
    } else {
        None
    };
    let agent_config = build_agent_config(&request)?;
    let allowed_paths = serde_json::to_string(&request.allowed_paths).map_err(|e| e.to_string())?;
    let shell_config = build_shell_config(&request)?;
    let schedule_config =
        serde_json::to_string(&request.schedule_config).map_err(|e| e.to_string())?;

    let continuous_context = normalized_continuous_context(&request);

    Ok(WorkflowAutomationUpsert {
        id,
        title: request.title.trim().to_string(),
        prompt: request.prompt,
        prompt_file_path: request.prompt_file_path,
        agent_id: request.agent_id,
        agent_config: Some(agent_config),
        allowed_paths,
        shell_config,
        schedule_kind: request.schedule_kind,
        schedule_config,
        continuous_context,
        current_workflow_session_id: if continuous_context {
            existing_workflow_session_id
        } else {
            None
        },
        self_review: request.self_review,
        enabled: request.enabled,
        next_run_at,
    })
}

fn parse_allowed_paths(raw: &str) -> Vec<String> {
    serde_json::from_str(raw).unwrap_or_default()
}

fn normalize_shell_config(
    value: Option<Value>,
) -> Result<Option<WorkflowAutomationShellConfig>, String> {
    let Some(value) = value else {
        return Ok(None);
    };

    let mut config: WorkflowAutomationShellConfig =
        serde_json::from_value(value).map_err(|e| e.to_string())?;
    if let Some(command) = config.command.as_mut() {
        *command = command.trim().to_string();
    }
    if let Some(file_path) = config.file_path.as_mut() {
        *file_path = file_path.trim().to_string();
    }
    if let Some(args) = config.args.as_mut() {
        *args = args.trim().to_string();
    }

    if config
        .command
        .as_deref()
        .is_none_or(|command| command.is_empty())
    {
        config.command = None;
    }
    if config
        .file_path
        .as_deref()
        .is_none_or(|file_path| file_path.is_empty())
    {
        config.file_path = None;
    }
    if config.args.as_deref().is_none_or(|args| args.is_empty()) {
        config.args = None;
    }

    if config.command.is_none() {
        if let Some(file_path) = config.file_path.as_deref() {
            let mut command = file_path.to_string();
            if let Some(args) = config.args.as_deref() {
                command.push(' ');
                command.push_str(args);
            }
            config.command = Some(command);
        }
    }

    if config.command.is_none() {
        return Ok(None);
    }

    Ok(Some(config))
}

fn validate_shell_config(value: Option<&Value>) -> Result<(), String> {
    normalize_shell_config(value.cloned()).map(|_| ())
}

fn build_shell_config(request: &WorkflowAutomationRequest) -> Result<Option<String>, String> {
    normalize_shell_config(request.shell_config.clone())?
        .map(|config| serde_json::to_string(&config).map_err(|e| e.to_string()))
        .transpose()
}

fn parse_shell_config(raw: Option<&str>) -> Result<Option<WorkflowAutomationShellConfig>, String> {
    let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return Ok(None);
    };

    let value = serde_json::from_str::<Value>(raw).map_err(|e| e.to_string())?;
    normalize_shell_config(Some(value))
}

fn automation_prompt(automation: &WorkflowAutomation) -> Result<String, String> {
    if let Some(prompt_from_file) =
        read_prompt_file_if_exists(automation.prompt_file_path.as_deref())?
    {
        return Ok(prompt_from_file);
    }

    if let Some(prompt) = automation
        .prompt
        .as_deref()
        .map(str::trim)
        .filter(|prompt| !prompt.is_empty())
    {
        return Ok(prompt.to_string());
    }

    let path = automation
        .prompt_file_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| "Automation prompt is empty".to_string())?;
    std::fs::read_to_string(PathBuf::from(path)).map_err(|e| e.to_string())
}

fn read_prompt_file_if_exists(path: Option<&str>) -> Result<Option<String>, String> {
    let Some(path) = path.map(str::trim).filter(|path| !path.is_empty()) else {
        return Ok(None);
    };

    let path_buf = PathBuf::from(path);
    if !path_buf.exists() {
        return Ok(None);
    }

    std::fs::read_to_string(&path_buf)
        .map(Some)
        .map_err(|e| e.to_string())
}

fn truncate_text(value: &str, limit: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= limit {
        return value.to_string();
    }

    let truncated: String = value.chars().take(limit).collect();
    format!(
        "{}\n...[truncated {} chars]",
        truncated,
        char_count.saturating_sub(limit)
    )
}

fn shell_result_prompt_suffix(result: &PreWorkflowShellResult) -> String {
    let stdout = truncate_text(&result.stdout, SHELL_PROMPT_OUTPUT_LIMIT);
    let stderr = truncate_text(&result.stderr, SHELL_PROMPT_OUTPUT_LIMIT);

    format!(
        "<automation_shell_result>\ncommand: {}\nstdout:\n{}\nstderr:\n{}\n</automation_shell_result>",
        result.command, stdout, stderr
    )
}

fn shell_result_attached_context(result: &PreWorkflowShellResult) -> String {
    format!(
        "<automation_shell_context>\n{}\n</automation_shell_context>",
        truncate_text(
            &shell_result_prompt_suffix(result),
            SHELL_ATTACHED_CONTEXT_LIMIT
        )
    )
}

fn build_workflow_prompt(prompt: &str, shell_result: Option<&PreWorkflowShellResult>) -> String {
    let Some(shell_result) = shell_result else {
        return prompt.to_string();
    };

    format!("{}\n\n{}", prompt, shell_result_prompt_suffix(shell_result))
}

fn combine_context_sections(sections: impl IntoIterator<Item = Option<String>>) -> Option<String> {
    let combined = sections
        .into_iter()
        .flatten()
        .filter(|section| !section.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

    if combined.trim().is_empty() {
        None
    } else {
        Some(combined)
    }
}

async fn execute_shell_command(command_text: &str) -> Result<std::process::Output, String> {
    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("cmd");
        command.args(["/C", command_text]);
        command
    };

    #[cfg(not(windows))]
    let mut command = {
        let mut command = Command::new("sh");
        command.args(["-c", command_text]);
        command
    };

    command
        .output()
        .await
        .map_err(|e| format!("Failed to execute shell command: {}", e))
}

async fn execute_pre_workflow_shell(
    automation: &WorkflowAutomation,
) -> Result<Option<PreWorkflowShellResult>, String> {
    let Some(shell_config) = parse_shell_config(automation.shell_config.as_deref())? else {
        return Ok(None);
    };

    let command_text = shell_config
        .command
        .clone()
        .ok_or_else(|| "Shell command is required".to_string())?;
    let output = execute_shell_command(&command_text).await?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() {
        let status = output.status.code().unwrap_or(-1);
        let detail = if !stderr.is_empty() { &stderr } else { &stdout };
        return Err(format!(
            "Pre-workflow shell failed with exit code {}: {}",
            status,
            truncate_text(detail, 500)
        ));
    }

    Ok(Some(PreWorkflowShellResult {
        command: command_text,
        stdout,
        stderr,
    }))
}

pub fn save_automation(
    tsid_generator: &Arc<TsidGenerator>,
    store: &MainStore,
    request: WorkflowAutomationRequest,
) -> Result<WorkflowAutomation, String> {
    let id = request
        .id
        .clone()
        .filter(|id| !id.trim().is_empty())
        .unwrap_or_else(|| {
            tsid_generator
                .generate()
                .unwrap_or_else(|_| uuid::Uuid::new_v4().to_string())
        });
    let existing_workflow_session_id = store
        .get_workflow_automation(&id)
        .map_err(|e| e.to_string())?
        .and_then(|automation| automation.current_workflow_session_id);
    let upsert = request_to_upsert(request, id, existing_workflow_session_id)?;
    store
        .upsert_workflow_automation(&upsert)
        .map_err(|e| e.to_string())
}

pub fn set_automation_enabled(store: &MainStore, id: &str, enabled: bool) -> Result<(), String> {
    let automation = store
        .get_workflow_automation(id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Automation {} not found", id))?;
    let schedule_config: Value =
        serde_json::from_str(&automation.schedule_config).map_err(|e| e.to_string())?;
    let next_run_at = if enabled {
        compute_next_run_at(&automation.schedule_kind, &schedule_config)?
    } else {
        None
    };
    store
        .set_workflow_automation_enabled(id, enabled, next_run_at)
        .map_err(|e| e.to_string())
}

pub fn advance_automation_after_scheduler_tick(
    store: &MainStore,
    automation: &WorkflowAutomation,
) -> Result<(), String> {
    let schedule_config: Value =
        serde_json::from_str(&automation.schedule_config).map_err(|e| e.to_string())?;
    let next_run_at = compute_next_run_at(&automation.schedule_kind, &schedule_config)?;
    let enabled = automation.schedule_kind != "once" || next_run_at.is_some();
    store
        .set_workflow_automation_enabled(&automation.id, enabled, next_run_at)
        .map_err(|e| e.to_string())
}

pub async fn run_automation_now(
    app: AppHandle,
    state: State<'_, Arc<std::sync::RwLock<MainStore>>>,
    chat_state: State<'_, Arc<ChatState>>,
    tsid_generator: State<'_, Arc<TsidGenerator>>,
    gateway: State<'_, Arc<TauriGateway>>,
    factory: State<'_, Arc<dyn SubAgentFactory>>,
    workflow_manager: State<'_, Arc<WorkflowManager>>,
    automation_id: String,
) -> Result<WorkflowAutomationRunNowResult, String> {
    let automation = {
        let store = state.read().map_err(|e| e.to_string())?;
        store
            .get_workflow_automation(&automation_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Automation {} not found", automation_id))?
    };
    let run_id = tsid_generator.generate().map_err(|e| e.to_string())?;
    let scheduled_for = normalize_datetime_for_db(Local::now());

    let run = {
        let store = state.read().map_err(|e| e.to_string())?;
        store
            .add_workflow_automation_run(&WorkflowAutomationRunInsert {
                id: run_id.clone(),
                automation_id: automation.id.clone(),
                workflow_session_id: None,
                status: "pending".to_string(),
                scheduled_for: scheduled_for.clone(),
                started_at: None,
                finished_at: None,
                error: None,
            })
            .map_err(|e| e.to_string())?
    };

    let prompt = automation_prompt(&automation)?;
    let shell_result = match execute_pre_workflow_shell(&automation).await {
        Ok(result) => result,
        Err(error) => {
            let store = state.read().map_err(|e| e.to_string())?;
            if let Err(update_error) = store.update_workflow_automation_run_failed(&run_id, &error)
            {
                log::error!(
                    "[WorkflowAutomation][automation={}][run={}] Failed to mark shell failure: {}",
                    automation.id,
                    run_id,
                    update_error
                );
            }
            return Err(error);
        }
    };
    let initial_prompt = build_workflow_prompt(&prompt, shell_result.as_ref());
    let workflow_session_id = {
        let existing_session_id = automation
            .current_workflow_session_id
            .clone()
            .filter(|_| automation.continuous_context);

        if let Some(session_id) = existing_session_id {
            let store = state.read().map_err(|e| e.to_string())?;
            let exists = store
                .get_workflow(&session_id)
                .map_err(|e| e.to_string())?
                .is_some();
            if exists {
                session_id
            } else {
                tsid_generator.generate().map_err(|e| e.to_string())?
            }
        } else {
            tsid_generator.generate().map_err(|e| e.to_string())?
        }
    };

    let workflow_exists = {
        let store = state.read().map_err(|e| e.to_string())?;
        store
            .get_workflow(&workflow_session_id)
            .map_err(|e| e.to_string())?
            .is_some()
    };

    if !workflow_exists {
        let store = state.read().map_err(|e| e.to_string())?;
        store
            .create_workflow(
                &workflow_session_id,
                &prompt,
                &automation.agent_id,
                automation.agent_config.clone(),
                None,
            )
            .map_err(|e| e.to_string())?;
    }

    let allowed_paths = parse_allowed_paths(&automation.allowed_paths);
    let mut metadata = json!({
        "automation_id": automation.id.clone(),
        "automation_run_id": run_id.clone(),
        "scheduled_for": scheduled_for,
    });

    if let Some(shell_result) = shell_result.as_ref() {
        metadata["shell"] = json!({
            "command": shell_result.command,
        });
    }

    let allowed_paths_context = if allowed_paths.is_empty() {
        None
    } else {
        Some(format!(
            "<automation_allowed_paths>{}</automation_allowed_paths>",
            allowed_paths.join("\n")
        ))
    };
    let attached_context = combine_context_sections([
        allowed_paths_context,
        shell_result.as_ref().map(shell_result_attached_context),
    ]);

    {
        let store = state.read().map_err(|e| e.to_string())?;
        store
            .update_workflow_automation_run_after_start(
                &automation.id,
                &run_id,
                &workflow_session_id,
                &scheduled_for,
                automation
                    .continuous_context
                    .then_some(workflow_session_id.as_str()),
            )
            .map_err(|e| e.to_string())?;
    }

    let state_after_start = state.inner().clone();

    if let Err(error) = workflow_start(
        app,
        state,
        chat_state,
        tsid_generator,
        gateway,
        factory,
        workflow_manager,
        workflow_session_id.clone(),
        automation.agent_id.clone(),
        Some(initial_prompt),
        Some(metadata),
        attached_context,
        Some(false),
    )
    .await
    {
        let store = state_after_start.read().map_err(|e| e.to_string())?;
        if let Err(update_error) = store.update_workflow_automation_run_failed(&run_id, &error) {
            log::error!(
                "[WorkflowAutomation][automation={}][run={}] Failed to mark run failed after start error: {}",
                automation.id,
                run_id,
                update_error
            );
        }
        return Err(error);
    }

    Ok(WorkflowAutomationRunNowResult {
        automation,
        run,
        workflow_session_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_now(value: &str) -> DateTime<Local> {
        let naive = chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
            .expect("valid test datetime");
        Local
            .from_local_datetime(&naive)
            .single()
            .or_else(|| Local.from_local_datetime(&naive).earliest())
            .expect("valid local test datetime")
    }

    #[test]
    fn daily_schedule_triggers_later_today_when_time_is_future() {
        let now = test_now("2026-06-25 08:30:00");
        let next = compute_next_run_at_from(
            "daily",
            &json!({
                "time": "09:00",
                "weekdays": [1, 2, 3, 4, 5, 6, 7],
                "start_date": null,
                "end_date": null
            }),
            now,
        )
        .expect("daily schedule should compute");

        assert_eq!(next, Some("2026-06-25 09:00:00".to_string()));
    }

    #[test]
    fn daily_schedule_skips_unselected_weekdays() {
        let now = test_now("2026-06-25 08:30:00");
        let next = compute_next_run_at_from(
            "daily",
            &json!({
                "time": "09:00",
                "weekdays": [5],
                "start_date": null,
                "end_date": null
            }),
            now,
        )
        .expect("daily schedule should compute");

        assert_eq!(next, Some("2026-06-26 09:00:00".to_string()));
    }

    #[test]
    fn daily_schedule_returns_none_after_effective_range() {
        let now = test_now("2026-06-25 10:00:00");
        let next = compute_next_run_at_from(
            "daily",
            &json!({
                "time": "09:00",
                "weekdays": [1, 2, 3, 4, 5, 6, 7],
                "start_date": "2026-06-20",
                "end_date": "2026-06-25"
            }),
            now,
        )
        .expect("daily schedule should compute");

        assert_eq!(next, None);
    }

    #[test]
    fn daily_schedule_waits_until_effective_range_start() {
        let now = test_now("2026-06-25 10:00:00");
        let next = compute_next_run_at_from(
            "daily",
            &json!({
                "time": "09:00",
                "weekdays": [1, 2, 3, 4, 5, 6, 7],
                "start_date": "2026-06-27",
                "end_date": "2026-06-30"
            }),
            now,
        )
        .expect("daily schedule should compute");

        assert_eq!(next, Some("2026-06-27 09:00:00".to_string()));
    }

    #[test]
    fn interval_schedule_triggers_next_interval_slot() {
        let now = test_now("2026-06-25 10:30:00");
        let next = compute_next_run_at_from(
            "interval",
            &json!({
                "interval_minutes": 180,
                "weekdays": [1, 2, 3, 4, 5, 6, 7],
                "start_date": "2026-06-25",
                "end_date": null,
                "anchor_time": "09:00"
            }),
            now,
        )
        .expect("interval schedule should compute");

        assert_eq!(next, Some("2026-06-25 12:00:00".to_string()));
    }

    #[test]
    fn interval_schedule_skips_to_next_allowed_weekday() {
        let now = test_now("2026-06-25 10:30:00");
        let next = compute_next_run_at_from(
            "interval",
            &json!({
                "interval_minutes": 360,
                "weekdays": [5],
                "start_date": "2026-06-25",
                "end_date": null,
                "anchor_time": "09:00"
            }),
            now,
        )
        .expect("interval schedule should compute");

        assert_eq!(next, Some("2026-06-26 03:00:00".to_string()));
    }

    #[test]
    fn interval_schedule_returns_none_after_effective_range() {
        let now = test_now("2026-06-26 00:30:00");
        let next = compute_next_run_at_from(
            "interval",
            &json!({
                "interval_minutes": 120,
                "weekdays": [1, 2, 3, 4, 5, 6, 7],
                "start_date": "2026-06-25",
                "end_date": "2026-06-25",
                "anchor_time": "09:00"
            }),
            now,
        )
        .expect("interval schedule should compute");

        assert_eq!(next, None);
    }

    #[test]
    fn interval_schedule_accepts_legacy_hour_config() {
        let now = test_now("2026-06-25 10:30:00");
        let next = compute_next_run_at_from(
            "interval",
            &json!({
                "interval_hours": 3,
                "weekdays": [1, 2, 3, 4, 5, 6, 7],
                "start_date": "2026-06-25",
                "end_date": null,
                "anchor_time": "09:00"
            }),
            now,
        )
        .expect("legacy interval schedule should compute");

        assert_eq!(next, Some("2026-06-25 12:00:00".to_string()));
    }

    #[test]
    fn once_schedule_triggers_only_when_future() {
        let now = test_now("2026-06-25 10:30:00");
        let future =
            compute_next_run_at_from("once", &json!({ "run_at": "2026-06-25 11:00" }), now)
                .expect("future once schedule should compute");
        let past = compute_next_run_at_from("once", &json!({ "run_at": "2026-06-25 10:00" }), now)
            .expect("past once schedule should compute");

        assert_eq!(future, Some("2026-06-25 11:00:00".to_string()));
        assert_eq!(past, None);
    }

    #[test]
    fn automation_agent_config_forces_full_approval() {
        let request = WorkflowAutomationRequest {
            id: None,
            title: "Nightly task".to_string(),
            prompt: Some("Run checks".to_string()),
            prompt_file_path: None,
            agent_id: "agent-1".to_string(),
            agent_config: Some(json!({
                "approvalLevel": "default",
                "models": {
                    "act": { "id": 1, "model": "test-model" }
                }
            })),
            allowed_paths: vec!["/tmp/project".to_string()],
            shell_config: None,
            schedule_kind: "daily".to_string(),
            schedule_config: json!({
                "time": "09:00",
                "weekdays": [1, 2, 3, 4, 5, 6, 7],
                "start_date": null,
                "end_date": null
            }),
            continuous_context: true,
            self_review: false,
            enabled: true,
        };

        let config = build_agent_config(&request).expect("agent config should serialize");
        let config: Value = serde_json::from_str(&config).expect("valid agent config json");

        assert_eq!(config["approvalLevel"], "full");
        assert_eq!(config["allowedPaths"], json!(["/tmp/project"]));
        assert_eq!(config["finalAudit"], false);
    }
}
