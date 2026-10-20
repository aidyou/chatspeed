//! Friendly-input translation for `cs automation create|update|draft`.
//!
//! The friendly flags are a validated front-end to the canonical `snake_case`
//! `AutomationSpec` the raw `--spec-json` document already carries. A field here
//! is only ever derived from an explicit flag or from the existing stored value,
//! so the translation cannot mint a shell/path/network permission or a schedule
//! the caller did not ask for; an inapplicable combination is rejected instead of
//! being silently dropped. The raw document path never passes through this
//! module: it stays exactly what the caller supplied.

use crate::args::AutomationFriendlyArgs;
use crate::error::CliError;
use serde_json::{json, Map, Value};

/// Canonical `AutomationSpec` fields, in the exact `snake_case` shape the control
/// plane deserializes. Only these are copied out of an `AutomationView`, so
/// view-only metadata (ids, revision, timestamps) can never leak into a spec.
pub const SPEC_FIELDS: [&str; 12] = [
    "title",
    "prompt",
    "prompt_file_path",
    "agent_id",
    "agent_config",
    "allowed_paths",
    "shell_config",
    "schedule_kind",
    "schedule_config",
    "continuous_context",
    "self_review",
    "enabled",
];

/// The scheduler never fires faster than this, so an interval below it is
/// rejected early rather than being changed behind the caller's back.
pub const MIN_INTERVAL_MINUTES: u32 = 5;

/// The schedule kinds the scheduler and this translator both understand.
///
/// A stored `schedule_kind` is parsed through this instead of being compared as a
/// raw string, so an unknown value is reported rather than preserved next to
/// parameters the scheduler would then reject.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScheduleKind {
    Interval,
    Daily,
    Once,
}

impl ScheduleKind {
    /// The canonical `schedule_kind` string.
    fn as_str(self) -> &'static str {
        match self {
            ScheduleKind::Interval => "interval",
            ScheduleKind::Daily => "daily",
            ScheduleKind::Once => "once",
        }
    }

    /// Parses a canonical `schedule_kind`; an unknown value is `None`.
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "interval" => Some(ScheduleKind::Interval),
            "daily" => Some(ScheduleKind::Daily),
            "once" => Some(ScheduleKind::Once),
            _ => None,
        }
    }
}

