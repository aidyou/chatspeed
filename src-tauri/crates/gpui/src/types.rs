//! Wire adapters preserve opaque metadata and compatibility-route casing.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{path::PathBuf, time::Instant};

/// Standalone observer arguments; discovery never implies runtime startup.
#[derive(Clone, clap::Parser)]
pub struct Options {
    #[arg(long)]
    pub discovery: Option<PathBuf>,
    #[arg(long)]
    pub metrics: Option<PathBuf>,
    #[arg(long)]
    pub demo: bool,
}

#[derive(Clone, Copy)]
pub enum AutomationAction {
    Enable,
    Disable,
    Run,
    Delete,
}

#[derive(Clone, Deserialize)]
pub struct AutomationRun {
    pub status: String,
    pub scheduled_for: String,
}

#[derive(Clone, Deserialize)]
pub struct AutomationSummary {
    pub automation_id: String,
    pub title: String,
    pub enabled: bool,
}

/// Minimal read-only workflow projection.
#[derive(Clone, Default, Deserialize)]
pub struct Session {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub user_query: String,
    #[serde(default)]
    pub status: String,
    /// Agent configuration: the list route serializes it as a JSON string, the
    /// snapshot route as an object once it has been normalized in memory.
    #[serde(default)]
    pub agent_config: Value,
    #[serde(default)]
    pub is_automation_run: bool,
    #[serde(default)]
    pub parent_session_id: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

impl Session {
    /// Vue shows the title, then the user query, then the untitled placeholder.
    pub fn heading(&self) -> String {
        match self.title.trim() {
            "" => self.user_query.trim().to_owned(),
            title => title.to_owned(),
        }
    }

    /// The primary authorized root, used for the badge and the row subtitle.
    pub fn primary_root(&self) -> Option<String> {
        roots_from(Some(&self.agent_config)).into_iter().next()
    }
}

/// Authorized roots of a workflow payload.
///
/// The workflow row and its snapshot both carry `agent_config`; it arrives as a
/// JSON string from the list route and as an object from the snapshot route.
pub fn workflow_roots(workflow: &Value) -> Vec<String> {
    let direct = workflow
        .get("allowedPaths")
        .or_else(|| workflow.get("allowed_paths"))
        .or_else(|| workflow.get("agentConfig"))
        .or_else(|| workflow.get("agent_config"));
    roots_from(direct)
}

fn roots_from(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(roots)) => roots
            .iter()
            .filter_map(Value::as_str)
            .map(|root| root.trim_end_matches(['/', '\\']).to_owned())
            .filter(|root| !root.is_empty())
            .collect(),
        // `agent_config` is a serialized agent config, so its roots are nested.
        Some(Value::String(config)) => serde_json::from_str::<Value>(config)
            .ok()
            .and_then(|config| {
                config
                    .get("allowedPaths")
                    .or_else(|| config.get("allowed_paths"))
                    .cloned()
            })
            .map(|roots| roots_from(Some(&roots)))
            .unwrap_or_default(),
        Some(config @ Value::Object(_)) => config
            .get("allowedPaths")
            .or_else(|| config.get("allowed_paths"))
            .map(|roots| roots_from(Some(roots)))
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Folder name of a root path, as Vue renders it in the badge and the selector.
pub fn root_name(path: &str) -> &str {
    path.rsplit(['/', '\\'])
        .find(|segment| !segment.is_empty())
        .unwrap_or(path)
}

/// One child of a directory in the authorized-path tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

/// Durable transcript row; aliases exist only at the HTTP compatibility boundary.
///
/// The authoritative columns (`messageKind`, `stepType`, ...) stay outside
/// `metadata`, so the native list must read them to find snapshots, completion
/// reports and hidden observations.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Message {
    pub id: i64,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub message: String,
    pub reasoning: Option<String>,
    #[serde(default, alias = "messageKind")]
    pub message_kind: String,
    #[serde(default, alias = "messageSubtype")]
    pub message_subtype: Option<String>,
    #[serde(default, alias = "stepType")]
    pub step_type: Option<String>,
    #[serde(default, alias = "stepIndex")]
    pub step_index: i64,
    #[serde(default, alias = "segmentId")]
    pub segment_id: i64,
    #[serde(default, alias = "isError")]
    pub is_error: bool,
    #[serde(default, alias = "errorType")]
    pub error_type: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

/// Bounded history response, either a snapshot or the older-message adapter.
#[derive(Default, Deserialize)]
pub struct Page {
    #[serde(skip)]
    pub fetched_from: Option<Instant>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub workflow: Value,
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default, alias = "beforeMessageId", alias = "messageWindowBeforeId")]
    pub message_window_before_id: Option<String>,
    #[serde(default, alias = "hiddenEarlierMessageCount")]
    pub hidden_earlier_message_count: usize,
    /// Runtime execution context: wait reason, awaiting tool call, pending tools.
    #[serde(default, alias = "executionContext")]
    pub execution_context: Value,
    #[serde(default, alias = "canRewindTail")]
    pub can_rewind_tail: bool,
    #[serde(default, alias = "tailRewindKind")]
    pub tail_rewind_kind: Option<String>,
    #[serde(default, alias = "hasLiveSession")]
    pub has_live_session: bool,
}

