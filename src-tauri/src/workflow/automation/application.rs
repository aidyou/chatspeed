//! The single transport-neutral automation facade (`AutomationApplicationService`).
//!
//! Every automation mutation and observation — Tauri, the control-plane HTTP
//! surface, the `cs` CLI (through HTTP), the desktop store and the scheduler —
//! resolves to these methods, so there is exactly one automation state machine
//! and one DTO/error contract (AC-1/INV-2). The service is stateless over the
//! durable store: it holds the shared `MainStore` and TSID generator and reads
//! the same workflow snapshot the runtime writes, so it never becomes a second
//! runtime or journal authority (INV-1/INV-10).
//!
//! The execution kernel (`execute_automation_run`) and manual-run helper
//! (`create_manual_run`) live in `service.rs` and are reached from
//! `WorkflowApplicationService`; this module owns plan/validation/revision/CAS/
//! receipt/lifecycle-projection semantics and the canonical snake_case views.

use crate::capability::operation::{canonical_request_hash, now_ms};
use crate::db::automation::{CasOutcome, ClaimOutcome, ReceiptOutcome};
use crate::db::{
    MainStore, WorkflowAutomation, WorkflowAutomationRun, WorkflowAutomationUpsert,
};
use crate::libs::tsid::TsidGenerator;
use crate::workflow::automation::errors::{self, AutomationError};
use crate::workflow::automation::service::request_to_upsert;
use crate::workflow::automation::types::{
    AutomationApplyRequest, AutomationDispatchOutcome, AutomationDispatchResult,
    AutomationDraftInput, AutomationMutationOutcome, AutomationMutationResult, AutomationPlanStatus,
    AutomationPlanV1, AutomationRunView, AutomationSpec, AutomationView, AutomationWarning,
    PermissionSummary, WorkflowAutomationRequest, WorkflowAutomationRunNowResult,
    AUTOMATION_PLAN_VERSION,
};
use serde_json::{json, Value};
use std::sync::Arc;

/// A draft plan stays applyable for this long. A stale plan must be re-drafted
/// so a user never applies against a surprise-moved revision (AC-4).
const PLAN_TTL_MS: i64 = 15 * 60 * 1000;

/// Bounded shell-command preview length for the permission summary. The preview
/// is a user-authored configuration value, not runtime output (INV-8).
const SHELL_PREVIEW_LIMIT: usize = 200;

/// The automation facade. Stateless over the shared store; safe to construct per
/// call and cheap (two `Arc` clones).
pub struct AutomationApplicationService {
    main_store: Arc<MainStore>,
    tsid_generator: Arc<TsidGenerator>,
}

impl AutomationApplicationService {
    pub fn new(main_store: Arc<MainStore>, tsid_generator: Arc<TsidGenerator>) -> Self {
        Self {
            main_store,
            tsid_generator,
        }
    }

    fn store(&self) -> &MainStore {
        &self.main_store
    }

    // --- observations -----------------------------------------------------

    pub fn list(&self) -> Result<Vec<AutomationView>, AutomationError> {
        let rows = self
            .store()
            .list_workflow_automations()
            .map_err(|e| AutomationError::internal(e.to_string()))?;
        Ok(rows.iter().map(to_view).collect())
    }

    pub fn get(&self, automation_id: &str) -> Result<Option<AutomationView>, AutomationError> {
        let row = self
            .store()
            .get_workflow_automation(automation_id)
            .map_err(|e| AutomationError::internal(e.to_string()))?;
        Ok(row.as_ref().map(to_view))
    }

    /// Legacy camelCase row reads for the existing Tauri editor wire (INV-9).
    /// They are read-only observations over the same store authority; every
    /// *mutation* still resolves to the canonical facade methods.
    pub(crate) fn list_rows(&self) -> Result<Vec<WorkflowAutomation>, AutomationError> {
        self.store()
            .list_workflow_automations()
            .map_err(|e| AutomationError::internal(e.to_string()))
    }

    pub(crate) fn get_row(
        &self,
        automation_id: &str,
    ) -> Result<Option<WorkflowAutomation>, AutomationError> {
        self.store()
            .get_workflow_automation(automation_id)
            .map_err(|e| AutomationError::internal(e.to_string()))
    }

    /// Legacy camelCase run rows for the existing `workflow_automation_list_runs`
    /// wire. The projected, snake_case lifecycle is exposed via `runs()`.
    pub(crate) fn run_rows(
        &self,
        automation_id: &str,
    ) -> Result<Vec<WorkflowAutomationRun>, AutomationError> {
        self.store()
            .list_workflow_automation_runs(automation_id)
            .map_err(|e| AutomationError::internal(e.to_string()))
    }

    /// Lists an automation's runs with a bounded, structured terminal projection
    /// from the workflow snapshot (INV-7). Runs that cannot be proven terminal
    /// are left for `reconcile_all`/re-dispatch and surface as `needs_reconcile`.
    pub fn runs(&self, automation_id: &str) -> Result<Vec<AutomationRunView>, AutomationError> {
        let automation = self
            .store()
            .get_workflow_automation(automation_id)
            .map_err(|e| AutomationError::internal(e.to_string()))?
            .ok_or_else(|| AutomationError::not_found(format!("Automation {automation_id}")))?;
        let runs = self
            .store()
            .list_workflow_automation_runs(&automation.id)
            .map_err(|e| AutomationError::internal(e.to_string()))?;
        let mut views = Vec::with_capacity(runs.len());
        for run in &runs {
            let projected = self.project_run(run)?;
            views.push(self.run_view(&projected)?);
        }
        Ok(views)
    }

    /// Bounded reconciliation over not-yet-terminal runs. Called at startup and
    /// before a dispatch, it converges a run only on structured snapshot evidence
    /// and never re-starts an unknown effect (AC-8).
    pub fn reconcile_all(&self) -> Result<(), AutomationError> {
        let runs = self
            .store()
            .list_unreconciled_automation_runs()
            .map_err(|e| AutomationError::internal(e.to_string()))?;
        for run in &runs {
            let _ = self.project_run(run)?;
        }
        Ok(())
    }

    // --- draft / apply ----------------------------------------------------