/// The schedule a set of friendly flags selected.
#[derive(Debug)]
enum ScheduleSelector<'a> {
    Interval(&'a str),
    Daily(&'a [String]),
    Once(&'a str),
}

impl ScheduleSelector<'_> {
    fn kind(&self) -> ScheduleKind {
        match self {
            ScheduleSelector::Interval(_) => ScheduleKind::Interval,
            ScheduleSelector::Daily(_) => ScheduleKind::Daily,
            ScheduleSelector::Once(_) => ScheduleKind::Once,
        }
    }
}

// --- Diagnostic constructors ------------------------------------------------
// Each message is a localized `t!` lookup so a new diagnostic cannot ship as a
// hard-coded English string.

fn interval_invalid(raw: &str) -> CliError {
    CliError::usage(rust_i18n::t!("cs.automation_interval_invalid", value = raw).to_string())
}

fn interval_too_short(raw: &str) -> CliError {
    CliError::usage(rust_i18n::t!("cs.automation_interval_too_short", value = raw).to_string())
}

fn time_invalid(raw: &str) -> CliError {
    CliError::usage(rust_i18n::t!("cs.automation_time_invalid", value = raw).to_string())
}

fn once_invalid(raw: &str) -> CliError {
    CliError::usage(rust_i18n::t!("cs.automation_once_invalid", value = raw).to_string())
}

fn weekdays_invalid(raw: &str) -> CliError {
    CliError::usage(rust_i18n::t!("cs.automation_weekdays_invalid", value = raw).to_string())
}

fn date_invalid(flag: &str, raw: &str) -> CliError {
    CliError::usage(
        rust_i18n::t!("cs.automation_date_invalid", flag = flag, value = raw).to_string(),
    )
}

fn title_required() -> CliError {
    CliError::usage(rust_i18n::t!("cs.automation_title_required").to_string())
}

fn agent_required() -> CliError {
    CliError::usage(rust_i18n::t!("cs.automation_agent_required").to_string())
}

fn prompt_required() -> CliError {
    CliError::usage(rust_i18n::t!("cs.automation_prompt_required").to_string())
}

fn schedule_required() -> CliError {
    CliError::usage(rust_i18n::t!("cs.automation_schedule_required").to_string())
}

fn schedule_select_one() -> CliError {
    CliError::usage(rust_i18n::t!("cs.automation_schedule_select_one").to_string())
}

fn anchor_time_invalid(raw: &str) -> CliError {
    CliError::usage(rust_i18n::t!("cs.automation_anchor_time_invalid", value = raw).to_string())
}

fn date_range_invalid(start: &str, end: &str) -> CliError {
    CliError::usage(
        rust_i18n::t!("cs.automation_date_range_invalid", start = start, end = end).to_string(),
    )
}

fn schedule_kind_unknown(raw: &str) -> CliError {
    CliError::usage(rust_i18n::t!("cs.automation_schedule_kind_unknown", value = raw).to_string())
}

fn anchor_only_interval() -> CliError {
    CliError::usage(rust_i18n::t!("cs.automation_inapplicable_anchor_time").to_string())
}

fn inapplicable_window() -> CliError {
    CliError::usage(rust_i18n::t!("cs.automation_inapplicable_window").to_string())
}

fn once_continuous() -> CliError {
    CliError::usage(rust_i18n::t!("cs.automation_once_continuous").to_string())
}

fn clear_and_path_conflict() -> CliError {
    CliError::usage(rust_i18n::t!("cs.automation_paths_clear_conflict").to_string())
}

fn clear_requires_existing() -> CliError {
    CliError::usage(rust_i18n::t!("cs.automation_paths_clear_requires_existing").to_string())
}

fn view_incomplete(field: &str) -> CliError {
    CliError::protocol(rust_i18n::t!("cs.automation_view_incomplete", field = field).to_string())
}

// --- Flag helpers -----------------------------------------------------------

/// Whether any friendly field flag was given, so a caller can tell a friendly
/// request from "no fields at all".
pub fn friendly_requested(args: &AutomationFriendlyArgs) -> bool {
    args.title.is_some()
        || args.agent.is_some()
        || args.prompt.is_some()
        || args.prompt_file.is_some()
        || schedule_requested(args)
        || !args.allowed_paths.is_empty()
        || args.clear_allowed_paths
        || args.enabled
        || args.disabled
        || args.continuous_context
        || args.no_continuous_context
        || args.self_review
        || args.no_self_review
}

/// Whether any schedule selector flag was given.
pub fn schedule_selected(args: &AutomationFriendlyArgs) -> bool {
    args.every.is_some() || !args.daily.is_empty() || args.once.is_some()
}

/// Whether any schedule flag was given, selector or parameter.
///
/// A schedule parameter on its own is a valid same-kind update: it keeps the
/// stored kind and overlays only the parameters the caller restated.
pub fn schedule_requested(args: &AutomationFriendlyArgs) -> bool {
    schedule_selected(args)
        || args.weekdays.is_some()
        || args.start_date.is_some()
        || args.end_date.is_some()
        || args.anchor_time.is_some()
}

/// A tri-state flag pair: `Some(true)` for the on flag, `Some(false)` for the off
/// flag and `None` when neither was given (keep the stored value). Clap already
/// rejects both halves together.
pub fn tri_state(on: bool, off: bool) -> Option<bool> {
    match (on, off) {
        (true, _) => Some(true),
        (_, true) => Some(false),
        _ => None,
    }
}

/// Reads the prompt text from `--prompt`/`--prompt-file` (`-` reads stdin).
///
/// `Ok(None)` means no prompt flag was given, so an update keeps the stored
/// prompt. A given flag always yields text, never a file reference, so switching
/// a prompt clears any stored `prompt_file_path`.
pub fn read_prompt_text(args: &AutomationFriendlyArgs) -> Result<Option<String>, CliError> {
    if args.prompt.is_none() && args.prompt_file.is_none() {
        return Ok(None);
    }
    crate::args::resolve_prompt(&args.prompt, &args.prompt_file)
        .map(Some)
        .map_err(CliError::io)
}

/// Trims a required text flag, or reports the given "missing" diagnostic.
fn required(value: Option<&str>, missing: fn() -> CliError) -> Result<&str, CliError> {
    match value.map(str::trim).filter(|text| !text.is_empty()) {
        Some(text) => Ok(text),
        None => Err(missing()),
    }
}

// --- Schedule parsing -------------------------------------------------------

/// Selects the schedule kind from the selector flags.
///
/// `Ok(None)` means no selector was given, leaving it to the caller to decide
/// whether that is a valid same-kind update or a missing required schedule.
fn select_schedule(
    args: &AutomationFriendlyArgs,
) -> Result<Option<ScheduleSelector<'_>>, CliError> {
    let every = args.every.as_deref();
    let daily = (!args.daily.is_empty()).then_some(args.daily.as_slice());
    let once = args.once.as_deref();
    match (every, daily, once) {
        (Some(value), None, None) => Ok(Some(ScheduleSelector::Interval(value))),
        (None, Some(value), None) => Ok(Some(ScheduleSelector::Daily(value))),
        (None, None, Some(value)) => Ok(Some(ScheduleSelector::Once(value))),
        (None, None, None) => Ok(None),
        _ => Err(schedule_select_one()),
    }
}

/// Parses `--every` as minutes (`5m`) or hours (`1h`); a bare number is refused
/// so its unit is never guessed.
fn parse_interval(raw: &str) -> Result<u32, CliError> {
    let trimmed = raw.trim();
    let (digits, multiplier) = if let Some(digits) = trimmed.strip_suffix(['m', 'M']) {
        (digits, 1u32)
    } else if let Some(digits) = trimmed.strip_suffix(['h', 'H']) {
        (digits, 60u32)
    } else {
        return Err(interval_invalid(raw));
    };
    let value: u32 = digits.trim().parse().map_err(|_| interval_invalid(raw))?;
    if value == 0 {
        return Err(interval_invalid(raw));
    }
    let minutes = value
        .checked_mul(multiplier)
        .ok_or_else(|| interval_invalid(raw))?;
    if minutes < MIN_INTERVAL_MINUTES {
        return Err(interval_too_short(raw));
    }
    Ok(minutes)
}

