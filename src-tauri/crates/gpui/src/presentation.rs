//! Read-only presentation of the durable workflow transcript.
//!
//! The projection mirrors `src/composables/workflow/messageProjectionRules.js`
//! and `useWorkflowMessages.ts`: tool activity is grouped, tool titles come from
//! the backend `title` when it is authoritative, and snapshots, delegated tasks
//! and completion rows keep their own presentation instead of leaking raw
//! structured data into the transcript.

use crate::types::Message;
use rust_i18n::t;
use serde_json::Value;
use std::collections::BTreeMap;

const READ_TOOLS: &[&str] = &["read_file", "list_dir", "web_fetch"];
const SEARCH_TOOLS: &[&str] = &["grep", "glob", "web_search"];
const COMMAND_TOOLS: &[&str] = &["bash"];
const MUTATION_TOOLS: &[&str] = &["edit_file", "write_file"];
const TODO_TOOLS: &[&str] = &["todo_create", "todo_list", "todo_update"];
const NON_COLLAPSIBLE_TOOLS: &[&str] = &[
    "ask_user",
    "submit_plan",
    "complete_workflow",
    "sub_agent_run",
];
const DURATION_EXCLUDED_TOOLS: &[&str] = &["ask_user", "submit_plan", "submit_result", "skill"];

// ---------------------------------------------------------------------------
// Text helpers
// ---------------------------------------------------------------------------

/// Cost durations read `x h y m z s`, dropping leading zero units.
pub fn format_duration(duration_ms: Option<u64>) -> String {
    let Some(duration) = duration_ms else {
        return "--".to_owned();
    };
    let seconds = duration / 1000;
    let (hours, minutes, seconds) = (seconds / 3600, (seconds % 3600) / 60, seconds % 60);
    let mut parts = Vec::new();
    if hours > 0 {
        parts.push(format!("{hours}{}", t!("unit_hour")));
    }
    if hours > 0 || minutes > 0 {
        parts.push(format!("{minutes}{}", t!("unit_minute")));
    }
    parts.push(format!("{seconds}{}", t!("unit_second")));
    parts.join(" ")
}

/// Groups digits the way `Intl.NumberFormat` does for token counts.
pub fn grouped_number(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

/// Collapses whitespace runs so a value can occupy a single rendered line.
pub fn single_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Hide internal reminders, including a partially streamed closing boundary.
pub fn clean_text(text: &str) -> String {
    let mut result = text.to_owned();
    for tag in ["SYSTEM_REMINDER", "system_reminder"] {
        let open = format!("<{tag}>");
        let close = format!("</{tag}>");
        while let Some(start) = result.find(&open) {
            let end = result[start + open.len()..]
                .find(&close)
                .map(|offset| start + open.len() + offset + close.len())
                .unwrap_or(result.len());
            result.replace_range(start..end, "");
        }
    }
    result.trim().to_owned()
}

fn fence(text: &str, language: &str) -> String {
    let mut delimiter = "```".to_owned();
    while text.contains(&delimiter) {
        delimiter.push('`');
    }
    format!("{delimiter}{language}\n{text}\n{delimiter}")
}

fn decoded(value: &Value) -> Value {
    if let Some(text) = value.as_str() {
        serde_json::from_str(text).unwrap_or_else(|_| value.clone())
    } else {
        value.clone()
    }
}

fn truncate(text: &str, max: usize) -> String {
    let mut chars = text.chars();
    let mut result: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        result.push_str("...");
    }
    result
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn preview(text: &str, max: usize) -> String {
    let normalized = collapse_whitespace(&strip_tags(text));
    if normalized.is_empty() {
        String::new()
    } else {
        truncate(&normalized, max)
    }
}

fn strip_tags(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut depth = 0usize;
    for character in text.chars() {
        match character {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 => result.push(character),
            _ => {}
        }
    }
    result
}

pub fn has_visible_text(text: &str) -> bool {
    text.chars().any(|ch| !ch.is_whitespace() && !matches!(ch, '\u{200b}'..='\u{200d}' | '\u{2060}' | '\u{feff}'))
}

// ---------------------------------------------------------------------------
// Tool identity
// ---------------------------------------------------------------------------

/// Canonical call arrays occur on assistant rows, results on later tool rows.
pub fn primary_tool(tool: &Value) -> &Value {
    tool["tool_calls"]
        .as_array()
        .and_then(|calls| calls.first())
        .unwrap_or(tool)
}

/// Canonical structured tool identity; never infer it from transcript text.
pub fn tool_name(tool: &Value) -> &str {
    let tool = primary_tool(tool);
    tool["tool_name"]
        .as_str()
        .or_else(|| tool["name"].as_str())
        .or_else(|| tool["tool_call"]["name"].as_str())
        .or_else(|| tool["tool_call"]["function"]["name"].as_str())
        .or_else(|| tool["function"]["name"].as_str())
        .unwrap_or("")
}

pub fn tool_arguments(tool: &Value) -> Value {
    let tool = primary_tool(tool);
    let call = tool.get("tool_call").unwrap_or(tool);
    let function = call.get("function").unwrap_or(call);
    let details = decoded(&tool["details"]);
    let args = if details.is_object()
        && ["old_string", "new_string", "content"]
            .iter()
            .any(|key| details.get(*key).is_some())
    {
        details
    } else {
        decoded(
            tool.get("arguments")
                .or_else(|| function.get("arguments"))
                .unwrap_or(&Value::Null),
        )
    };
    if args.is_object() {
        args
    } else {
        Value::Object(serde_json::Map::new())
    }
}

fn is_mcp_tool(name: &str, category: Option<&str>) -> bool {
    if category.is_some_and(|value| value.eq_ignore_ascii_case("mcp")) {
        return true;
    }
    let normalized = name.trim().to_lowercase();
    normalized == "web_fetch"
        || normalized == "web_search"
        || normalized == "mcp_tool_expand"
        || normalized == "mcp_tool_execute"
        || normalized == "mcp_tool_load"
        || normalized.contains("__mcp__")
}

/// Tool lifecycle state derived only from structured metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Rejected,
}

impl ToolStatus {
    fn from_message(message: &Message) -> Self {
        let metadata = &message.metadata;
        let execution = metadata["execution_status"]
            .as_str()
            .unwrap_or("")
            .to_lowercase();
        let approval = metadata["approval_status"]
            .as_str()
            .unwrap_or("")
            .to_lowercase();
        let failed = message.is_error
            || metadata["is_error"].as_bool().unwrap_or(false)
            || execution == "failed";
        // Live tool lifecycle events carry `type` instead of `execution_status`.
        let event_type = metadata["type"].as_str().unwrap_or("");
        match execution.as_str() {
            "" if !event_type.is_empty() => match event_type {
                "tool_failed" => Self::Failed,
                "tool_completed" => Self::Completed,
                _ => Self::Running,
            },
            "rejected" => Self::Rejected,
            "failed" => Self::Failed,
            "running" | "approval_submitted" => Self::Running,
            "pending_approval" => Self::Pending,
            "completed" => {
                if failed {
                    Self::Failed
                } else {
                    Self::Completed
                }
            }
            _ => match approval.as_str() {
                "pending" => Self::Pending,
                "rejected" => Self::Rejected,
                "approved" => {
                    if failed {
                        Self::Failed
                    } else {
                        Self::Running
                    }
                }
                _ => {
                    if failed {
                        Self::Failed
                    } else if message.role == "tool" {
                        Self::Completed
                    } else {
                        Self::Running
                    }
                }
            },
        }
    }

    pub fn is_running(self) -> bool {
        matches!(self, Self::Running | Self::Pending)
    }

    pub fn is_error(self) -> bool {
        matches!(self, Self::Failed | Self::Rejected)
    }

    fn progress_rank(self) -> u8 {
        match self {
            Self::Completed | Self::Failed | Self::Rejected => 2,
            Self::Running => 1,
            Self::Pending => 0,
        }
    }
}

/// Ordered activity categories decide the group icon and summary ordering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolGroupKind {
    Read,
    Search,
    Command,
    Mutation,
    Task,
    SubAgentResult,
    Mcp,
    Other,
}