    /// Produces a side-effect-free plan. It never writes the database, runs
    /// shell or touches the workflow runtime (INV-4). A natural-language `intent`
    /// with no structured spec can only yield a `blocked` plan: the parser is not
    /// a permission manager and must not mint agent/path/shell/network/MCP grants
    /// from free text (INV-3).
    pub fn draft(&self, input: AutomationDraftInput) -> Result<AutomationPlanV1, AutomationError> {
        let created_at_ms = now_ms();
        let expires_at_ms = created_at_ms + PLAN_TTL_MS;

        let (automation_id, spec) = match (input.spec.clone(), input.intent.as_deref()) {
            (Some(spec), _) => (input.automation_id.clone(), Some(spec)),
            (None, Some(_intent)) => (input.automation_id.clone(), None),
            (None, None) => {
                return Err(AutomationError::invalid_request(
                    "draft requires a structured spec or an intent",
                ))
            }
        };

        // A bare intent cannot produce an applyable, non-escalating plan.
        let Some(spec) = spec else {
            return Ok(AutomationPlanV1 {
                plan_version: AUTOMATION_PLAN_VERSION.to_string(),
                automation_id: automation_id.clone(),
                base_revision: self.base_revision_for(automation_id.as_deref())?,
                plan_hash: String::new(),
                changes: AutomationSpec::default(),
                permission_summary: PermissionSummary::default(),
                warnings: vec![blocked_intent_warning()],
                status: AutomationPlanStatus::Blocked,
                expires_at_ms,
                created_at_ms,
            });
        };

        let mut warnings = Vec::new();
        let base_revision = self.base_revision_for(automation_id.as_deref())?;
        let existing = automation_id
            .as_deref()
            .map(|id| self.store().get_workflow_automation(id))
            .transpose()
            .map_err(|e| AutomationError::internal(e.to_string()))?
            .flatten();

        let permission_summary = self.permission_summary(&spec, existing.as_ref(), &mut warnings)?;
        // Validate the same way the write path will, but persist nothing.
        self.validate_spec(&spec)?;
        if let Some(existing) = &existing {
            // Reusing the persisted session id requires continuous context to
            // stay coherent; a plan that would drop it is flagged for review.
            if existing.continuous_context && !spec.continuous_context {
                warnings.push(AutomationWarning {
                    code: "continuous_context_disabled".to_string(),
                    message: "plan disables continuous context; the current session will not be reused".to_string(),
                });
            }
        }

        let status = AutomationPlanStatus::Ready;

        let plan_hash = plan_hash(automation_id.as_deref(), base_revision, &spec);
        Ok(AutomationPlanV1 {
            plan_version: AUTOMATION_PLAN_VERSION.to_string(),
            automation_id,
            base_revision,
            plan_hash,
            changes: spec,
            permission_summary,
            warnings,
            status,
            expires_at_ms,
            created_at_ms,
        })
    }

    /// Applies a previously returned plan only after re-deriving its hash and
    /// re-checking the base revision and permission acknowledgement (INV-5). It
    /// never creates: an unknown target is `not_found`, a moved revision is
    /// `revision_conflict`, and a tampered/stale/expired plan is rejected.
    pub fn apply(
        &self,
        request: &AutomationApplyRequest,
        actor_scope: &str,
        idempotency_key: Option<&str>,
    ) -> Result<AutomationMutationResult, AutomationError> {
        let plan = &request.plan;
        if plan.plan_version != AUTOMATION_PLAN_VERSION {
            return Err(AutomationError::invalid_request(format!(
                "unsupported plan version {}",
                plan.plan_version
            )));
        }
        if plan.status != AutomationPlanStatus::Ready {
            return Err(AutomationError::new(
                errors::code::PERMISSION_EXPANSION,
                "plan is blocked and cannot be applied",
            ));
        }
        if now_ms() > plan.expires_at_ms {
            return Err(AutomationError::plan_expired("plan has expired, re-draft before applying"));
        }
        // Recompute the hash from the plan's own changes and cross-check the
        // caller's authorized hash. Any mismatch is a tamper/expiry guard.
        let derived = plan_hash(
            plan.automation_id.as_deref(),
            plan.base_revision,
            &plan.changes,
        );
        if derived != plan.plan_hash || derived != request.expected_plan_hash {
            return Err(AutomationError::conflict(
                "plan hash does not match its changes or the authorized hash",
            ));
        }

        let Some(automation_id) = plan.automation_id.clone() else {
            // A create plan has no target to attach; require an explicit create.
            return Err(AutomationError::invalid_request(
                "a create plan must be applied via the create operation",
            ));
        };

        if plan.permission_summary.permission_expansion
            && !request.acknowledge_permission_changes
        {
            return Err(AutomationError::permission_expansion(
                "plan expands permissions; acknowledge before applying",
            ));
        }

        let existing = self
            .store()
            .get_workflow_automation(&automation_id)
            .map_err(|e| AutomationError::internal(e.to_string()))?
            .ok_or_else(|| AutomationError::not_found(format!("Automation {automation_id}")))?;

        if existing.revision != plan.base_revision.unwrap_or(i64::MIN) {
            return Err(AutomationError::revision_conflict(
                "the automation changed since this plan was drafted; re-draft",
            ));
        }

        let request_hash = canonical_request_hash(&json!({
            "operation": "apply",
            "plan_hash": derived,
            "automation_id": automation_id,
        }));
        self.apply_mutation_with_receipt(
            actor_scope,
            idempotency_key,
            "apply",
            &request_hash,
            |svc| svc.update_existing(&automation_id, &plan.changes, existing.revision),
        )
    }

    // --- explicit create / update / enable / disable / delete ------------

    pub fn create(
        &self,
        spec: &AutomationSpec,
        actor_scope: &str,
        idempotency_key: Option<&str>,
    ) -> Result<AutomationMutationResult, AutomationError> {
        self.validate_spec(spec)?;
        let request_hash = canonical_request_hash(&json!({
            "operation": "create",
            "spec": serde_json::to_value(spec).map_err(|e| AutomationError::internal(e.to_string()))?,
        }));
        self.apply_mutation_with_receipt(
            actor_scope,
            idempotency_key,
            "create",
            &request_hash,
            |svc| svc.insert_new(spec),
        )
    }

    pub fn update(
        &self,
        automation_id: &str,
        spec: &AutomationSpec,
        expected_revision: i64,
        actor_scope: &str,
        idempotency_key: Option<&str>,
    ) -> Result<AutomationMutationResult, AutomationError> {
        self.validate_spec(spec)?;
        let request_hash = canonical_request_hash(&json!({
            "operation": "update",
            "automation_id": automation_id,
            "expected_revision": expected_revision,
            "spec": serde_json::to_value(spec).map_err(|e| AutomationError::internal(e.to_string()))?,
        }));
        let id = automation_id.to_string();
        self.apply_mutation_with_receipt(
            actor_scope,
            idempotency_key,
            "update",
            &request_hash,
            move |svc| svc.update_existing(&id, spec, expected_revision),
        )
    }

