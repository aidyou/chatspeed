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

/// Minimal read-only workflow projection.
#[derive(Clone, Default, Deserialize)]
pub struct Session {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub status: String,
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
}

/// UI requests use generations to invalidate all old-selection results.
pub enum Command {
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
    Signal {
        id: String,
        signal: Value,
    },
    Stop {
        id: String,
    },
    /// Creates a session and starts it with the first prompt.
    StartNew {
        prompt: String,
    },
    /// Loads a child session for an expanded sub-agent card.
    Child {
        id: String,
        session: String,
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
    Sessions(Vec<Session>),
    Snapshot(Page),
    Older(Page),
    Event(Value),
    Connected(bool),
    /// Runtime configuration (`get_all_config`) for locale and appearance.
    Config(Value),
    Reconnect,
    Reset,
    DecodeUs(u64),
    /// A session the runtime accepted and started for the first prompt.
    Opened(String),
    /// A child session snapshot for an expanded sub-agent card.
    Child(String, Page),
    /// A user-action failure worth showing above the composer.
    Error(String),
}