/// UI requests use generations to invalidate all old-selection results.
pub enum Command {
    DesktopMenu(chatspeed_runtime_client::desktop_menu::Action),
    ApprovalBatch {
        session: String,
        tool_ids: Vec<String>,
    },
    SetSound {
        chime: crate::chimes::Chime,
        muted: bool,
    },
    Metrics(crate::metrics::Metrics, Option<PathBuf>),
    Select {
        generation: u64,
        id: String,
    },
    Older {
        generation: u64,
        id: String,
        before: String,
    },
    /// A typed workflow signal (`user_message`, `approval`, `continue`, ...).
    SubmitQueuedMessage {
        generation: u64,
        session: String,
        queue_id: String,
        signal: Value,
    },
    SubmitTurn {
        generation: u64,
        session: String,
        client_id: String,
        signal: Value,
    },
    Signal {
        id: String,
        signal: Value,
    },
    Rewind {
        generation: u64,
        session: String,
    },
    EditSession {
        session: String,
        title: Option<String>,
    },
    ListAutomations,
    ListAutomationRuns(String),
    ActOnAutomation { id: String, action: AutomationAction },
    CreateAutomation(Value),
    Stop {
        id: String,
    },
    /// Creates a session and starts it with the first prompt.
    StartNew {
        prompt: String,
        attached_context: Option<String>,
        metadata: Option<Value>,
    },
    /// Loads a child session for an expanded sub-agent card.
    Child {
        generation: u64,
        session: String,
    },
    /// Lists one directory for the authorized-path tree.
    ListDir {
        path: String,
    },
    /// Reloads the agents, models and skills the composer selectors list.
    Catalogue,
    /// Fetches the native and MCP tool catalogue on demand.
    Tools,
    /// Switches the selected workflow to another primary agent.
    SetAgent {
        session: String,
        agent: String,
    },
    /// Replaces the workflow's per-slot model configs.
    SetModelConfig {
        generation: u64,
        session: String,
        configs: Value,
    },
    /// Changes the workflow's sandbox execution mode and scheme.
    SetSandbox {
        session: String,
        mode: String,
        scheme: Option<String>,
    },
    /// Starts a new context frame so later turns ignore earlier history.
    ClearContext {
        generation: u64,
        session: String,
    },
    /// Toggles automatic rollup compression for the workflow.
    SetAutoCompress {
        session: String,
        enabled: bool,
    },
    /// Toggles the final audit pass for the workflow.
    SetFinalAudit {
        session: String,
        enabled: bool,
    },
    /// Sets the workflow's tool approval level (`default`, `smart`, `full`).
    SetApprovalLevel {
        session: String,
        level: String,
    },
    /// Toggles planning mode through the workflow phase.
    SetPlanning {
        session: String,
        enabled: bool,
    },
    /// Replaces the workflow's personality (execution style).
    SetPersonality {
        session: String,
        personality: String,
    },
    /// Enables skills and selects the workflow's skill names.
    SetSkills {
        session: String,
        enabled: bool,
        selected: Vec<String>,
    },
    /// Replaces the workflow's effective agent config (tools, MCP, plan approval).
    SetAgentConfig {
        session: String,
        config: Value,
    },
    /// Searches the authorized roots for the `@` file panel.
    SearchFiles {
        request: u64,
        paths: Vec<String>,
        query: String,
    },
}