    pub fn set_enabled(
        &self,
        automation_id: &str,
        enabled: bool,
        expected_revision: Option<i64>,
        actor_scope: &str,
        idempotency_key: Option<&str>,
    ) -> Result<AutomationMutationResult, AutomationError> {
        let request_hash = canonical_request_hash(&json!({
            "operation": if enabled { "enable" } else { "disable" },
            "automation_id": automation_id,
            "expected_revision": expected_revision,
        }));
        let id = automation_id.to_string();
        self.apply_mutation_with_receipt(
            actor_scope,
            idempotency_key,
            if enabled { "enable" } else { "disable" },
            &request_hash,
            move |svc| svc.set_enabled_inner(&id, enabled, expected_revision),
        )
    }

    /// Destructive delete. Refuses an automation with an active/unknown run and
    /// requires explicit confirmation before cascading the workflow tree
    /// (AC-10). The prior non-confirming behavior is preserved only as a stable
    /// `confirmation_required` error, never a silent cascade.
    pub fn delete(
        &self,
        automation_id: &str,
        confirm: bool,
        actor_scope: &str,
        idempotency_key: Option<&str>,
    ) -> Result<AutomationMutationResult, AutomationError> {
        let request_hash = canonical_request_hash(&json!({
            "operation": "delete",
            "automation_id": automation_id,
            "confirm": confirm,
        }));
        let id = automation_id.to_string();
        self.apply_mutation_with_receipt(
            actor_scope,
            idempotency_key,
            "delete",
            &request_hash,
            move |svc| svc.delete_inner(&id, confirm),
        )
    }

    // --- shared mutation mechanics ---------------------------------------

    /// Reserves a durable receipt, and on first execution runs `execute` and
    /// completes the receipt with a redacted result reference. A completed
    /// identical mutation replays without re-running (AC-5).
    fn apply_mutation_with_receipt<F>(
        &self,
        actor_scope: &str,
        idempotency_key: Option<&str>,
        operation: &str,
        request_hash: &str,
        execute: F,
    ) -> Result<AutomationMutationResult, AutomationError>
    where
        F: FnOnce(&AutomationApplicationService) -> Result<AutomationMutationResult, AutomationError>,
    {
        let Some(key) = idempotency_key.map(str::trim).filter(|key| !key.is_empty()) else {
            return execute(self);
        };

        match self
            .store()
            .reserve_automation_mutation(actor_scope, key, operation, request_hash)
            .map_err(|e| AutomationError::internal(e.to_string()))?
        {
            ReceiptOutcome::Conflict => Err(AutomationError::conflict(
                "idempotency key was already used with a different request",
            )),
            ReceiptOutcome::Replay(result_json) => {
                // A completed identical mutation replays without re-running. The
                // stored reference carries the automation id; re-read the current
                // durable row so the replay returns the same observable view.
                let automation_id = result_json
                    .as_deref()
                    .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                    .and_then(|value| {
                        value
                            .get("automation_id")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    });
                let automation = match automation_id {
                    Some(id) => self
                        .store()
                        .get_workflow_automation(&id)
                        .ok()
                        .flatten()
                        .as_ref()
                        .map(to_view),
                    None => None,
                };
                Ok(AutomationMutationResult {
                    outcome: AutomationMutationOutcome::Replayed,
                    automation,
                })
            }
            ReceiptOutcome::Proceed => {
                let result = execute(self)?;
                if result.outcome != AutomationMutationOutcome::Replayed {
                    let redacted = result
                        .automation
                        .as_ref()
                        .map(view_ref)
                        .and_then(|value| serde_json::to_string(&value).ok());
                    let _ = self
                        .store()
                        .complete_automation_mutation(actor_scope, key, redacted.as_deref());
                }
                Ok(result)
            }
        }
    }

    fn insert_new(&self, spec: &AutomationSpec) -> Result<AutomationMutationResult, AutomationError> {
        let id = self
            .tsid_generator
            .generate()
            .map_err(|e| AutomationError::internal(e.to_string()))?;
        self.insert_with_id(spec, id)
    }

    /// Inserts an automation with a specific id. The compat save adapter may pass
    /// a client-generated id for a brand-new automation, preserving the legacy
    /// editor behavior while still sharing the facade's single write path.
    fn insert_with_id(
        &self,
        spec: &AutomationSpec,
        id: String,
    ) -> Result<AutomationMutationResult, AutomationError> {
        let upsert = self.build_upsert(spec, id.clone(), None)?;
        let created = self
            .store()
            .create_workflow_automation(&upsert)
            .map_err(map_store_write_error)?;
        Ok(mutation_result(AutomationMutationOutcome::Applied, &created))
    }

