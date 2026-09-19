use crate::db::{WorkflowAutomation, WorkflowAutomationRun};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowAutomationShellConfig {
    pub command: Option<String>,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub args: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowAutomationRequest {
    pub id: Option<String>,
    pub title: String,
    pub prompt: Option<String>,
    pub prompt_file_path: Option<String>,
    pub agent_id: String,
    pub agent_config: Option<Value>,
    pub allowed_paths: Vec<String>,
    pub shell_config: Option<Value>,
    pub schedule_kind: String,
    pub schedule_config: Value,
    #[serde(default)]
    pub continuous_context: bool,
    pub self_review: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowAutomationRunNowResult {
    pub automation: WorkflowAutomation,
    pub run: WorkflowAutomationRun,
    pub workflow_session_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DailyScheduleConfig {
    #[serde(default)]
    pub time: Option<String>,
    #[serde(default)]
    pub times: Vec<String>,
    #[serde(default)]
    pub weekdays: Vec<u32>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct IntervalScheduleConfig {
    #[serde(alias = "interval_hours")]
    pub interval_minutes: u32,
    #[serde(default)]
    pub weekdays: Vec<u32>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub anchor_time: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct OnceScheduleConfig {
    pub run_at: String,
}

// --- Phase 3D canonical transport-neutral contract -------------------------
//
// Every type below is the single projection shared by Tauri, the control-plane
// HTTP surface, the `cs` CLI and the scheduler. Rust/internal/HTTP use
// canonical `snake_case`; camelCase appears only at the Tauri/JavaScript edge
// (INV-9). None of these are a second state machine: they project the durable
// automation row and its workflow snapshot (INV-2/INV-7).

/// Current plan-schema version. A `draft` records it so `apply` can reject a
/// plan produced by an incompatible build rather than misinterpreting it.
pub const AUTOMATION_PLAN_VERSION: &str = "automation-plan-v1";

/// The actor-scope prefix for a mutation made over the control plane, so a CLI
/// retry and a desktop click with the same idempotency key never collide with
/// each other's durable receipt.
pub const AUTOMATION_ACTOR_SCOPE_CONTROL_PLANE: &str = "control-plane";
/// The actor-scope prefix for a mutation made from the desktop/Tauri layer.
pub const AUTOMATION_ACTOR_SCOPE_DESKTOP: &str = "desktop";

/// Explicit, persistable automation configuration. Permission-bearing fields
/// (`allowed_paths`, `shell_config`, and the referenced `agent_id`) are kept
/// distinct from ordinary content so the plan layer can reason about escalation
/// without granting anything (INV-3).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", default)]
pub struct AutomationSpec {
    pub title: String,
    pub prompt: Option<String>,
    pub prompt_file_path: Option<String>,
    pub agent_id: String,
    pub agent_config: Option<Value>,
    pub allowed_paths: Vec<String>,
    pub shell_config: Option<Value>,
    pub schedule_kind: String,
    pub schedule_config: Value,
    pub continuous_context: bool,
    pub self_review: bool,
    pub enabled: bool,
}

/// Structured or intent-constrained input to `draft`. Exactly one of `spec` or
/// `intent` is expected; a natural-language `intent` may only express ordinary
/// content and references to *existing* agents/paths/shell policy — it can never
/// mint a new shell command, path, network, MCP or Skill permission (INV-3).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", default)]
pub struct AutomationDraftInput {
    /// Optional structured target. When present with `automation_id`, the plan
    /// is relative to that existing automation's current revision.
    pub automation_id: Option<String>,
    /// A fully specified desired state. Preferred by the desktop editor and CLI.
    pub spec: Option<AutomationSpec>,
    /// A free-form request that the constrained parser may turn into an
    /// ordinary-content plan. It is never a permission grant.
    pub intent: Option<String>,
}

/// A single stable-code advisory attached to a plan. `message` is a code-like
/// human string produced by the backend, never raw command/prompt/secret text.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AutomationWarning {
    pub code: String,
    pub message: String,
}

/// Auditable summary of what a plan would change about permissions, so `apply`
/// can require explicit acknowledgement only for real expansions (INV-3/INV-5).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case", default)]
pub struct PermissionSummary {
    /// Whether the plan references an existing agent (validated by the backend).
    pub agent_id: String,
    /// Ordered allowed paths the plan would persist.
    pub allowed_paths: Vec<String>,
    /// Whether the plan carries a pre-workflow shell command (only an existing
    /// user-authored command; the parser never invents one).
    pub has_shell: bool,
    /// Bounded preview of the configured shell command for review only.
    pub shell_preview: Option<String>,
    /// Paths/privileges present in the plan but not on the existing automation,
    /// i.e. a permission expansion that `apply` must have acknowledged.
    pub permission_expansion: bool,
}

/// Plan lifecycle. `ready` may be applied; `blocked` cannot because the intent
/// requested a permission the parser is not allowed to mint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationPlanStatus {
    Ready,
    Blocked,
}

/// The side-effect-free plan a `draft` produces. It never writes the database,
/// never runs shell, and never starts a workflow (INV-4). `apply` re-derives the
/// hash and re-checks the base revision before mutating (INV-5).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AutomationPlanV1 {
    pub plan_version: String,
    /// `None` for a create plan; `Some(id)` for an update plan against an
    /// existing automation.
    pub automation_id: Option<String>,
    /// Current stored revision the plan was derived from; `None` for create.
    pub base_revision: Option<i64>,
    /// Canonical SHA-256 (lowercase hex) over the restricted `changes`.
    pub plan_hash: String,
    /// The desired state this plan would persist.
    pub changes: AutomationSpec,
    pub permission_summary: PermissionSummary,
    pub warnings: Vec<AutomationWarning>,
    pub status: AutomationPlanStatus,
    /// Unix-millis after which the plan is stale and `apply` must reject it.
    pub expires_at_ms: i64,
    /// Unix-millis the plan was created, for display and expiry accounting.
    pub created_at_ms: i64,
}

/// A request to apply a previously returned plan. There is deliberately no
/// force/skip-check field: a stale hash, moved revision or unknown target must
/// be rejected, not overridden (AC-4/INV-5).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AutomationApplyRequest {
    pub plan: AutomationPlanV1,
    /// The exact `plan_hash` the caller is authorizing. `apply` recomputes the
    /// hash from `plan.changes` and rejects a mismatch (tamper/expiry guard).
    pub expected_plan_hash: String,
    /// Acknowledge a permission expansion identified in the plan summary. When
    /// the plan expands permissions but this is false, `apply` is rejected.
    pub acknowledge_permission_changes: bool,
}

/// The public automation projection returned by every observation/mutation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AutomationView {
    pub automation_id: String,
    pub title: String,
    pub prompt: Option<String>,
    pub prompt_file_path: Option<String>,
    pub agent_id: String,
    pub allowed_paths: Vec<String>,
    pub shell_config: Option<Value>,
    pub schedule_kind: String,
    pub schedule_config: Value,
    pub continuous_context: bool,
    pub self_review: bool,
    pub enabled: bool,
    pub current_workflow_session_id: Option<String>,
    pub next_run_at: Option<String>,
    pub last_run_at: Option<String>,
    pub revision: i64,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

/// The public run projection. `status` is the automation lifecycle
/// (`pending|starting|running|completed|failed|cancelled|needs_reconcile`);
/// `workflow_status`/`wait_reason` are the structured workflow facts that back
/// the projection. `error` is redacted (bounded, no raw shell output) (INV-8).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AutomationRunView {
    pub run_id: String,
    pub automation_id: String,
    pub trigger: String,
    pub dispatch_key: Option<String>,
    pub status: String,
    pub workflow_session_id: Option<String>,
    pub scheduled_for: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub error: Option<String>,
    pub workflow_status: Option<String>,
    pub wait_reason: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

/// Result of a mutating automation operation. `Replayed` carries the same
/// effect as a prior completed mutation with the same idempotency evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AutomationMutationResult {
    pub outcome: AutomationMutationOutcome,
    pub automation: Option<AutomationView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationMutationOutcome {
    /// The mutation applied and advanced the revision.
    Applied,
    /// An identical mutation already completed; nothing new ran.
    Replayed,
    /// The enabled flag flipped (enable/disable), which carries no revision
    /// bump beyond the CAS that guarded it.
    Enabled,
    /// The automation and its non-active runs were deleted after confirmation.
    Deleted,
}

/// Result of `run` / `dispatch_due`. Async start success is *not* completion;
/// the run reaches a terminal state only via structured workflow projection
/// (AC-8).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AutomationDispatchResult {
    pub outcome: AutomationDispatchOutcome,
    pub run: Option<AutomationRunView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationDispatchOutcome {
    /// A run was created and the workflow start was accepted (still non-terminal).
    Accepted,
    /// An active run already exists for this automation (manual overlap guard).
    Busy,
    /// The scheduled slot was already claimed or is no longer due (stale claim).
    Skipped,
}
