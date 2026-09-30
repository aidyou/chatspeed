//! Durable capability-operation contract.
//!
//! Every capability mutation is one durable operation with a stable id, a
//! canonical request hash, a structured state and per-effect intents and
//! observations. The DB transaction never spans an external effect: an
//! operation records intent *before* the effect and the observation *after*,
//! so a crash between the two is classified as `needs_reconcile` rather than
//! silently retried (AC-2/INV-8).

use serde::{Deserialize, Serialize};

/// The capability family an operation belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
    Skill,
    Mcp,
}

impl CapabilityKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            CapabilityKind::Skill => "skill",
            CapabilityKind::Mcp => "mcp",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "skill" => Some(CapabilityKind::Skill),
            "mcp" => Some(CapabilityKind::Mcp),
            _ => None,
        }
    }
}

impl std::fmt::Display for CapabilityKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The stable operation state vocabulary.
///
/// `Completed`, `Blocked` and `Failed` are terminal. `NeedsReconcile` is a
/// terminal-for-this-attempt state: the effect may have happened, so the
/// operation is never blindly retried; doctor or an explicit reconcile
/// resolves it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    Planned,
    Staging,
    Checking,
    Applying,
    Completed,
    Blocked,
    Failed,
    NeedsReconcile,
}

impl OperationState {
    pub fn as_str(&self) -> &'static str {
        match self {
            OperationState::Planned => "planned",
            OperationState::Staging => "staging",
            OperationState::Checking => "checking",
            OperationState::Applying => "applying",
            OperationState::Completed => "completed",
            OperationState::Blocked => "blocked",
            OperationState::Failed => "failed",
            OperationState::NeedsReconcile => "needs_reconcile",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "planned" => Some(OperationState::Planned),
            "staging" => Some(OperationState::Staging),
            "checking" => Some(OperationState::Checking),
            "applying" => Some(OperationState::Applying),
            "completed" => Some(OperationState::Completed),
            "blocked" => Some(OperationState::Blocked),
            "failed" => Some(OperationState::Failed),
            "needs_reconcile" => Some(OperationState::NeedsReconcile),
            _ => None,
        }
    }

    /// Terminal states never accept another mutation under the same attempt.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            OperationState::Completed | OperationState::Blocked | OperationState::Failed
        )
    }

    /// In-flight states, as found after a crash. `NeedsReconcile` is excluded
    /// because it is already the resolved classification.
    pub fn is_interrupted(&self) -> bool {
        matches!(
            self,
            OperationState::Planned
                | OperationState::Staging
                | OperationState::Checking
                | OperationState::Applying
        )
    }

    /// Whether a successful terminal result is claimed.
    pub fn is_success(&self) -> bool {
        matches!(self, OperationState::Completed)
    }
}

impl std::fmt::Display for OperationState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Intent marker for one externally visible effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectIntent {
    NotStarted,
    IntentRecorded,
    Completed,
}

impl EffectIntent {
    pub fn as_str(&self) -> &'static str {
        match self {
            EffectIntent::NotStarted => "not_started",
            EffectIntent::IntentRecorded => "intent_recorded",
            EffectIntent::Completed => "completed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "not_started" => Some(EffectIntent::NotStarted),
            "intent_recorded" => Some(EffectIntent::IntentRecorded),
            "completed" => Some(EffectIntent::Completed),
            _ => None,
        }
    }
}

/// Observation marker for one externally visible effect.
///
/// `Unknown` is the crash-window state: an intent was recorded but the
/// observation never arrived, so the effect may or may not have happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectOutcome {
    Pending,
    Applied,
    Skipped,
    Blocked,
    Failed,
    Unknown,
}

impl EffectOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            EffectOutcome::Pending => "pending",
            EffectOutcome::Applied => "applied",
            EffectOutcome::Skipped => "skipped",
            EffectOutcome::Blocked => "blocked",
            EffectOutcome::Failed => "failed",
            EffectOutcome::Unknown => "unknown",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(EffectOutcome::Pending),
            "applied" => Some(EffectOutcome::Applied),
            "skipped" => Some(EffectOutcome::Skipped),
            "blocked" => Some(EffectOutcome::Blocked),
            "failed" => Some(EffectOutcome::Failed),
            "unknown" => Some(EffectOutcome::Unknown),
            _ => None,
        }
    }

    /// Whether the effect state cannot be proven from the journal alone.
    pub fn is_ambiguous(&self) -> bool {
        matches!(self, EffectOutcome::Unknown)
    }
}