    /// Legacy editor save adapter. It creates when the id is absent or the row
    /// does not exist yet, and compare-and-set updates an existing row using its
    /// current revision — preserving the historical camelCase request/return wire
    /// while every write still flows through the facade internals (INV-2/INV-9).
    pub(crate) fn compat_save(
        &self,
        request: &WorkflowAutomationRequest,
    ) -> Result<WorkflowAutomation, AutomationError> {
        let spec = request_to_spec(request);
        self.validate_spec(&spec)?;
        let provided = request
            .id
            .clone()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty());
        let result = match provided.as_deref() {
            Some(id) if self.get_row(id)?.is_some() => {
                let revision = self
                    .get_row(id)?
                    .ok_or_else(|| AutomationError::not_found(format!("Automation {id}")))?
                    .revision;
                self.update_existing(id, &spec, revision)?
            }
            Some(id) => self.insert_with_id(&spec, id.to_string())?,
            None => self.insert_new(&spec)?,
        };
        let automation_id = result
            .automation
            .as_ref()
            .map(|view| view.automation_id.clone())
            .ok_or_else(|| AutomationError::internal("save produced no automation"))?;
        self.get_row(&automation_id)?
            .ok_or_else(|| AutomationError::internal("saved automation disappeared"))
    }

    fn update_existing(
        &self,
        automation_id: &str,
        spec: &AutomationSpec,
        expected_revision: i64,
    ) -> Result<AutomationMutationResult, AutomationError> {
        let existing = self
            .store()
            .get_workflow_automation(automation_id)
            .map_err(|e| AutomationError::internal(e.to_string()))?
            .ok_or_else(|| AutomationError::not_found(format!("Automation {automation_id}")))?;
        let upsert = self.build_upsert(
            spec,
            automation_id.to_string(),
            existing.current_workflow_session_id.clone(),
        )?;
        match self
            .store()
            .update_workflow_automation_cas(&upsert, expected_revision)
            .map_err(map_store_write_error)?
        {
            CasOutcome::Updated(row) => {
                Ok(mutation_result(AutomationMutationOutcome::Applied, &row))
            }
            CasOutcome::NotFound => {
                Err(AutomationError::not_found(format!("Automation {automation_id}")))
            }
            CasOutcome::RevisionConflict => Err(AutomationError::revision_conflict(
                "the automation was modified concurrently; reload and retry",
            )),
        }
    }

    fn set_enabled_inner(
        &self,
        automation_id: &str,
        enabled: bool,
        expected_revision: Option<i64>,
    ) -> Result<AutomationMutationResult, AutomationError> {
        let existing = self
            .store()
            .get_workflow_automation(automation_id)
            .map_err(|e| AutomationError::internal(e.to_string()))?
            .ok_or_else(|| AutomationError::not_found(format!("Automation {automation_id}")))?;
        let expected = expected_revision.unwrap_or(existing.revision);
        let next_run_at = if enabled {
            let schedule_config: Value = serde_json::from_str(&existing.schedule_config)
                .map_err(|e| AutomationError::internal(e.to_string()))?;
            crate::workflow::automation::service::compute_next_run_at(
                &existing.schedule_kind,
                &schedule_config,
            )
            .map_err(AutomationError::invalid_request)?
        } else {
            None
        };
        match self
            .store()
            .set_workflow_automation_enabled_cas(automation_id, enabled, next_run_at, expected)
            .map_err(map_store_write_error)?
        {
            CasOutcome::Updated(row) => Ok(mutation_result(AutomationMutationOutcome::Enabled, &row)),
            CasOutcome::NotFound => {
                Err(AutomationError::not_found(format!("Automation {automation_id}")))
            }
            CasOutcome::RevisionConflict => Err(AutomationError::revision_conflict(
                "the automation was modified concurrently; reload and retry",
            )),
        }
    }

    fn delete_inner(
        &self,
        automation_id: &str,
        confirm: bool,
    ) -> Result<AutomationMutationResult, AutomationError> {
        if self
            .store()
            .get_workflow_automation(automation_id)
            .map_err(|e| AutomationError::internal(e.to_string()))?
            .is_none()
        {
            return Err(AutomationError::not_found(format!(
                "Automation {automation_id}"
            )));
        }
        if !confirm {
            return Err(AutomationError::confirmation_required(
                "delete is destructive and cascades the automation's runs and workflow tree; confirm explicitly",
            ));
        }
        if self
            .store()
            .automation_has_blocking_run(automation_id)
            .map_err(|e| AutomationError::internal(e.to_string()))?
        {
            return Err(AutomationError::busy(
                "automation has an active or unreconciled run; it cannot be deleted",
            ));
        }
        self.store()
            .delete_workflow_automation(automation_id)
            .map_err(|e| AutomationError::internal(e.to_string()))?;
        Ok(AutomationMutationResult {
            outcome: AutomationMutationOutcome::Deleted,
            automation: None,
        })
    }

    // --- internal helpers -------------------------------------------------

    fn base_revision_for(
        &self,
        automation_id: Option<&str>,
    ) -> Result<Option<i64>, AutomationError> {
        match automation_id {
            None => Ok(None),
            Some(id) => {
                let existing = self
                    .store()
                    .get_workflow_automation(id)
                    .map_err(|e| AutomationError::internal(e.to_string()))?
                    .ok_or_else(|| AutomationError::not_found(format!("Automation {id}")))?;
                Ok(Some(existing.revision))
            }
        }
    }

    fn validate_spec(&self, spec: &AutomationSpec) -> Result<(), AutomationError> {
        // The full write-time validation (title/agent/prompt/shell/schedule)
        // happens in `build_upsert`; this just guards the empty/unknown target
        // so a caller cannot create a target-less automation via `update`.
        if spec.title.trim().is_empty() {
            return Err(AutomationError::invalid_request("automation title is required"));
        }
        Ok(())
    }

    fn build_upsert(
        &self,
        spec: &AutomationSpec,
        id: String,
        existing_session: Option<String>,
    ) -> Result<WorkflowAutomationUpsert, AutomationError> {
        let request = spec_to_request(spec, Some(id.clone()));
        request_to_upsert(request, id, existing_session).map_err(|e| {
            AutomationError::new(errors::code::INVALID_REQUEST, format!("invalid automation spec: {e}"))
        })
    }

    fn permission_summary(
        &self,
        spec: &AutomationSpec,
        existing: Option<&WorkflowAutomation>,
        warnings: &mut Vec<AutomationWarning>,
    ) -> Result<PermissionSummary, AutomationError> {
        let shell = parse_shell_preview(spec.shell_config.as_ref());
        let has_shell = shell.is_some();
        if has_shell {
            warnings.push(AutomationWarning {
                code: "shell_config_present".to_string(),
                message: "plan carries a pre-workflow shell command".to_string(),
            });
        }
        let existing_paths: Vec<String> = existing
            .map(|row| serde_json::from_str(&row.allowed_paths).unwrap_or_default())
            .unwrap_or_default();
        let existing_has_shell = existing
            .map(|row| {
                row.shell_config
                    .as_deref()
                    .map(|raw| !raw.trim().is_empty() && raw.trim() != "null")
                    .unwrap_or(false)
            })
            .unwrap_or(false);

        // A permission expansion is anything the plan grants that the existing
        // automation did not already have; for a create, any granted permission.
        let new_paths: Vec<String> = spec
            .allowed_paths
            .iter()
            .filter(|path| !existing_paths.iter().any(|existing| existing == *path))
            .cloned()
            .collect();
        let new_shell = has_shell && !existing_has_shell;
        let agent_changed = existing
            .map(|row| row.agent_id != spec.agent_id)
            .unwrap_or(true);
        let permission_expansion = !new_paths.is_empty() || new_shell || agent_changed;
        if agent_changed && existing.is_some() {
            warnings.push(AutomationWarning {
                code: "agent_changed".to_string(),
                message: "plan references a different agent than the current automation".to_string(),
            });
        }

        Ok(PermissionSummary {
            agent_id: spec.agent_id.clone(),
            allowed_paths: spec.allowed_paths.clone(),
            has_shell,
            shell_preview: shell,
            permission_expansion,
        })
    }

    /// Projects one run against the durable snapshot state and persists a newly
    /// provable terminal transition. Returns the (possibly updated) run row.
    fn project_run(&self, run: &WorkflowAutomationRun) -> Result<WorkflowAutomationRun, AutomationError> {
        if matches!(run.status.as_str(), "completed" | "failed" | "cancelled") {
            return Ok(run.clone());
        }
        let target = match run.workflow_session_id.as_deref() {
            Some(session_id) => {
                match self
                    .store()
                    .workflow_snapshot_state(session_id)
                    .map_err(|e| AutomationError::internal(e.to_string()))?
                {
                    Some(state) => terminal_from_snapshot_state(&state),
                    // No snapshot yet: if the workflow row also vanished the
                    // effect is unknown; otherwise it is still starting.
                    None => {
                        let exists = self
                            .store()
                            .workflow_session_exists(session_id)
                            .map_err(|e| AutomationError::internal(e.to_string()))?;
                        if exists {
                            None
                        } else {
                            Some("needs_reconcile")
                        }
                    }
                }
            }
            // A run with no session linked can never be proven to have started.
            None => Some("needs_reconcile"),
        };

        match target {
            Some(status) if status != run.status => {
                self.store()
                    .update_workflow_automation_run_status(run_id_str(run), status, None)
                    .map_err(|e| AutomationError::internal(e.to_string()))?;
                let mut updated = run.clone();
                updated.status = status.to_string();
                Ok(updated)
            }
            _ => Ok(run.clone()),
        }
    }

    /// Builds the public run view, joining the structured workflow status and
    /// wait reason. Error text is bounded/redacted (INV-8).
    pub fn run_view(
        &self,
        run: &WorkflowAutomationRun,
    ) -> Result<AutomationRunView, AutomationError> {
        let mut workflow_status = None;
        let mut wait_reason = None;
        if let Some(session_id) = run.workflow_session_id.as_deref() {
            if let Some(state) = self
                .store()
                .workflow_snapshot_state(session_id)
                .map_err(|e| AutomationError::internal(e.to_string()))?
            {
                workflow_status = Some(state.clone());
                wait_reason = (state == "waiting")
                    .then(|| {
                        self.store()
                            .get_workflow(session_id)
                            .ok()
                            .flatten()
                            .and_then(|wf| wf.wait_reason)
                    })
                    .flatten();
            }
        }
        Ok(AutomationRunView {
            run_id: run.id.clone(),
            automation_id: run.automation_id.clone(),
            trigger: run.trigger.clone(),
            dispatch_key: run.dispatch_key.clone(),
            status: run.status.clone(),
            workflow_session_id: run.workflow_session_id.clone(),
            scheduled_for: run.scheduled_for.clone(),
            started_at: run.started_at.clone(),
            finished_at: run.finished_at.clone(),
            error: run.error.as_deref().map(redact_error),
            workflow_status,
            wait_reason,
            created_at: run.created_at.clone(),
            updated_at: run.updated_at.clone(),
        })
    }
}