/// Parses `--weekdays` as a comma-separated 1..=7 list (1 = Monday).
fn parse_weekdays(raw: &str) -> Result<Vec<u32>, CliError> {
    let mut days = Vec::new();
    for token in raw.split(',') {
        let day: u32 = token.trim().parse().map_err(|_| weekdays_invalid(raw))?;
        if !(1..=7).contains(&day) {
            return Err(weekdays_invalid(raw));
        }
        if !days.contains(&day) {
            days.push(day);
        }
    }
    if days.is_empty() {
        return Err(weekdays_invalid(raw));
    }
    days.sort_unstable();
    Ok(days)
}

/// Parses one `--daily` time. The plan and `--help` pin this flag to `HH:MM`.
fn validate_daily_time(raw: &str) -> Result<(), CliError> {
    chrono::NaiveTime::parse_from_str(raw.trim(), "%H:%M")
        .map(|_| ())
        .map_err(|_| time_invalid(raw))
}

/// Parses `--anchor-time` with the two forms the scheduler's `parse_time`
/// accepts: `HH:MM` or `HH:MM:SS`.
fn validate_anchor_time(raw: &str) -> Result<(), CliError> {
    let trimmed = raw.trim();
    chrono::NaiveTime::parse_from_str(trimmed, "%H:%M")
        .or_else(|_| chrono::NaiveTime::parse_from_str(trimmed, "%H:%M:%S"))
        .map(|_| ())
        .map_err(|_| anchor_time_invalid(raw))
}

fn validate_date(flag: &str, raw: &str) -> Result<(), CliError> {
    chrono::NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d")
        .map(|_| ())
        .map_err(|_| date_invalid(flag, raw))
}

/// Accepts exactly the one-shot timestamp forms the scheduler reads: RFC3339 or a
/// local `YYYY-MM-DD HH:MM[:SS]`.
fn validate_once(raw: &str) -> Result<(), CliError> {
    let trimmed = raw.trim();
    if chrono::DateTime::parse_from_rfc3339(trimmed).is_ok() {
        return Ok(());
    }
    if chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M:%S").is_ok() {
        return Ok(());
    }
    if chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%d %H:%M").is_ok() {
        return Ok(());
    }
    Err(once_invalid(raw))
}

/// Overlays the shared weekday/date window onto a schedule config.
///
/// A one-shot schedule cannot express the window, so that combination is
/// rejected instead of being dropped. The merged start/end pair is checked too,
/// so an update that restates only one end of the range cannot persist a window
/// the scheduler would read as empty.
fn apply_window(
    args: &AutomationFriendlyArgs,
    kind: ScheduleKind,
    config: &mut Map<String, Value>,
) -> Result<(), CliError> {
    let window_given =
        args.weekdays.is_some() || args.start_date.is_some() || args.end_date.is_some();
    if window_given && kind == ScheduleKind::Once {
        return Err(inapplicable_window());
    }
    if let Some(raw) = args.weekdays.as_deref() {
        config.insert("weekdays".to_string(), json!(parse_weekdays(raw)?));
    }
    if let Some(raw) = args.start_date.as_deref() {
        validate_date("start-date", raw)?;
        config.insert("start_date".to_string(), Value::from(raw.trim()));
    }
    if let Some(raw) = args.end_date.as_deref() {
        validate_date("end-date", raw)?;
        config.insert("end_date".to_string(), Value::from(raw.trim()));
    }
    validate_window_range(config)
}

/// Rejects an effective window whose start is after its end.
///
/// The comparison is chronological for the `YYYY-MM-DD` form both flags are
/// validated to, and it runs on the merged window so a range supplied across two
/// updates is still caught.
fn validate_window_range(config: &Map<String, Value>) -> Result<(), CliError> {
    match (
        config.get("start_date").and_then(Value::as_str),
        config.get("end_date").and_then(Value::as_str),
    ) {
        (Some(start), Some(end)) if start > end => Err(date_range_invalid(start, end)),
        _ => Ok(()),
    }
}

/// Overlays the anchor time, which only an interval schedule can express.
fn apply_anchor(
    args: &AutomationFriendlyArgs,
    kind: ScheduleKind,
    config: &mut Map<String, Value>,
) -> Result<(), CliError> {
    let Some(raw) = args.anchor_time.as_deref() else {
        return Ok(());
    };
    if kind != ScheduleKind::Interval {
        return Err(anchor_only_interval());
    }
    validate_anchor_time(raw)?;
    config.insert("anchor_time".to_string(), Value::from(raw.trim()));
    Ok(())
}

