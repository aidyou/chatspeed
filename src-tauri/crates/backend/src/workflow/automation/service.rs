use crate::commands::workflow::workflow_start_core;
use crate::db::automation::ManualClaimOutcome;
use crate::db::{WorkflowAutomation, WorkflowAutomationRun, WorkflowAutomationUpsert};
use crate::error_diagnostics::{log_error_diagnostic, ErrorProjection};
use crate::owned_process::{run_owned_shell, OwnedProcessCancellation, OwnedShellOutcome};
use crate::workflow::automation::errors::AutomationError;
use crate::workflow::automation::types::{
    DailyScheduleConfig, IntervalScheduleConfig, OnceScheduleConfig, WorkflowAutomationRequest,
    WorkflowAutomationShellConfig,
};
use crate::workflow::react::application::WorkflowApplicationService;
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveTime, TimeZone, Timelike};
use serde_json::{json, Value};
use std::path::PathBuf;

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

fn normalize_daily_schedule_times(config: &DailyScheduleConfig) -> Result<Vec<NaiveTime>, String> {
    let mut raw_times = config.times.clone();
    if raw_times.is_empty() {
        if let Some(time) = config.time.as_ref() {
            raw_times.push(time.clone());
        }
    }
    if raw_times.is_empty() {
        raw_times.push("09:00".to_string());
    }

    let mut normalized = raw_times
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .map(|value| parse_time(&value))
        .collect::<Result<Vec<_>, _>>()?;

    normalized.sort_unstable();
    normalized.dedup();

    if normalized.is_empty() {
        return Err("Daily schedule requires at least one execution time".to_string());
    }

    Ok(normalized)
}