// --- WorkflowApplicationService automation entry points --------------------
//
// `run` and `dispatch_due` are the only operations that reach the shared
// workflow runtime, so they live on the runtime owner and call the `service.rs`
// kernel after the facade has durably created/claimed the run.

use crate::workflow::react::application::WorkflowApplicationService;

impl WorkflowApplicationService {
    /// A transient facade bound to this runtime owner's shared store.
    pub fn automation(&self) -> AutomationApplicationService {
        AutomationApplicationService::new(self.main_store.clone(), self.tsid_generator.clone())
    }

    /// Manual run through the canonical kernel. Busy-guarded; a successful start
    /// is `Accepted`, not a completion (AC-6/AC-8).
    pub async fn automation_run(
        &self,
        automation_id: &str,
    ) -> Result<AutomationDispatchResult, AutomationError> {
        // A created run — even one whose start failed synchronously — reports
        // `Accepted`: the effect was dispatched, and its lifecycle is observable
        // through the run view. A busy overlap is returned as an error instead.
        let (_automation, run) =
            crate::workflow::automation::service::create_manual_run(self, automation_id).await?;
        let view = self.automation().run_view(&run)?;
        Ok(AutomationDispatchResult {
            outcome: AutomationDispatchOutcome::Accepted,
            run: Some(view),
        })
    }

    /// Backwards-compatible manual run for the legacy Tauri `run_now` command.
    /// The mutation goes exclusively through the typed facade `automation_run`
    /// (AC-1/INV-2) — this method never calls the service kernel itself — and the
    /// historical camelCase `WorkflowAutomationRunNowResult` is then rebuilt from
    /// facade reads (INV-9). The single manual-run kernel stays inside
    /// `automation_run`.
    pub async fn automation_run_compat(
        &self,
        automation_id: String,
    ) -> Result<WorkflowAutomationRunNowResult, String> {
        let dispatch = self
            .automation_run(&automation_id)
            .await
            .map_err(|e| e.message().to_string())?;
        let view = dispatch
            .run
            .ok_or_else(|| "automation run produced no dispatch view".to_string())?;
        let automation = self
            .automation()
            .get_row(&automation_id)
            .map_err(|e| e.message().to_string())?
            .ok_or_else(|| format!("Automation {automation_id} not found"))?;
        let run = self
            .automation()
            .run_rows(&view.automation_id)
            .map_err(|e| e.message().to_string())?
            .into_iter()
            .find(|row| row.id == view.run_id)
            .ok_or_else(|| "created run disappeared".to_string())?;
        let workflow_session_id = run.workflow_session_id.clone().unwrap_or_default();
        Ok(WorkflowAutomationRunNowResult {
            automation,
            run,
            workflow_session_id,
        })
    }

    /// Claims each due slot atomically and executes only the runs this caller
    /// won, so a concurrent scheduler/manual actor never double-dispatches a slot
    /// (AC-6/INV-6). Returns one result per successfully claimed run.
    pub async fn automation_dispatch_due(
        &self,
        now: &str,
    ) -> Result<Vec<AutomationDispatchResult>, AutomationError> {
        let facade = self.automation();
        // Converge any stale runs first so a dispatch never overlaps an unknown
        // effect from a previous process (AC-8).
        facade.reconcile_all()?;
        let due = self
            .main_store
            .list_due_workflow_automations(now)
            .map_err(|e| AutomationError::internal(e.to_string()))?;
        let mut results = Vec::new();
        for automation in due {
            let Some(next_run_at) = self.compute_next_for(&automation)? else {
                // Non-recurring schedule with nothing left: disable-safe no-op.
                continue;
            };
            let dispatch_key = automation.next_run_at.clone().unwrap_or_else(|| now.to_string());
            let run_id = self
                .tsid_generator
                .generate()
                .map_err(|e| AutomationError::internal(e.to_string()))?;
            let scheduled_for = dispatch_key.clone();
            let claim = self
                .main_store
                .claim_due_automation_slot(
                    &automation.id,
                    automation.revision,
                    &dispatch_key,
                    Some(next_run_at),
                    &dispatch_key,
                    &run_id,
                    &scheduled_for,
                )
                .map_err(|e| AutomationError::internal(e.to_string()))?;
            let claimed_run = match claim {
                ClaimOutcome::Claimed(run) => run,
                ClaimOutcome::NotEligible
                | ClaimOutcome::SlotTaken
                | ClaimOutcome::ActiveRunExists => {
                    results.push(AutomationDispatchResult {
                        outcome: AutomationDispatchOutcome::Skipped,
                        run: None,
                    });
                    continue;
                }
            };
            // Execute the shared kernel for the run we just claimed. A start
            // failure is recorded inside the kernel; the slot stays claimed so it
            // is never retried blindly.
            let _ = crate::workflow::automation::service::execute_automation_run(
                self,
                &automation,
                &claimed_run.id,
                &scheduled_for,
            )
            .await;
            let reloaded = self
                .main_store
                .get_workflow_automation_run(&claimed_run.id)
                .map_err(|e| AutomationError::internal(e.to_string()))?
                .unwrap_or(claimed_run);
            let view = self.automation().run_view(&reloaded)?;
            results.push(AutomationDispatchResult {
                outcome: AutomationDispatchOutcome::Accepted,
                run: Some(view),
            });
        }
        Ok(results)
    }