/// A durable capability operation as stored in the journal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityOperation {
    pub operation_id: String,
    pub capability: CapabilityKind,
    pub operation_kind: String,
    pub actor_scope: String,
    pub idempotency_key: String,
    pub request_hash: String,
    /// The redacted request projection; never a raw secret.
    pub request: serde_json::Value,
    pub resource_key: String,
    pub state: OperationState,
    pub phase: Option<String>,
    /// The redacted result projection, present once the operation terminates.
    pub result: Option<serde_json::Value>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub reconcile_reason: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub completed_at_ms: Option<i64>,
}

/// A durable per-effect journal row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationEffect {
    pub effect_id: String,
    pub operation_id: String,
    pub effect_key: String,
    pub target: Option<String>,
    pub intent: EffectIntent,
    pub outcome: EffectOutcome,
    /// The redacted effect detail; never a raw secret.
    pub detail: Option<serde_json::Value>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// The envelope used to open one operation.
#[derive(Debug, Clone)]
pub struct OperationRequest {
    pub capability: CapabilityKind,
    pub operation_kind: String,
    pub actor_scope: String,
    pub idempotency_key: String,
    /// The full canonical request; it is redacted before persistence.
    pub request: serde_json::Value,
    /// The serialization scope: mutations on the same key never overlap.
    pub resource_key: String,
}

/// The result of opening an operation.
#[derive(Debug, Clone)]
pub enum OperationBegin {
    /// A new operation row was created and owns this attempt.
    Started(CapabilityOperation),
    /// The same idempotency key with the same request was already recorded;
    /// the caller must replay the stored result instead of applying an effect.
    Replay(CapabilityOperation),
}

impl OperationBegin {
    pub fn operation(&self) -> &CapabilityOperation {
        match self {
            OperationBegin::Started(operation) => operation,
            OperationBegin::Replay(operation) => operation,
        }
    }

    pub fn is_replay(&self) -> bool {
        matches!(self, OperationBegin::Replay(_))
    }

    pub fn into_operation(self) -> CapabilityOperation {
        match self {
            OperationBegin::Started(operation) => operation,
            OperationBegin::Replay(operation) => operation,
        }
    }
}

/// The stable actor scope of the single local owner.
///
/// The desktop application is the only runtime owner (INV-1), so every
/// capability mutation shares one scope; the column exists so a future
/// multi-owner contract does not require a migration.
pub const LOCAL_ACTOR_SCOPE: &str = "local";

/// One file recorded in an installation's ownership manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillFileEntry {
    /// The path relative to the skill directory, using `/` separators.
    pub path: String,
    pub sha256: String,
    pub size_bytes: i64,
}

/// Lifecycle of a managed Skill installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillInstallationState {
    /// The commit is in flight; the directory may be partially written.
    Installing,
    /// Committed and owned by ChatSpeed.
    Installed,
    /// Moved aside by an uninstall that has not finalized yet.
    Quarantined,
    /// Removed from the target directory; the row is retained as evidence.
    Removed,
    /// The on-disk content no longer matches the recorded manifest.
    Drifted,
}

impl SkillInstallationState {
    pub fn as_str(&self) -> &'static str {
        match self {
            SkillInstallationState::Installing => "installing",
            SkillInstallationState::Installed => "installed",
            SkillInstallationState::Quarantined => "quarantined",
            SkillInstallationState::Removed => "removed",
            SkillInstallationState::Drifted => "drifted",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "installing" => Some(SkillInstallationState::Installing),
            "installed" => Some(SkillInstallationState::Installed),
            "quarantined" => Some(SkillInstallationState::Quarantined),
            "removed" => Some(SkillInstallationState::Removed),
            "drifted" => Some(SkillInstallationState::Drifted),
            _ => None,
        }
    }

    /// Whether ChatSpeed may delete the directory under this state.
    pub fn is_managed(&self) -> bool {
        matches!(
            self,
            SkillInstallationState::Installed | SkillInstallationState::Installing
        )
    }
}

/// The durable ownership proof of one Skill installation.
///
/// A row is the only thing that authorizes a later uninstall: without it the
/// directory is non-managed and is never deleted (AC-7/INV-6).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillInstallation {
    pub installation_id: String,
    pub skill_name: String,
    pub target_id: String,
    pub install_path: String,
    pub source_kind: String,
    pub source_ref: String,
    pub checker_version: String,
    pub verdict: String,
    pub content_digest: String,
    pub file_manifest: Vec<SkillFileEntry>,
    pub marker_nonce: String,
    pub manifest_digest: String,
    pub state: SkillInstallationState,
    pub operation_id: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}