/// Ordered, bounded background-to-UI handoff; payloads are never logged.
pub struct Update {
    pub generation: u64,
    pub queued: Instant,
    pub queue_depth: usize,
    pub kind: UpdateKind,
}

/// Control messages share the reliable queue with text deltas.
pub enum UpdateKind {
    ConfigSaved {
        session: String,
        key: &'static str,
        result: Result<Value, String>,
    },
    SettingsChanged {
        session: String,
        key: &'static str,
        result: Result<(), String>,
    },
    TodayCost(f64),
    Rewound(Result<bool, String>),
    SessionEdited {
        session: String,
        title: Option<String>,
        result: Result<(), String>,
    },
    Automations(Result<Vec<AutomationSummary>, String>),
    AutomationRuns(String, Result<Vec<AutomationRun>, String>),
    AutomationCreated(Result<(), String>),
    Sessions(Vec<Session>),
    Snapshot(Page),
    Older(Page),
    TurnUnconfirmed { client_id: String, message: String },
    QueueSubmissionFailed {
        queue_id: String,
        message: String,
    },
    Event(Value),
    Decisions {
        owner: String,
        pending: Vec<crate::chimes::PendingDecision>,
        reminder: Option<crate::chimes::Reminder>,
    },
    Connected(bool),
    /// Runtime configuration (`get_all_config`) for locale and appearance.
    Config(Value, Instant),
    SoundSaved {
        chime: crate::chimes::Chime,
        muted: bool,
        saved: bool,
    },
    DesktopMenuUnavailable,
    Reconnect,
    Reset,
    DecodeUs(u64),
    /// A session the runtime accepted and started for the first prompt.
    Opened(String),
    StartFailed(String),
    /// A child session snapshot for an expanded sub-agent card.
    Child(String, Result<Page, String>),
    /// A user-action failure worth showing above the composer.
    Error(String),
    /// A user-action success worth showing above the composer.
    ContextCleared(Result<Value, String>),
    ModelsSaved(Result<Value, String>),
    /// Agent, model and skill catalogues for the composer selectors.
    Catalogue {
        agents: Vec<Value>,
        models: Vec<Value>,
        skills: Vec<Value>,
        schemes: Vec<Value>,
    },
    /// Native and MCP tools for the configuration panels.
    Tools(Vec<Value>),
    /// Files matching the `@` query, keyed by the query they answered.
    Files {
        request: u64,
        files: Vec<Value>,
    },
    /// Children of a directory the authorized-path tree expanded.
    Dir {
        path: String,
        entries: Vec<DirEntry>,
        error: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn roots_come_from_the_serialized_agent_config() {
        let session: Session = serde_json::from_value(json!({
            "id": "s1",
            "title": "",
            "user_query": "Fix the build",
            "status": "executing",
            "agent_config": "{\"allowedPaths\":[\"/work/app/\",\"/work/docs\"]}",
        }))
        .expect("session");
        assert_eq!(session.primary_root().as_deref(), Some("/work/app"));
        assert_eq!(session.heading(), "Fix the build");
        assert_eq!(root_name("/work/app"), "app");

        // The snapshot route hands the config over as an object.
        assert_eq!(
            workflow_roots(&json!({"agent_config": {"allowedPaths": ["/work/lib"]}})),
            vec!["/work/lib".to_owned()]
        );
        assert_eq!(
            workflow_roots(&json!({"allowedPaths": ["/work/top"]})),
            vec!["/work/top".to_owned()]
        );
        assert!(workflow_roots(&json!({"status": "completed"})).is_empty());
    }

    #[test]
    fn untitled_sessions_have_no_heading() {
        let session = Session {
            id: "s2".into(),
            ..Session::default()
        };
        assert_eq!(session.heading(), "");
        assert!(session.primary_root().is_none());
    }
}