    fn compute_next_for(
        &self,
        automation: &WorkflowAutomation,
    ) -> Result<Option<String>, AutomationError> {
        let schedule_config: Value = serde_json::from_str(&automation.schedule_config)
            .map_err(|e| AutomationError::internal(e.to_string()))?;
        crate::workflow::automation::service::compute_next_run_at(&automation.schedule_kind, &schedule_config)
            .map_err(AutomationError::invalid_request)
    }
}

// --- free helpers -----------------------------------------------------------

fn spec_to_request(spec: &AutomationSpec, id: Option<String>) -> WorkflowAutomationRequest {
    WorkflowAutomationRequest {
        id,
        title: spec.title.clone(),
        prompt: spec.prompt.clone(),
        prompt_file_path: spec.prompt_file_path.clone(),
        agent_id: spec.agent_id.clone(),
        agent_config: spec.agent_config.clone(),
        allowed_paths: spec.allowed_paths.clone(),
        shell_config: spec.shell_config.clone(),
        schedule_kind: spec.schedule_kind.clone(),
        schedule_config: spec.schedule_config.clone(),
        continuous_context: spec.continuous_context,
        self_review: spec.self_review,
        enabled: spec.enabled,
    }
}

/// Converts a legacy camelCase editor request into the canonical spec so the
/// compat save adapter can share the facade's single write path (INV-2).
fn request_to_spec(request: &WorkflowAutomationRequest) -> AutomationSpec {
    AutomationSpec {
        title: request.title.clone(),
        prompt: request.prompt.clone(),
        prompt_file_path: request.prompt_file_path.clone(),
        agent_id: request.agent_id.clone(),
        agent_config: request.agent_config.clone(),
        allowed_paths: request.allowed_paths.clone(),
        shell_config: request.shell_config.clone(),
        schedule_kind: request.schedule_kind.clone(),
        schedule_config: request.schedule_config.clone(),
        continuous_context: request.continuous_context,
        self_review: request.self_review,
        enabled: request.enabled,
    }
}

fn to_view(row: &WorkflowAutomation) -> AutomationView {
    let allowed_paths: Vec<String> = serde_json::from_str(&row.allowed_paths).unwrap_or_default();
    let schedule_config: Value =
        serde_json::from_str(&row.schedule_config).unwrap_or_else(|_| json!({}));
    let shell_config: Option<Value> = row
        .shell_config
        .as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok());
    AutomationView {
        automation_id: row.id.clone(),
        title: row.title.clone(),
        prompt: row.prompt.clone(),
        prompt_file_path: row.prompt_file_path.clone(),
        agent_id: row.agent_id.clone(),
        allowed_paths,
        shell_config,
        schedule_kind: row.schedule_kind.clone(),
        schedule_config,
        continuous_context: row.continuous_context,
        self_review: row.self_review,
        enabled: row.enabled,
        current_workflow_session_id: row.current_workflow_session_id.clone(),
        next_run_at: row.next_run_at.clone(),
        last_run_at: row.last_run_at.clone(),
        revision: row.revision,
        created_at: row.created_at.clone(),
        updated_at: row.updated_at.clone(),
    }
}

/// The canonical plan hash over the effect-bearing fields only.
fn plan_hash(
    automation_id: Option<&str>,
    base_revision: Option<i64>,
    changes: &AutomationSpec,
) -> String {
    let changes_value = serde_json::to_value(changes).unwrap_or(Value::Null);
    canonical_request_hash(&json!({
        "plan_version": AUTOMATION_PLAN_VERSION,
        "automation_id": automation_id,
        "base_revision": base_revision,
        "changes": changes_value,
    }))
}

fn mutation_result(
    outcome: AutomationMutationOutcome,
    row: &WorkflowAutomation,
) -> AutomationMutationResult {
    AutomationMutationResult {
        outcome,
        automation: Some(to_view(row)),
    }
}

fn view_ref(view: &AutomationView) -> Value {
    json!({ "automation_id": view.automation_id, "revision": view.revision })
}

fn terminal_from_snapshot_state(state: &str) -> Option<&'static str> {
    match state {
        "completed" => Some("completed"),
        "failed" => Some("failed"),
        "cancelled" => Some("cancelled"),
        _ => None,
    }
}

fn run_id_str(run: &WorkflowAutomationRun) -> &str {
    &run.id
}