impl ToolGroupKind {
    fn of(name: &str, category: Option<&str>) -> Option<Self> {
        if TODO_TOOLS.contains(&name) {
            return Some(Self::Task);
        }
        if READ_TOOLS.contains(&name) {
            return Some(Self::Read);
        }
        if SEARCH_TOOLS.contains(&name) {
            return Some(Self::Search);
        }
        if COMMAND_TOOLS.contains(&name) {
            return Some(Self::Command);
        }
        if MUTATION_TOOLS.contains(&name) {
            return Some(Self::Mutation);
        }
        if name == "sub_agent_output" {
            return Some(Self::SubAgentResult);
        }
        if is_mcp_tool(name, category) {
            return Some(Self::Mcp);
        }
        if NON_COLLAPSIBLE_TOOLS.contains(&name) || name.is_empty() {
            return None;
        }
        Some(Self::Other)
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Read | Self::Search => "search",
            Self::Command => "bash",
            Self::Mutation => "edit",
            Self::Task => "todo",
            Self::SubAgentResult => "skill-relation-chart",
            Self::Mcp => "mcp",
            Self::Other => "tool",
        }
    }

    fn label(self) -> String {
        match self {
            Self::Read => t!("group_read_file").to_string(),
            Self::Search => t!("group_file_search").to_string(),
            Self::Command => t!("group_run_command").to_string(),
            Self::Mutation => t!("group_edit_file").to_string(),
            Self::Task => t!("group_task_changes").to_string(),
            Self::SubAgentResult => t!("group_fetch_result").to_string(),
            Self::Mcp => t!("group_call_mcp").to_string(),
            Self::Other => t!("group_use_tool").to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct ToolView {
    pub key: String,
    pub detail_key: String,
    /// Runtime tool-call id; the approval signal names this value.
    pub tool_call_id: String,
    pub name: String,
    pub icon: &'static str,
    pub kind: Option<ToolGroupKind>,
    pub action: String,
    pub target: String,
    pub badges: Vec<String>,
    pub summary: String,
    pub status: ToolStatus,
    pub approved: bool,
    pub command: Option<String>,
    pub arguments: Option<String>,
    pub diff: Option<Vec<DiffLine>>,
    pub detail: String,
    pub choices: Vec<ChoiceGroupView>,
}

impl ToolView {
    pub fn new(message: &Message, roots: &[String]) -> Self {
        let name = tool_name(&message.metadata).to_owned();
        let category = message.metadata["tool_category"].as_str();
        let args = tool_arguments(&message.metadata);
        let status = ToolStatus::from_message(message);
        let (action, target) = tool_title(&name, &args, &message.metadata, roots);
        // The title is one ellipsized line, so multi-line arguments (bash
        // scripts, heredocs) must not reach the layout.
        let action = single_line(&action);
        let target = single_line(&target);
        let mut badges = Vec::new();
        let route = route_label(&message.metadata);
        if !route.is_empty() {
            badges.push(route);
        }
        let duration = duration_label(&name, &message.metadata);
        if !duration.is_empty() {
            badges.push(duration);
        }
        let approval = message.metadata["approval_status"]
            .as_str()
            .unwrap_or("")
            .to_lowercase();
        let approved = status == ToolStatus::Completed
            && (approval == "approved" || tool_call_id(message).is_some())
            && !matches!(name.as_str(), "ask_user" | "submit_plan");
        let key = message_key(message);
        let is_mcp = is_mcp_tool(&name, category);
        let display_type = message.metadata["display_type"].as_str().unwrap_or("");
        let diff = edit_lines(&message.metadata);
        let command = (name == "bash")
            .then(|| args["command"].as_str())
            .flatten()
            .map(|command| normalize_display_text(command, roots))
            .filter(|command| !command.trim().is_empty());
        let arguments = is_mcp
            .then(|| {
                if args.as_object().is_some_and(|object| object.is_empty()) {
                    String::new()
                } else {
                    serde_json::to_string_pretty(&args).unwrap_or_default()
                }
            })
            .filter(|arguments| !arguments.is_empty());
        let detail = tool_detail(
            message,
            &name,
            command.as_deref(),
            display_type,
            diff.is_some(),
        );
        let icon = crate::icons::tool_icon_for(&name, category);
        let kind = ToolGroupKind::of(&name, category);
        let summary = tool_summary(message, &name, status);
        Self {
            detail_key: format!("{key}:detail"),
            tool_call_id: tool_call_id(message).unwrap_or_default().to_owned(),
            key,
            name,
            icon,
            kind,
            action,
            target,
            badges,
            summary,
            status,
            approved,
            command,
            arguments,
            diff,
            detail,
            choices: Vec::new(),
        }
    }
}

/// Token and cost totals for one view of a task (alone or with sub-agents).
#[derive(Clone, Debug, Default)]
pub struct CostTotals {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_tokens: u64,
    pub total_tokens: u64,
    pub estimated_cost: Option<f64>,
    pub effective_cost_per_million: Option<f64>,
}

impl CostTotals {
    fn parse(value: &Value) -> Option<Self> {
        let number = |key: &str| value[key].as_u64();
        Some(Self {
            input_tokens: number("input_tokens")?,
            output_tokens: number("output_tokens")?,
            cache_tokens: number("cache_tokens")?,
            total_tokens: number("total_tokens")?,
            estimated_cost: value["estimated_cost"].as_f64(),
            effective_cost_per_million: value["effective_cost_per_million"].as_f64(),
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct CostModel {
    pub backend_model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_tokens: u64,
    pub cache_hit_rate: Option<f64>,
    pub input_per_million: Option<f64>,
    pub output_per_million: Option<f64>,
    pub cache_per_million: Option<f64>,
    pub multiplier: Option<f64>,
    pub estimated_cost: Option<f64>,
}

impl CostModel {
    fn parse(value: &Value) -> Option<Self> {
        let input_tokens = value["input_tokens"].as_u64()?;
        let cache_tokens = value["cache_tokens"].as_u64()?;
        Some(Self {
            backend_model: value["backend_model"].as_str().unwrap_or("").to_owned(),
            input_tokens,
            output_tokens: value["output_tokens"].as_u64()?,
            cache_tokens,
            cache_hit_rate: (input_tokens > 0)
                .then(|| cache_tokens as f64 / input_tokens as f64 * 100.0),
            input_per_million: value["input_per_million"].as_f64(),
            output_per_million: value["output_per_million"].as_f64(),
            cache_per_million: value["cache_per_million"].as_f64(),
            multiplier: value["multiplier"].as_f64(),
            estimated_cost: value["estimated_cost"].as_f64(),
        })
    }
}

/// Cost card shared by completion reports and delegated-task cards.
#[derive(Clone, Debug, Default)]
pub struct CostView {
    pub duration_ms: Option<u64>,
    pub self_usage: CostTotals,
    pub with_sub_agents: CostTotals,
    pub has_sub_agents: bool,
    pub is_partial: bool,
    pub models: Vec<CostModel>,
}

impl CostView {
    /// Vue's `normalizeUsageSummary`: version 1 envelopes with complete totals.
    pub fn parse(value: &Value) -> Option<Self> {
        if value["version"].as_i64() != Some(1) {
            return None;
        }
        Some(Self {
            duration_ms: value["duration_ms"].as_u64(),
            self_usage: CostTotals::parse(&value["self_usage"])?,
            with_sub_agents: CostTotals::parse(&value["with_sub_agents"])?,
            has_sub_agents: value["has_sub_agents"].as_bool().unwrap_or(false),
            is_partial: value["is_partial"].as_bool().unwrap_or(false),
            models: value["model_breakdowns"]
                .as_array()
                .map(|models| models.iter().filter_map(CostModel::parse).collect())
                .unwrap_or_default(),
        })
    }
}

/// One `ask_user` question with its read-only answer state.
#[derive(Clone, Debug, Default)]
pub struct ChoiceGroupView {
    pub title: String,
    pub options: Vec<String>,
    pub selected: Option<usize>,
    pub other_selected: bool,
    pub answer: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ActivityItem {
    pub key: String,
    pub thought: Option<String>,
    pub thought_key: String,
    pub tool: Option<ToolView>,
}

#[derive(Clone, Debug)]
pub struct ActivityView {
    pub key: String,
    pub icon: &'static str,
    pub summary: String,
    pub thought_count: usize,
    pub error_count: usize,
    pub ongoing: bool,
    pub items: Vec<ActivityItem>,
}

#[derive(Clone, Debug)]
pub struct AssistantView {
    pub key: String,
    pub reasoning: Option<String>,
    pub thought_key: String,
    pub thought_title: String,
    pub markdown: String,
}

#[derive(Clone, Debug)]
pub struct UserView {
    pub key: String,
    pub text: String,
    pub delivery_label: Option<String>,
}

#[derive(Clone, Debug)]
pub struct FinishTaskView {
    pub key: String,
    pub failed: bool,
    pub label: String,
    pub cost_key: String,
    pub cost: Option<CostView>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertKind {
    Warning,
    Error,
}

/// Vue keeps error records out of the transcript and surfaces the trailing one
/// as an Element Plus alert below the list.
#[derive(Clone, Debug)]
pub struct AlertView {
    pub key: String,
    pub kind: AlertKind,
    pub title: String,
    pub body: String,
}

#[derive(Clone, Debug)]
pub struct SnapshotView {
    pub key: String,
    pub title: String,
    pub preview: String,
    pub body_key: String,
    pub body: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubAgentStatus {
    Starting,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl SubAgentStatus {
    fn parse(value: &str) -> Self {
        match value.trim().to_lowercase().as_str() {
            "completed" | "success" => Self::Completed,
            "failed" | "error" => Self::Failed,
            "cancelled" | "interrupted" | "stopped" => Self::Cancelled,
            _ => Self::Running,
        }
    }

    fn label(self) -> String {
        match self {
            Self::Starting => t!("sub_agent_starting").to_string(),
            Self::Running => t!("sub_agent_running").to_string(),
            Self::Completed => t!("sub_agent_completed").to_string(),
            Self::Failed => t!("sub_agent_failed").to_string(),
            Self::Cancelled => t!("sub_agent_cancelled").to_string(),
        }
    }

    pub fn is_running(self) -> bool {
        self == Self::Running
    }
}

#[derive(Clone, Debug)]
pub struct SubAgentView {
    pub key: String,
    /// Child session id whose transcript the expanded card loads.
    pub task_id: Option<String>,
    pub status: SubAgentStatus,
    pub status_label: String,
    pub agent: String,
    pub mode: String,
    pub tools: String,
    pub context: String,
    pub task: String,
    pub task_preview: String,
    pub task_key: String,
    pub result: String,
    pub result_preview: String,
    pub result_key: String,
    pub has_result: bool,
    pub cost_key: String,
    pub cost: Option<CostView>,
}

#[derive(Clone, Debug)]
pub enum Row {
    User(UserView),
    Assistant(AssistantView),
    Activity(ActivityView),
    Tool(Box<ToolView>),
    SubAgent(Box<SubAgentView>),
    FinishTask(FinishTaskView),
    Snapshot(SnapshotView),
    Alert(AlertView),
    Divider,
}

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

/// Stable expansion/cache identity survives live-to-durable reconciliation.
pub fn message_key(message: &Message) -> String {
    message.metadata["tool_call_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .map(|id| format!("call-{id}"))
        .unwrap_or_else(|| format!("msg-{}", message.id))
}

fn tool_call_id(message: &Message) -> Option<&str> {
    message.metadata["tool_call_id"]
        .as_str()
        .filter(|id| !id.is_empty())
}

// ---------------------------------------------------------------------------
// Tool titles, summaries and details
// ---------------------------------------------------------------------------

pub fn format_display_path(path: &str, roots: &[String]) -> String {
    if path.is_empty() {
        return String::new();
    }
    let normalized = path.trim().replace('\\', "/");
    let mut best: Option<&String> = None;
    for root in roots {
        let candidate = root.trim().replace('\\', "/");
        let candidate = candidate.trim_end_matches('/');
        if candidate.is_empty() {
            continue;
        }
        let inside = normalized == candidate
            || normalized.starts_with(&format!("{candidate}/"))
            || candidate == "/";
        if inside && best.is_none_or(|current| candidate.len() > current.len()) {
            best = roots
                .iter()
                .find(|value| value.trim().replace('\\', "/").trim_end_matches('/') == candidate);
        }
    }
    if let Some(root) = best {
        let root = root.trim().replace('\\', "/");
        let root = root.trim_end_matches('/');
        if normalized == root {
            return ".".into();
        }
        if root.is_empty() {
            return normalized;
        }
        return normalized
            .strip_prefix(root)
            .map(|rest| rest.trim_start_matches('/'))
            .filter(|rest| !rest.is_empty())
            .unwrap_or(".")
            .to_owned();
    }
    normalized
}

fn normalize_display_text(text: &str, roots: &[String]) -> String {
    let mut result = text.to_owned();
    for root in roots {
        let normalized = root.trim().replace('\\', "/");
        let normalized = normalized.trim_end_matches('/');
        if normalized.len() > 1 {
            result = result.replace(&format!("{normalized}/"), "");
        }
    }
    result
}

fn format_tool_title(name: &str, args: &Value, roots: &[String]) -> (String, String) {
    let path = |value: &str| format_display_path(value, roots);
    match name {
        "read_file" => {
            let path = path(
                args["file_path"]
                    .as_str()
                    .or(args["path"].as_str())
                    .unwrap_or(""),
            );
            let limit = args["limit"].as_u64();
            let offset = args["offset"].as_u64();
            let suffix = match (limit, offset) {
                (Some(limit), Some(offset)) => format!(" L{}-{}", offset + 1, offset + limit),
                (Some(limit), None) => format!(" L1-{limit}"),
                (None, Some(offset)) => format!(" L{}", offset + 1),
                (None, None) => String::new(),
            };
            ("Read".into(), format!("{path}{suffix}"))
        }
        "write_file" => (
            "Write".into(),
            path(
                args["file_path"]
                    .as_str()
                    .or(args["path"].as_str())
                    .unwrap_or(""),
            ),
        ),
        "edit_file" => (
            "Edit".into(),
            path(
                args["file_path"]
                    .as_str()
                    .or(args["path"].as_str())
                    .unwrap_or(""),
            ),
        ),
        "list_dir" => (
            "List".into(),
            path(
                args["path"]
                    .as_str()
                    .or(args["dir"].as_str())
                    .unwrap_or("."),
            ),
        ),
        "glob" => {
            let pattern = args["pattern"]
                .as_str()
                .or(args["glob"].as_str())
                .unwrap_or("");
            (
                format!("Glob {pattern}").trim_end().to_owned(),
                path(args["path"].as_str().unwrap_or("")),
            )
        }
        "grep" => {
            let pattern = args["pattern"]
                .as_str()
                .or(args["query"].as_str())
                .unwrap_or("");
            let path = path(args["path"].as_str().unwrap_or(""));
            if pattern.is_empty() {
                (
                    format!("Grep {}", args["glob"].as_str().unwrap_or("*")),
                    path,
                )
            } else if path.is_empty() {
                (format!("Grep \"{pattern}\""), String::new())
            } else {
                (format!("Grep \"{pattern}\" in {path}"), String::new())
            }
        }
        "web_fetch" => (
            format!("Fetch {}", args["url"].as_str().unwrap_or(""))
                .trim_end()
                .to_owned(),
            String::new(),
        ),
        "web_search" => {
            let query = args["query"].as_str().unwrap_or("");
            match args["num_results"].as_u64() {
                Some(count) => (
                    format!("Search \"{query}\" (Count: {count})"),
                    String::new(),
                ),
                None => (format!("Search \"{query}\""), String::new()),
            }
        }
        "bash" => {
            let command = normalize_display_text(args["command"].as_str().unwrap_or(""), roots);
            let command = command.trim();
            (
                if command.is_empty() {
                    "Run".into()
                } else {
                    format!("Run {command}")
                },
                String::new(),
            )
        }
        "sub_agent_run" => {
            let agent = args["child_agent_name"]
                .as_str()
                .or(args["child_agent_id"].as_str())
                .unwrap_or("");
            let background = args["execution_mode"].as_str() == Some("background");
            (
                if background {
                    "Run Sub-agent in Background".into()
                } else {
                    "Run Sub-agent".into()
                },
                agent.to_owned(),
            )
        }
        "sub_agent_output" => (
            "Get Sub-agent Output".into(),
            args["task_id"].as_str().unwrap_or("").to_owned(),
        ),
        "sub_agent_stop" => (
            "Stop Sub-agent".into(),
            args["task_id"].as_str().unwrap_or("").to_owned(),
        ),
        "todo_create" => {
            let subject = args["subject"]
                .as_str()
                .or(args["title"].as_str())
                .unwrap_or("");
            if !subject.is_empty() {
                (t!("todo_create").to_string(), truncate(subject, 25))
            } else if let Some(tasks) = args["tasks"].as_array() {
                (
                    t!("todo_create_batch").to_string(),
                    format!("{} {}", tasks.len(), t!("todo_items")),
                )
            } else {
                (t!("todo_create").to_string(), String::new())
            }
        }
        "todo_update" => {
            let subject = args["subject"]
                .as_str()
                .or(args["title"].as_str())
                .unwrap_or("");
            let status = match args["status"].as_str().unwrap_or("") {
                "completed" => t!("todo_status_completed").to_string(),
                "in_progress" => t!("todo_status_in_progress").to_string(),
                "pending" => t!("todo_status_pending").to_string(),
                other => other.to_owned(),
            };
            if !subject.is_empty() && !status.is_empty() {
                (
                    format!("Update {} to {status}", truncate(subject, 20)),
                    String::new(),
                )
            } else {
                (t!("todo_update").to_string(), String::new())
            }
        }
        "todo_list" => (
            match args["todo_id"].as_str() {
                Some(id) => format!("{} {id}", t!("todo_view")),
                None => t!("todo_list").to_string(),
            },
            String::new(),
        ),
        "plan_note" => (
            t!("todo_plan_note").to_string(),
            args["action"].as_str().unwrap_or("read").to_owned(),
        ),
        "complete_workflow" => (t!("finish_task").to_string(), String::new()),
        "" => (String::new(), String::new()),
        other => (
            other
                .split('_')
                .map(|word| {
                    let mut chars = word.chars();
                    match chars.next() {
                        Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                        None => String::new(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" "),
            String::new(),
        ),
    }
}

fn tool_title(name: &str, args: &Value, metadata: &Value, roots: &[String]) -> (String, String) {
    let (mut action, mut target) = format_tool_title(name, args, roots);
    if name == "complete_workflow" {
        return (t!("finish_task").to_string(), String::new());
    }
    if name != "bash" {
        if let Some(title) = metadata["title"]
            .as_str()
            .map(clean_text)
            .filter(|title| !title.is_empty())
        {
            let normalized_title = normalize_display_text(&title, roots);
            let normalized_action = normalize_display_text(&action, roots);
            let title_includes_target = !target.is_empty() && normalized_title.contains(&target);
            action = title;
            if target.is_empty() || title_includes_target || normalized_title != normalized_action {
                target.clear();
            }
        }
    }
    (action, target)
}

fn tool_summary(message: &Message, name: &str, status: ToolStatus) -> String {
    if name == "complete_workflow" {
        return String::new();
    }
    if let Some(display) = message.metadata["user_error"]["display"]
        .as_str()
        .map(clean_text)
        .filter(|display| !display.is_empty())
    {
        return display;
    }
    let fallback = message.metadata["summary"]
        .as_str()
        .map(clean_text)
        .unwrap_or_else(|| clean_text(&message.message));
    match status {
        ToolStatus::Pending => t!("awaiting_approval").to_string(),
        ToolStatus::Running => t!("executing").to_string(),
        ToolStatus::Rejected => t!("rejected_label").to_string(),
        ToolStatus::Failed => fallback,
        ToolStatus::Completed => match name {
            "edit_file" => t!("edited").to_string(),
            "write_file" => t!("written").to_string(),
            _ => fallback,
        },
    }
}

fn route_label(metadata: &Value) -> String {
    let plan = &metadata["execution_plan"];
    if !plan.is_object() {
        return String::new();
    }
    let backend = match plan["backend"].as_str().unwrap_or("") {
        "host" => t!("exec_backend_host").to_string(),
        "docker" => t!("exec_backend_docker").to_string(),
        "msb" => t!("exec_backend_msb").to_string(),
        "" => String::new(),
        other => other.to_owned(),
    };
    let route = match plan["backend_origin"]
        .as_str()
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "explicit_host_rule" => t!("exec_route_explicit_host").to_string(),
        "host_only" => t!("exec_route_host_only").to_string(),
        "sandbox_profile" => t!("exec_route_sandbox").to_string(),
        _ => String::new(),
    };
    [backend, route]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

fn duration_label(name: &str, metadata: &Value) -> String {
    if DURATION_EXCLUDED_TOOLS.contains(&name) || name.starts_with("todo_") {
        return String::new();
    }
    let Some(duration_ms) = metadata["duration_ms"].as_u64() else {
        return String::new();
    };
    let total_seconds = (duration_ms / 1000).max(1);
    if duration_ms <= 60_000 {
        return format!("{total_seconds}s");
    }
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    if seconds == 0 {
        format!("{minutes}m")
    } else {
        format!("{minutes}m{seconds}s")
    }
}

/// MCP results arrive as `{"content":[{"text": ...}]}`. The structured envelope
/// is transport, not output: the panel shows the text the tool actually
/// returned instead of the raw JSON wrapper.
fn mcp_result_text(message: &Message, display_type: &str) -> String {
    let cleaned = clean_text(&message.message);
    if display_type == "markdown" {
        return cleaned;
    }
    let Ok(value) = serde_json::from_str::<Value>(cleaned.trim()) else {
        return cleaned;
    };
    let mut parts = Vec::new();
    match &value["content"] {
        Value::Array(items) => {
            for item in items {
                if let Some(text) = item["text"].as_str() {
                    if !text.trim().is_empty() {
                        parts.push(text.trim().to_owned());
                    }
                } else if let Some(text) = item.as_str() {
                    if !text.trim().is_empty() {
                        parts.push(text.trim().to_owned());
                    }
                }
            }
        }
        Value::String(text) if !text.trim().is_empty() => parts.push(text.trim().to_owned()),
        _ => {}
    }
    if parts.is_empty() {
        cleaned
    } else {
        parts.join("\n\n")
    }
}

fn tool_detail(
    message: &Message,
    name: &str,
    command: Option<&str>,
    display_type: &str,
    has_diff: bool,
) -> String {
    let mut sections = Vec::new();
    let mcp = is_mcp_tool(name, message.metadata["tool_category"].as_str());
    if mcp {
        let args = tool_arguments(&message.metadata);
        if args.as_object().is_some_and(|object| !object.is_empty()) {
            sections.push(format!(
                "**{}**\n\n{}",
                t!("arguments_label"),
                fence(&serde_json::to_string_pretty(&args).unwrap_or_default(), "json")
            ));
        }
    }
    let cleaned = if mcp {
        // Loading a folded MCP definition is an internal step; its "result" is
        // the declaration itself, which users read as noise. Real calls show
        // the returned text.
        if matches!(name, "mcp_tool_expand" | "mcp_tool_load") {
            String::new()
        } else {
            mcp_result_text(message, display_type)
        }
    } else {
        clean_text(&message.message)
    };
    let duplicate_command = command.is_some_and(|command| {
        cleaned.replace("\r\n", "\n").trim() == command.replace("\r\n", "\n").trim()
    });
    // Structured diffs own the expanded body; the JSON result would repeat it.
    // Choice payloads belong to the `ask_user` card, not the raw text section.
    if !cleaned.is_empty() && !duplicate_command && !has_diff && display_type != "choice" {
        let body = if display_type == "markdown" {
            cleaned
        } else {
            fence(&cleaned, "text")
        };
        if mcp {
            sections.push(format!("**{}**\n\n{body}", t!("result_label")));
        } else {
            sections.push(body);
        }
    }
    sections.join("\n\n")
}

/// One rendered diff row with Vue's two line-number columns.
#[derive(Clone, Debug)]
pub struct DiffLine {
    pub kind: DiffKind,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffKind {
    Context,
    Removed,
    Added,
}

fn string_list(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|line| line.as_str().map(str::to_owned))
        .collect()
}

fn split_diff_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&str> = text.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    lines
}

/// Line-level LCS, like the `diff` package Vue uses for `FilePreviewDiff`.
/// Replacements too large for the table fall back to remove-then-add.
fn diff_sequence<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<(DiffKind, &'a str)> {
    let stride = new.len() + 1;
    if (old.len() + 1).saturating_mul(stride) > 250_000 {
        return old
            .iter()
            .map(|line| (DiffKind::Removed, *line))
            .chain(new.iter().map(|line| (DiffKind::Added, *line)))
            .collect();
    }
    let mut lengths = vec![0_u32; (old.len() + 1) * stride];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            let at = i * stride + j;
            lengths[at] = if old[i] == new[j] {
                1 + lengths[at + stride + 1]
            } else {
                lengths[at + stride].max(lengths[at + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut rows = Vec::new();
    while i < old.len() || j < new.len() {
        if i < old.len() && j < new.len() && old[i] == new[j] {
            rows.push((DiffKind::Context, old[i]));
            i += 1;
            j += 1;
        } else if i < old.len()
            && (j == new.len() || lengths[(i + 1) * stride + j] >= lengths[i * stride + j + 1])
        {
            rows.push((DiffKind::Removed, old[i]));
            i += 1;
        } else {
            rows.push((DiffKind::Added, new[j]));
            j += 1;
        }
    }
    rows
}

/// Extract canonical edit details for the native diff renderer, including the
/// `context_before` / `context_after` lines the runtime sends with the edit.
pub fn edit_lines(tool: &Value) -> Option<Vec<DiffLine>> {
    let name = tool_name(tool);
    if !matches!(name, "edit_file" | "write_file" | "plan_note") && tool["display_type"] != "diff" {
        return None;
    }
    let args = tool_arguments(tool);
    let start_line = args["start_line"].as_u64().unwrap_or(1).max(1) as usize;
    let before = string_list(&args["context_before"]);
    let after = string_list(&args["context_after"]);
    let old = args["old_string"].as_str().unwrap_or("");
    let new = args["new_string"]
        .as_str()
        .or_else(|| args["content"].as_str())?;

    let mut lines = Vec::new();
    let before_start = args["context_before_start_line"]
        .as_u64()
        .map(|value| value.max(1) as usize)
        .unwrap_or_else(|| start_line.saturating_sub(before.len()).max(1));
    for (index, text) in before.iter().enumerate() {
        let number = before_start + index;
        lines.push(DiffLine {
            kind: DiffKind::Context,
            old_line: Some(number),
            new_line: Some(number),
            text: text.clone(),
        });
    }

    let old_lines = split_diff_lines(old);
    let new_lines = split_diff_lines(new);
    let mut old_number = start_line;
    let mut new_number = start_line;
    for (kind, text) in diff_sequence(&old_lines, &new_lines) {
        let (old_line, new_line) = match kind {
            DiffKind::Context => {
                let numbers = (Some(old_number), Some(new_number));
                old_number += 1;
                new_number += 1;
                numbers
            }
            DiffKind::Removed => {
                let numbers = (Some(old_number), None);
                old_number += 1;
                numbers
            }
            DiffKind::Added => {
                let numbers = (None, Some(new_number));
                new_number += 1;
                numbers
            }
        };
        lines.push(DiffLine {
            kind,
            old_line,
            new_line,
            text: text.to_owned(),
        });
    }

    let after_start = args["context_after_start_line"]
        .as_u64()
        .map(|value| value.max(1) as usize)
        .unwrap_or(new_number);
    for (index, text) in after.iter().enumerate() {
        let number = after_start + index;
        lines.push(DiffLine {
            kind: DiffKind::Context,
            old_line: Some(number),
            new_line: Some(number),
            text: text.clone(),
        });
    }
    Some(lines)
}

// ---------------------------------------------------------------------------
// Snapshot parsing
// ---------------------------------------------------------------------------

fn snapshot_content(message: &Message) -> String {
    let content = clean_text(&message.message);
    let trimmed = content.trim_start();
    let without_heading = [
        "## Previous Context Snapshot",
        "# Previous Context Snapshot",
    ]
    .iter()
    .find_map(|heading| trimmed.strip_prefix(*heading))
    .unwrap_or(trimmed);
    without_heading.trim().to_owned()
}

fn json_scalar(value: &Value) -> String {
    match value {
        Value::String(text) => text.trim().to_owned(),
        Value::Number(number) => number.to_string(),
        Value::Bool(boolean) => boolean.to_string(),
        Value::Null => String::new(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn json_section_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.trim().to_owned(),
        Value::Array(items) => items
            .iter()
            .map(|item| match item {
                Value::Object(object) => object
                    .iter()
                    .map(|(key, entry)| format!("{key}: {}", json_scalar(entry)))
                    .collect::<Vec<_>>()
                    .join("\n"),
                other => json_scalar(other),
            })
            .map(|line| line.trim().to_owned())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(object) => object
            .iter()
            .map(|(key, entry)| match entry {
                Value::Array(items) => {
                    let lines = items
                        .iter()
                        .map(json_scalar)
                        .filter(|line| !line.is_empty())
                        .collect::<Vec<_>>();
                    if lines.is_empty() {
                        String::new()
                    } else {
                        format!("{key}:\n{}", lines.join("\n"))
                    }
                }
                other => format!("{key}: {}", json_scalar(other)),
            })
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn markdown_list(value: &Value) -> String {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(json_scalar)
                .filter(|line| !line.is_empty())
                .map(|line| format!("- {line}"))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn boundary_open_items(value: &Value) -> String {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let summary = item["summary"].as_str().map(str::trim).unwrap_or("");
                    if summary.is_empty() {
                        return None;
                    }
                    let kind = match item["kind"].as_str().unwrap_or("") {
                        "verification" => t!("snapshot_open_verification").to_string(),
                        "decision" => t!("snapshot_open_decision").to_string(),
                        "remediation" => t!("snapshot_open_remediation").to_string(),
                        other => other.to_owned(),
                    };
                    Some(format!("- **{kind}:** {summary}"))
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn review_round_status(review: &Value) -> String {
    let status = review["resolution_status"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| {
            if review["verdict"]["approved"].as_bool() == Some(true) {
                "approved_review".into()
            } else {
                "open".into()
            }
        });
    match status.as_str() {
        "approved_review" => t!("snapshot_review_approved").to_string(),
        "verified_by_re_review" => t!("snapshot_review_verified").to_string(),
        "open" => t!("snapshot_review_open").to_string(),
        other => other.to_owned(),
    }
}

fn review_round(review: &Value) -> String {
    let id = json_scalar(&review["id"]);
    let heading = format!(
        "#### {}",
        t!(
            "snapshot_review_round",
            id = if id.is_empty() { "?" } else { &id },
            status = review_round_status(review)
        )
    );
    let mut sections = Vec::new();
    let summary = review["verdict"]["summary"]
        .as_str()
        .map(str::trim)
        .unwrap_or("");
    if !summary.is_empty() {
        sections.push(summary.to_owned());
    }
    let findings = markdown_list(&review["findings"]);
    if !findings.is_empty() {
        sections.push(format!("**{}**\n{}", t!("snapshot_findings"), findings));
    }
    let fixes = if review["required_fixes"].is_null() {
        markdown_list(&review["requiredFixes"])
    } else {
        markdown_list(&review["required_fixes"])
    };
    if !fixes.is_empty() {
        sections.push(format!("**{}**\n{}", t!("snapshot_required_fixes"), fixes));
    }
    let evidence = markdown_list(&review["evidence_refs"]);
    if !evidence.is_empty() {
        sections.push(format!("**{}**\n{}", t!("snapshot_evidence"), evidence));
    }
    let mut body = heading;
    if !sections.is_empty() {
        body.push_str("\n\n");
        body.push_str(&sections.join("\n\n"));
    }
    body
}

fn snapshot_v2_markdown(snapshot: &Value) -> String {
    let kind = match snapshot["schema_version"].as_u64() {
        Some(2) => snapshot["kind"].as_str().unwrap_or(""),
        _ => "",
    };
    let sections: Vec<(String, String)> = match kind {
        "pressure_handoff" => vec![
            (
                t!("snapshot_user_directives").to_string(),
                markdown_list(&snapshot["user_directives"]),
            ),
            (
                t!("snapshot_confirmed_facts").to_string(),
                markdown_list(&snapshot["confirmed_facts"]),
            ),
            (
                t!("snapshot_boundary_open_items").to_string(),
                boundary_open_items(&snapshot["boundary_open_items"]),
            ),
            (
                t!("snapshot_completed_work").to_string(),
                markdown_list(&snapshot["completed_work"]),
            ),
            (
                t!("snapshot_file_changes").to_string(),
                markdown_list(&snapshot["file_changes"]),
            ),
            (
                t!("snapshot_constraints").to_string(),
                markdown_list(&snapshot["constraints_and_guards"]),
            ),
            (
                t!("snapshot_review_rounds").to_string(),
                snapshot["review_rounds"]
                    .as_array()
                    .map(|rounds| {
                        rounds
                            .iter()
                            .map(review_round)
                            .filter(|round| !round.is_empty())
                            .collect::<Vec<_>>()
                            .join("\n\n")
                    })
                    .unwrap_or_default(),
            ),
        ],
        "completed_task_rollup" => vec![
            (
                t!("snapshot_confirmed_facts").to_string(),
                markdown_list(&snapshot["confirmed_facts"]),
            ),
            (
                t!("snapshot_unresolved_carryovers").to_string(),
                markdown_list(&snapshot["unresolved_carryovers"]),
            ),
            (
                t!("snapshot_completed_work").to_string(),
                markdown_list(&snapshot["completed_work"]),
            ),
            (
                t!("snapshot_file_changes").to_string(),
                markdown_list(&snapshot["file_changes"]),
            ),
            (
                t!("snapshot_constraints").to_string(),
                markdown_list(&snapshot["constraints_and_guards"]),
            ),
            (
                t!("snapshot_review_rounds").to_string(),
                snapshot["review_rounds"]
                    .as_array()
                    .map(|rounds| {
                        rounds
                            .iter()
                            .map(review_round)
                            .filter(|round| !round.is_empty())
                            .collect::<Vec<_>>()
                            .join("\n\n")
                    })
                    .unwrap_or_default(),
            ),
        ],
        _ => Vec::new(),
    };
    sections
        .into_iter()
        .filter(|(_, body)| !body.trim().is_empty())
        .map(|(title, body)| format!("### {title}\n\n{body}"))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn xml_tag_text(document: &str, tag: &str) -> String {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let Some(start) = document.find(&open) else {
        return String::new();
    };
    let body = &document[start + open.len()..];
    body.find(&close)
        .map(|end| body[..end].trim().to_owned())
        .unwrap_or_default()
}

fn snapshot_body(content: &str) -> String {
    if content.trim_start().starts_with('{') {
        if let Ok(parsed) = serde_json::from_str::<Value>(content) {
            let v2 = snapshot_v2_markdown(&parsed);
            if !v2.is_empty() {
                return v2;
            }
            let sections = [
                ("snapshot_overall_goal", &parsed["overall_goal"]),
                ("snapshot_previous_tasks", &parsed["prev_tasks"]),
                ("snapshot_key_knowledge", &parsed["key_knowledge"]),
                ("snapshot_error_log", &parsed["error_log"]),
                ("snapshot_file_system_state", &parsed["file_system_state"]),
                ("snapshot_recent_actions", &parsed["recent_actions"]),
                ("snapshot_task_state", &parsed["task_state"]),
            ];
            let rendered = sections
                .into_iter()
                .map(|(title, value)| (t!(title).to_string(), json_section_text(value)))
                .filter(|(_, value)| !value.is_empty())
                .map(|(title, value)| format!("### {title}\n\n{value}"))
                .collect::<Vec<_>>()
                .join("\n\n");
            if !rendered.is_empty() {
                return rendered;
            }
        }
    }
    if content.contains("<state_snapshot") {
        let sections = [
            ("snapshot_overall_goal", "overall_goal"),
            ("snapshot_key_knowledge", "key_knowledge"),
            ("snapshot_error_log", "error_log"),
            ("snapshot_file_system_state", "file_system_state"),
            ("snapshot_recent_actions", "recent_actions"),
            ("snapshot_task_state", "task_state"),
        ];
        let rendered = sections
            .into_iter()
            .map(|(title, tag)| (t!(title).to_string(), xml_tag_text(content, tag)))
            .filter(|(_, value)| !value.is_empty())
            .map(|(title, value)| format!("### {title}\n\n{value}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        if !rendered.is_empty() {
            return rendered;
        }
    }
    content.to_owned()
}

fn snapshot_view(message: &Message) -> SnapshotView {
    let content = snapshot_content(message);
    let parsed = serde_json::from_str::<Value>(&content).ok();
    let title = match parsed.as_ref() {
        Some(parsed) if parsed["schema_version"].as_u64() == Some(2) => {
            match parsed["kind"].as_str() {
                Some("pressure_handoff") => t!("snapshot_context_handoff").to_string(),
                Some("completed_task_rollup") => t!("snapshot_completed_rollup").to_string(),
                _ => t!("snapshot_previous").to_string(),
            }
        }
        Some(parsed) => {
            let task_state = json_section_text(&parsed["task_state"]);
            match task_state
                .lines()
                .next()
                .map(str::trim)
                .filter(|line| !line.is_empty())
            {
                Some(first) => format!("{} · {first}", t!("snapshot_context_title")),
                None => t!("snapshot_previous").to_string(),
            }
        }
        None => t!("snapshot_previous").to_string(),
    };
    let body = snapshot_body(&content);
    let key = message_key(message);
    SnapshotView {
        body_key: format!("{key}:body"),
        preview: preview(&body, 96),
        title: truncate(&title, 72),
        body,
        key,
    }
}

// ---------------------------------------------------------------------------
// Delegated-task cards
// ---------------------------------------------------------------------------

fn sub_agent_task_id(message: &Message) -> Option<String> {
    let metadata = &message.metadata;
    let data = &metadata["data"];
    [
        &metadata["sub_agent_id"],
        &metadata["subAgentId"],
        &data["sub_agent_id"],
        &data["subAgentId"],
        &data["result"]["task_id"],
        &data["result"]["taskId"],
        &data["task_id"],
    ]
    .into_iter()
    .find_map(|value| value.as_str())
    .map(str::to_owned)
    .or_else(|| {
        let text = message.message.as_str();
        if !text.contains("<tool_result") {
            return None;
        }
        let start = text.find("id=\"")? + 4;
        text[start..]
            .find('"')
            .map(|end| text[start..start + end].to_owned())
    })
    .filter(|id| !id.is_empty())
}

fn collect_sub_agent_completions(messages: &[Message]) -> BTreeMap<String, Value> {
    let mut completions = BTreeMap::new();
    for message in messages {
        let is_completion = message.metadata["observation_type"].as_str()
            == Some("sub_agent_completion")
            || message.metadata["data"]["result"]["status"].is_string()
            || message.metadata["data"]["execution_status"].is_string();
        if !is_completion {
            continue;
        }
        if let Some(id) = sub_agent_task_id(message) {
            completions.insert(id, message.metadata["data"].clone());
        }
    }
    completions
}

pub fn apply_child_context(card: &mut SubAgentView, progress: &Value) {
    if card.status.is_running() || card.context == "--" {
        let current = progress["current_context_tokens"].as_u64();
        let max = progress["max_context_tokens"].as_u64().filter(|max| *max > 0);
        if let Some((current, max)) = current.zip(max) {
            let percent = (100.0 * current as f64 / max as f64).min(100.0);
            card.context = format!("{percent:.0}% ctx");
        }
    }
}

pub fn live_child_card(id: &str, progress: &Value) -> Option<SubAgentView> {
    let status = progress["workflow_state"].as_str().unwrap_or("running");
    let task = progress["task"].as_str().filter(|task| !task.trim().is_empty())?;
    let message = Message {
        role: "tool".into(),
        metadata: serde_json::json!({
            "tool_name": "sub_agent_run",
            "sub_agent_id": id,
            "sub_agent_name": progress["agent_name"],
            "sub_agent_task": task,
            "sub_agent_status": status,
        }),
        ..Message::default()
    };
    let mut card = sub_agent_view(&message, &BTreeMap::new())?;
    card.key = format!("child-progress-{id}");
    card.task_key = format!("{}:task", card.key);
    if let Some(count) = progress["tool_calls_count"].as_u64() {
        card.tools = format!("{count} tools");
    }
    apply_child_context(&mut card, progress);
    Some(card)
}

/// A starting card yields to the runtime's child card once progress arrives.
pub fn row_shown_by_live_child(row: &Row, live_tasks: &[&str]) -> bool {
    matches!(row, Row::SubAgent(card) if card.status == SubAgentStatus::Starting
        && live_tasks.iter().any(|task| clean_text(task) == card.task))
}

#[cfg(test)]
#[test]
fn live_child_card_shows_tool_count_without_a_completion_message() {
    let progress = serde_json::json!({"task":"Inspect files", "agent_name":"Explorer", "workflow_state":"executing", "tool_calls_count":3, "current_context_tokens":50, "max_context_tokens":200});
    let card = live_child_card("child-1", &progress).expect("live card");
    assert_eq!(card.task_id.as_deref(), Some("child-1"));
    assert!(card.status.is_running());
    assert_eq!(card.tools, "3 tools");
    assert_eq!(card.context, "25% ctx");
}

fn sub_agent_view(
    message: &Message,
    completions: &BTreeMap<String, Value>,
) -> Option<SubAgentView> {
    let name = tool_name(&message.metadata).to_lowercase();
    let direct_id = sub_agent_task_id(message);
    let review_state = message.metadata["review_display_state"]
        .as_str()
        .unwrap_or("")
        .to_lowercase();
    if name != "sub_agent_run" && direct_id.is_none() && review_state != "final_review_pending" {
        return None;
    }
    let tool_status = ToolStatus::from_message(message);
    let without_child = direct_id.is_none() && review_state != "final_review_pending";
    let starting = without_child && tool_status == ToolStatus::Running;
    // The normal tool card owns approval controls; a proposed call must not hide them.
    if without_child && (tool_status == ToolStatus::Pending || message.metadata["approval_status"] == "pending") {
        return None;
    }
    let args = tool_arguments(&message.metadata);
    let data = &message.metadata["data"];
    let completion = direct_id
        .as_ref()
        .and_then(|id| completions.get(id))
        .cloned()
        .unwrap_or(Value::Null);
    let completion_result = if completion["result"].is_object() {
        completion["result"].clone()
    } else if completion.is_null() && data["result"].is_object() {
        data["result"].clone()
    } else {
        completion.clone()
    };
    let text = |value: &Value| {
        value
            .as_str()
            .map(clean_text)
            .filter(|text| !text.is_empty())
    };
    let agent = text(&message.metadata["sub_agent_name"])
        .or_else(|| text(&data["sub_agent_name"]))
        .or_else(|| text(&completion["sub_agent_name"]))
        .or_else(|| text(&args["child_agent_name"]))
        .unwrap_or_else(|| t!("sub_agent_default_agent").to_string());
    let task = text(&message.metadata["sub_agent_task"])
        .or_else(|| text(&data["sub_agent_task"]))
        .or_else(|| text(&completion["sub_agent_task"]))
        .or_else(|| text(&args["prompt"]))
        .unwrap_or_else(|| t!("sub_agent_default_task").to_string());
    let mode = text(&message.metadata["execution_mode"])
        .or_else(|| text(&data["execution_mode"]))
        .or_else(|| text(&args["execution_mode"]))
        .unwrap_or_else(|| "call".into());
    let status_text = completion["execution_status"]
        .as_str()
        .or(completion_result["status"].as_str())
        .or(message.metadata["sub_agent_status"].as_str())
        .or(data["execution_status"].as_str())
        .or(message.metadata["execution_status"].as_str())
        .unwrap_or("running");
    let status = if starting {
        SubAgentStatus::Starting
    } else if without_child && tool_status.is_error() {
        SubAgentStatus::Failed
    } else {
        SubAgentStatus::parse(status_text)
    };
    let tool_calls = completion_result["tool_calls_count"]
        .as_u64()
        .or(completion_result["toolCallsCount"].as_u64())
        .or(completion["tool_calls_count"].as_u64())
        .unwrap_or(0);
    let context = completion_result["current_context_tokens"]
        .as_u64()
        .or_else(|| completion_result["currentContextTokens"].as_u64())
        .zip(completion_result["max_context_tokens"].as_u64().or_else(|| completion_result["maxContextTokens"].as_u64()))
        .map(|(current, max)| {
            if max > 0 {
                format!(
                    "{}% ctx",
                    (current as f64 / max as f64 * 100.0).min(100.0).round() as u64
                )
            } else {
                "--".into()
            }
        })
        .unwrap_or_else(|| "--".into());
    let raw_result = completion_result["result"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| completion_result["error"].as_str().map(str::to_owned))
        .or_else(|| completion["result"].as_str().map(str::to_owned))
        .or_else(|| {
            completion_result
                .is_object()
                .then(|| serde_json::to_string_pretty(&completion_result).unwrap_or_default())
                .filter(|text| text != "null")
        })
        .unwrap_or_default();
    let result = clean_text(&raw_result);
    let result = if result.trim_start().starts_with('{') {
        serde_json::from_str::<Value>(&result)
            .map(|value| {
                let approved = value["approved"].as_bool();
                let summary = value["summary"].as_str().unwrap_or("");
                let findings = markdown_list(&value["findings"]);
                let fixes = markdown_list(&value["required_fixes"]);
                let mut sections = Vec::new();
                if let Some(approved) = approved {
                    sections.push(format!(
                        "**Verdict:** {}",
                        if approved {
                            t!("sub_agent_approved")
                        } else {
                            t!("sub_agent_changes_required")
                        }
                    ));
                }
                if !summary.is_empty() {
                    sections.push(format!("**Summary**\n\n{summary}"));
                }
                if !findings.is_empty() {
                    sections.push(format!("**Findings**\n\n{findings}"));
                }
                if !fixes.is_empty() {
                    sections.push(format!("**Required Fixes**\n\n{fixes}"));
                }
                if sections.is_empty() {
                    fence(&result, "json")
                } else {
                    sections.join("\n\n")
                }
            })
            .unwrap_or(result)
    } else {
        result
    };
    let cost = CostView::parse(
        completion_result
            .get("usage_summary")
            .or(completion.get("usage_summary"))
            .or(message.metadata.get("usage_summary"))
            .unwrap_or(&Value::Null),
    );
    let key = message_key(message);
    Some(SubAgentView {
        task_id: direct_id.clone(),
        task_key: format!("{key}:task"),
        result_key: format!("{key}:result"),
        cost_key: format!("{key}:cost"),
        result_preview: preview(&result, 96),
        task_preview: preview(&task, 96),
        has_result: !result.is_empty(),
        status_label: status.label(),
        status,
        agent,
        mode,
        tools: format!("{tool_calls} tools"),
        context,
        task,
        result,
        cost,
        key,
    })
}

/// One question's draft answer: a picked option, or the free text typed for
/// the "Other" row that Vue appends after the listed options.
#[derive(Clone, Copy, Default)]
pub struct AskUserDraft<'a> {
    pub choice: Option<usize>,
    pub supplement: &'a str,
}

impl AskUserDraft<'_> {
    /// Typed text supplies an Other answer only in the absence of an explicit choice.
    pub fn effective_choice(self, option_count: usize) -> Option<usize> {
        self.choice
            .or_else(|| (!self.supplement.trim().is_empty()).then_some(option_count))
    }
}

/// Why a draft cannot be submitted yet, so the window can name the gap.
#[derive(Debug, PartialEq, Eq)]
pub enum AskUserDraftError {
    Required,
    OtherRequired,
}

/// Builds the hidden `<ask_user_response>` payload the engine reads back.
///
/// The shape matches Vue's `buildAskUserResponse`: one entry per question with
/// the resolved option text, so a partial draft produces no message at all.
/// Picking the row after the listed options submits the typed text with
/// `source: "other"`, the index Vue gives that row.
pub fn ask_user_response(
    groups: &[ChoiceGroupView],
    drafts: &[AskUserDraft<'_>],
) -> Result<String, AskUserDraftError> {
    if groups.is_empty() || groups.len() != drafts.len() {
        return Err(AskUserDraftError::Required);
    }
    let mut selections = Vec::new();
    for (group, draft) in groups.iter().zip(drafts) {
        let index = draft
            .effective_choice(group.options.len())
            .ok_or(AskUserDraftError::Required)?;
        if index == group.options.len() {
            let supplement = draft.supplement.trim();
            if supplement.is_empty() {
                return Err(AskUserDraftError::OtherRequired);
            }
            selections.push(serde_json::json!({
                "title": group.title,
                "choice_index": index + 1,
                "choice": supplement,
                "source": "other",
            }));
            continue;
        }
        let text = group
            .options
            .get(index)
            .ok_or(AskUserDraftError::Required)?;
        // Vue keeps a note typed beside the chosen option by appending it to
        // the option text; the parser strips the option prefix again.
        let supplement = draft.supplement.trim();
        let choice = if supplement.is_empty() {
            text.clone()
        } else {
            format!("{text}\n{supplement}")
        };
        selections.push(serde_json::json!({
            "title": group.title,
            "choice_index": index + 1,
            "choice": choice,
            "source": "option",
        }));
    }
    Ok(format!(
        "<ask_user_response>\n{}\n</ask_user_response>",
        serde_json::to_string_pretty(&selections).map_err(|_| AskUserDraftError::Required)?
    ))
}

// ---------------------------------------------------------------------------
// Projection
// ---------------------------------------------------------------------------

fn is_hidden(message: &Message) -> bool {
    let visibility = message.metadata["ui_visibility"]
        .as_str()
        .or(message.metadata["uiVisibility"].as_str())
        .unwrap_or("");
    if visibility == "hide" {
        return true;
    }
    if message.role == "system"
        && message.message_kind == "summary"
        && message.message_subtype.as_deref() == Some("approved_plan")
    {
        return true;
    }
    if message.error_type.as_deref() == Some("SubAgentInterrupted") {
        return true;
    }
    if message.role == "user"
        && message
            .step_type
            .as_deref()
            .is_some_and(|step| step.eq_ignore_ascii_case("observe"))
        && tool_call_id(message).is_none()
        && !matches!(visibility, "show" | "card")
    {
        return true;
    }
    false
}

fn is_think_only(message: &Message) -> bool {
    message.role == "assistant"
        && clean_text(&message.message).is_empty()
        && message
            .reasoning
            .as_deref()
            .is_some_and(|text| has_visible_text(&clean_text(text)))
}

fn is_completion_message(message: &Message) -> bool {
    message.role == "tool" && tool_name(&message.metadata) == "complete_workflow"
}

fn is_collapsible_tool(message: &Message) -> bool {
    if message.role != "tool" {
        return false;
    }
    let name = tool_name(&message.metadata);
    if name.is_empty() || NON_COLLAPSIBLE_TOOLS.contains(&name) {
        return false;
    }
    let approval = message.metadata["approval_status"].as_str().unwrap_or("");
    let execution = message.metadata["execution_status"].as_str().unwrap_or("");
    !(approval == "pending" || execution == "pending_approval")
}

fn pending_call_message(source: &Message, call: &Value, index: usize) -> Message {
    let id = call["id"].as_str().unwrap_or("");
    let name = call["function"]["name"]
        .as_str()
        .or(call["name"].as_str())
        .unwrap_or("");
    Message {
        id: -source.id.abs().saturating_mul(1000) - index as i64 - 100,
        role: "tool".into(),
        metadata: serde_json::json!({
            "tool_name": name,
            "tool_call": call,
            "tool_call_id": id,
            "execution_status": "running",
        }),
        ..Message::default()
    }
}

fn finish_task_label(message: &Message) -> String {
    let count = message.metadata["finish_task_error_count"]
        .as_u64()
        .unwrap_or(1);
    let label = t!("finish_task").to_string();
    if count > 1 {
        format!("{label} ({count})")
    } else {
        label
    }
}

fn is_tool_group_boundary(message: &Message) -> bool {
    let kind = message.metadata["message_kind"].as_str();
    kind == Some("exploration_batch")
        || match message.role.as_str() {
            "user" => true,
            "system" => message.message_kind == "summary",
            "assistant" => {
                message.message_kind == "completion_report"
                    || kind == Some("completion_report")
                    || has_visible_text(&clean_text(&message.message))
            }
            "tool" => !is_collapsible_tool(message),
            _ => false,
        }
}

// Match Vue's display states without changing the tool's lifecycle or deduplication rank.
fn is_tool_running_for_group(message: &Message) -> bool {
    let execution = message.metadata["execution_status"]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    match execution.as_str() {
        "waiting" if tool_name(&message.metadata) == "ask_user" => false,
        "running" | "pending" | "queued" | "waiting" | "awaiting_execution" | "approval_submitted" => true,
        "" => match message.metadata["type"].as_str() {
            Some("tool_started") => true,
            Some("tool_completed" | "tool_failed") => false,
            _ => ToolStatus::from_message(message) == ToolStatus::Running,
        },
        _ => false,
    }
}

fn activity_view(items: Vec<Message>, roots: &[String], open_tail: bool) -> ActivityView {
    let first = items.first().cloned().unwrap_or_default();
    let key = format!(
        "activity-{}",
        first.metadata["tool_call_id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| first.id.to_string())
    );
    let tools: Vec<&Message> = items
        .iter()
        .filter(|message| message.role == "tool")
        .collect();
    let thought_count = items.len() - tools.len();
    let mut error_count = 0usize;
    let mut counts: Vec<(String, usize)> = Vec::new();
    let mut kinds = Vec::new();
    let mut mutation = false;
    for message in &tools {
        let status = ToolStatus::from_message(message);
        if status.is_error() || message.metadata["approval_status"].as_str() == Some("rejected") {
            error_count += 1;
        }
        let name = tool_name(&message.metadata);
        let kind = ToolGroupKind::of(name, message.metadata["tool_category"].as_str());
        if let Some(kind) = kind {
            if kind == ToolGroupKind::Mutation {
                mutation = true;
            }
            if !kinds.contains(&kind) {
                kinds.push(kind);
            }
            let label = kind.label();
            match counts.iter_mut().find(|(current, _)| *current == label) {
                Some((_, count)) => *count += 1,
                None => counts.push((label, 1)),
            }
        }
    }
    let icon = if mutation {
        "edit"
    } else if kinds.len() == 1 {
        kinds[0].icon()
    } else {
        "tool"
    };
    let summary = truncate(
        &counts
            .iter()
            .map(|(label, count)| format!("{label} x{count}"))
            .collect::<Vec<_>>()
            .join(" · "),
        120,
    );
    let ongoing = open_tail
        || items
            .iter()
            .any(|message| message.metadata["tool_group_is_ongoing"] == true)
        || tools.iter().any(|message| is_tool_running_for_group(message));
    let items = items
        .into_iter()
        .map(|message| {
            let key = message_key(&message);
            if message.role == "tool" {
                ActivityItem {
                    thought_key: format!("{key}:thought"),
                    key,
                    thought: None,
                    tool: Some(ToolView::new(&message, roots)),
                }
            } else {
                ActivityItem {
                    thought_key: format!("{key}:thought"),
                    key,
                    thought: Some(clean_text(
                        message.reasoning.as_deref().unwrap_or(&message.message),
                    )),
                    tool: None,
                }
            }
        })
        .collect();
    ActivityView {
        key,
        icon,
        summary,
        thought_count,
        error_count,
        ongoing,
        items,
    }
}

/// Vue's `isWorkflowAlertMessage`: durable error records never join the transcript.
pub fn is_alert(message: &Message) -> bool {
    if message.role == "tool" {
        return false;
    }
    let visibility = message.metadata["ui_visibility"]
        .as_str()
        .or(message.metadata["uiVisibility"].as_str())
        .unwrap_or("");
    if visibility == "hide" || message.error_type.as_deref() == Some("SubAgentInterrupted") {
        return false;
    }
    let category = message.metadata["category"].as_str().unwrap_or("");
    let error_type = message
        .error_type
        .as_deref()
        .or(message.metadata["error_type"].as_str())
        .unwrap_or("");
    message.is_error
        || message.metadata["is_error"].as_bool().unwrap_or(false)
        || !error_type.trim().is_empty()
        || matches!(category, "warning" | "error")
}

/// Vue's `isWorkflowAlertActivity`: only the last activity decides the tail alert.
pub fn is_alert_activity(message: &Message) -> bool {
    let visibility = message.metadata["ui_visibility"]
        .as_str()
        .or(message.metadata["uiVisibility"].as_str())
        .unwrap_or("");
    if visibility == "hide" && message.role != "user" {
        return false;
    }
    is_alert(message)
        || message.role == "tool"
        || has_visible_text(&clean_text(&message.message))
        || message.reasoning.as_deref().is_some_and(has_visible_text)
        || message.metadata["tool_calls"]
            .as_array()
            .is_some_and(|calls| !calls.is_empty())
}

fn tail_alert(messages: &[Message]) -> Option<AlertView> {
    let last = messages
        .iter()
        .rev()
        .find(|message| is_alert_activity(message))?;
    is_alert(last).then(|| alert_view(last))
}

pub fn alert_view(message: &Message) -> AlertView {
    let projected = message.metadata["user_error"]["message"].as_str();
    let detail = projected.unwrap_or(&message.message);
    let normalized = normalize_alert_content(&clean_text(detail));
    // Allow the public documentation link without relaxing filtering of request URLs.
    const OPENCODE_DOCS: &str = "https://opencode.ai/v2/docs/console/go/#where-can-i-use-it";
    let body = if projected.is_some() && normalized.strip_suffix(OPENCODE_DOCS)
        .and_then(crate::types::workflow_error_detail)
        .is_some()
    {
        normalized
    } else {
        crate::types::workflow_error_detail(&normalized).unwrap_or_default().to_owned()
    };
    let warning = message.metadata["category"].as_str() == Some("warning");
    AlertView {
        key: format!("msg-{}", message.id),
        kind: if warning {
            AlertKind::Warning
        } else {
            AlertKind::Error
        },
        title: alert_title(message),
        body,
    }
}

fn alert_title(message: &Message) -> String {
    let content = clean_text(&message.message);
    if content.to_lowercase().starts_with("critical error:") {
        return t!("critical_error").to_string();
    }
    let error_type = message
        .error_type
        .as_deref()
        .or(message.metadata["error_type"].as_str())
        .unwrap_or("")
        .trim();
    match error_type {
        "llm_authentication" => t!("error_llm_authentication").to_string(),
        "llm_billing" => t!("error_llm_billing").to_string(),
        "context_window" => t!("error_context_window").to_string(),
        "llm_retry_exhausted" => {
            let attempt = message.metadata["retry_attempt"].as_u64();
            let max = message.metadata["retry_max_attempts"].as_u64();
            match (attempt, max) {
                (Some(attempt), Some(max)) => {
                    format!("{} ({attempt}/{max})", t!("error_llm_retry_exhausted"))
                }
                _ => t!("error_llm_retry_exhausted").to_string(),
            }
        }
        "" => t!("error").to_string(),
        other => {
            let mut title = String::with_capacity(other.len() + 4);
            for (index, character) in other.chars().enumerate() {
                if index > 0 && character.is_uppercase() {
                    title.push(' ');
                }
                title.push(character);
            }
            title
        }
    }
}

/// Vue's `normalizeWorkflowErrorAlertContent`: unwrap provider JSON envelopes so
/// only the user-facing message survives.
fn normalize_alert_content(content: &str) -> String {
    let mut text = content.trim();
    for prefix in ["critical error:", "[error]:", "error:"] {
        if text
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
        {
            text = text[prefix.len()..].trim();
            break;
        }
    }
    if !text.starts_with('{') {
        return text.to_owned();
    }
    let Ok(parsed) = serde_json::from_str::<Value>(text) else {
        return text.to_owned();
    };
    let server = parsed["error"]["message"]
        .as_str()
        .or_else(|| parsed["error"].as_str())
        .or_else(|| parsed["message"].as_str())
        .or_else(|| parsed["details"].as_str())
        .map(str::trim)
        .unwrap_or("");
    if server.is_empty() {
        text.to_owned()
    } else {
        server.to_owned()
    }
}

#[derive(Clone, Debug)]
struct AskUserAnswer {
    title: String,
    choice_index: u64,
    choice: String,
    source: String,
}

/// Answers arrive as `<ask_user_response>[{title, choice_index, choice, source}]</ask_user_response>`.
fn parse_ask_user_answers(text: &str) -> Option<Vec<AskUserAnswer>> {
    const OPEN: &str = "<ask_user_response>";
    const CLOSE: &str = "</ask_user_response>";
    let start = text.find(OPEN)? + OPEN.len();
    let end = start + text[start..].find(CLOSE)?;
    let payload = serde_json::from_str::<Value>(text[start..end].trim()).ok()?;
    Some(
        payload
            .as_array()?
            .iter()
            .map(|item| AskUserAnswer {
                title: item["title"].as_str().unwrap_or("").to_owned(),
                choice_index: item["choice_index"].as_u64().unwrap_or(0),
                choice: item["choice"].as_str().unwrap_or("").to_owned(),
                source: item["source"].as_str().unwrap_or("").to_lowercase(),
            })
            .collect(),
    )
}

/// Matches each answered `ask_user` tool call to the response that follows it.
fn collect_ask_user_answers(messages: &[Message]) -> BTreeMap<String, Vec<AskUserAnswer>> {
    let mut answers = BTreeMap::new();
    let mut pending: Option<String> = None;
    for message in messages {
        if message.role == "tool" && tool_name(&message.metadata) == "ask_user" {
            pending = tool_call_id(message).map(str::to_owned);
            continue;
        }
        if message.role != "user" {
            continue;
        }
        if let Some(items) = parse_ask_user_answers(&message.message) {
            // Modern responses carry the runtime's authoritative association.
            // The preceding question is only a fallback for old hidden rows.
            if let Some(id) = tool_call_id(message) {
                answers.insert(id.to_owned(), items);
                if pending.as_deref() == Some(id) {
                    pending = None;
                }
            } else if message.metadata["ui_visibility"] == "hide" {
                if let Some(id) = pending.take() {
                    answers.entry(id).or_insert(items);
                }
            }
        }
    }
    answers
}

fn ask_user_choices(message: &Message, answers: Option<&[AskUserAnswer]>) -> Vec<ChoiceGroupView> {
    let args = tool_arguments(&message.metadata);
    let items = args["items"]
        .as_array()
        .cloned()
        .or_else(|| {
            serde_json::from_str::<Value>(&clean_text(&message.message))
                .ok()
                .and_then(|value| value.as_array().cloned())
        })
        .unwrap_or_default();
    items
        .iter()
        .filter_map(|item| {
            let title = item["title"].as_str()?.trim().to_owned();
            if title.is_empty() {
                return None;
            }
            let options: Vec<String> = item["options"]
                .as_array()
                .map(|options| {
                    options
                        .iter()
                        .filter_map(|option| option.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            let answer = answers.and_then(|answers| {
                answers
                    .iter()
                    .find(|answer| answer.title.trim() == title)
                    .cloned()
            });
            let (selected, other_selected, supplement) = match &answer {
                Some(answer) => {
                    let other = matches!(answer.source.as_str(), "other" | "custom")
                        || answer.choice_index == options.len() as u64 + 1;
                    let selected = (!other
                        && answer.choice_index >= 1
                        && answer.choice_index <= options.len() as u64)
                        .then(|| answer.choice_index as usize - 1);
                    let answer_text = match selected {
                        Some(index) => {
                            let prefix = format!("{}\n", options[index]);
                            answer
                                .choice
                                .strip_prefix(&prefix)
                                .map(str::trim)
                                .filter(|text| !text.is_empty())
                                .map(str::to_owned)
                        }
                        None => {
                            Some(answer.choice.trim().to_owned()).filter(|text| !text.is_empty())
                        }
                    };
                    (selected, other, answer_text)
                }
                None => (None, false, None),
            };
            Some(ChoiceGroupView {
                title,
                options,
                selected,
                other_selected,
                answer: supplement,
            })
        })
        .collect()
}

/// Build the renderable transcript.
///
/// `roots` are the workflow's authorized display roots; they only shorten
/// absolute paths the same way the Vue list does.
pub fn project(messages: Vec<Message>, roots: &[String]) -> Vec<Row> {
    // A tool call can be persisted more than once (approval submitted, then the
    // terminal result). Vue's list keeps only the most progressed row per call.
    // Hidden responses share the question's tool ID but are not tool observations.
    let ask_answers = collect_ask_user_answers(&messages);
    let is_observation = |message: &Message| {
        !(message.role == "user"
            && (message.metadata["ask_user_response"] == true
                || message.metadata["ui_visibility"] == "hide"))
    };
    let mut best: BTreeMap<String, (u8, i64)> = BTreeMap::new();
    for message in messages.iter().filter(|message| is_observation(message)) {
        if let Some(id) = tool_call_id(message) {
            let rank = ToolStatus::from_message(message).progress_rank();
            let entry = best.entry(id.to_owned()).or_insert((rank, message.id));
            if (rank, message.id) > *entry {
                *entry = (rank, message.id);
            }
        }
    }
    let superseded = |message: &Message| {
        is_observation(message)
            && tool_call_id(message).is_some_and(|id| {
                best.get(id).is_some_and(|best| {
                    *best
                        != (
                            ToolStatus::from_message(message).progress_rank(),
                            message.id,
                        )
                })
            })
    };
    let messages: Vec<Message> = messages
        .into_iter()
        .filter(|message| !superseded(message))
        .collect();
    let completions = collect_sub_agent_completions(&messages);
    let tail = tail_alert(&messages);
    let durable_tool_ids: Vec<String> = messages
        .iter()
        .filter(|message| is_observation(message))
        .filter_map(|message| tool_call_id(message).map(str::to_owned))
        .collect();

    // Expand assistant tool calls that have no durable result row yet.
    let mut expanded = Vec::with_capacity(messages.len());
    for message in messages {
        if is_hidden(&message) || is_alert(&message) {
            continue;
        }
        if message.role == "assistant" {
            let calls = message.metadata["tool_calls"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if !calls.is_empty() {
                let mut kept = message.clone();
                if let Some(metadata) = kept.metadata.as_object_mut() {
                    metadata.remove("tool_calls");
                }
                if has_visible_text(&clean_text(&kept.message))
                    || kept.reasoning.as_deref().is_some_and(has_visible_text)
                {
                    expanded.push(kept);
                }
                for (index, call) in calls.iter().enumerate() {
                    let id = call["id"].as_str().unwrap_or("");
                    if !id.is_empty() && durable_tool_ids.iter().any(|known| known == id) {
                        continue;
                    }
                    expanded.push(pending_call_message(&message, call, index));
                }
                continue;
            }
            if clean_text(&message.message).is_empty()
                && message.reasoning.is_none()
                && message.message_kind != "completion_report"
            {
                continue;
            }
        }
        expanded.push(message);
    }

    // Collapse repeated rejected completion rows into one line with a count.
    let mut deduped: Vec<Message> = Vec::with_capacity(expanded.len());
    for message in expanded {
        if let Some(previous) = deduped.last_mut() {
            if is_completion_message(previous)
                && is_completion_message(&message)
                && ToolStatus::from_message(previous) == ToolStatus::Failed
                && ToolStatus::from_message(&message) == ToolStatus::Failed
                && clean_text(&previous.message) == clean_text(&message.message)
                && previous.metadata["summary"] == message.metadata["summary"]
            {
                let count = previous.metadata["finish_task_error_count"]
                    .as_u64()
                    .unwrap_or(1)
                    + 1;
                previous.metadata["finish_task_error_count"] = Value::from(count);
                continue;
            }
        }
        deduped.push(message);
    }

    // Like Vue, a tool result does not close the group. Only a later turn,
    // reply, or standalone activity does; trailing thought text can join it.
    let last_group_boundary = deduped.iter().rposition(is_tool_group_boundary);
    let mut rows = Vec::new();
    let mut index = 0usize;
    let total = deduped.len();
    while index < total {
        let message = &deduped[index];
        let next = deduped.get(index + 1);
        if is_think_only(message)
            && next.is_some_and(|next| {
                next.role == "assistant"
                    && next.message_kind == "completion_report"
                    && next.step_index == message.step_index
            })
        {
            index += 1;
            continue;
        }
        if !is_tool_group_boundary(message) && (is_think_only(message) || is_collapsible_tool(message)) {
            let mut collected: Vec<Message> = Vec::new();
            let mut cursor = index;
            while cursor < total {
                let candidate = &deduped[cursor];
                if is_tool_group_boundary(candidate)
                    || !(is_think_only(candidate) || is_collapsible_tool(candidate))
                {
                    break;
                }
                collected.push(candidate.clone());
                cursor += 1;
            }
            let has_tool = collected.iter().any(|message| message.role == "tool");
            if has_tool {
                let open_tail = last_group_boundary.is_none_or(|boundary| boundary < cursor);
                rows.push(Row::Activity(activity_view(collected, roots, open_tail)));
            } else {
                rows.extend(collected.into_iter().map(|message| {
                    Row::Assistant(AssistantView {
                        thought_key: format!("{}:thought", message_key(&message)),
                        key: message_key(&message),
                        reasoning: Some(clean_text(
                            message.reasoning.as_deref().unwrap_or(&message.message),
                        )),
                        thought_title: t!("thought_completed").to_string(),
                        markdown: String::new(),
                    })
                }));
            }
            index = cursor;
            continue;
        }
        if message.role == "system" && message.message_kind == "summary" {
            if message.message_subtype.as_deref() == Some("manual_clear_context") {
                rows.push(Row::Divider);
            } else {
                let snapshot = snapshot_view(message);
                if !snapshot.body.is_empty() {
                    rows.push(Row::Snapshot(snapshot));
                }
            }
            index += 1;
            continue;
        }
        if message.role == "user" {
            let mut text = clean_text(&message.message);
            if text.is_empty() && message.metadata["client_message_id"].as_str().is_some_and(|id| id.starts_with("gpui-turn-")) {
                if let Some(attachments) = message.metadata["attachments"].as_array() {
                    text = attachments.iter().filter_map(|file| file["name"].as_str()).collect::<Vec<_>>().join(", ");
                }
            }
            if !text.is_empty() {
                let delivery_label = if message.id == 0 {
                    match message.metadata["gpui_turn_status"].as_str() {
                        Some("pending") => Some(t!("turn_delivery_pending").to_string()),
                        Some("unconfirmed") => Some(t!("turn_delivery_unconfirmed").to_string()),
                        _ => None,
                    }
                } else {
                    None
                };
                rows.push(Row::User(UserView {
                    key: message.metadata["client_message_id"].as_str()
                        .map(|id| format!("user-turn-{id}"))
                        .unwrap_or_else(|| message_key(message)),
                    text,
                    delivery_label,
                }));
            }
            index += 1;
            continue;
        }
        if message.role == "tool" {
            if let Some(card) = sub_agent_view(message, &completions) {
                rows.push(Row::SubAgent(Box::new(card)));
            } else if is_completion_message(message) {
                let key = message_key(message);
                let status = ToolStatus::from_message(message);
                rows.push(Row::FinishTask(FinishTaskView {
                    key: key.clone(),
                    failed: status.is_error(),
                    label: finish_task_label(message),
                    cost_key: format!("{key}:cost"),
                    cost: (status == ToolStatus::Completed)
                        .then(|| CostView::parse(&message.metadata["usage_summary"]))
                        .flatten(),
                }));
            } else {
                let mut tool = ToolView::new(message, roots);
                if tool.name == "ask_user" {
                    let answers = tool_call_id(message)
                        .and_then(|id| ask_answers.get(id))
                        .map(Vec::as_slice);
                    tool.choices = ask_user_choices(message, answers);
                }
                rows.push(Row::Tool(Box::new(tool)));
            }
            index += 1;
            continue;
        }
        if message.role == "assistant" {
            let markdown = clean_text(&message.message);
            let reasoning = message
                .reasoning
                .as_deref()
                .map(clean_text)
                .filter(|reasoning| !reasoning.is_empty());
            if !markdown.is_empty() || reasoning.is_some() {
                let thought_title = if markdown.is_empty() {
                    t!("thinking").to_string()
                } else {
                    t!("thought_completed").to_string()
                };
                rows.push(Row::Assistant(AssistantView {
                    thought_key: format!("{}:thought", message_key(message)),
                    key: message_key(message),
                    reasoning,
                    thought_title,
                    markdown,
                }));
            }
            index += 1;
            continue;
        }
        index += 1;
    }
    if let Some(alert) = tail {
        rows.push(Row::Alert(alert));
    }
    rows
}

/// Wrapped-line estimate for a user bubble capped at 680px: about 92 Latin
/// characters per row, with wide glyphs counting double. Vue measures the real
/// height; this keeps the same collapse behaviour without a measurement pass.
pub fn user_message_lines(text: &str) -> usize {
    const UNITS_PER_LINE: usize = 92;
    text.split('\n')
        .map(|line| {
            let units: usize = line
                .chars()
                .map(|character| if character.is_ascii() { 1 } else { 2 })
                .sum();
            units.div_ceil(UNITS_PER_LINE).max(1)
        })
        .sum()
}

/// Text rows keep the same clean/format rules as the Markdown cache.
pub fn message_text(message: &Message) -> String {
    let mut text = clean_text(&message.message);
    if message.role == "tool" && !message.message.is_empty() {
        text = fence(&text, "text");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(id: i64, name: &str, extra: Value) -> Message {
        let mut metadata = json!({"tool_name": name});
        if let (Some(target), Some(source)) = (metadata.as_object_mut(), extra.as_object()) {
            target.extend(source.clone());
        }
        Message {
            id,
            role: "tool".into(),
            metadata,
            ..Message::default()
        }
    }

    #[test]
    fn ask_user_response_requires_every_answer() {
        let groups = vec![
            ChoiceGroupView {
                title: "Scope".into(),
                options: vec!["Narrow".into(), "Wide".into()],
                ..ChoiceGroupView::default()
            },
            ChoiceGroupView {
                title: "Style".into(),
                options: vec!["Direct".into()],
                ..ChoiceGroupView::default()
            },
        ];
        assert_eq!(
            ask_user_response(&groups, &[AskUserDraft::default()]).unwrap_err(),
            AskUserDraftError::Required
        );
        assert_eq!(
            ask_user_response(
                &groups,
                &[
                    AskUserDraft {
                        choice: Some(1),
                        supplement: "",
                    },
                    AskUserDraft::default(),
                ],
            )
            .unwrap_err(),
            AskUserDraftError::Required
        );
        assert_eq!(
            ask_user_response(
                &groups,
                &[
                    AskUserDraft {
                        choice: Some(0),
                        supplement: "",
                    },
                    AskUserDraft {
                        choice: Some(4),
                        supplement: "",
                    },
                ],
            )
            .unwrap_err(),
            AskUserDraftError::Required
        );
        // The row after the options is "Other", and it needs typed text.
        assert_eq!(
            ask_user_response(
                &groups,
                &[
                    AskUserDraft {
                        choice: Some(2),
                        supplement: "  ",
                    },
                    AskUserDraft {
                        choice: Some(0),
                        supplement: "",
                    },
                ],
            )
            .unwrap_err(),
            AskUserDraftError::OtherRequired
        );

        let payload = ask_user_response(
            &groups,
            &[
                AskUserDraft {
                    choice: Some(2),
                    supplement: "Balanced",
                },
                AskUserDraft {
                    choice: Some(0),
                    supplement: "",
                },
            ],
        )
        .expect("complete draft");
        assert!(payload.starts_with("<ask_user_response>\n"));
        assert!(payload.ends_with("\n</ask_user_response>"));
        let body = payload
            .trim_start_matches("<ask_user_response>\n")
            .trim_end_matches("\n</ask_user_response>");
        let parsed: Value = serde_json::from_str(body).expect("json body");
        assert_eq!(parsed[0]["choice_index"], 3);
        assert_eq!(parsed[0]["choice"], "Balanced");
        assert_eq!(parsed[0]["source"], "other");
        assert_eq!(parsed[1]["title"], "Style");

        let payload = ask_user_response(
            &groups,
            &[
                AskUserDraft {
                    choice: Some(1),
                    supplement: "",
                },
                AskUserDraft {
                    choice: Some(0),
                    supplement: "",
                },
            ],
        )
        .expect("complete draft");
        let body = payload
            .trim_start_matches("<ask_user_response>\n")
            .trim_end_matches("\n</ask_user_response>");
        let parsed: Value = serde_json::from_str(body).expect("json body");
        assert_eq!(parsed[0]["choice_index"], 2);
        assert_eq!(parsed[0]["choice"], "Wide");
        assert_eq!(parsed[0]["source"], "option");
        assert_eq!(parsed[1]["title"], "Style");
    }

    #[test]
    fn activity_groups_keep_thought_order_and_vue_summary() {
        let thought = Message {
            id: 1,
            role: "assistant".into(),
            reasoning: Some("keep all reasoning".into()),
            ..Message::default()
        };
        let rows = project(
            vec![
                thought,
                tool(
                    2,
                    "edit_file",
                    json!({"execution_status":"completed","summary":"Edited a.rs"}),
                ),
                tool(3, "todo_update", json!({"execution_status":"completed"})),
                tool(4, "edit_file", json!({"execution_status":"completed"})),
                tool(
                    5,
                    "complete_workflow",
                    json!({"execution_status":"completed"}),
                ),
            ],
            &[],
        );
        assert_eq!(rows.len(), 2);
        let Row::Activity(activity) = &rows[0] else {
            panic!("expected activity group");
        };
        assert_eq!(activity.thought_count, 1);
        assert_eq!(
            activity.summary,
            format!(
                "{} x2 · {} x1",
                t!("group_edit_file"),
                t!("group_task_changes")
            )
        );
        assert_eq!(activity.icon, "edit");
        assert_eq!(activity.items.len(), 4);
        assert!(activity.items[0].thought.is_some());
        assert!(matches!(&rows[1], Row::FinishTask(task) if !task.failed));
    }

    #[test]
    fn activity_group_stays_open_after_tool_completion_until_a_boundary() {
        let completed = || tool(1, "read_file", json!({"execution_status":"completed"}));
        let reasoning = Message {
            id: -1,
            role: "assistant".into(),
            reasoning: Some("Checking the result".into()),
            ..Message::default()
        };
        for messages in [vec![completed()], vec![completed(), reasoning]] {
            let rows = project(messages, &[]);
            assert!(matches!(rows.as_slice(), [Row::Activity(group)] if group.ongoing));
        }
        for boundary in [
            Message {
                role: "assistant".into(),
                message: "Done".into(),
                ..Message::default()
            },
            Message {
                role: "user".into(),
                message: "Next turn".into(),
                ..Message::default()
            },
            tool(2, "write_file", json!({"execution_status":"pending_approval"})),
            tool(2, "ask_user", json!({"execution_status":"waiting"})),
        ] {
            let rows = project(vec![completed(), boundary], &[]);
            assert!(matches!(&rows[0], Row::Activity(group) if !group.ongoing));
        }
    }

    #[test]
    fn activity_group_uses_vue_display_states_for_tools() {
        for status in ["running", "pending", "queued", "waiting", "awaiting_execution", "approval_submitted"] {
            let call = tool(1, "read_file", json!({"execution_status":status}));
            assert!(is_tool_running_for_group(&call), "{status}");
        }
        for status in ["completed", "failed", "interrupted", "rejected", "pending_approval"] {
            let call = tool(1, "read_file", json!({"execution_status":status}));
            assert!(!is_tool_running_for_group(&call), "{status}");
        }
        assert!(!is_tool_running_for_group(&tool(
            1,
            "ask_user",
            json!({"execution_status":"waiting"}),
        )));
        assert!(is_tool_running_for_group(&tool(1, "read_file", json!({"type":"tool_started"}))));
        assert!(!is_tool_running_for_group(&tool(1, "read_file", json!({"type":"tool_completed"}))));
    }

    #[test]
    fn non_collapsible_tools_stay_standalone() {
        for name in ["ask_user", "submit_plan"] {
            let rows = project(
                vec![tool(1, name, json!({"execution_status":"completed"}))],
                &[],
            );
            assert!(matches!(&rows[0], Row::Tool(view) if view.name == name));
        }
        let pending = tool(2, "read_file", json!({"approval_status":"pending"}));
        let rows = project(vec![pending], &[]);
        assert!(matches!(&rows[0], Row::Tool(view) if view.status == ToolStatus::Pending));
    }

    #[test]
    fn tool_title_uses_backend_title_and_normalizes_arguments() {
        let message = tool(
            1,
            "read_file",
            json!({
                "title": "Read src/lib.rs from L10",
                "tool_call": {"function": {"name": "read_file", "arguments": "{\"file_path\":\"src/lib.rs\",\"offset\":9}"}},
                "execution_status": "completed",
                "duration_ms": 1500,
            }),
        );
        let view = ToolView::new(&message, &[]);
        assert_eq!(view.action, "Read src/lib.rs from L10");
        assert!(view.target.is_empty());
        assert_eq!(view.badges.last().map(String::as_str), Some("1s"));
        let bash = ToolView::new(
            &tool(
                2,
                "bash",
                json!({"tool_call": {"function": {"arguments": "{\"command\":\"git status --short\"}"}}, "execution_status":"running"}),
            ),
            &[],
        );
        assert_eq!(bash.action, "Run git status --short");
        assert_eq!(bash.status, ToolStatus::Running);
        assert_eq!(bash.summary, t!("executing"));
    }

    #[test]
    fn completion_summary_row_and_repeated_errors() {
        let failed = tool(
            3,
            "complete_workflow",
            json!({"execution_status":"failed","summary":"rejected","is_error":true}),
        );
        let rows = project(vec![failed.clone(), Message { id: 4, ..failed }], &[]);
        assert_eq!(rows.len(), 1);
        assert!(
            matches!(&rows[0], Row::FinishTask(task) if task.failed && task.label.ends_with("(2)"))
        );
    }

    #[test]
    fn snapshots_render_parsed_sections_and_manual_clear_is_a_divider() {
        let snapshot = Message {
            id: 8,
            role: "system".into(),
            message_kind: "summary".into(),
            message: "## Previous Context Snapshot\n{\"overall_goal\":\"Ship it\",\"error_log\":[\"boom\"]}".into(),
            ..Message::default()
        };
        let manual = Message {
            id: 9,
            role: "system".into(),
            message_kind: "summary".into(),
            message_subtype: Some("manual_clear_context".into()),
            ..Message::default()
        };
        let rows = project(vec![snapshot, manual], &[]);
        let Row::Snapshot(view) = &rows[0] else {
            panic!("expected snapshot");
        };
        assert!(view.body.contains("Ship it"));
        assert!(view.body.contains("boom"));
        assert!(!view.body.contains("error_log\":["));
        assert!(matches!(&rows[1], Row::Divider));
    }

    #[test]
    fn sub_agent_card_joins_completion_observation() {
        let spawn = tool(
            10,
            "sub_agent_run",
            json!({
                "sub_agent_id": "subagent_1",
                "sub_agent_name": "Code Explorer",
                "sub_agent_task": "Review the change",
                "sub_agent_status": "running",
                "execution_mode": "call",
                "execution_status": "running",
            }),
        );
        let completion = Message {
            id: 11,
            role: "user".into(),
            step_type: Some("observe".into()),
            message: "<tool_result tool=\"sub_agent_run\" id=\"subagent_1\" mode=\"call\" status=\"completed\">".into(),
            metadata: json!({"data": {"execution_status": "completed", "result": {"result": "All good", "tool_calls_count": 4, "current_context_tokens": 50, "max_context_tokens": 200}}}),
            ..Message::default()
        };
        let rows = project(vec![spawn, completion], &[]);
        assert_eq!(rows.len(), 1);
        let Row::SubAgent(card) = &rows[0] else {
            panic!("expected sub-agent card");
        };
        assert_eq!(card.agent, "Code Explorer");
        assert_eq!(card.status, SubAgentStatus::Completed);
        assert_eq!(card.tools, "4 tools");
        assert_eq!(card.context, "25% ctx");
        assert!(card.result.contains("All good"));
    }

    #[test]
    fn sub_agent_card_reads_inline_terminal_context_without_separate_observation() {
        let row = tool(10, "sub_agent_run", json!({
            "data": {"result": {"task_id": "child-1", "status": "completed",
                "tool_calls_count": 4, "current_context_tokens": 50, "max_context_tokens": 200,
                "result": "Done"}}
        }));
        let rows = project(vec![row], &[]);
        let [Row::SubAgent(card)] = rows.as_slice() else {
            panic!("expected one child card");
        };
        assert_eq!(card.status, SubAgentStatus::Completed);
        assert_eq!(card.tools, "4 tools");
        assert_eq!(card.context, "25% ctx");
    }

    #[test]
    fn pending_tool_calls_without_results_are_projected_once() {
        let call = json!({"tool_calls":[{"id":"a","function":{"name":"bash","arguments":"{\"command\":\"pwd\"}"}}]});
        let assistant = Message {
            id: 20,
            role: "assistant".into(),
            metadata: call.clone(),
            ..Message::default()
        };
        let result = tool(
            21,
            "bash",
            json!({"tool_call_id":"a","execution_status":"completed"}),
        );
        let rows = project(vec![assistant, result], &[]);
        assert_eq!(rows.len(), 1);
        let Row::Activity(activity) = &rows[0] else {
            panic!("expected grouped tool");
        };
        assert_eq!(activity.items.len(), 1);
        let rows = project(
            vec![Message {
                id: 22,
                role: "assistant".into(),
                metadata: call,
                ..Message::default()
            }],
            &[],
        );
        assert!(matches!(&rows[0], Row::Activity(activity) if activity.ongoing));
    }

    #[test]
    fn a_running_sub_agent_row_yields_to_its_live_child_card() {
        let args = json!({
            "description": "Diagnose spins",
            "prompt": "Find why the spinners freeze",
            "child_agent_name": "Zhugeliang",
            "execution_mode": "call"
        })
        .to_string();
        let assistant = Message {
            id: 30,
            role: "assistant".into(),
            metadata: json!({"tool_calls": [{
                "id": "call-30",
                "function": {"name": "sub_agent_run", "arguments": args}
            }]}),
            ..Message::default()
        };
        let rows = project(vec![assistant], &[]);
        let [Row::SubAgent(card)] = rows.as_slice() else {
            panic!("expected a child card before runtime progress");
        };
        assert_eq!(card.status, SubAgentStatus::Starting);
        assert_eq!(card.agent, "Zhugeliang");
        assert_eq!(card.task_id, None);
        assert_eq!(card.task, "Find why the spinners freeze");
        assert!(row_shown_by_live_child(
            &rows[0],
            &["Find why the spinners freeze"]
        ));
        assert!(!row_shown_by_live_child(&rows[0], &["another task"]));
    }

    #[test]
    fn sub_agent_starting_card_does_not_replace_approval_controls() {
        let pending = tool(30, "sub_agent_run", json!({
            "tool_call_id": "call-30",
            "execution_status": "pending_approval",
            "arguments": {"child_agent_name": "Zhugeliang", "prompt": "Review this"}
        }));
        let rows = project(vec![pending], &[]);
        let [Row::Tool(card)] = rows.as_slice() else {
            panic!("expected a tool card for the pending approval");
        };
        assert_eq!(card.status, ToolStatus::Pending);
        assert!(!row_shown_by_live_child(&rows[0], &["Review this"]));
    }

    #[test]
    fn reminders_and_hidden_observations_never_reach_rows() {
        assert_eq!(
            clean_text("before<SYSTEM_REMINDER>secret</SYSTEM_REMINDER>after"),
            "beforeafter"
        );
        let hidden = Message {
            id: 1,
            role: "user".into(),
            step_type: Some("observe".into()),
            message: "<tool_result/>".into(),
            ..Message::default()
        };
        assert!(is_hidden(&hidden));
        let visible = Message {
            metadata: json!({"ui_visibility":"card"}),
            ..hidden
        };
        assert!(!is_hidden(&visible));
        let interrupted = tool(2, "read_file", json!({}));
        let interrupted = Message {
            error_type: Some("SubAgentInterrupted".into()),
            ..interrupted
        };
        assert!(is_hidden(&interrupted));
    }

    #[test]
    fn duplicate_persisted_rows_keep_the_terminal_state() {
        let running = tool(
            1,
            "bash",
            json!({"tool_call_id":"same","execution_status":"approval_submitted"}),
        );
        let completed = tool(
            2,
            "bash",
            json!({"tool_call_id":"same","execution_status":"completed","summary":"ok"}),
        );
        let rows = project(vec![running, completed], &[]);
        assert_eq!(rows.len(), 1);
        let Row::Activity(group) = &rows[0] else {
            panic!("expected grouped tool");
        };
        let tool = group.items[0].tool.as_ref().unwrap();
        assert_eq!(tool.status, ToolStatus::Completed);
        assert_eq!(tool.summary, "ok");
    }

    #[test]
    fn empty_snapshots_do_not_render_an_empty_card() {
        let empty = Message {
            id: 30,
            role: "system".into(),
            message_kind: "summary".into(),
            ..Message::default()
        };
        assert!(project(vec![empty], &[]).is_empty());
    }

    #[test]
    fn trailing_errors_render_as_an_alert_instead_of_a_transcript_row() {
        let error = Message {
            id: 40,
            role: "assistant".into(),
            message: concat!(
                r#"{"error":{"code":"invalid_parameter_error","#,
                r#""message":"Range of input length should be [1, 1000000]","#,
                r#""param":null,"type":"invalid_request_error"},"#,
                r#""request_id":"927df974"}"#
            )
            .into(),
            is_error: true,
            ..Message::default()
        };
        let rows = project(vec![error], &[]);
        assert_eq!(rows.len(), 1);
        let Row::Alert(alert) = &rows[0] else {
            panic!("expected the error to surface as a tail alert");
        };
        assert_eq!(alert.kind, AlertKind::Error);
        assert_eq!(alert.title, t!("error").to_string());
        assert_eq!(alert.body, "Range of input length should be [1, 1000000]");
    }

    #[test]
    fn workflow_alert_prefers_projected_detail_and_hides_backend_urls() {
        let raw = "Request to backend failed: error sending request for url (https://backend.example/v1/chat/\ncompletions)";
        let mut message = Message {
            id: 40,
            role: "assistant".into(),
            message: raw.into(),
            is_error: true,
            ..Message::default()
        };
        assert!(alert_view(&message).body.is_empty());
        message.metadata = json!({"user_error":{"message":"Model is unavailable"}});
        assert_eq!(alert_view(&message).body, "Model is unavailable");
        message.metadata["user_error"]["message"] = json!(raw);
        assert!(alert_view(&message).body.is_empty());
    }

    #[test]
    fn opencode_retry_alert_preserves_the_public_documentation_link() {
        let detail = "Request is missing x-opencode-session and cannot be routed efficiently. Please see https://opencode.ai/v2/docs/console/go/#where-can-i-use-it";
        let mut message = Message {
            id: 42,
            role: "assistant".into(),
            error_type: Some("llm_retry_exhausted".into()),
            metadata: json!({"retry_attempt": 10, "retry_max_attempts": 10, "user_error": {"message": detail}}),
            ..Message::default()
        };
        let alert = alert_view(&message);
        assert_eq!(alert.body, detail);
        assert!(alert.title.ends_with("(10/10)"));
        message.metadata["user_error"]["message"] = json!(format!("https://private.example/v1 {detail}"));
        assert!(alert_view(&message).body.is_empty());
    }

    #[test]
    fn errors_before_later_activity_stay_out_of_the_list() {
        let error = Message {
            id: 41,
            role: "assistant".into(),
            message: "boom".into(),
            is_error: true,
            ..Message::default()
        };
        let rows = project(
            vec![
                error,
                tool(42, "read_file", json!({"execution_status":"completed"})),
            ],
            &[],
        );
        assert!(!rows.iter().any(|row| matches!(row, Row::Alert(_))));
        assert!(rows.iter().any(|row| matches!(row, Row::Activity(_))));
    }

    #[test]
    fn alert_titles_follow_the_error_type() {
        let titled = |error_type: &str, metadata: Value, message: &str| {
            let rows = project(
                vec![Message {
                    id: 43,
                    role: "assistant".into(),
                    message: message.into(),
                    error_type: Some(error_type.into()),
                    metadata,
                    ..Message::default()
                }],
                &[],
            );
            let Row::Alert(alert) = &rows[0] else {
                panic!("expected alert");
            };
            alert.title.clone()
        };
        assert_eq!(
            titled("llm_authentication", json!({}), "denied"),
            t!("error_llm_authentication").to_string()
        );
        assert_eq!(
            titled(
                "llm_retry_exhausted",
                json!({"retry_attempt": 3, "retry_max_attempts": 5}),
                "retry"
            ),
            format!("{} (3/5)", t!("error_llm_retry_exhausted"))
        );
        assert_eq!(
            titled("context_window", json!({}), "too long"),
            t!("error_context_window").to_string()
        );
        assert_eq!(
            titled("invalidParameterError", json!({}), "bad"),
            "invalid Parameter Error"
        );
    }

    #[test]
    fn user_message_line_estimate_collapses_long_text() {
        assert_eq!(user_message_lines("short"), 1);
        assert_eq!(user_message_lines("a\nb\nc\nd\ne"), 5);
        assert!(user_message_lines(&"a".repeat(400)) > 4);
        assert!(user_message_lines(&"中".repeat(200)) > 4);
    }

    #[test]
    fn ask_user_typed_text_selects_other_only_without_an_explicit_choice() {
        let groups = [
            ChoiceGroupView {
                title: "Scope".into(),
                options: vec!["Narrow".into(), "Wide".into()],
                ..ChoiceGroupView::default()
            },
            ChoiceGroupView {
                title: "Style".into(),
                options: vec!["Concise".into()],
                ..ChoiceGroupView::default()
            },
        ];
        let drafts = [
            AskUserDraft { choice: None, supplement: " Custom scope " },
            AskUserDraft { choice: Some(0), supplement: " Extra detail " },
        ];
        let payload = ask_user_response(&groups, &drafts).expect("complete draft");
        let answers = parse_ask_user_answers(&payload).expect("answers");
        assert_eq!(answers[0].choice_index, 3);
        assert_eq!(answers[0].choice, "Custom scope");
        assert_eq!(answers[0].source, "other");
        assert_eq!(answers[1].choice, "Concise\nExtra detail");
        assert_eq!(answers[1].source, "option");
        assert_eq!(AskUserDraft { choice: None, supplement: " \n " }.effective_choice(2), None);
    }

    #[test]
    fn ask_user_option_note_is_appended_to_choice() {
        let groups = vec![ChoiceGroupView {
            title: "Scope".into(),
            options: vec!["Narrow".into(), "Wide".into()],
            ..ChoiceGroupView::default()
        }];
        let content = ask_user_response(
            &groups,
            &[AskUserDraft {
                choice: Some(0),
                supplement: "only the parser",
            }],
        )
        .expect("answer");
        assert!(content.contains(r#""choice": "Narrow\nonly the parser""#));
        assert!(content.contains(r#""source": "option""#));
    }

    #[test]
    fn ask_user_choices_keep_the_selected_option() {
        let ask = Message {
            id: 50,
            role: "tool".into(),
            message: r#"[{"title":"Pick one","options":["A","B"]}]"#.into(),
            metadata: json!({
                "tool_name": "ask_user",
                "display_type": "choice",
                "tool_call_id": "call_ask",
                "tool_call": {"function": {"arguments": r#"{"items":[{"title":"Pick one","options":["A","B"]}]}"#}}
            }),
            ..Message::default()
        };
        let other_ask = Message {
            id: 51,
            metadata: json!({"tool_name":"ask_user", "tool_call_id":"another"}),
            role: "tool".into(),
            message: ask.message.clone(),
            ..Message::default()
        };
        let answer = Message {
            id: 52,
            role: "user".into(),
            message: concat!(
                "<ask_user_response>[",
                r#"{"title":"Pick one","choice_index":2,"choice":"B\nExtra context","source":"option"}"#,
                "]</ask_user_response>"
            )
            .into(),
            metadata: json!({"ui_visibility":"hide", "ask_user_response":true, "tool_call_id":"call_ask"}),
            ..Message::default()
        };
        let rows = project(vec![ask, other_ask, answer], &[]);
        let tool = rows
            .iter()
            .find_map(|row| match row {
                Row::Tool(tool) => Some(tool),
                _ => None,
            })
            .expect("ask_user card");
        assert_eq!(tool.choices.len(), 1);
        assert_eq!(tool.choices[0].title, "Pick one");
        assert_eq!(tool.choices[0].options, ["A", "B"]);
        assert_eq!(tool.choices[0].selected, Some(1));
        assert!(!tool.choices[0].other_selected);
        assert_eq!(tool.choices[0].answer.as_deref(), Some("Extra context"));
        assert!(rows.iter().any(|row| matches!(row, Row::Tool(tool) if tool.tool_call_id == "another" && tool.choices.first().is_some_and(|choice| choice.selected.is_none()))));
    }

    #[test]
    fn ask_user_custom_answer_marks_other() {
        let ask = Message {
            id: 52,
            role: "tool".into(),
            message: r#"[{"title":"Pick one","options":["A"]}]"#.into(),
            metadata: json!({"tool_name": "ask_user", "display_type": "choice", "tool_call_id": "call_x"}),
            ..Message::default()
        };
        let answer = Message {
            id: 53,
            role: "user".into(),
            message: concat!(
                "<ask_user_response>[",
                r#"{"title":"Pick one","choice_index":2,"choice":"something else","source":"custom"}"#,
                "]</ask_user_response>"
            )
            .into(),
            metadata: json!({"ui_visibility": "hide"}),
            ..Message::default()
        };
        let rows = project(vec![ask, answer], &[]);
        let Row::Tool(tool) = rows
            .iter()
            .find(|row| matches!(row, Row::Tool(_)))
            .expect("ask_user card")
        else {
            unreachable!()
        };
        assert_eq!(tool.choices[0].selected, None);
        assert!(tool.choices[0].other_selected);
        assert_eq!(tool.choices[0].answer.as_deref(), Some("something else"));
    }

    #[test]
    fn completion_reports_expose_the_cost_card() {
        let message = Message {
            id: 60,
            role: "tool".into(),
            message: "Finished".into(),
            metadata: json!({
                "tool_name": "complete_workflow",
                "execution_status": "completed",
                "usage_summary": {
                    "version": 1,
                    "duration_ms": 65000,
                    "has_sub_agents": true,
                    "is_partial": false,
                    "self_usage": {
                        "input_tokens": 1200,
                        "output_tokens": 300,
                        "cache_tokens": 100,
                        "total_tokens": 1500,
                        "estimated_cost": 0.42,
                        "effective_cost_per_million": 0.25
                    },
                    "with_sub_agents": {
                        "input_tokens": 2200,
                        "output_tokens": 500,
                        "cache_tokens": 300,
                        "total_tokens": 2700,
                        "estimated_cost": null,
                        "effective_cost_per_million": null
                    },
                    "model_breakdowns": [{
                        "backend_model": "qwen3.8-flash",
                        "input_tokens": 1200,
                        "output_tokens": 300,
                        "cache_tokens": 100,
                        "input_per_million": 0.1,
                        "output_per_million": 0.4,
                        "cache_per_million": 0.02,
                        "multiplier": 1.0,
                        "estimated_cost": 0.42
                    }]
                }
            }),
            ..Message::default()
        };
        let rows = project(vec![message], &[]);
        let Row::FinishTask(view) = &rows[0] else {
            panic!("expected a completion row");
        };
        let cost = view.cost.as_ref().expect("cost card");
        assert_eq!(cost.duration_ms, Some(65000));
        assert_eq!(cost.with_sub_agents.total_tokens, 2700);
        assert!(cost.has_sub_agents);
        assert_eq!(cost.models.len(), 1);
        assert!((cost.models[0].cache_hit_rate.unwrap_or_default() - 100.0 / 12.0).abs() < 1e-9);
    }

    #[test]
    fn mcp_arguments_and_result_render_as_labelled_sections() {
        let message = tool(
            71,
            "mcp__demo__search",
            json!({
                "tool_category": "mcp",
                "tool_call": {"function": {"name": "mcp__demo__search", "arguments": "{\"query\":\"gpui\"}"}},
                "execution_status": "completed",
            }),
        );
        let mut message = message;
        message.message = json!({"content": [{"text": "search hit", "type": "text"}]}).to_string();
        let view = ToolView::new(&message, &[]);
        assert!(view.arguments.is_some());
        assert!(view.detail.contains(&t!("arguments_label").to_string()));
        assert!(view.detail.contains("\"query\": \"gpui\""));
        assert!(view.detail.contains(&t!("result_label").to_string()));
        assert!(view.detail.contains("search hit"));
        assert!(!view.detail.contains("\"content\""), "raw envelope leaked");
    }

    #[test]
    fn folded_mcp_definition_is_not_rendered_as_a_result() {
        let mut message = tool(
            72,
            "mcp_tool_expand",
            json!({
                "tool_category": "mcp",
                "tool_call": {"function": {"name": "mcp_tool_expand", "arguments": "{\"tool_name\":\"browser_go_back\"}"}},
                "execution_status": "completed",
            }),
        );
        message.message =
            "Loaded the complete definition for folded MCP tool 'browser_go_back'.".to_owned();
        let view = ToolView::new(&message, &[]);
        assert!(view.detail.contains(&t!("arguments_label").to_string()));
        assert!(!view.detail.contains("Loaded the complete definition"));
    }

    #[test]
    fn cost_durations_reach_hours() {
        assert_eq!(format_duration(None), "--");
        assert_eq!(format_duration(Some(0)), "0s");
        assert_eq!(format_duration(Some(59_000)), "59s");
        assert_eq!(format_duration(Some(61_000)), "1m 1s");
        assert_eq!(format_duration(Some(3_661_000)), "1h 1m 1s");
    }

    #[test]
    fn tool_titles_collapse_to_one_line() {
        assert_eq!(single_line("Run\n  git   status"), "Run git status");
        assert_eq!(single_line("   "), "");
        assert_eq!(grouped_number(1_397_223), "1,397,223");
        assert_eq!(grouped_number(999), "999");
    }

    #[test]
    fn bash_targets_with_newlines_stay_on_one_line() {
        let message = tool(
            70,
            "bash",
            json!({
                "tool_call": {"function": {"name": "bash", "arguments": "{\"command\":\"echo one\\necho two\"}"}},
                "execution_status": "completed",
                "duration_ms": 1200,
            }),
        );
        let view = ToolView::new(&message, &[]);
        assert!(!view.action.contains('\n'), "action stayed multiline");
        assert!(!view.target.contains('\n'), "target stayed multiline");
    }

    #[test]
    fn opaque_metadata_is_not_rendered_and_fences_are_safe() {
        let message = Message {
            message: "hello".into(),
            metadata: json!({"internal":"private"}),
            ..Message::default()
        };
        assert_eq!(message_text(&message), "hello");
        assert!(fence("```", "text").starts_with("````text"));
    }
}