fn next_daily_run_from(
    config: DailyScheduleConfig,
    now: DateTime<Local>,
) -> Result<Option<String>, String> {
    let times = normalize_daily_schedule_times(&config)?;
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
        for time in &times {
            let candidate = local_datetime(candidate_date, *time)?;
            if candidate > now {
                return Ok(Some(normalize_datetime_for_db(candidate)));
            }
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

fn parse_once_run_at(run_at: &str) -> Result<DateTime<Local>, String> {
    if let Ok(value) = DateTime::parse_from_rfc3339(run_at) {
        return Ok(value.with_timezone(&Local));
    }
    let naive = chrono::NaiveDateTime::parse_from_str(run_at, "%Y-%m-%d %H:%M:%S")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(run_at, "%Y-%m-%d %H:%M"))
        .map_err(|e| e.to_string())?;
    Local
        .from_local_datetime(&naive)
        .single()
        .or_else(|| Local.from_local_datetime(&naive).earliest())
        .ok_or_else(|| format!("Invalid local datetime: {}", naive))
}

fn next_once_run_from(
    config: OnceScheduleConfig,
    now: DateTime<Local>,
) -> Result<Option<String>, String> {
    let run_at = parse_once_run_at(&config.run_at)?;

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

/// Computes the next occurrence strictly after `now` for the given schedule. The
/// explicit `now` keeps startup normalization and the runtime `dispatch` tick on
/// the same deterministic clock instead of each reading `Local::now()`.
pub(crate) fn compute_next_run_at_from(
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

/// Whether `slot` is a slot the schedule configuration actually defines, i.e.
/// its own date is inside the effective range, its weekday is allowed, and its
/// time matches a configured time (daily), lies on the interval grid (interval),
/// or equals the `once` instant. The runtime uses this to reject a corrupted or
/// stale cursor instead of consuming it, without any arbitrary lateness
/// threshold: the slot is validated, not how late `now` is relative to it.
pub(crate) fn is_valid_schedule_slot(
    schedule_kind: &str,
    schedule_config: &Value,
    slot: DateTime<Local>,
) -> Result<bool, String> {
    match schedule_kind {
        "daily" => {
            let config: DailyScheduleConfig =
                serde_json::from_value(schedule_config.clone()).map_err(|e| e.to_string())?;
            let times = normalize_daily_schedule_times(&config)?;
            let start_date = parse_date(&config.start_date)?;
            let end_date = parse_date(&config.end_date)?;
            let date = slot.date_naive();
            if !within_effective_range(date, start_date, end_date) {
                return Ok(false);
            }
            if !matches_weekday(date, &config.weekdays) {
                return Ok(false);
            }
            Ok(times.contains(&slot.time()))
        }
        "interval" => {
            let config = normalize_interval_schedule_config(schedule_config)?;
            let start_date = parse_date(&config.start_date)?;
            let end_date = parse_date(&config.end_date)?;
            let date = slot.date_naive();
            if !within_effective_range(date, start_date, end_date) {
                return Ok(false);
            }
            if !matches_weekday(date, &config.weekdays) {
                return Ok(false);
            }
            let Some(anchor_time) = config.anchor_time.as_deref().map(parse_time).transpose()?
            else {
                // Without a configured anchor time the interval origin floats
                // with the wall clock at computation time, so no grid is
                // reproducible: only the date range and weekday are checked.
                return Ok(true);
            };
            // The grid is compared in exact *seconds*: rounding to whole minutes
            // would accept a slot such as 09:00:59 on a 09:00 whole-minute grid.
            let step_seconds = i64::from(config.interval_minutes.max(5)) * 60;
            match start_date {
                Some(start) => {
                    let anchor = local_datetime(start, anchor_time)?;
                    let delta_seconds = slot.signed_duration_since(anchor).num_seconds();
                    Ok(interval_slot_is_on_grid(delta_seconds, step_seconds, false))
                }
                None => {
                    // No persistent anchor date: the origin floats to the day a
                    // cursor was computed on, so only the day-reduced grid is
                    // reproducible (see `interval_slot_is_on_grid`).
                    let delta_seconds = i64::from(slot.time().num_seconds_from_midnight())
                        - i64::from(anchor_time.num_seconds_from_midnight());
                    Ok(interval_slot_is_on_grid(delta_seconds, step_seconds, true))
                }
            }
        }
        "once" => {
            let config: OnceScheduleConfig =
                serde_json::from_value(schedule_config.clone()).map_err(|e| e.to_string())?;
            // Compare at second granularity: the stored cursor is written in the
            // DB second format, so sub-second digits in the configured instant
            // must not make a legitimate slot look corrupt.
            Ok(parse_once_run_at(&config.run_at)?.timestamp() == slot.timestamp())
        }
        other => Err(format!("Unsupported schedule kind: {}", other)),
    }
}

/// Whether a slot lies on an interval grid, compared in exact seconds.
///
/// With a persistent anchor (`start_date` plus `anchor_time`) the origin is
/// fixed, so the offset from it must be a non-negative multiple of the step.
/// Without a persistent anchor date the origin is `now.date_naive()` and drifts
/// by whole days between computations, so a whole day (86_400s) may be added to
/// any reachable slot; the reachable offsets are then exactly the multiples of
/// `gcd(step, 86_400)`. That is the tightest restriction the floating origin
/// permits, and it still guarantees every slot the generator itself produces
/// validates.
fn interval_slot_is_on_grid(delta_seconds: i64, step_seconds: i64, floating_origin: bool) -> bool {
    if step_seconds <= 0 {
        return false;
    }
    if floating_origin {
        delta_seconds.rem_euclid(gcd_i64(step_seconds, 86_400)) == 0
    } else {
        delta_seconds >= 0 && delta_seconds.rem_euclid(step_seconds) == 0
    }
}

fn gcd_i64(a: i64, b: i64) -> i64 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != 0 {
        let remainder = a % b;
        a = b;
        b = remainder;
    }
    a.max(1)
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

pub(crate) fn validate_automation_request(
    request: &WorkflowAutomationRequest,
) -> Result<(), String> {
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

pub(crate) fn request_to_upsert(
    mut request: WorkflowAutomationRequest,
    id: String,
    existing_workflow_session_id: Option<String>,
) -> Result<WorkflowAutomationUpsert, String> {
    if let Some(prompt_from_file) = read_prompt_file_if_exists(request.prompt_file_path.as_deref())?
    {
        request.prompt = Some(prompt_from_file);
    }
    validate_automation_request(&request)?;
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

/// The outcome of the optional pre-workflow shell.
enum PreWorkflowShell {
    /// The automation has no shell configured.
    NotConfigured,
    /// The shell completed successfully with its captured output.
    Ran(PreWorkflowShellResult),
    /// The runtime cancelled the shell while it was in flight. The whole owned
    /// tree was terminated and the root reaped; the run must project `cancelled`
    /// rather than a deterministic-effect `failed`.
    Cancelled,
}

/// Runs the optional pre-workflow shell as an owned, cancellable child process.
///
/// The shell is started in its own process session/group by
/// [`run_owned_shell`], so a coordinated shutdown reaches its whole tree and
/// awaits the teardown. A deterministic non-zero exit is an ordinary failure; a
/// cancellation is reported separately so the caller never records a cancelled
/// run as `failed`.
async fn execute_pre_workflow_shell(
    automation: &WorkflowAutomation,
    cancel: &OwnedProcessCancellation,
) -> Result<PreWorkflowShell, String> {
    let Some(shell_config) = parse_shell_config(automation.shell_config.as_deref())? else {
        return Ok(PreWorkflowShell::NotConfigured);
    };

    let command_text = shell_config
        .command
        .clone()
        .ok_or_else(|| "Shell command is required".to_string())?;
    let output = match run_owned_shell(&command_text, cancel).await? {
        OwnedShellOutcome::Cancelled => return Ok(PreWorkflowShell::Cancelled),
        OwnedShellOutcome::Completed(output) => output,
    };
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

    Ok(PreWorkflowShell::Ran(PreWorkflowShellResult {
        command: command_text,
        stdout,
        stderr,
    }))
}

/// Executes an already-created automation run against the shared workflow
/// runtime. This is the single canonical run kernel: it resolves the prompt,
/// runs the optional pre-workflow shell, creates or reuses the workflow session,
/// links the run, and starts the workflow.
///
/// A *successful start is not a completion*: the run is left in `running` and
/// only reaches a terminal state through the structured workflow snapshot/events
/// projection in the facade (AC-8/INV-7). Deterministic pre-shell / start errors
/// mark the run `failed` with a bounded, redacted reason (INV-8).
pub(crate) async fn execute_automation_run(
    svc: &WorkflowApplicationService,
    automation: &WorkflowAutomation,
    run_id: &str,
    scheduled_for: &str,
) -> Result<(), AutomationError> {
    let store = &*svc.main_store;
    let tsid_generator = &svc.tsid_generator;

    // Non-terminal transition into the execution phase.
    let _ = store.update_workflow_automation_run_status(run_id, "starting", None);

    let prompt = automation_prompt(automation).map_err(AutomationError::invalid_request)?;
    // The pre-workflow shell is owned by this runtime instance, so a coordinated
    // shutdown can cancel its whole tree and await the teardown.
    let cancel = svc.automation_owner().owned_process_cancellation();
    let shell_outcome = match execute_pre_workflow_shell(automation, &cancel).await {
        Ok(outcome) => outcome,
        Err(error) => {
            // A deterministic pre-workflow shell failure has no external effect
            // to reconcile, so it is a clean terminal `failed`.
            if let Err(update_error) =
                store.update_workflow_automation_run_status(run_id, "failed", Some(&error))
            {
                let mut diagnostic = update_error.diagnostic();
                diagnostic.module = "workflow.automation".to_string();
                diagnostic.operation = "mark_shell_failure".to_string();
                diagnostic.session_id = Some(run_id.to_string());
                diagnostic
                    .context_chain
                    .push(crate::error_diagnostics::ErrorFrame::new(
                        "workflow.automation",
                        "mark_shell_failure",
                        Some(format!("automation={}", automation.id)),
                    ));
                log_error_diagnostic(&diagnostic);
            }
            return Err(AutomationError::internal(error));
        }
    };
    let shell_result = match shell_outcome {
        PreWorkflowShell::NotConfigured => None,
        PreWorkflowShell::Ran(result) => Some(result),
        PreWorkflowShell::Cancelled => {
            // The shell was cancelled by a coordinated shutdown, not a
            // deterministic failure: the run is terminal `cancelled` and the
            // typed error is the one canonical `runtime_stopping` code, so no
            // transport mistakes it for a retryable/internal failure.
            let reason = "runtime cancelled during pre-workflow shell";
            if let Err(update_error) =
                store.update_workflow_automation_run_status(run_id, "cancelled", Some(reason))
            {
                let mut diagnostic = update_error.diagnostic();
                diagnostic.module = "workflow.automation".to_string();
                diagnostic.operation = "mark_run_cancelled".to_string();
                diagnostic.session_id = Some(run_id.to_string());
                diagnostic
                    .context_chain
                    .push(crate::error_diagnostics::ErrorFrame::new(
                        "workflow.automation",
                        "mark_run_cancelled",
                        Some(format!("automation={}", automation.id)),
                    ));
                log_error_diagnostic(&diagnostic);
            }
            return Err(AutomationError::stopping(reason));
        }
    };
    let initial_prompt = build_workflow_prompt(&prompt, shell_result.as_ref());
    let workflow_session_id = {
        let existing_session_id = automation
            .current_workflow_session_id
            .clone()
            .filter(|_| automation.continuous_context);

        if let Some(session_id) = existing_session_id {
            let exists = store
                .get_workflow(&session_id)
                .map_err(|e| AutomationError::internal_from(e))?
                .is_some();
            if exists {
                session_id
            } else {
                tsid_generator
                    .generate()
                    .map_err(|e| AutomationError::internal(e))?
            }
        } else {
            tsid_generator
                .generate()
                .map_err(|e| AutomationError::internal(e))?
        }
    };

    let workflow_exists = store
        .get_workflow(&workflow_session_id)
        .map_err(|e| AutomationError::internal_from(e))?
        .is_some();

    if !workflow_exists {
        store
            .create_workflow(
                &workflow_session_id,
                &prompt,
                &automation.agent_id,
                automation.agent_config.clone(),
                None,
            )
            .map_err(|e| AutomationError::internal_from(e))?;
    }

    let allowed_paths = parse_allowed_paths(&automation.allowed_paths);
    let mut metadata = json!({
        "automation_id": automation.id.clone(),
        "automation_run_id": run_id,
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

    store
        .update_workflow_automation_run_after_start(
            &automation.id,
            run_id,
            &workflow_session_id,
            scheduled_for,
            automation
                .continuous_context
                .then_some(workflow_session_id.as_str()),
        )
        .map_err(|e| AutomationError::internal_from(e))?;

    if let Err(error) = workflow_start_core(
        svc,
        workflow_session_id.clone(),
        automation.agent_id.clone(),
        Some(initial_prompt),
        Some(metadata),
        attached_context,
        Some(false),
    )
    .await
    {
        let origin = error.origin.clone();
        if let Err(update_error) =
            store.update_workflow_automation_run_status(run_id, "failed", Some(&error.message))
        {
            let mut diagnostic = update_error.diagnostic();
            diagnostic.module = "workflow.automation".to_string();
            diagnostic.operation = "mark_start_failure".to_string();
            diagnostic.session_id = Some(run_id.to_string());
            diagnostic
                .context_chain
                .push(crate::error_diagnostics::ErrorFrame::new(
                    "workflow.automation",
                    "mark_start_failure",
                    Some(format!("automation={}", automation.id)),
                ));
            log_error_diagnostic(&diagnostic);
        }
        let mut failure = AutomationError::internal(error.message);
        if let Some(origin) = origin {
            failure = failure.with_origin(origin);
        }
        return Err(failure);
    }

    Ok(())
}

/// Creates a pending manual run for an automation via a single atomic DB claim
/// (active-run guard + pending insert in one write transaction), then executes
/// the shared kernel. Returns the freshly re-read run so callers observe the
/// post-start lifecycle. A concurrent overlapping run yields `busy`
/// (AC-6/INV-6), closing the former check-then-insert race.
pub(crate) async fn create_manual_run(
    svc: &WorkflowApplicationService,
    automation_id: &str,
) -> Result<(WorkflowAutomation, WorkflowAutomationRun), AutomationError> {
    let store = &*svc.main_store;
    let automation = store
        .get_workflow_automation(automation_id)
        .map_err(|e| AutomationError::internal_from(e))?
        .ok_or_else(|| {
            AutomationError::not_found(format!("Automation {automation_id} not found"))
        })?;

    let run_id = svc
        .tsid_generator
        .generate()
        .map_err(|e| AutomationError::internal(e))?;
    let scheduled_for = normalize_datetime_for_db(Local::now());
    // Admit the run in this instance's short-lived evidence *before* the durable
    // claim. The claim commits the pending row, and another worker thread running
    // a facade read/reconcile could observe it before the guard existed, so
    // admitting only afterwards would leave exactly that window unprotected. The
    // RAII guard releases on drop, including the `Busy` path below, and the DB
    // claim stays the authority (no session is reserved here).
    let _in_flight = svc.automation_owner().begin_run(&run_id);
    let claimed_run = match store
        .claim_manual_run(&automation.id, &run_id, &scheduled_for)
        .map_err(|e| AutomationError::internal_from(e))?
    {
        ManualClaimOutcome::Claimed(run) => run,
        ManualClaimOutcome::Busy => {
            return Err(AutomationError::busy(format!(
                "Automation {} already has an active run",
                automation.id
            )));
        }
    };

    // Execute the kernel; the pending -> running/failed transition happens
    // inside it. Even an error path leaves a durable run row to reconcile.
    let _ = execute_automation_run(svc, &automation, &claimed_run.id, &scheduled_for).await;

    let run = store
        .get_workflow_automation_run(&claimed_run.id)
        .map_err(|e| AutomationError::internal_from(e))?
        .ok_or_else(|| AutomationError::internal("created run disappeared"))?;
    Ok((automation, run))
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
    fn daily_schedule_uses_next_time_slot_on_same_day() {
        let now = test_now("2026-06-25 08:30:00");
        let next = compute_next_run_at_from(
            "daily",
            &json!({
                "times": ["08:00", "09:00", "18:00"],
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
    fn daily_schedule_rolls_to_next_day_after_all_time_slots_pass() {
        let now = test_now("2026-06-25 19:00:00");
        let next = compute_next_run_at_from(
            "daily",
            &json!({
                "times": ["08:00", "09:00", "18:00"],
                "weekdays": [1, 2, 3, 4, 5, 6, 7],
                "start_date": null,
                "end_date": null
            }),
            now,
        )
        .expect("daily schedule should compute");

        assert_eq!(next, Some("2026-06-26 08:00:00".to_string()));
    }

    #[test]
    fn daily_schedule_accepts_legacy_single_time_config() {
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
        .expect("legacy daily schedule should compute");

        assert_eq!(next, Some("2026-06-25 09:00:00".to_string()));
    }

    #[test]
    fn daily_schedule_defaults_to_morning_when_time_list_is_empty() {
        let now = test_now("2026-06-25 08:30:00");
        let next = compute_next_run_at_from(
            "daily",
            &json!({
                "times": [],
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

    #[test]
    fn schedule_slot_validation_matches_the_configuration() {
        let daily = json!({
            "times": ["09:00", "18:00"],
            "weekdays": [],
            "start_date": null,
            "end_date": null
        });
        assert!(
            is_valid_schedule_slot("daily", &daily, test_now("2026-06-25 09:00:00"))
                .expect("valid")
        );
        // A time the configuration does not define is not a slot.
        assert!(
            !is_valid_schedule_slot("daily", &daily, test_now("2026-06-25 12:00:00"))
                .expect("valid")
        );

        let interval = json!({
            "interval_minutes": 120,
            "weekdays": [],
            "start_date": "2026-06-25",
            "end_date": null,
            "anchor_time": "08:00"
        });
        // 10:00 is 08:00 + 120m: on the grid.
        assert!(
            is_valid_schedule_slot("interval", &interval, test_now("2026-06-25 10:00:00"))
                .expect("valid")
        );
        // 09:00 is off the anchor grid and is not a slot.
        assert!(
            !is_valid_schedule_slot("interval", &interval, test_now("2026-06-25 09:00:00"))
                .expect("valid")
        );

        // A configured instant carrying sub-second digits must still match the
        // DB-second cursor, so an RFC3339 `run_at` never looks corrupt.
        let once = json!({"run_at": "2026-06-25T09:00:00.500+00:00"});
        let cursor = parse_once_run_at("2026-06-25T09:00:00+00:00").expect("parse once");
        assert!(is_valid_schedule_slot("once", &once, cursor).expect("valid"));
        assert!(
            !is_valid_schedule_slot("once", &once, test_now("2026-06-25 10:00:00")).expect("valid")
        );
    }

    #[test]
    fn interval_slot_validation_uses_an_exact_second_grid() {
        // A 09:00 one-hour grid with a persistent anchor. The grid is compared in
        // seconds, so a sub-minute offset such as 09:00:59 is not a slot (minute
        // rounding used to accept it).
        let persistent = json!({
            "interval_minutes": 60,
            "weekdays": [],
            "start_date": "2026-06-25",
            "end_date": null,
            "anchor_time": "09:00"
        });
        assert!(
            is_valid_schedule_slot("interval", &persistent, test_now("2026-06-25 09:00:00"))
                .expect("valid")
        );
        assert!(
            !is_valid_schedule_slot("interval", &persistent, test_now("2026-06-25 09:00:59"))
                .expect("valid")
        );
        // A slot before the anchor is not on the forward grid.
        assert!(
            !is_valid_schedule_slot("interval", &persistent, test_now("2026-06-25 08:59:00"))
                .expect("valid")
        );

        // The 5-minute floor is kept: 3 minutes canonicalises to 5, so 09:05 is
        // on the grid and 09:03 is not.
        let floor = json!({
            "interval_minutes": 3,
            "weekdays": [],
            "start_date": "2026-06-25",
            "end_date": null,
            "anchor_time": "09:00"
        });
        assert!(
            is_valid_schedule_slot("interval", &floor, test_now("2026-06-25 09:05:00"))
                .expect("valid")
        );
        assert!(
            !is_valid_schedule_slot("interval", &floor, test_now("2026-06-25 09:03:00"))
                .expect("valid")
        );

        // An unparseable configuration must never be reported as a legal slot.
        let malformed = json!({ "interval_minutes": "not-a-number" });
        assert!(
            is_valid_schedule_slot("interval", &malformed, test_now("2026-06-25 09:00:00"))
                .is_err()
        );
    }

    #[test]
    fn interval_slot_without_a_persistent_anchor_uses_the_day_reduced_grid() {
        // No `start_date`: the generator anchors on the day it computes on, so a
        // whole day may be added to any reachable slot. The day-reduced grid must
        // still reject an off-grid slot, including one that crosses midnight.
        let config = json!({
            "interval_minutes": 240,
            "weekdays": [],
            "start_date": null,
            "end_date": null,
            "anchor_time": "22:00"
        });
        assert!(
            is_valid_schedule_slot("interval", &config, test_now("2026-06-25 22:00:00"))
                .expect("valid")
        );
        // 02:00 the next day is 22:00 + 4h across midnight.
        assert!(
            is_valid_schedule_slot("interval", &config, test_now("2026-06-26 02:00:00"))
                .expect("valid")
        );
        assert!(
            !is_valid_schedule_slot("interval", &config, test_now("2026-06-26 02:01:00"))
                .expect("valid")
        );

        // With no anchor time at all no grid is reproducible, so only the date
        // range and weekday are enforced.
        let no_anchor = json!({
            "interval_minutes": 240,
            "weekdays": [],
            "start_date": null,
            "end_date": null,
            "anchor_time": null
        });
        assert!(
            is_valid_schedule_slot("interval", &no_anchor, test_now("2026-06-26 03:17:00"))
                .expect("valid")
        );
    }

    #[test]
    fn interval_grid_is_independent_of_a_fixed_utc_offset() {
        // The grid is expressed in absolute seconds, so the same local wall clock
        // must validate identically under any fixed offset. A fixed-offset fixture
        // pins that without mutating the process timezone, which the parallel test
        // harness must never do.
        let offsets = [
            chrono::FixedOffset::east_opt(5 * 3600 + 45 * 60).expect("+05:45"),
            chrono::FixedOffset::west_opt(4 * 3600).expect("-04:00"),
        ];
        let mut verdicts = Vec::new();
        for offset in offsets {
            let anchor = offset
                .with_ymd_and_hms(2026, 6, 25, 9, 0, 0)
                .single()
                .expect("anchor");
            let on_grid = offset
                .with_ymd_and_hms(2026, 6, 25, 11, 0, 0)
                .single()
                .expect("on grid");
            let off_grid = offset
                .with_ymd_and_hms(2026, 6, 25, 11, 1, 0)
                .single()
                .expect("off grid");
            let step_seconds = 120 * 60;
            verdicts.push((
                interval_slot_is_on_grid(
                    on_grid.signed_duration_since(anchor).num_seconds(),
                    step_seconds,
                    false,
                ),
                interval_slot_is_on_grid(
                    off_grid.signed_duration_since(anchor).num_seconds(),
                    step_seconds,
                    false,
                ),
            ));
        }
        assert_eq!(verdicts, vec![(true, false), (true, false)]);
    }

    /// The interval grid across a real DST transition is checked in an isolated
    /// re-exec of this test binary with `TZ` set. `Local` reads `TZ` at process
    /// start, so mutating it inside the parallel test harness would be unsound.
    #[test]
    fn interval_grid_is_stable_across_a_dst_transition() {
        const PROBE_ENV: &str = "CS_AUTOMATION_DST_PROBE";
        const TEST_NAME: &str =
            "workflow::automation::service::tests::interval_grid_is_stable_across_a_dst_transition";

        if std::env::var(PROBE_ENV).is_err() {
            let binary = std::env::current_exe().expect("current test binary");
            let status = std::process::Command::new(binary)
                .args(["--exact", TEST_NAME, "--nocapture"])
                .env(PROBE_ENV, "1")
                .env("TZ", "America/New_York")
                .status()
                .expect("spawn the isolated timezone probe");
            assert!(status.success(), "the isolated timezone probe failed");
            return;
        }

        // Child process, now running in the requested zone.
        let winter = test_now("2026-01-15 12:00:00");
        let summer = test_now("2026-07-15 12:00:00");
        if winter.offset() == summer.offset() {
            // No zone database is available, so there is no transition to
            // observe here; the grid arithmetic itself is still covered by the
            // fixed-offset test.
            eprintln!("skipping the DST probe: the zone database is unavailable");
            return;
        }

        // 2026-03-08 is a 23-hour day in this zone. With a persistent anchor the
        // grid is absolute, so 09:00 stays on the one-hour grid every day.
        let config = json!({
            "interval_minutes": 60,
            "weekdays": [],
            "start_date": "2026-03-07",
            "end_date": null,
            "anchor_time": "09:00"
        });
        for day in ["2026-03-07", "2026-03-08", "2026-03-09"] {
            let slot = test_now(&format!("{day} 09:00:00"));
            assert!(
                is_valid_schedule_slot("interval", &config, slot).expect("valid"),
                "{day} 09:00 must stay on the absolute grid across the DST transition"
            );
        }
    }
}