fn parse_shell_preview(shell_config: Option<&Value>) -> Option<String> {
    let config = shell_config?;
    let command = config
        .get("command")
        .or_else(|| config.get("filePath"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let preview: String = command.chars().take(SHELL_PREVIEW_LIMIT).collect();
    Some(preview)
}

fn blocked_intent_warning() -> AutomationWarning {
    AutomationWarning {
        code: "requires_explicit_mutation".to_string(),
        message: "a natural-language intent cannot grant agent/path/shell/network/MCP permissions; provide a structured spec and explicit references".to_string(),
    }
}

/// Bounds a stored error string for public projection; the durable run error is
/// already generated from bounded sources, this keeps it short for the wire.
fn redact_error(error: &str) -> String {
    const LIMIT: usize = 500;
    let trimmed = error.trim();
    if trimmed.chars().count() <= LIMIT {
        return trimmed.to_string();
    }
    let mut bounded: String = trimmed.chars().take(LIMIT).collect();
    bounded.push_str("…[truncated]");
    bounded
}

fn map_store_write_error(error: crate::db::StoreError) -> AutomationError {
    // An agent foreign-key violation or an insert-on-existing id is a stable
    // invalid/conflict, not an opaque internal error.
    let text = error.to_string();
    if text.contains("FOREIGN KEY") || text.contains("constraint failed") {
        return AutomationError::invalid_request(format!("automation write rejected: {text}"));
    }
    AutomationError::internal(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::automation::WorkflowAutomationRunInsert;
    use crate::workflow::automation::errors::code;
    use crate::workflow::automation::types::{
        AUTOMATION_ACTOR_SCOPE_CONTROL_PLANE, AUTOMATION_ACTOR_SCOPE_DESKTOP,
    };
    use rusqlite::params;
    use tempfile::tempdir;

    fn seeded_store(agent_ids: &[&str]) -> (tempfile::TempDir, Arc<MainStore>) {
        let dir = tempdir().expect("temp dir");
        let store = Arc::new(MainStore::new(dir.path().join("auto.db")).expect("create store"));
        for id in agent_ids {
            let id = id.to_string();
            store
                .db_runtime()
                .expect("runtime")
                .write_blocking(move |conn| {
                    conn.execute(
                        "INSERT INTO agents (id, name, system_prompt, agent_type, max_contexts)
                         VALUES (?1, ?2, 'p', 'autonomous', 20)",
                        params![id, format!("Agent {id}")],
                    )?;
                    Ok(())
                })
                .expect("seed agent");
        }
        (dir, store)
    }

    fn facade(store: &Arc<MainStore>) -> AutomationApplicationService {
        let tsid = Arc::new(crate::libs::tsid::TsidGenerator::new(1).expect("tsid"));
        AutomationApplicationService::new(store.clone(), tsid)
    }

    fn spec() -> AutomationSpec {
        AutomationSpec {
            title: "Nightly".to_string(),
            prompt: Some("do work".to_string()),
            prompt_file_path: None,
            agent_id: "agent-main".to_string(),
            agent_config: None,
            allowed_paths: vec![],
            shell_config: None,
            schedule_kind: "interval".to_string(),
            schedule_config: json!({"interval_minutes": 60}),
            continuous_context: false,
            self_review: false,
            enabled: false,
        }
    }

    #[test]
    fn draft_structured_plan_has_no_side_effects_and_stable_hash() {
        let (_dir, store) = seeded_store(&["agent-main"]);
        let svc = facade(&store);
        let plan = svc
            .draft(AutomationDraftInput {
                automation_id: None,
                spec: Some(spec()),
                intent: None,
            })
            .expect("draft should succeed");

        // INV-4: no DB mutation, no run, no workflow created.
        assert!(store.list_workflow_automations().expect("list").is_empty());
        assert_eq!(plan.status, AutomationPlanStatus::Ready);
        assert!(!plan.plan_hash.is_empty());

        // AC-3: the same input yields the same canonical hash.
        let again = svc
            .draft(AutomationDraftInput {
                automation_id: None,
                spec: Some(spec()),
                intent: None,
            })
            .expect("re-draft");
        assert_eq!(plan.plan_hash, again.plan_hash);
    }

    #[test]
    fn draft_intent_is_blocked_and_never_grants_permissions() {
        let (_dir, store) = seeded_store(&["agent-main"]);
        let svc = facade(&store);
        let plan = svc
            .draft(AutomationDraftInput {
                automation_id: None,
                spec: None,
                intent: Some("give it network access and run rm -rf".to_string()),
            })
            .expect("intent draft should not error");
        // INV-3: the parser cannot mint permissions; the plan is not applyable.
        assert_eq!(plan.status, AutomationPlanStatus::Blocked);
        assert!(plan.plan_hash.is_empty());
        assert!(plan
            .warnings
            .iter()
            .any(|warning| warning.code == "requires_explicit_mutation"));
        assert!(store.list_workflow_automations().expect("list").is_empty());
    }

    #[test]
    fn create_then_update_separates_revision_conflict_and_unknown_id() {
        let (_dir, store) = seeded_store(&["agent-main"]);
        let svc = facade(&store);
        let created = svc
            .create(&spec(), AUTOMATION_ACTOR_SCOPE_DESKTOP, None)
            .expect("create");
        let id = created
            .automation
            .as_ref()
            .expect("created view")
            .automation_id
            .clone();
        assert_eq!(created.outcome, AutomationMutationOutcome::Applied);

        // AC-5: unknown id is not_found, never an implicit create.
        let unknown = svc.update(
            "does-not-exist",
            &spec(),
            1,
            AUTOMATION_ACTOR_SCOPE_DESKTOP,
            None,
        );
        assert_eq!(unknown.expect_err("unknown update").code(), code::NOT_FOUND);

        // Stale revision is a conflict, not a silent overwrite.
        let conflict = svc.update(&id, &spec(), 99, AUTOMATION_ACTOR_SCOPE_DESKTOP, None);
        assert_eq!(
            conflict.expect_err("stale update").code(),
            code::REVISION_CONFLICT
        );
        let ok = svc
            .update(&id, &spec(), 1, AUTOMATION_ACTOR_SCOPE_DESKTOP, None)
            .expect("valid update");
        assert_eq!(ok.automation.expect("view").revision, 2);
    }

    #[test]
    fn apply_enforces_hash_and_revision_guards() {
        let (_dir, store) = seeded_store(&["agent-main"]);
        let svc = facade(&store);
        let id = svc
            .create(&spec(), AUTOMATION_ACTOR_SCOPE_DESKTOP, None)
            .expect("create")
            .automation
            .expect("view")
            .automation_id
            .clone();
        let mut updated = spec();
        updated.title = "Renamed".to_string();
        let plan = svc
            .draft(AutomationDraftInput {
                automation_id: Some(id.clone()),
                spec: Some(updated),
                intent: None,
            })
            .expect("draft update plan");
        assert_eq!(plan.base_revision, Some(1));

        // Tampered authorized hash is refused (AC-4).
        let tampered = svc.apply(
            &AutomationApplyRequest {
                plan: plan.clone(),
                expected_plan_hash: "deadbeef".to_string(),
                acknowledge_permission_changes: true,
            },
            AUTOMATION_ACTOR_SCOPE_CONTROL_PLANE,
            None,
        );
        assert_eq!(tampered.expect_err("tamper").code(), code::CONFLICT);

        // A clean apply succeeds and advances the revision.
        let applied = svc
            .apply(
                &AutomationApplyRequest {
                    plan: plan.clone(),
                    expected_plan_hash: plan.plan_hash.clone(),
                    acknowledge_permission_changes: true,
                },
                AUTOMATION_ACTOR_SCOPE_CONTROL_PLANE,
                None,
            )
            .expect("apply");
        assert_eq!(applied.automation.expect("view").revision, 2);

        // Re-applying the same stale plan is now a revision conflict.
        let stale = svc.apply(
            &AutomationApplyRequest {
                plan: plan.clone(),
                expected_plan_hash: plan.plan_hash.clone(),
                acknowledge_permission_changes: true,
            },
            AUTOMATION_ACTOR_SCOPE_CONTROL_PLANE,
            None,
        );
        assert_eq!(stale.expect_err("stale").code(), code::REVISION_CONFLICT);
    }

    #[test]
    fn apply_permission_expansion_requires_acknowledgement() {
        let (_dir, store) = seeded_store(&["agent-main", "agent-two"]);
        let svc = facade(&store);
        let id = svc
            .create(&spec(), AUTOMATION_ACTOR_SCOPE_DESKTOP, None)
            .expect("create")
            .automation
            .expect("view")
            .automation_id
            .clone();
        let mut expanded = spec();
        expanded.agent_id = "agent-two".to_string();
        let plan = svc
            .draft(AutomationDraftInput {
                automation_id: Some(id.clone()),
                spec: Some(expanded),
                intent: None,
            })
            .expect("draft");
        assert!(plan.permission_summary.permission_expansion);

        let refused = svc.apply(
            &AutomationApplyRequest {
                plan: plan.clone(),
                expected_plan_hash: plan.plan_hash.clone(),
                acknowledge_permission_changes: false,
            },
            AUTOMATION_ACTOR_SCOPE_CONTROL_PLANE,
            None,
        );
        assert_eq!(
            refused.expect_err("expansion").code(),
            code::PERMISSION_EXPANSION
        );
    }

    #[test]
    fn delete_requires_confirmation_and_refuses_active_run() {
        let (_dir, store) = seeded_store(&["agent-main"]);
        let svc = facade(&store);
        let id = svc
            .create(&spec(), AUTOMATION_ACTOR_SCOPE_DESKTOP, None)
            .expect("create")
            .automation
            .expect("view")
            .automation_id
            .clone();

        let unconfirmed = svc.delete(&id, false, AUTOMATION_ACTOR_SCOPE_CONTROL_PLANE, None);
        assert_eq!(
            unconfirmed.expect_err("confirm").code(),
            code::CONFIRMATION_REQUIRED
        );

        store
            .add_workflow_automation_run(&WorkflowAutomationRunInsert {
                id: "run-active".to_string(),
                automation_id: id.clone(),
                workflow_session_id: None,
                status: "running".to_string(),
                scheduled_for: "2026-06-25 09:00:00".to_string(),
                started_at: None,
                finished_at: None,
                error: None,
                trigger: "manual".to_string(),
                dispatch_key: None,
            })
            .expect("insert active run");
        let busy = svc.delete(&id, true, AUTOMATION_ACTOR_SCOPE_CONTROL_PLANE, None);
        assert_eq!(busy.expect_err("busy").code(), code::BUSY);
    }

    #[test]
    fn idempotency_key_replays_and_conflicts() {
        let (_dir, store) = seeded_store(&["agent-main"]);
        let svc = facade(&store);
        let first = svc
            .create(&spec(), AUTOMATION_ACTOR_SCOPE_CONTROL_PLANE, Some("k1"))
            .expect("first create");
        assert_eq!(first.outcome, AutomationMutationOutcome::Applied);

        let replay = svc
            .create(&spec(), AUTOMATION_ACTOR_SCOPE_CONTROL_PLANE, Some("k1"))
            .expect("replay");
        assert_eq!(replay.outcome, AutomationMutationOutcome::Replayed);
        // No duplicate effect (AC-5).
        assert_eq!(store.list_workflow_automations().expect("list").len(), 1);

        let mut different = spec();
        different.title = "Other".to_string();
        let conflict = svc.create(
            &different,
            AUTOMATION_ACTOR_SCOPE_CONTROL_PLANE,
            Some("k1"),
        );
        assert_eq!(conflict.expect_err("conflict").code(), code::CONFLICT);
    }

    #[test]
    fn runs_project_structured_terminal_without_transcript() {
        let (_dir, store) = seeded_store(&["agent-main"]);
        let svc = facade(&store);
        let id = svc
            .create(&spec(), AUTOMATION_ACTOR_SCOPE_DESKTOP, None)
            .expect("create")
            .automation
            .expect("view")
            .automation_id
            .clone();

        // A run stuck pending with no linked session is unknown, never completed.
        store
            .add_workflow_automation_run(&WorkflowAutomationRunInsert {
                id: "run-orphan".to_string(),
                automation_id: id.clone(),
                workflow_session_id: None,
                status: "pending".to_string(),
                scheduled_for: "2026-06-25 09:00:00".to_string(),
                started_at: None,
                finished_at: None,
                error: None,
                trigger: "manual".to_string(),
                dispatch_key: None,
            })
            .expect("insert orphan");
        let orphan = svc.runs(&id).expect("runs")[0].clone();
        assert_eq!(orphan.status, "needs_reconcile");

        // A running run whose durable snapshot says completed projects completed.
        store
            .create_workflow("sess-1", "t", "agent-main", None, None)
            .expect("workflow");
        store
            .db_runtime()
            .expect("runtime")
            .write_blocking(|conn| {
                conn.execute(
                    "INSERT INTO workflow_snapshots (session_id, context_json, version, state)
                     VALUES ('sess-1', '{}', '1', 'completed')",
                    [],
                )?;
                Ok(())
            })
            .expect("seed completed snapshot");
        store
            .add_workflow_automation_run(&WorkflowAutomationRunInsert {
                id: "run-done".to_string(),
                automation_id: id.clone(),
                workflow_session_id: Some("sess-1".to_string()),
                status: "running".to_string(),
                scheduled_for: "2026-06-25 10:00:00".to_string(),
                started_at: None,
                finished_at: None,
                error: None,
                trigger: "manual".to_string(),
                dispatch_key: None,
            })
            .expect("insert running");
        let done = svc.runs(&id).expect("runs").into_iter().find(|run| run.run_id == "run-done").expect("find run");
        assert_eq!(done.status, "completed");
        assert_eq!(done.workflow_status.as_deref(), Some("completed"));
    }

    /// Source wiring guard (AC-1/INV-2): the legacy Tauri `run_now` command and
    /// the compat facade must reach the manual-run kernel only through the typed
    /// `automation_run` facade, never by calling the `service` kernel directly.
    /// This locks the re-review fix so no second public mutation path can return.
    #[test]
    fn legacy_run_now_routes_through_typed_facade() {
        let command_src = include_str!("../../commands/workflow_automation.rs");
        assert!(
            command_src.contains("svc.automation_run_compat"),
            "run_now command must delegate to the compat facade"
        );
        assert!(
            !command_src.contains("run_automation_now") && !command_src.contains("create_manual_run"),
            "run_now command must not reach the raw kernel helper"
        );

        let app_src = include_str!("./application.rs");
        let compat_body = app_src
            .split("pub async fn automation_run_compat(")
            .nth(1)
            .and_then(|rest| rest.split("pub async fn automation_dispatch_due(").next())
            .expect("automation_run_compat body");
        assert!(
            compat_body.contains(".automation_run(&automation_id)"),
            "compat must call the typed automation_run facade"
        );
        assert!(
            !compat_body.contains("create_manual_run"),
            "compat must not call the service kernel directly"
        );

        let run_body = app_src
            .split("pub async fn automation_run(")
            .nth(1)
            .and_then(|rest| rest.split("/// Backwards-compatible manual run").next())
            .expect("automation_run body");
        assert!(
            run_body.contains("create_manual_run"),
            "automation_run is the sole owner of the manual-run kernel"
        );
    }
}