/// Resolves the schedule kind and the exact `schedule_config` document.
///
/// A selector flag chooses the kind. A change to the *same* kind keeps the stored
/// parameters the caller did not restate; a change to a *different* kind starts
/// from an empty object, so the previous kind's keys are gone instead of
/// lingering. With no selector the stored kind is kept and only the given
/// weekday/date/anchor parameters are overlaid, so a same-kind update may restate
/// just `--weekdays`/`--start-date`/`--end-date`/`--anchor-time`.
fn schedule_for(
    args: &AutomationFriendlyArgs,
    base_kind: Option<&str>,
    base_config: Option<&Value>,
) -> Result<(ScheduleKind, Value), CliError> {
    let selector = select_schedule(args)?;
    let kind = match &selector {
        Some(selector) => selector.kind(),
        // No selector: the stored kind is authoritative. A kind this CLI cannot
        // translate, or no stored kind at all, is reported rather than guessed.
        None => {
            let existing = base_kind.ok_or_else(schedule_required)?;
            ScheduleKind::parse(existing).ok_or_else(|| schedule_kind_unknown(existing))?
        }
    };

    let mut config = match (base_kind, base_config) {
        (Some(existing), Some(Value::Object(map)))
            if ScheduleKind::parse(existing) == Some(kind) =>
        {
            map.clone()
        }
        _ => Map::new(),
    };

    if let Some(selector) = selector {
        match selector {
            ScheduleSelector::Interval(raw) => {
                let minutes = parse_interval(raw)?;
                // `interval_hours` is a legacy alias the scheduler normalizes; keep
                // the document on the canonical minutes field so the two cannot
                // disagree.
                config.remove("interval_hours");
                config.insert("interval_minutes".to_string(), Value::from(minutes));
            }
            ScheduleSelector::Daily(times) => {
                let mut normalized = Vec::with_capacity(times.len());
                for raw in times {
                    validate_daily_time(raw)?;
                    normalized.push(Value::from(raw.trim()));
                }
                // Replace the singular form so the two can never disagree.
                config.remove("time");
                config.insert("times".to_string(), Value::Array(normalized));
            }
            ScheduleSelector::Once(raw) => {
                validate_once(raw)?;
                config.insert("run_at".to_string(), Value::from(raw.trim()));
            }
        }
    }

    apply_window(args, kind, &mut config)?;
    apply_anchor(args, kind, &mut config)?;
    Ok((kind, Value::Object(config)))
}

// --- Spec construction ------------------------------------------------------

/// Builds a fresh `AutomationSpec` document from the friendly flags.
///
/// Every unspecified optional field takes a safe default (disabled, no continuous
/// context, no self review, no allowed paths); permission-bearing fields are
/// never inferred from the environment.
pub fn build_create_spec(args: &AutomationFriendlyArgs) -> Result<Value, CliError> {
    if args.clear_allowed_paths {
        return Err(clear_requires_existing());
    }
    let title = required(args.title.as_deref(), title_required)?;
    let agent = required(args.agent.as_deref(), agent_required)?;
    let prompt = match read_prompt_text(args)? {
        Some(text) if !text.trim().is_empty() => text,
        _ => return Err(prompt_required()),
    };
    let (kind, schedule_config) = schedule_for(args, None, None)?;
    let enabled = tri_state(args.enabled, args.disabled).unwrap_or(false);
    let continuous_context =
        tri_state(args.continuous_context, args.no_continuous_context).unwrap_or(false);
    let self_review = tri_state(args.self_review, args.no_self_review).unwrap_or(false);
    if kind == ScheduleKind::Once && continuous_context {
        return Err(once_continuous());
    }

    let mut spec = Map::new();
    spec.insert("title".to_string(), Value::from(title));
    spec.insert("prompt".to_string(), Value::from(prompt));
    spec.insert("prompt_file_path".to_string(), Value::Null);
    spec.insert("agent_id".to_string(), Value::from(agent));
    spec.insert("agent_config".to_string(), Value::Null);
    spec.insert("allowed_paths".to_string(), json!(args.allowed_paths));
    spec.insert("shell_config".to_string(), Value::Null);
    spec.insert("schedule_kind".to_string(), Value::from(kind.as_str()));
    spec.insert("schedule_config".to_string(), schedule_config);
    spec.insert(
        "continuous_context".to_string(),
        Value::Bool(continuous_context),
    );
    spec.insert("self_review".to_string(), Value::Bool(self_review));
    spec.insert("enabled".to_string(), Value::Bool(enabled));
    Ok(Value::Object(spec))
}

/// Projects the canonical `AutomationSpec` fields out of an `AutomationView`.
///
/// A missing field is a protocol error, not something to paper over with a
/// default, because the result is written back as the next stored spec.
pub fn spec_from_view(view: &Value) -> Result<Map<String, Value>, CliError> {
    let object = view
        .as_object()
        .ok_or_else(|| view_incomplete("automation"))?;
    let mut spec = Map::new();
    for field in SPEC_FIELDS {
        let value = object.get(field).ok_or_else(|| view_incomplete(field))?;
        spec.insert(field.to_string(), value.clone());
    }
    Ok(spec)
}

/// The current revision from an `AutomationView`. A missing revision is a protocol
/// error, because the compare-and-set cannot be formed without it.
pub fn view_revision(view: &Value) -> Result<i64, CliError> {
    view.get("revision")
        .and_then(Value::as_i64)
        .ok_or_else(|| view_incomplete("revision"))
}

/// Overlays the friendly flags onto an existing `AutomationView`, producing the
/// full `AutomationSpec` an update (or an update-shaped draft) would persist.
///
/// The base is the whitelisted projection of the view, so `agent_config`,
/// `shell_config`, `allowed_paths` and every unspecified boolean are preserved;
/// only an explicit flag changes a field. Explicit paths replace the stored set
/// (never a union) and `--clear-allowed-paths` empties it.
pub fn overlay_spec(view: &Value, args: &AutomationFriendlyArgs) -> Result<Value, CliError> {
    let mut spec = spec_from_view(view)?;

    if let Some(raw) = args.title.as_deref() {
        spec.insert(
            "title".to_string(),
            Value::from(required(Some(raw), title_required)?),
        );
    }
    if let Some(raw) = args.agent.as_deref() {
        spec.insert(
            "agent_id".to_string(),
            Value::from(required(Some(raw), agent_required)?),
        );
    }
    if let Some(prompt) = read_prompt_text(args)? {
        if prompt.trim().is_empty() {
            return Err(prompt_required());
        }
        spec.insert("prompt".to_string(), Value::from(prompt));
        // Switching the prompt to inline text drops a stale file reference.
        spec.insert("prompt_file_path".to_string(), Value::Null);
    }

    if schedule_requested(args) {
        let base_kind = view.get("schedule_kind").and_then(Value::as_str);
        let base_config = view.get("schedule_config");
        let (kind, config) = schedule_for(args, base_kind, base_config)?;
        spec.insert("schedule_kind".to_string(), Value::from(kind.as_str()));
        spec.insert("schedule_config".to_string(), config);
    }

    if args.clear_allowed_paths {
        if !args.allowed_paths.is_empty() {
            return Err(clear_and_path_conflict());
        }
        spec.insert("allowed_paths".to_string(), Value::Array(Vec::new()));
    } else if !args.allowed_paths.is_empty() {
        spec.insert("allowed_paths".to_string(), json!(args.allowed_paths));
    }

    if let Some(value) = tri_state(args.enabled, args.disabled) {
        spec.insert("enabled".to_string(), Value::Bool(value));
    }
    if let Some(value) = tri_state(args.continuous_context, args.no_continuous_context) {
        spec.insert("continuous_context".to_string(), Value::Bool(value));
    }
    if let Some(value) = tri_state(args.self_review, args.no_self_review) {
        spec.insert("self_review".to_string(), Value::Bool(value));
    }

    // The scheduler silently drops continuous context for a one-shot schedule;
    // reject the contradiction instead so the stored intent is never changed.
    let kind = spec.get("schedule_kind").and_then(Value::as_str);
    let continuous = spec.get("continuous_context").and_then(Value::as_bool);
    if kind == Some(ScheduleKind::Once.as_str()) && continuous == Some(true) {
        return Err(once_continuous());
    }

    Ok(Value::Object(spec))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty friendly input, so a test only states the flags it exercises.
    fn args() -> AutomationFriendlyArgs {
        AutomationFriendlyArgs {
            title: None,
            agent: None,
            prompt: None,
            prompt_file: None,
            every: None,
            daily: Vec::new(),
            once: None,
            weekdays: None,
            start_date: None,
            end_date: None,
            anchor_time: None,
            allowed_paths: Vec::new(),
            clear_allowed_paths: false,
            enabled: false,
            disabled: false,
            continuous_context: false,
            no_continuous_context: false,
            self_review: false,
            no_self_review: false,
        }
    }

    /// A complete create input: title, agent, prompt and an interval schedule.
    fn create_args() -> AutomationFriendlyArgs {
        let mut args = args();
        args.title = Some("Nightly".to_string());
        args.agent = Some("agent-1".to_string());
        args.prompt = Some("do the thing".to_string());
        args.every = Some("1h".to_string());
        args
    }

    /// An `AutomationView` shaped like the control-plane GET response.
    fn view() -> Value {
        json!({
            "automation_id": "auto-1",
            "title": "Stored",
            "prompt": "old prompt",
            "prompt_file_path": "/tmp/prompt.txt",
            "agent_id": "agent-1",
            "agent_config": { "approvalLevel": "full", "models": { "act": "x" } },
            "allowed_paths": ["/keep"],
            "shell_config": { "command": "echo kept" },
            "schedule_kind": "interval",
            "schedule_config": {
                "interval_minutes": 30,
                "weekdays": [1, 2],
                "start_date": "2026-01-01",
                "anchor_time": "08:00",
            },
            "continuous_context": true,
            "self_review": false,
            "enabled": false,
            "current_workflow_session_id": "sess-9",
            "next_run_at": "2026-01-02 08:00:00",
            "last_run_at": null,
            "revision": 7,
            "created_at": "2026-01-01 00:00:00",
            "updated_at": "2026-01-01 00:00:00",
        })
    }

    #[test]
    fn interval_values_are_parsed_with_an_explicit_unit() {
        assert_eq!(parse_interval("5m").expect("5m"), 5);
        assert_eq!(parse_interval("1h").expect("1h"), 60);
        assert_eq!(parse_interval("90m").expect("90m"), 90);
        assert_eq!(parse_interval("2H").expect("2H"), 120);
        // A bare number, a zero value, an unknown unit and a composite form are all
        // refused rather than guessed.
        for raw in ["5", "0m", "5d", "1h30m", "", "abc", "m"] {
            assert!(parse_interval(raw).is_err(), "{raw} must be rejected");
        }
        assert_eq!(parse_interval("3m").expect_err("too short").exit_code(), 2);
    }

    #[test]
    fn weekdays_are_a_deduped_sorted_one_to_seven_list() {
        assert_eq!(
            parse_weekdays("1,2,3,4,5").expect("range"),
            vec![1, 2, 3, 4, 5]
        );
        assert_eq!(parse_weekdays("5, 1 ,5").expect("dedup"), vec![1, 5]);
        for raw in ["0", "8", "1,,2", "", "a", "-1"] {
            assert!(parse_weekdays(raw).is_err(), "{raw} must be rejected");
        }
    }

    #[test]
    fn dates_times_and_once_timestamps_are_validated() {
        assert!(validate_date("start-date", "2026-01-31").is_ok());
        assert!(validate_date("start-date", "2026-13-01").is_err());
        assert!(validate_date("end-date", "31/01/2026").is_err());

        // `--daily` is pinned to HH:MM; `--anchor-time` matches the scheduler's
        // own `parse_time` and so also accepts seconds.
        assert!(validate_daily_time("09:00").is_ok());
        assert!(validate_daily_time("09:00:00").is_err());
        assert!(validate_daily_time("24:00").is_err());
        assert!(validate_daily_time("09").is_err());

        assert!(validate_anchor_time("09:00").is_ok());
        assert!(validate_anchor_time("23:59:59").is_ok());
        assert!(validate_anchor_time("24:00").is_err());
        assert!(validate_anchor_time("09").is_err());

        assert!(validate_once("2026-06-25T10:00:00Z").is_ok());
        assert!(validate_once("2026-06-25T10:00:00+08:00").is_ok());
        assert!(validate_once("2026-06-25 10:00").is_ok());
        assert!(validate_once("2026-06-25 10:00:00").is_ok());
        assert!(validate_once("tomorrow").is_err());
    }

    #[test]
    fn a_schedule_selector_is_optional_but_never_ambiguous() {
        // No selector is `None`, not an error: the caller decides whether the
        // stored kind may be kept (update) or a schedule is required (create).
        let empty = args();
        assert!(select_schedule(&empty).expect("none").is_none());

        let mut two = args();
        two.every = Some("1h".to_string());
        two.daily = vec!["09:00".to_string()];
        let error = select_schedule(&two).expect_err("two selectors");
        assert_eq!(error.exit_code(), 2);

        let mut daily = args();
        daily.daily = vec!["09:00".to_string(), "18:00".to_string()];
        assert!(matches!(
            select_schedule(&daily).expect("daily"),
            Some(ScheduleSelector::Daily(_))
        ));
    }

    #[test]
    fn schedule_config_shapes_match_each_kind() {
        let (kind, config) = schedule_for(&create_args(), None, None).expect("interval");
        assert_eq!(kind, ScheduleKind::Interval);
        assert_eq!(config["interval_minutes"], 60);

        let mut daily = args();
        daily.daily = vec!["09:00".to_string(), "18:00".to_string()];
        daily.weekdays = Some("1,2,3,4,5".to_string());
        daily.start_date = Some("2026-02-01".to_string());
        let (kind, config) = schedule_for(&daily, None, None).expect("daily");
        assert_eq!(kind, ScheduleKind::Daily);
        assert_eq!(config["times"], json!(["09:00", "18:00"]));
        assert_eq!(config["weekdays"], json!([1, 2, 3, 4, 5]));
        assert_eq!(config["start_date"], "2026-02-01");

        let mut once = args();
        once.once = Some("2026-06-25T10:00:00Z".to_string());
        let (kind, config) = schedule_for(&once, None, None).expect("once");
        assert_eq!(kind, ScheduleKind::Once);
        assert_eq!(config["run_at"], "2026-06-25T10:00:00Z");
    }

    #[test]
    fn inapplicable_schedule_parameters_are_rejected() {
        // A weekday/date window cannot be expressed by a one-shot schedule.
        let mut once = args();
        once.once = Some("2026-06-25T10:00:00Z".to_string());
        once.weekdays = Some("1".to_string());
        assert!(schedule_for(&once, None, None).is_err());

        // The anchor time belongs to an interval schedule.
        let mut daily = args();
        daily.daily = vec!["09:00".to_string()];
        daily.anchor_time = Some("08:00".to_string());
        assert!(schedule_for(&daily, None, None).is_err());

        // The same rules hold when no selector is restated and the stored kind
        // decides: a window on a stored `once`, an anchor on a stored `daily`.
        let stored_once = json!({ "run_at": "2026-06-25T10:00:00Z" });
        let mut window_only = args();
        window_only.weekdays = Some("1".to_string());
        assert!(schedule_for(&window_only, Some("once"), Some(&stored_once)).is_err());

        let stored_daily = json!({ "times": ["09:00"] });
        let mut anchor_only = args();
        anchor_only.anchor_time = Some("08:00".to_string());
        assert!(schedule_for(&anchor_only, Some("daily"), Some(&stored_daily)).is_err());
    }

    #[test]
    fn a_create_spec_uses_safe_defaults_and_no_inferred_permission() {
        let spec = build_create_spec(&create_args()).expect("create spec");
        let object = spec.as_object().expect("object");
        // Only the canonical fields are present, so the backend's
        // deny_unknown_fields never sees a foreign key.
        assert_eq!(object.len(), SPEC_FIELDS.len());
        for field in SPEC_FIELDS {
            assert!(object.contains_key(field), "missing {field}");
        }
        assert_eq!(spec["enabled"], false);
        assert_eq!(spec["continuous_context"], false);
        assert_eq!(spec["self_review"], false);
        assert_eq!(spec["allowed_paths"], json!([]));
        assert_eq!(spec["agent_config"], Value::Null);
        assert_eq!(spec["shell_config"], Value::Null);
        assert_eq!(spec["prompt_file_path"], Value::Null);
        assert_eq!(spec["prompt"], "do the thing");
        assert_eq!(spec["schedule_kind"], "interval");
    }

    #[test]
    fn a_create_spec_requires_title_agent_prompt_and_a_schedule() {
        let mut missing_title = create_args();
        missing_title.title = Some("   ".to_string());
        assert!(build_create_spec(&missing_title).is_err());

        let mut missing_agent = create_args();
        missing_agent.agent = None;
        assert!(build_create_spec(&missing_agent).is_err());

        let mut missing_prompt = create_args();
        missing_prompt.prompt = Some("  ".to_string());
        assert!(build_create_spec(&missing_prompt).is_err());

        let mut missing_schedule = create_args();
        missing_schedule.every = None;
        assert!(build_create_spec(&missing_schedule).is_err());
    }

    #[test]
    fn a_create_spec_rejects_clear_paths_and_once_continuous_context() {
        let mut clear = create_args();
        clear.clear_allowed_paths = true;
        assert!(build_create_spec(&clear).is_err());

        let mut once = create_args();
        once.every = None;
        once.once = Some("2026-06-25 10:00".to_string());
        once.continuous_context = true;
        assert!(build_create_spec(&once).is_err());
        // The off flag stays valid for a one-shot schedule.
        once.continuous_context = false;
        once.no_continuous_context = true;
        assert!(build_create_spec(&once).is_ok());
    }

    #[test]
    fn a_create_spec_honours_explicit_booleans_and_paths() {
        let mut args = create_args();
        args.enabled = true;
        args.self_review = true;
        args.allowed_paths = vec!["/tmp/a".to_string()];
        let spec = build_create_spec(&args).expect("create spec");
        assert_eq!(spec["enabled"], true);
        assert_eq!(spec["self_review"], true);
        assert_eq!(spec["allowed_paths"], json!(["/tmp/a"]));
    }

    /// `--prompt-file` reads the prompt text from a real file, and the read text
    /// is what the spec carries.
    #[test]
    fn a_prompt_file_supplies_the_prompt_text() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("prompt.txt");
        std::fs::write(&path, "from the file").expect("write");

        let mut args = create_args();
        args.prompt = None;
        args.prompt_file = Some(path);
        let spec = build_create_spec(&args).expect("create spec");
        assert_eq!(spec["prompt"], "from the file");
        assert_eq!(spec["prompt_file_path"], Value::Null);
    }

    #[test]
    fn a_view_projection_drops_metadata_and_keeps_every_spec_field() {
        let spec = spec_from_view(&view()).expect("spec");
        assert_eq!(spec.len(), SPEC_FIELDS.len());
        for leaked in [
            "automation_id",
            "revision",
            "current_workflow_session_id",
            "next_run_at",
            "created_at",
            "updated_at",
        ] {
            assert!(!spec.contains_key(leaked), "{leaked} must not leak");
        }
        assert_eq!(spec["agent_config"], view()["agent_config"]);
        assert_eq!(spec["shell_config"], view()["shell_config"]);
    }

    #[test]
    fn a_view_projection_rejects_an_incomplete_document() {
        let error = spec_from_view(&json!({ "title": "only" })).expect_err("incomplete");
        assert_eq!(error.exit_code(), 5);
        assert!(spec_from_view(&json!("not an object")).is_err());
    }

    #[test]
    fn an_overlay_preserves_unspecified_fields_and_clears_the_prompt_path() {
        let mut args = args();
        args.title = Some("Renamed".to_string());
        args.prompt = Some("new prompt".to_string());
        let spec = overlay_spec(&view(), &args).expect("overlay");

        assert_eq!(spec["title"], "Renamed");
        assert_eq!(spec["prompt"], "new prompt");
        // Switching to inline text drops the stored file reference.
        assert_eq!(spec["prompt_file_path"], Value::Null);
        // Permission-bearing and unspecified fields survive unchanged.
        assert_eq!(spec["agent_id"], "agent-1");
        assert_eq!(spec["agent_config"], view()["agent_config"]);
        assert_eq!(spec["shell_config"], view()["shell_config"]);
        assert_eq!(spec["allowed_paths"], json!(["/keep"]));
        assert_eq!(spec["enabled"], false);
        assert_eq!(spec["continuous_context"], true);
        assert_eq!(spec["self_review"], false);
        assert_eq!(spec["schedule_kind"], "interval");
        assert_eq!(spec["schedule_config"], view()["schedule_config"]);
    }

    #[test]
    fn an_overlay_replaces_paths_and_honours_an_explicit_clear() {
        let mut replace = args();
        replace.allowed_paths = vec!["/new".to_string()];
        let spec = overlay_spec(&view(), &replace).expect("replace");
        assert_eq!(spec["allowed_paths"], json!(["/new"]));

        let mut clear = args();
        clear.clear_allowed_paths = true;
        let spec = overlay_spec(&view(), &clear).expect("clear");
        assert_eq!(spec["allowed_paths"], json!([]));

        let mut both = args();
        both.clear_allowed_paths = true;
        both.allowed_paths = vec!["/new".to_string()];
        assert!(overlay_spec(&view(), &both).is_err());
    }

    #[test]
    fn an_overlay_keeps_the_same_kind_parameters_it_did_not_restate() {
        let mut same_kind = args();
        same_kind.every = Some("15m".to_string());
        let spec = overlay_spec(&view(), &same_kind).expect("same kind");
        let config = &spec["schedule_config"];
        assert_eq!(config["interval_minutes"], 15);
        // Weekdays, window and anchor are preserved from the stored config.
        assert_eq!(config["weekdays"], json!([1, 2]));
        assert_eq!(config["start_date"], "2026-01-01");
        assert_eq!(config["anchor_time"], "08:00");
    }

    #[test]
    fn an_overlay_change_of_kind_drops_the_old_kind_parameters() {
        let mut to_daily = args();
        to_daily.daily = vec!["07:00".to_string()];
        let spec = overlay_spec(&view(), &to_daily).expect("switch kind");
        assert_eq!(spec["schedule_kind"], "daily");
        let config = spec["schedule_config"].as_object().expect("object");
        assert_eq!(config["times"], json!(["07:00"]));
        // None of the interval keys survive the switch.
        assert!(!config.contains_key("interval_minutes"));
        assert!(!config.contains_key("anchor_time"));
        assert!(!config.contains_key("weekdays"));
    }

    #[test]
    fn an_overlay_may_restate_only_schedule_parameters() {
        // The approved plan allows a same-kind update that states only window
        // parameters: the stored kind and its untouched parameters are kept.
        let mut weekdays_only = args();
        weekdays_only.weekdays = Some("3,5".to_string());
        let spec = overlay_spec(&view(), &weekdays_only).expect("weekdays only");
        assert_eq!(spec["schedule_kind"], "interval");
        let config = &spec["schedule_config"];
        assert_eq!(config["weekdays"], json!([3, 5]));
        // The interval length and the rest of the window survive.
        assert_eq!(config["interval_minutes"], 30);
        assert_eq!(config["start_date"], "2026-01-01");
        assert_eq!(config["anchor_time"], "08:00");

        let mut anchor_only = args();
        anchor_only.anchor_time = Some("07:30:00".to_string());
        let spec = overlay_spec(&view(), &anchor_only).expect("anchor only");
        assert_eq!(spec["schedule_config"]["anchor_time"], "07:30:00");
        assert_eq!(spec["schedule_config"]["interval_minutes"], 30);

        // `create` has no stored kind to keep, so a window on its own is not a
        // schedule and stays a usage error.
        let mut create_window = create_args();
        create_window.every = None;
        create_window.weekdays = Some("1".to_string());
        assert_eq!(
            build_create_spec(&create_window)
                .expect_err("selector required")
                .exit_code(),
            2
        );
    }

    #[test]
    fn an_overlay_rejects_a_window_that_starts_after_it_ends() {
        let mut reversed = args();
        reversed.start_date = Some("2026-05-01".to_string());
        reversed.end_date = Some("2026-04-01".to_string());
        let error = overlay_spec(&view(), &reversed).expect_err("reversed range");
        assert_eq!(
            error.exit_code(),
            2,
            "an invalid date range is a usage error, not a schedule sent to the server"
        );

        // The stored start date participates: restating only the end date before
        // it is a usage error too, not an empty schedule sent to the server.
        let mut end_only = args();
        end_only.end_date = Some("2025-12-31".to_string());
        let error = overlay_spec(&view(), &end_only).expect_err("end before stored start");
        assert_eq!(error.exit_code(), 2);

        // The same check guards the selector path, so a fresh daily schedule
        // cannot be created with a reversed window either.
        let mut selector_range = args();
        selector_range.daily = vec!["09:00".to_string()];
        selector_range.start_date = Some("2026-05-01".to_string());
        selector_range.end_date = Some("2026-04-01".to_string());
        let error = schedule_for(&selector_range, None, None).expect_err("reversed selector range");
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn an_overlay_rejects_an_unknown_stored_schedule_kind() {
        // A kind this CLI cannot translate must not be preserved next to
        // parameters the scheduler would then reject.
        let mut args = args();
        args.weekdays = Some("1".to_string());
        let mut unknown = view();
        unknown["schedule_kind"] = json!("weekly");
        unknown["schedule_config"] = json!({});
        let error = overlay_spec(&unknown, &args).expect_err("unknown kind");
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn an_overlay_rejects_a_final_once_with_continuous_context() {
        // The stored automation is continuous; switching it to a one-shot schedule
        // without turning that off is a contradiction, not a silent downgrade.
        let mut to_once = args();
        to_once.once = Some("2026-06-25 10:00".to_string());
        assert!(overlay_spec(&view(), &to_once).is_err());

        let mut to_once_with_off = args();
        to_once_with_off.once = Some("2026-06-25 10:00".to_string());
        to_once_with_off.no_continuous_context = true;
        assert!(overlay_spec(&view(), &to_once_with_off).is_ok());
    }

    #[test]
    fn tri_state_reports_on_off_and_unspecified() {
        assert_eq!(tri_state(true, false), Some(true));
        assert_eq!(tri_state(false, true), Some(false));
        assert_eq!(tri_state(false, false), None);
    }

    #[test]
    fn friendly_requested_detects_every_flag_family() {
        assert!(!friendly_requested(&args()));
        let mut title = args();
        title.title = Some("T".to_string());
        assert!(friendly_requested(&title));
        let mut disabled = args();
        disabled.disabled = true;
        assert!(friendly_requested(&disabled));
        let mut paths = args();
        paths.allowed_paths = vec!["/p".to_string()];
        assert!(friendly_requested(&paths));
        // A schedule parameter alone counts as friendly input, so an update can
        // restate just the window.
        let mut window = args();
        window.weekdays = Some("1".to_string());
        assert!(friendly_requested(&window));
        assert!(schedule_requested(&window));
        assert!(!schedule_selected(&window));
    }
}
