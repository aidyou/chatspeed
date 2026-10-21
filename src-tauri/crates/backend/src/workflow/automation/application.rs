//! The single transport-neutral automation facade (`AutomationApplicationService`).
//!
//! Every automation mutation and observation — Tauri, the control-plane HTTP
//! surface, the `cs` CLI (through HTTP), the desktop store and the scheduler —
//! resolves to these methods, so there is exactly one automation state machine
//! and one DTO/error contract (AC-1/INV-2). The service holds no automation
//! state of its own: it reads the shared `MainStore` and the same workflow
//! snapshot the runtime writes, plus this instance's short-lived execution
//! evidence, so it never becomes a second runtime or journal authority
//! (INV-1/INV-10).
//!
//! The execution kernel (`execute_automation_run`) and manual-run helper
//! (`create_manual_run`) live in `service.rs` and are reached from
//! `WorkflowApplicationService`; this module owns plan/validation/revision/CAS/
//! receipt/lifecycle-projection semantics and the canonical snake_case views.

use crate::capability::operation::{canonical_request_hash, now_ms};
use crate::db::automation::{CasOutcome, ClaimOutcome, DeleteAutomationOutcome, ReceiptOutcome};
use crate::db::{MainStore, WorkflowAutomation, WorkflowAutomationRun, WorkflowAutomationUpsert};
use crate::libs::tsid::TsidGenerator;
use crate::workflow::automation::errors::{self, AutomationError};
use crate::workflow::automation::runtime::{
    manager_terminal_status, session_status_is_live, AutomationRuntimeOwner,
};
use crate::workflow::automation::service::request_to_upsert;
use crate::workflow::automation::types::{
    AutomationApplyRequest, AutomationDispatchOutcome, AutomationDispatchResult,
    AutomationDraftInput, AutomationMutationOutcome, AutomationMutationResult,
    AutomationPlanStatus, AutomationPlanV1, AutomationRunView, AutomationRuntimeHealth,
    AutomationRuntimeSummary, AutomationSpec, AutomationStartupNormalizeReport, AutomationView,
    AutomationWarning, PermissionSummary, WorkflowAutomationRequest,
    WorkflowAutomationRunNowResult, AUTOMATION_PLAN_VERSION,
};
use crate::workflow::react::manager::ManagedSessionStatus;
use chrono::{DateTime, Local};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;

/// A draft plan stays applyable for this long. A stale plan must be re-drafted
/// so a user never applies against a surprise-moved revision (AC-4).
const PLAN_TTL_MS: i64 = 15 * 60 * 1000;

/// Bounded shell-command preview length for the permission summary. The preview
/// is a user-authored configuration value, not runtime output (INV-8).
const SHELL_PREVIEW_LIMIT: usize = 200;

/// The automation facade. Cheap to construct per call (it clones a few `Arc`
/// handles) and stateless over the shared store; the only non-durable state it
/// touches is this instance's short-lived execution evidence, so two concurrent
/// reads can never disagree about the durable projection.
pub struct AutomationApplicationService {
    main_store: Arc<MainStore>,
    tsid_generator: Arc<TsidGenerator>,
    /// The runtime instance's short-lived execution evidence. `None` for a
    /// standalone facade (a one-off read, a test): it can neither confirm a live
    /// session nor an in-flight admission, so an untrusted run then converges to
    /// `needs_reconcile` instead of being read as live.
    owner: Option<Arc<AutomationRuntimeOwner>>,
    /// The one runtime work-admission gate, when this facade is bound to a
    /// running instance. `None` for a standalone facade (a one-off read/test),
    /// which then admits nothing and mutates as before.
    admission: Option<Arc<dyn crate::lifecycle::WorkAdmission>>,
}

impl AutomationApplicationService {
    pub fn new(main_store: Arc<MainStore>, tsid_generator: Arc<TsidGenerator>) -> Self {
        Self {
            main_store,
            tsid_generator,
            owner: None,
            admission: None,
        }
    }

    /// A facade bound to the running instance's execution evidence, so reads can
    /// verify the current in-flight guard and the live workflow manager.
    pub(crate) fn with_owner(
        main_store: Arc<MainStore>,
        tsid_generator: Arc<TsidGenerator>,
        owner: Arc<AutomationRuntimeOwner>,
    ) -> Self {
        Self {
            main_store,
            tsid_generator,
            owner: Some(owner),
            admission: None,
        }
    }

    /// Binds the one runtime admission gate so automation mutations are fenced
    /// at this lowest layer, not only by their transport.
    pub(crate) fn with_admission(
        mut self,
        admission: Option<Arc<dyn crate::lifecycle::WorkAdmission>>,
    ) -> Self {
        self.admission = admission;
        self
    }

    /// Fences one automation work start/mutation, mapping a closed gate onto the
    /// stable `runtime_stopping` automation error.
    fn admit(
        &self,
        kind: crate::lifecycle::WorkKind,
    ) -> Result<crate::lifecycle::AdmissionTicket, AutomationError> {
        match &self.admission {
            Some(gate) => gate
                .admit(kind)
                .map_err(|denied| AutomationError::stopping(denied.message)),
            None => Ok(crate::lifecycle::AdmissionTicket::permissive()),
        }
    }

    fn store(&self) -> &MainStore {
        &self.main_store
    }

    /// Whether this instance is executing `run_id` in its pre-session window.
    fn is_in_flight(&self, run_id: &str) -> bool {
        self.owner
            .as_ref()
            .is_some_and(|owner| owner.is_in_flight(run_id))
    }

    /// The live manager status for a session, or `None` when no owner is wired.
    /// A missing/unavailable manager propagates as an error so a caller never
    /// mistakes "cannot observe" for "not live".
    fn owner_session_status(
        &self,
        session_id: &str,
    ) -> Result<Option<ManagedSessionStatus>, AutomationError> {
        match self.owner.as_ref() {
            Some(owner) => owner
                .manager_status(session_id)
                .map_err(AutomationError::internal),
            None => Ok(None),
        }
    }

    // --- observations -----------------------------------------------------

    pub fn list(&self) -> Result<Vec<AutomationView>, AutomationError> {
        let rows = self
            .store()
            .list_workflow_automations()
            .map_err(|e| AutomationError::internal_from(e))?;
        Ok(rows.iter().map(to_view).collect())
    }

    pub fn get(&self, automation_id: &str) -> Result<Option<AutomationView>, AutomationError> {
        let row = self
            .store()
            .get_workflow_automation(automation_id)
            .map_err(|e| AutomationError::internal_from(e))?;
        Ok(row.as_ref().map(to_view))
    }

    pub(crate) fn get_row(
        &self,
        automation_id: &str,
    ) -> Result<Option<WorkflowAutomation>, AutomationError> {
        self.store()
            .get_workflow_automation(automation_id)
            .map_err(|e| AutomationError::internal_from(e))
    }

    /// Legacy camelCase run rows for the existing `workflow_automation_list_runs`
    /// wire. The projected, snake_case lifecycle is exposed via `runs()`.
    pub(crate) fn run_rows(
        &self,
        automation_id: &str,
    ) -> Result<Vec<WorkflowAutomationRun>, AutomationError> {
        self.store()
            .list_workflow_automation_runs(automation_id)
            .map_err(|e| AutomationError::internal_from(e))
    }

    /// Lists an automation's runs with a bounded, structured terminal projection
    /// from the workflow snapshot (INV-7). Runs that cannot be proven terminal
    /// are left for `reconcile_all`/re-dispatch and surface as `needs_reconcile`.
    pub fn runs(&self, automation_id: &str) -> Result<Vec<AutomationRunView>, AutomationError> {
        let automation = self
            .store()
            .get_workflow_automation(automation_id)
            .map_err(|e| AutomationError::internal_from(e))?
            .ok_or_else(|| AutomationError::not_found(format!("Automation {automation_id}")))?;
        let runs = self
            .store()
            .list_workflow_automation_runs(&automation.id)
            .map_err(|e| AutomationError::internal_from(e))?;
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
            .map_err(|e| AutomationError::internal_from(e))?;
        for run in &runs {
            let _ = self.project_run(run)?;
        }
        Ok(())
    }

    // --- startup normalization / runtime health ---------------------------

    /// The one-time startup normalization pass. It advances an enabled
    /// automation only when its stored slot was missed while this instance was
    /// offline (i.e. the stored slot is present and at or before `cutoff`) and
    /// clears a non-recurring slot that has already elapsed. A stored future slot
    /// and a stored `NULL` slot are both left untouched: `NULL` means the
    /// configuration currently defines no next slot, so filling it here would
    /// reactivate a schedule the user no longer has (AC-3). `enabled` is never
    /// changed, and every write is a revision CAS, so a concurrent edit is
    /// reported rather than overwritten (AC-4).
    ///
    /// The runtime daemon calls this before its first dispatch tick. A repeated
    /// call after a *successful* pass is a no-op; a pass that failed before
    /// completing is retried with the same cutoff instead of being treated as
    /// done, so the daemon never continues into the scheduler on the strength of
    /// a normalization that never ran.
    pub fn normalize_schedules_at_startup(
        &self,
        cutoff: &str,
    ) -> Result<AutomationStartupNormalizeReport, AutomationError> {
        let cutoff_dt = parse_local_datetime(cutoff)?;
        if let Some(owner) = self.owner.as_ref() {
            if owner.startup_pass_completed() {
                // Startup normalization is a one-time pass; a duplicate startup
                // signal must not re-advance an already-normalized schedule.
                return Ok(AutomationStartupNormalizeReport::default());
            }
            // Record the boundary before the pass reads the store, so a due slot
            // cannot be consumed later as if it became due while online. A failed
            // pass leaves the boundary in place, so a retry reuses this cutoff
            // rather than advancing to a later one.
            owner.mark_startup(cutoff_dt);
        }

        let automations = self
            .store()
            .list_workflow_automations()
            .map_err(|e| AutomationError::internal_from(e))?;
        let mut report = AutomationStartupNormalizeReport::default();
        for automation in &automations {
            if !automation.enabled {
                continue;
            }
            report.examined += 1;
            // Only an existing cursor is advanced. A `NULL` cursor is a
            // legitimate "no next slot" state (an expired or one-shot
            // configuration), so it is neither filled nor kept alive (AC-3).
            let Some(stored) = automation.next_run_at.clone() else {
                report.unchanged += 1;
                continue;
            };
            // A cursor that cannot be read back is never rewritten: the cursor
            // classifier reports it as non-schedulable, which is the safe
            // direction for a value this pass cannot interpret.
            let Ok(stored_dt) = parse_local_datetime(&stored) else {
                report.unchanged += 1;
                continue;
            };
            // A stored slot strictly after the cutoff is a legitimate pending
            // schedule and stays untouched.
            if stored_dt > cutoff_dt {
                report.unchanged += 1;
                continue;
            }
            let recomputed = match self.successor_for(automation, cutoff_dt) {
                Ok(next) => next,
                Err(_) => {
                    // A row whose schedule cannot be recomputed is left for a
                    // later edit rather than failing the whole startup pass.
                    report.failed += 1;
                    continue;
                }
            };
            if recomputed.as_deref() == Some(stored.as_str()) {
                report.unchanged += 1;
                continue;
            }
            match self
                .store()
                .normalize_workflow_automation_next_run_at_cas(
                    &automation.id,
                    recomputed.clone(),
                    automation.revision,
                )
                .map_err(|e| AutomationError::internal_from(e))?
            {
                CasOutcome::Updated(_) => {
                    if recomputed.is_some() {
                        report.advanced += 1;
                    } else {
                        report.expired += 1;
                    }
                }
                CasOutcome::RevisionConflict => report.conflicts += 1,
                CasOutcome::NotFound => {}
            }
        }
        if let Some(owner) = self.owner.as_ref() {
            // Only a pass that reached the end without a store error is done; a
            // failed pass stays retryable so the daemon cannot treat it as
            // normalized and continue into the scheduler.
            owner.complete_startup_pass();
        }
        Ok(report)
    }

    /// Runtime keep-alive and health summary. Only a legal future schedule, a
    /// continuously-online due slot, a live manager-owned run, or an in-flight
    /// admission keeps the runtime alive; a disabled or slot-less automation, an
    /// expired configuration, a historical terminal run, and an orphaned run
    /// this instance does not own never do (AC-3/AC-4/INV-1).
    ///
    /// An unobservable live manager is a hard observation failure and is
    /// returned as an error. Every other failure — an unreadable store or a
    /// structurally invalid user schedule — is reported as `unknown` health
    /// instead of an error, so a broken configuration can never make the
    /// daemon's decision sticky.
    pub fn runtime_summary(&self, now: &str) -> Result<AutomationRuntimeSummary, AutomationError> {
        let now_dt = parse_local_datetime(now)?;
        // Observe the live manager first: this is the one failure the summary
        // must surface rather than classify away.
        self.probe_active_run_managers()?;
        Ok(self
            .classify_runtime_summary(now_dt)
            .unwrap_or_else(|_| unknown_runtime_summary()))
    }

    /// Probes the live manager for every non-terminal run that has a session, so
    /// an unobservable manager fails the summary instead of being silently
    /// classified as "no live runs".
    fn probe_active_run_managers(&self) -> Result<(), AutomationError> {
        if self.owner.is_none() {
            return Ok(());
        }
        let Ok(runs) = self.store().list_active_automation_runs() else {
            // The store is unreadable; the classification step below reports
            // `unknown` health rather than pretending the manager was observed.
            return Ok(());
        };
        for run in &runs {
            if let Some(session_id) = run.workflow_session_id.as_deref() {
                let _ = self.owner_session_status(session_id)?;
            }
        }
        Ok(())
    }

    /// Classifies the durable state into a keep-alive/health report. Runs on
    /// structured evidence only; see [`Self::runtime_summary`] for how failures
    /// are surfaced.
    fn classify_runtime_summary(
        &self,
        now: DateTime<Local>,
    ) -> Result<AutomationRuntimeSummary, AutomationError> {
        // Converge not-yet-terminal runs on structured evidence first, so a run
        // whose manager/snapshot already reached a terminal state stops blocking
        // its automation's next enabled slot. Without this, a just-completed run
        // would keep the future schedule suppressed and the daemon would idle out
        // with work still pending.
        self.reconcile_all()?;

        // Only enabled candidates are read, and the not-yet-terminal runs are
        // read once: the blocking set is derived from that single result instead
        // of a per-automation blocking query (which reintroduced an N+1).
        let automations = self
            .store()
            .list_enabled_workflow_automations()
            .map_err(|e| AutomationError::internal_from(e))?;
        let active_runs = self
            .store()
            .list_active_automation_runs()
            .map_err(|e| AutomationError::internal_from(e))?;

        let mut blocking_automations: HashSet<&str> = HashSet::new();
        let mut live_automation_runs = 0i64;
        let mut blocked_reconcile = 0i64;
        for run in &active_runs {
            let live = match run.workflow_session_id.as_deref() {
                Some(session_id) => self
                    .owner_session_status(session_id)?
                    .is_some_and(|status| session_status_is_live(&status)),
                None => false,
            };
            if live {
                live_automation_runs += 1;
                continue;
            }
            // Not live: it blocks its schedule from dispatching, and unless this
            // instance admitted it in flight it is unknown work that never keeps
            // the runtime alive on its own.
            blocking_automations.insert(run.automation_id.as_str());
            if !self.is_in_flight(&run.id) {
                blocked_reconcile += 1;
            }
        }

        let mut effective_schedules = 0i64;
        let mut unusable_schedule = false;
        for automation in &automations {
            if blocking_automations.contains(automation.id.as_str()) {
                continue;
            }
            match self.schedule_cursor_state(automation, now) {
                Ok(ScheduleCursor::Future | ScheduleCursor::DueOnline) => effective_schedules += 1,
                Ok(ScheduleCursor::NotEffective) => {}
                Err(_) => {
                    // A structurally invalid *schedule configuration* is not a
                    // query error: it counts as non-schedulable and downgrades
                    // the health diagnostic, but must never fail the whole
                    // summary (or the daemon could never leave `unknown`).
                    unusable_schedule = true;
                }
            }
        }

        let in_flight_admissions = self
            .owner
            .as_ref()
            .map(|owner| owner.in_flight_count() as i64)
            .unwrap_or(0);
        let health = if self.owner.is_none() {
            // Without a runtime owner there is no live state to observe at all.
            AutomationRuntimeHealth::Unknown
        } else if blocked_reconcile > 0 || unusable_schedule {
            AutomationRuntimeHealth::Degraded
        } else {
            AutomationRuntimeHealth::Healthy
        };
        // An unobservable state is never reported as "nothing to do": a runtime
        // that cannot see its automation state must not shut down as if idle.
        let keep_alive = matches!(health, AutomationRuntimeHealth::Unknown)
            || effective_schedules > 0
            || live_automation_runs > 0
            || in_flight_admissions > 0;
        Ok(AutomationRuntimeSummary {
            effective_schedules,
            live_automation_runs,
            in_flight_admissions,
            blocked_reconcile,
            keep_alive,
            health,
        })
    }

    /// Classifies the stored schedule cursor against the configuration and this
    /// instance's start boundary. Shared by the dispatch eligibility check and
    /// the runtime summary so the two can never disagree on what "effective"
    /// means.
    fn schedule_cursor_state(
        &self,
        automation: &WorkflowAutomation,
        now: DateTime<Local>,
    ) -> Result<ScheduleCursor, AutomationError> {
        let Some(cursor) = automation.next_run_at.as_deref() else {
            return Ok(ScheduleCursor::NotEffective);
        };
        let Ok(cursor_dt) = parse_local_datetime(cursor) else {
            return Ok(ScheduleCursor::NotEffective);
        };
        let valid = crate::workflow::automation::service::is_valid_schedule_slot(
            &automation.schedule_kind,
            &self.schedule_config_value(automation)?,
            cursor_dt,
        )
        .map_err(AutomationError::invalid_request)?;
        if !valid {
            return Ok(ScheduleCursor::NotEffective);
        }
        if cursor_dt > now {
            return Ok(ScheduleCursor::Future);
        }
        match self.owner.as_ref().and_then(|owner| owner.startup_cutoff()) {
            // A slot before the instance start boundary was missed while offline
            // and must neither be consumed nor keep the runtime alive.
            Some(cutoff) if cursor_dt < cutoff => Ok(ScheduleCursor::NotEffective),
            _ => Ok(ScheduleCursor::DueOnline),
        }
    }

    /// The next occurrence strictly after `now` for an automation's schedule.
    fn successor_for(
        &self,
        automation: &WorkflowAutomation,
        now: DateTime<Local>,
    ) -> Result<Option<String>, AutomationError> {
        crate::workflow::automation::service::compute_next_run_at_from(
            &automation.schedule_kind,
            &self.schedule_config_value(automation)?,
            now,
        )
        .map_err(AutomationError::invalid_request)
    }

    fn schedule_config_value(
        &self,
        automation: &WorkflowAutomation,
    ) -> Result<Value, AutomationError> {
        serde_json::from_str(&automation.schedule_config)
            .map_err(|e| AutomationError::internal_from(e))
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
            .map_err(|e| AutomationError::internal_from(e))?
            .flatten();

        let permission_summary =
            self.permission_summary(&spec, existing.as_ref(), &mut warnings)?;
        // Validate the same way the write path will, but persist nothing.
        self.validate_spec(&spec)?;
        if let Some(existing) = &existing {
            // Reusing the persisted session id requires continuous context to
            // stay coherent; a plan that would drop it is flagged for review.
            if existing.continuous_context && !spec.continuous_context {
                warnings.push(AutomationWarning {
                    code: "continuous_context_disabled".to_string(),
                    message:
                        "plan disables continuous context; the current session will not be reused"
                            .to_string(),
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
        // Applying a plan creates/updates an automation that may start scheduled
        // work, so it is fenced at this lowest layer.
        let _ticket = self.admit(crate::lifecycle::WorkKind::AutomationConfigure)?;
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
            return Err(AutomationError::plan_expired(
                "plan has expired, re-draft before applying",
            ));
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

        if plan.permission_summary.permission_expansion && !request.acknowledge_permission_changes {
            return Err(AutomationError::permission_expansion(
                "plan expands permissions; acknowledge before applying",
            ));
        }
        let request_hash = canonical_request_hash(&json!({
            "operation": "apply",
            "plan_hash": derived,
            "automation_id": plan.automation_id,
        }));
        let result = if let Some(automation_id) = plan.automation_id.clone() {
            let existing = self
                .store()
                .get_workflow_automation(&automation_id)
                .map_err(|e| AutomationError::internal_from(e))?
                .ok_or_else(|| AutomationError::not_found(format!("Automation {automation_id}")))?;
            if existing.revision != plan.base_revision.unwrap_or(i64::MIN) {
                return Err(AutomationError::revision_conflict(
                    "the automation changed since this plan was drafted; re-draft",
                ));
            }
            self.apply_mutation_with_receipt(
                actor_scope,
                idempotency_key,
                "apply",
                &request_hash,
                |svc| svc.update_existing(&automation_id, &plan.changes, existing.revision),
            )?
        } else {
            self.apply_mutation_with_receipt(
                actor_scope,
                idempotency_key,
                "apply",
                &request_hash,
                |svc| svc.insert_new(&plan.changes),
            )?
        };
        Ok(result)
    }

    // --- explicit create / update / enable / disable / delete ------------

    pub fn create(
        &self,
        spec: &AutomationSpec,
        actor_scope: &str,
        idempotency_key: Option<&str>,
    ) -> Result<AutomationMutationResult, AutomationError> {
        self.validate_spec(spec)?;
        // Creating an automation may start scheduled work, so it is fenced at
        // this lowest layer; a direct, non-HTTP caller is refused on the gate.
        let _ticket = self.admit(crate::lifecycle::WorkKind::AutomationConfigure)?;
        let request_hash = canonical_request_hash(&json!({
            "operation": "create",
            "spec": serde_json::to_value(spec).map_err(|e| AutomationError::internal_from(e))?,
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
        let _ticket = self.admit(crate::lifecycle::WorkKind::AutomationConfigure)?;
        let request_hash = canonical_request_hash(&json!({
            "operation": "update",
            "automation_id": automation_id,
            "expected_revision": expected_revision,
            "spec": serde_json::to_value(spec).map_err(|e| AutomationError::internal_from(e))?,
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
        let _ticket = self.admit(crate::lifecycle::WorkKind::AutomationConfigure)?;
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
        F: FnOnce(
            &AutomationApplicationService,
        ) -> Result<AutomationMutationResult, AutomationError>,
    {
        let Some(key) = idempotency_key.map(str::trim).filter(|key| !key.is_empty()) else {
            return execute(self);
        };

        match self
            .store()
            .reserve_automation_mutation(actor_scope, key, operation, request_hash)
            .map_err(|e| AutomationError::internal_from(e))?
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
                    let _ = self.store().complete_automation_mutation(
                        actor_scope,
                        key,
                        redacted.as_deref(),
                    );
                }
                Ok(result)
            }
        }
    }

    fn insert_new(
        &self,
        spec: &AutomationSpec,
    ) -> Result<AutomationMutationResult, AutomationError> {
        let id = self
            .tsid_generator
            .generate()
            .map_err(|e| AutomationError::internal(e))?;
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
        Ok(mutation_result(
            AutomationMutationOutcome::Applied,
            &created,
        ))
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
            .map_err(|e| AutomationError::internal_from(e))?
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
            CasOutcome::NotFound => Err(AutomationError::not_found(format!(
                "Automation {automation_id}"
            ))),
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
            .map_err(|e| AutomationError::internal_from(e))?
            .ok_or_else(|| AutomationError::not_found(format!("Automation {automation_id}")))?;
        let expected = expected_revision.unwrap_or(existing.revision);
        let next_run_at = if enabled {
            let schedule_config: Value = serde_json::from_str(&existing.schedule_config)
                .map_err(|e| AutomationError::internal_from(e))?;
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
            CasOutcome::Updated(row) => {
                Ok(mutation_result(AutomationMutationOutcome::Enabled, &row))
            }
            CasOutcome::NotFound => Err(AutomationError::not_found(format!(
                "Automation {automation_id}"
            ))),
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
            .map_err(|e| AutomationError::internal_from(e))?
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
        match self
            .store()
            .delete_workflow_automation_if_idle(automation_id)
            .map_err(|e| AutomationError::internal_from(e))?
        {
            DeleteAutomationOutcome::Deleted => Ok(AutomationMutationResult {
                outcome: AutomationMutationOutcome::Deleted,
                automation: None,
            }),
            DeleteAutomationOutcome::NotFound => Err(AutomationError::not_found(format!(
                "Automation {automation_id}"
            ))),
            DeleteAutomationOutcome::ActiveRun => Err(AutomationError::busy(
                "automation has an active or unreconciled run; it cannot be deleted",
            )),
        }
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
                    .map_err(|e| AutomationError::internal_from(e))?
                    .ok_or_else(|| AutomationError::not_found(format!("Automation {id}")))?;
                Ok(Some(existing.revision))
            }
        }
    }

    fn validate_spec(&self, spec: &AutomationSpec) -> Result<(), AutomationError> {
        if spec.title.trim().is_empty() {
            return Err(AutomationError::invalid_request(
                "automation title is required",
            ));
        }
        let request = spec_to_request(spec, None);
        crate::workflow::automation::service::validate_automation_request(&request).map_err(
            |error| AutomationError::invalid_request(format!("invalid automation spec: {error}")),
        )
    }

    fn build_upsert(
        &self,
        spec: &AutomationSpec,
        id: String,
        existing_session: Option<String>,
    ) -> Result<WorkflowAutomationUpsert, AutomationError> {
        let request = spec_to_request(spec, Some(id.clone()));
        request_to_upsert(request, id, existing_session).map_err(|e| {
            AutomationError::new(
                errors::code::INVALID_REQUEST,
                format!("invalid automation spec: {e}"),
            )
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
                message: "plan references a different agent than the current automation"
                    .to_string(),
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

    /// Projects one run against the live manager and the durable snapshot state
    /// and persists a newly provable terminal transition. Returns the (possibly
    /// updated) run row.
    ///
    /// Evidence order: a live manager session outranks any historical snapshot,
    /// because a hot-recovered session can still be active while the snapshot
    /// still holds a terminal state from the previous run of that session. Only
    /// when the manager does not prove the session live is the canonical snapshot
    /// terminal used, and anything that is neither live nor provably terminal is
    /// left as `needs_reconcile` (never inferred from a workflow row or the run's
    /// own status) so the unknown effect never dispatches.
    fn project_run(
        &self,
        run: &WorkflowAutomationRun,
    ) -> Result<WorkflowAutomationRun, AutomationError> {
        if matches!(run.status.as_str(), "completed" | "failed" | "cancelled") {
            return Ok(run.clone());
        }
        // This instance is executing the run right now (durable admission through
        // the pre-workflow shell to the session link), so the effect is ours and
        // not yet judgeable: a concurrent read/reconcile must not converge it.
        if self.is_in_flight(&run.id) {
            return Ok(run.clone());
        }
        let Some(session_id) = run.workflow_session_id.as_deref() else {
            // A run with no session linked can never be proven to have started.
            return self.persist_run_status(run, "needs_reconcile");
        };
        // An unavailable manager is an observation failure, not "not live".
        let manager_status = self.owner_session_status(session_id)?;
        if manager_status.as_ref().is_some_and(session_status_is_live) {
            // Live manager evidence wins over a stale terminal snapshot.
            return Ok(run.clone());
        }
        // Not live: a canonical snapshot terminal proves the effect finished.
        // Otherwise the manager's own terminal projection is the best structured
        // evidence, and anything else is unknown.
        let snapshot_terminal = self
            .store()
            .workflow_snapshot_state(session_id)
            .map_err(|e| AutomationError::internal_from(e))?
            .as_deref()
            .and_then(terminal_from_snapshot_state);
        let target = snapshot_terminal
            .or_else(|| manager_status.as_ref().and_then(manager_terminal_status))
            .unwrap_or("needs_reconcile");

        self.persist_run_status(run, target)
    }

    /// Persists a terminal/unknown status transition (no-op when unchanged) and
    /// returns the updated row.
    fn persist_run_status(
        &self,
        run: &WorkflowAutomationRun,
        status: &str,
    ) -> Result<WorkflowAutomationRun, AutomationError> {
        if status == run.status {
            return Ok(run.clone());
        }
        self.store()
            .update_workflow_automation_run_status(run_id_str(run), status, None)
            .map_err(|e| AutomationError::internal_from(e))?;
        let mut updated = run.clone();
        updated.status = status.to_string();
        Ok(updated)
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
                .map_err(|e| AutomationError::internal_from(e))?
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
    /// A transient facade bound to this runtime owner's shared store and its
    /// short-lived execution evidence.
    pub fn automation(&self) -> AutomationApplicationService {
        AutomationApplicationService::with_owner(
            self.main_store.clone(),
            self.tsid_generator.clone(),
            self.automation_owner.clone(),
        )
        .with_admission(self.admission.get().cloned())
    }

    /// Manual run through the canonical kernel. Busy-guarded; a successful start
    /// is `Accepted`, not a completion (AC-6/AC-8).
    pub async fn automation_run(
        &self,
        automation_id: &str,
    ) -> Result<AutomationDispatchResult, AutomationError> {
        // A manual run is canonical runtime work: fence it at this lowest layer.
        let _ticket = self
            .admit_work(crate::lifecycle::WorkKind::ManualClaim)
            .map_err(|denied| AutomationError::stopping(denied.message))?;
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

    pub async fn automation_run_with_receipt(
        &self,
        automation_id: &str,
        actor_scope: &str,
        idempotency_key: Option<&str>,
    ) -> Result<AutomationDispatchResult, AutomationError> {
        let Some(key) = idempotency_key.map(str::trim).filter(|key| !key.is_empty()) else {
            return self.automation_run(automation_id).await;
        };
        let request_hash = canonical_request_hash(&json!({
            "operation": "run",
            "automation_id": automation_id,
        }));
        match self
            .automation()
            .store()
            .reserve_automation_mutation(actor_scope, key, "run", &request_hash)
            .map_err(|e| AutomationError::internal_from(e))?
        {
            ReceiptOutcome::Conflict => Err(AutomationError::conflict(
                "idempotency key was already used with a different request",
            )),
            ReceiptOutcome::Replay(Some(raw)) => {
                serde_json::from_str(&raw).map_err(|e| AutomationError::internal_from(e))
            }
            ReceiptOutcome::Replay(None) => Err(AutomationError::internal(
                "idempotent run has no replay result",
            )),
            ReceiptOutcome::Proceed => {
                let result = self.automation_run(automation_id).await?;
                let stored = serde_json::to_string(&result)
                    .map_err(|e| AutomationError::internal_from(e))?;
                self.automation()
                    .store()
                    .complete_automation_mutation(actor_scope, key, Some(&stored))
                    .map_err(|e| AutomationError::internal_from(e))?;
                Ok(result)
            }
        }
    }

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
        // A due slot is canonical runtime work: fence it at this lowest layer, so
        // a direct, non-HTTP scheduler caller is refused on the one gate. The
        // background scheduler's own admission joins this scope instead of
        // admitting twice.
        let _ticket = self
            .admit_work(crate::lifecycle::WorkKind::AutomationDispatch)
            .map_err(|denied| AutomationError::stopping(denied.message))?;
        let facade = self.automation();
        let now_dt = parse_local_datetime(now)?;
        // Converge any stale runs first so a dispatch never overlaps an unknown
        // effect from a previous process (AC-8).
        facade.reconcile_all()?;
        let due = self
            .main_store
            .list_due_workflow_automations(now)
            .map_err(|e| AutomationError::internal_from(e))?;
        let mut results = Vec::new();
        for automation in due {
            // The due slot is the stored cursor; its successor is computed
            // separately so a final slot with no successor is still executed
            // exactly once. A corrupt cursor, or one this instance did not see
            // become due while online, is never consumed (AC-4/AC-6).
            let Some(current_slot) = automation.next_run_at.clone() else {
                continue;
            };
            if facade.schedule_cursor_state(&automation, now_dt)? != ScheduleCursor::DueOnline {
                results.push(AutomationDispatchResult {
                    outcome: AutomationDispatchOutcome::Skipped,
                    run: None,
                });
                continue;
            }
            let successor = facade.successor_for(&automation, now_dt)?;
            let dispatch_key = current_slot.clone();
            let run_id = self
                .tsid_generator
                .generate()
                .map_err(|e| AutomationError::internal(e))?;
            let scheduled_for = current_slot.clone();
            // Admit the run in this instance's short-lived evidence *before* the
            // claim. The claim commits the pending run and another worker thread
            // could observe it on a facade read/reconcile before the guard
            // existed, so admitting only afterwards would leave that window
            // unprotected. The RAII guard releases on drop, including every
            // skipped-claim path, and the durable claim stays the authority.
            let _in_flight = self.automation_owner.begin_run(&run_id);
            let claim = self
                .main_store
                .claim_due_automation_slot(
                    &automation.id,
                    automation.revision,
                    &current_slot,
                    successor,
                    &dispatch_key,
                    &run_id,
                    &scheduled_for,
                )
                .map_err(|e| AutomationError::internal_from(e))?;
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
                .map_err(|e| AutomationError::internal_from(e))?
                .unwrap_or(claimed_run);
            let view = self.automation().run_view(&reloaded)?;
            results.push(AutomationDispatchResult {
                outcome: AutomationDispatchOutcome::Accepted,
                run: Some(view),
            });
        }
        Ok(results)
    }

    /// The one-time startup schedule normalization the daemon runs before its
    /// first tick. See [`AutomationApplicationService::normalize_schedules_at_startup`].
    pub fn automation_normalize_at_startup(
        &self,
        cutoff: &str,
    ) -> Result<AutomationStartupNormalizeReport, AutomationError> {
        self.automation().normalize_schedules_at_startup(cutoff)
    }

    /// The runtime keep-alive/health summary the daemon reads to decide whether
    /// to stay alive. See [`AutomationApplicationService::runtime_summary`].
    pub fn automation_runtime_summary(
        &self,
        now: &str,
    ) -> Result<AutomationRuntimeSummary, AutomationError> {
        self.automation().runtime_summary(now)
    }

    /// The running instance's short-lived automation execution evidence. It is
    /// exposed so the execution kernels admit a run before its durable claim is
    /// committed, which keeps the claim -> start window from being misread as an
    /// unknown effect by a concurrent read.
    pub(crate) fn automation_owner(&self) -> &Arc<AutomationRuntimeOwner> {
        &self.automation_owner
    }

    /// Cancels every owned automation pre-workflow shell this instance is running
    /// and awaits each tree's teardown, reporting how many were in flight.
    ///
    /// The coordinated shutdown calls this so a cancelled runtime really
    /// terminates the shells it started — including descendants that ignore
    /// `SIGTERM` or hold the inherited pipe open — instead of only closing the
    /// admission gate. The caller's deadline bounds the await, so a shell that
    /// cannot finish never blocks the shutdown past the shared budget.
    pub async fn cancel_owned_automation_shells(&self) -> usize {
        self.automation_owner.cancel_owned_processes().await
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
        agent_config: row
            .agent_config
            .as_deref()
            .and_then(|raw| serde_json::from_str(raw).ok()),
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

/// The conservative report for a state this process cannot classify (an
/// unreadable store). It carries no fabricated counts and keeps the runtime
/// alive, because an unobservable state must not be mistaken for "nothing to do".
fn unknown_runtime_summary() -> AutomationRuntimeSummary {
    AutomationRuntimeSummary {
        effective_schedules: 0,
        live_automation_runs: 0,
        in_flight_admissions: 0,
        blocked_reconcile: 0,
        keep_alive: true,
        health: AutomationRuntimeHealth::Unknown,
    }
}

/// The runtime classification of an automation's stored schedule cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScheduleCursor {
    /// Strictly future: a legitimate pending schedule.
    Future,
    /// Due at or before `now` and at or after the instance start boundary: it
    /// became due while this instance was online, so it may be consumed.
    DueOnline,
    /// No cursor, a corrupt slot, or a slot missed while the instance was
    /// offline. It is never consumed and never keeps the runtime alive.
    NotEffective,
}

/// Parses a stored `next_run_at`/dispatch instant. Accepts the canonical DB form
/// and RFC3339; anything else is an invalid request rather than a silent zero.
fn parse_local_datetime(value: &str) -> Result<DateTime<Local>, AutomationError> {
    use chrono::{NaiveDateTime, TimeZone};
    let trimmed = value.trim();
    if let Ok(parsed) = DateTime::parse_from_rfc3339(trimmed) {
        return Ok(parsed.with_timezone(&Local));
    }
    for format in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(trimmed, format) {
            if let Some(datetime) = Local
                .from_local_datetime(&naive)
                .single()
                .or_else(|| Local.from_local_datetime(&naive).earliest())
            {
                return Ok(datetime);
            }
        }
    }
    Err(AutomationError::invalid_request(format!(
        "invalid datetime: {value}"
    )))
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
    // invalid/conflict, not an opaque internal error. The typed origin is
    // captured before the display text is inspected, so the classification
    // decision never becomes the only surviving evidence.
    let origin = error.error_origin();
    let text = error.to_string();
    if text.contains("FOREIGN KEY") || text.contains("constraint failed") {
        return AutomationError::invalid_request(format!("automation write rejected: {text}"))
            .with_origin(origin);
    }
    AutomationError::internal(text).with_origin(origin)
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
        let conflict = svc.create(&different, AUTOMATION_ACTOR_SCOPE_CONTROL_PLANE, Some("k1"));
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
        let done = svc
            .runs(&id)
            .expect("runs")
            .into_iter()
            .find(|run| run.run_id == "run-done")
            .expect("find run");
        assert_eq!(done.status, "completed");
        assert_eq!(done.workflow_status.as_deref(), Some("completed"));
    }

    /// Source wiring guard (AC-1/INV-2): the legacy Tauri `run_now` command must
    /// reach the manual-run kernel only through the runtime control plane, and
    /// the backend `/run` route must reach it only through the typed facade. The
    /// chain is: desktop command -> `runtime_automation::automation_run_now`
    /// (idempotent HTTP POST `/control/v1/automations/{id}/run`) -> backend
    /// `run_automation` -> `automation_run_with_receipt` -> the single
    /// `automation_run` that owns `create_manual_run`. This locks the re-review
    /// fix so no second public mutation path can return.
    #[test]
    fn legacy_run_now_routes_through_typed_facade() {
        // Desktop Tauri command: delegates to the runtime control-plane adapter
        // and holds no run logic of its own.
        let command_src = include_str!("../../../../../src/commands/workflow_automation.rs");
        assert!(
            command_src.contains("runtime_automation::automation_run_now"),
            "run_now command must delegate to the runtime control-plane adapter"
        );
        assert!(
            !command_src.contains("create_manual_run")
                && !command_src.contains("automation_run_compat"),
            "run_now command must not reach the kernel or a local facade directly"
        );

        // Desktop adapter: maps the command onto the canonical `/run` HTTP route.
        let adapter_src = include_str!("../../../../../src/runtime_automation.rs");
        assert!(
            adapter_src.contains("{AUTOMATIONS_ROUTE}/{}/run"),
            "the runtime adapter must post the canonical `/run` route"
        );
        let run_now_body = adapter_src
            .split("pub async fn automation_run_now(")
            .nth(1)
            .and_then(|rest| {
                rest.split("/// Produces the side-effect-free draft plan")
                    .next()
            })
            .expect("automation_run_now body");
        assert!(
            run_now_body.contains("{AUTOMATIONS_ROUTE}/{}/run")
                && run_now_body.contains("post_idempotent"),
            "the run_now adapter must post the idempotent `/run` route"
        );
        assert!(
            !run_now_body.contains("create_manual_run"),
            "the adapter must not reach the kernel directly"
        );

        // Backend control plane: the `/run` route dispatches to the typed facade.
        let server_src = include_str!("../react/client/http/server.rs");
        assert!(
            server_src.contains("\"/control/v1/automations/{automation_id}/run\""),
            "the control plane must expose the canonical `/run` route"
        );
        let handler_body = server_src
            .split("async fn run_automation(")
            .nth(1)
            .and_then(|rest| rest.split("async fn delete_automation(").next())
            .expect("run_automation handler body");
        assert!(
            handler_body.contains("automation_run_with_receipt"),
            "the `/run` route must call the typed automation_run_with_receipt facade"
        );
        assert!(
            !handler_body.contains("create_manual_run"),
            "the `/run` route must not reach the kernel directly"
        );

        let app_src = include_str!("./application.rs");

        // The receipt-wrapped entry point only forwards to the typed run facade.
        let with_receipt_body = app_src
            .split("pub async fn automation_run_with_receipt(")
            .nth(1)
            .and_then(|rest| {
                rest.split("/// The mutation goes exclusively through the typed facade")
                    .next()
            })
            .expect("automation_run_with_receipt body");
        assert!(
            with_receipt_body.contains("self.automation_run(automation_id)"),
            "automation_run_with_receipt must call the typed automation_run facade"
        );
        assert!(
            !with_receipt_body.contains("create_manual_run"),
            "automation_run_with_receipt must not call the service kernel directly"
        );

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

        // `automation_run` remains the sole owner of the manual-run kernel; its
        // body is bounded by the next public entry point so the assertion cannot
        // be satisfied by the receipt wrapper instead.
        let run_body = app_src
            .split("pub async fn automation_run(")
            .nth(1)
            .and_then(|rest| {
                rest.split("pub async fn automation_run_with_receipt(")
                    .next()
            })
            .expect("automation_run body");
        assert!(
            run_body.contains("create_manual_run"),
            "automation_run is the sole owner of the manual-run kernel"
        );
    }

    // --- runtime owner / normalisation / summary --------------------------

    fn runtime_owner(
        manager: Option<&Arc<crate::workflow::react::manager::WorkflowManager>>,
    ) -> Arc<AutomationRuntimeOwner> {
        match manager {
            Some(manager) => Arc::new(AutomationRuntimeOwner::new(Arc::downgrade(manager))),
            // A dangling weak handle stands in for an owner whose manager is
            // gone: live observation must then report an error, never a zero.
            None => Arc::new(AutomationRuntimeOwner::new(std::sync::Weak::new())),
        }
    }

    fn facade_with_owner(
        store: &Arc<MainStore>,
        owner: Arc<AutomationRuntimeOwner>,
    ) -> AutomationApplicationService {
        let tsid = Arc::new(crate::libs::tsid::TsidGenerator::new(1).expect("tsid"));
        AutomationApplicationService::with_owner(store.clone(), tsid, owner)
    }

    fn upsert(
        id: &str,
        kind: &str,
        config: Value,
        enabled: bool,
        next_run_at: Option<&str>,
    ) -> WorkflowAutomationUpsert {
        WorkflowAutomationUpsert {
            id: id.to_string(),
            title: id.to_string(),
            // No prompt: the run kernel then fails fast resolving it, which lets
            // a dispatch test observe the durable claim without starting a real
            // workflow.
            prompt: None,
            prompt_file_path: None,
            agent_id: "agent-main".to_string(),
            agent_config: Some("{}".to_string()),
            allowed_paths: "[]".to_string(),
            shell_config: None,
            schedule_kind: kind.to_string(),
            schedule_config: config.to_string(),
            continuous_context: false,
            current_workflow_session_id: None,
            self_review: false,
            enabled,
            next_run_at: next_run_at.map(ToString::to_string),
        }
    }

    fn scheduled_run(
        automation_id: &str,
        run_id: &str,
        status: &str,
    ) -> WorkflowAutomationRunInsert {
        WorkflowAutomationRunInsert {
            id: run_id.to_string(),
            automation_id: automation_id.to_string(),
            workflow_session_id: None,
            status: status.to_string(),
            scheduled_for: "2026-06-25 09:00:00".to_string(),
            started_at: None,
            finished_at: None,
            error: None,
            trigger: "scheduled".to_string(),
            dispatch_key: Some("2026-06-25 09:00:00".to_string()),
        }
    }

    fn daily_config() -> Value {
        json!({"time": "09:00", "weekdays": [], "start_date": null, "end_date": null})
    }

    /// A durable workflow row, required by the run's `workflow_session_id` FK.
    fn seed_session(store: &Arc<MainStore>, session_id: &str) {
        store
            .create_workflow(session_id, "session", "agent-main", None, None)
            .expect("seed workflow session");
    }

    /// A durable workflow snapshot row in the given lifecycle state, the
    /// structured terminal/looping evidence the run projection reads.
    fn seed_snapshot(store: &Arc<MainStore>, session_id: &str, state: &str) {
        let session_id = session_id.to_string();
        let state = state.to_string();
        store
            .db_runtime()
            .expect("runtime")
            .write_blocking(move |conn| {
                conn.execute(
                    "INSERT INTO workflow_snapshots (session_id, context_json, version, state)
                     VALUES (?1, '{}', '1', ?2)",
                    params![session_id, state],
                )?;
                Ok(())
            })
            .expect("seed snapshot");
    }

    /// Builds the real runtime owner (chat state, hub, workflow manager and
    /// factory) so a dispatch test exercises the canonical path end to end.
    fn full_service(
        dir: &std::path::Path,
        store: Arc<MainStore>,
    ) -> Arc<WorkflowApplicationService> {
        use crate::ai::interaction::chat_completion::ChatState;
        use crate::libs::window_channels::WindowChannels;
        use crate::workflow::react::client::hub::{NoWindowTransport, WorkflowRuntimeHub};
        use crate::workflow::react::manager::WorkflowManager;
        use crate::workflow::react::orchestrator::{DefaultSubAgentFactory, SubAgentFactory};
        let chat_state = ChatState::new(Arc::new(WindowChannels::new()), None, store.clone());
        let tsid = Arc::new(TsidGenerator::new(1).expect("tsid"));
        let hub = Arc::new(WorkflowRuntimeHub::with_transport(
            Arc::new(NoWindowTransport),
            "automation-test".to_string(),
        ));
        let manager = Arc::new(WorkflowManager::new());
        let factory: Arc<dyn SubAgentFactory> = Arc::new(DefaultSubAgentFactory {
            main_store: store.clone(),
            chat_state: chat_state.clone(),
            gateway: hub.clone(),
            workflow_manager: manager.clone(),
            app_data_dir: dir.to_path_buf(),
            tsid_generator: tsid.clone(),
        });
        Arc::new(WorkflowApplicationService::new(
            store,
            chat_state,
            tsid,
            hub,
            factory,
            manager,
            dir.to_path_buf(),
        ))
    }

    #[test]
    fn startup_normalize_advances_a_missed_slot_and_keeps_a_future_one() {
        let (_dir, store) = seeded_store(&["agent-main"]);
        store
            .create_workflow_automation(&upsert(
                "a-missed",
                "daily",
                daily_config(),
                true,
                Some("2026-06-24 09:00:00"),
            ))
            .expect("create missed automation");
        store
            .create_workflow_automation(&upsert(
                "a-future",
                "daily",
                daily_config(),
                true,
                Some("2026-06-26 09:00:00"),
            ))
            .expect("create future automation");

        let owner = runtime_owner(None);
        let facade = facade_with_owner(&store, owner.clone());
        let report = facade
            .normalize_schedules_at_startup("2026-06-25 10:00:00")
            .expect("normalize");

        assert_eq!(report.examined, 2);
        assert_eq!(report.advanced, 1);
        assert_eq!(report.unchanged, 1);
        assert_eq!(
            store
                .get_workflow_automation("a-missed")
                .expect("read")
                .expect("row")
                .next_run_at
                .as_deref(),
            Some("2026-06-26 09:00:00")
        );
        // A stored future slot is a legitimate pending schedule: untouched.
        assert_eq!(
            store
                .get_workflow_automation("a-future")
                .expect("read")
                .expect("row")
                .next_run_at
                .as_deref(),
            Some("2026-06-26 09:00:00")
        );
        assert!(owner.startup_cutoff().is_some());

        // The pass is one-time: a duplicate startup signal changes nothing.
        let second = facade
            .normalize_schedules_at_startup("2026-06-25 11:00:00")
            .expect("renormalize");
        assert_eq!(second.examined, 0);
    }

    #[test]
    fn startup_normalize_clears_an_elapsed_once_and_keeps_a_null_slot() {
        let (_dir, store) = seeded_store(&["agent-main"]);
        store
            .create_workflow_automation(&upsert(
                "a-once",
                "once",
                json!({"run_at": "2026-06-24 09:00:00"}),
                true,
                Some("2026-06-24 09:00:00"),
            ))
            .expect("create once");
        store
            .create_workflow_automation(&upsert("a-null", "daily", daily_config(), true, None))
            .expect("create null");
        store
            .create_workflow_automation(&upsert(
                "a-disabled",
                "daily",
                daily_config(),
                false,
                Some("2026-06-24 09:00:00"),
            ))
            .expect("create disabled");

        let facade = facade_with_owner(&store, runtime_owner(None));
        let report = facade
            .normalize_schedules_at_startup("2026-06-25 10:00:00")
            .expect("normalize");

        // Only the enabled rows are examined; the disabled row is left alone.
        assert_eq!(report.examined, 2);
        assert_eq!(report.expired, 1);
        // A NULL cursor defines no next slot, so the pass neither fills it nor
        // counts it as advanced (AC-3).
        assert_eq!(report.advanced, 0);
        assert_eq!(report.unchanged, 1);
        assert_eq!(
            store
                .get_workflow_automation("a-once")
                .expect("read")
                .expect("row")
                .next_run_at,
            None
        );
        assert_eq!(
            store
                .get_workflow_automation("a-null")
                .expect("read")
                .expect("row")
                .next_run_at,
            None
        );
        let disabled = store
            .get_workflow_automation("a-disabled")
            .expect("read")
            .expect("row");
        assert!(!disabled.enabled);
        assert_eq!(disabled.next_run_at.as_deref(), Some("2026-06-24 09:00:00"));
    }

    #[test]
    fn normalize_cas_rejects_a_moved_revision_and_preserves_enabled() {
        let (_dir, store) = seeded_store(&["agent-main"]);
        let created = store
            .create_workflow_automation(&upsert(
                "a-cas2",
                "daily",
                daily_config(),
                true,
                Some("2026-06-24 09:00:00"),
            ))
            .expect("create");
        assert_eq!(created.revision, 1);

        match store
            .normalize_workflow_automation_next_run_at_cas(
                "a-cas2",
                Some("2026-06-26 09:00:00".to_string()),
                99,
            )
            .expect("cas")
        {
            CasOutcome::RevisionConflict => {}
            other => panic!("expected revision conflict, got {other:?}"),
        }
        let row = store
            .get_workflow_automation("a-cas2")
            .expect("read")
            .expect("row");
        assert!(row.enabled);
        assert_eq!(row.next_run_at.as_deref(), Some("2026-06-24 09:00:00"));
        assert_eq!(row.revision, 1);

        match store
            .normalize_workflow_automation_next_run_at_cas(
                "a-cas2",
                Some("2026-06-26 09:00:00".to_string()),
                1,
            )
            .expect("cas")
        {
            CasOutcome::Updated(row) => {
                assert!(row.enabled, "normalization must not change enabled");
                assert_eq!(row.revision, 2);
                assert_eq!(row.next_run_at.as_deref(), Some("2026-06-26 09:00:00"));
            }
            other => panic!("expected update, got {other:?}"),
        }
    }

    #[test]
    fn in_flight_guard_keeps_a_session_less_run_from_needs_reconcile() {
        let (_dir, store) = seeded_store(&["agent-main"]);
        store
            .create_workflow_automation(&upsert("a-guard", "daily", daily_config(), true, None))
            .expect("create");
        store
            .add_workflow_automation_run(&scheduled_run("a-guard", "run-guard", "pending"))
            .expect("add run");

        let owner = runtime_owner(None);
        let facade = facade_with_owner(&store, owner.clone());

        // While this instance holds the in-flight guard, reads/reconcile leave
        // the session-less run untouched instead of converging it.
        let guard = owner.begin_run("run-guard");
        assert!(owner.is_in_flight("run-guard"));
        let views = facade.runs("a-guard").expect("runs");
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].status, "pending");
        facade.reconcile_all().expect("reconcile");
        assert_eq!(
            store
                .get_workflow_automation_run("run-guard")
                .expect("read")
                .expect("row")
                .status,
            "pending"
        );

        // Dropping the guard releases the evidence; with no session and no live
        // owner the run then converges to `needs_reconcile`.
        drop(guard);
        assert!(!owner.is_in_flight("run-guard"));
        facade.reconcile_all().expect("reconcile");
        assert_eq!(
            store
                .get_workflow_automation_run("run-guard")
                .expect("read")
                .expect("row")
                .status,
            "needs_reconcile"
        );
    }

    #[tokio::test]
    async fn dispatch_claims_a_due_final_slot_with_no_successor_and_runs_once() {
        let (dir, store) = seeded_store(&["agent-main"]);
        store
            .create_workflow_automation(&upsert(
                "a-once-due",
                "once",
                json!({"run_at": "2026-06-25 09:00:00"}),
                true,
                Some("2026-06-25 09:00:00"),
            ))
            .expect("create");
        let svc = full_service(dir.path(), store.clone());
        svc.automation_owner()
            .mark_startup(parse_local_datetime("2026-06-25 08:00:00").expect("cutoff"));

        let results = svc
            .automation_dispatch_due("2026-06-25 09:00:30")
            .await
            .expect("dispatch");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].outcome, AutomationDispatchOutcome::Accepted);

        let runs = store
            .list_workflow_automation_runs("a-once-due")
            .expect("runs");
        assert_eq!(runs.len(), 1, "the final once slot executes exactly once");
        assert_eq!(runs[0].trigger, "scheduled");
        assert_eq!(
            runs[0].dispatch_key.as_deref(),
            Some("2026-06-25 09:00:00"),
            "the due slot is the dispatch key, not the (absent) successor"
        );
        // The once cursor is consumed to `None` rather than left due.
        assert_eq!(
            store
                .get_workflow_automation("a-once-due")
                .expect("read")
                .expect("row")
                .next_run_at,
            None
        );

        // A later tick must not replay the consumed slot.
        let again = svc
            .automation_dispatch_due("2026-06-25 09:01:00")
            .await
            .expect("dispatch again");
        assert!(again.is_empty());
        assert_eq!(
            store
                .list_workflow_automation_runs("a-once-due")
                .expect("runs")
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn dispatch_runs_the_last_in_range_daily_slot_once() {
        let (dir, store) = seeded_store(&["agent-main"]);
        store
            .create_workflow_automation(&upsert(
                "a-last",
                "daily",
                json!({
                    "time": "09:00",
                    "weekdays": [],
                    "start_date": "2026-06-25",
                    "end_date": "2026-06-25"
                }),
                true,
                Some("2026-06-25 09:00:00"),
            ))
            .expect("create");
        let svc = full_service(dir.path(), store.clone());
        svc.automation_owner()
            .mark_startup(parse_local_datetime("2026-06-25 08:00:00").expect("cutoff"));

        let results = svc
            .automation_dispatch_due("2026-06-25 09:01:00")
            .await
            .expect("dispatch");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].outcome, AutomationDispatchOutcome::Accepted);
        assert_eq!(
            store
                .list_workflow_automation_runs("a-last")
                .expect("runs")
                .len(),
            1,
            "the last in-range slot runs once even with no successor"
        );
        assert_eq!(
            store
                .get_workflow_automation("a-last")
                .expect("read")
                .expect("row")
                .next_run_at,
            None
        );
    }

    #[tokio::test]
    async fn dispatch_refuses_a_slot_missed_while_the_instance_was_offline() {
        let (dir, store) = seeded_store(&["agent-main"]);
        store
            .create_workflow_automation(&upsert(
                "a-stale",
                "daily",
                daily_config(),
                true,
                Some("2026-06-25 09:00:00"),
            ))
            .expect("create");
        let svc = full_service(dir.path(), store.clone());
        // The instance only came online at 10:00, after the 09:00 slot was due.
        svc.automation_owner()
            .mark_startup(parse_local_datetime("2026-06-25 10:00:00").expect("cutoff"));

        let results = svc
            .automation_dispatch_due("2026-06-25 10:01:00")
            .await
            .expect("dispatch");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].outcome, AutomationDispatchOutcome::Skipped);
        assert!(
            store
                .list_workflow_automation_runs("a-stale")
                .expect("runs")
                .is_empty(),
            "a slot missed while offline is never replayed"
        );
    }

    #[test]
    fn cursor_validation_rejects_a_weekday_mismatch_and_accepts_the_right_day() {
        let (_dir, store) = seeded_store(&["agent-main"]);
        // Monday-only schedule with a Thursday cursor: corrupt, never consumed.
        store
            .create_workflow_automation(&upsert(
                "a-wd",
                "daily",
                json!({"time": "09:00", "weekdays": [1], "start_date": null, "end_date": null}),
                true,
                Some("2026-06-25 09:00:00"),
            ))
            .expect("create");
        let facade = facade_with_owner(&store, runtime_owner(None));
        let now = parse_local_datetime("2026-06-25 10:00:00").expect("now");
        let automation = store
            .get_workflow_automation("a-wd")
            .expect("read")
            .expect("row");
        assert_eq!(
            facade
                .schedule_cursor_state(&automation, now)
                .expect("state"),
            ScheduleCursor::NotEffective
        );

        // A valid Monday slot on the same schedule is a legal future cursor.
        store
            .normalize_workflow_automation_next_run_at_cas(
                "a-wd",
                Some("2026-06-29 09:00:00".to_string()),
                1,
            )
            .expect("cas");
        let automation = store
            .get_workflow_automation("a-wd")
            .expect("read")
            .expect("row");
        assert_eq!(
            facade
                .schedule_cursor_state(&automation, now)
                .expect("state"),
            ScheduleCursor::Future
        );
    }

    #[test]
    fn runtime_summary_counts_only_effective_keepalive_reasons() {
        let (_dir, store) = seeded_store(&["agent-main"]);
        store
            .create_workflow_automation(&upsert(
                "a-fut",
                "daily",
                daily_config(),
                true,
                Some("2026-06-26 09:00:00"),
            ))
            .expect("create future");
        store
            .create_workflow_automation(&upsert("a-off", "daily", daily_config(), false, None))
            .expect("create disabled");

        let facade = facade_with_owner(&store, runtime_owner(None));
        let summary = facade
            .runtime_summary("2026-06-25 10:00:00")
            .expect("summary");
        assert_eq!(summary.effective_schedules, 1);
        assert_eq!(summary.live_automation_runs, 0);
        assert_eq!(summary.blocked_reconcile, 0);
        assert!(summary.keep_alive);
        assert_eq!(summary.health, AutomationRuntimeHealth::Healthy);
    }

    #[test]
    fn runtime_summary_does_not_keep_alive_for_blocked_or_slotless_work() {
        let (_dir, store) = seeded_store(&["agent-main"]);
        // Enabled with a legal future slot, but blocked by an unknown run: the
        // schedule cannot dispatch, so it is not effective.
        store
            .create_workflow_automation(&upsert(
                "a-blocked",
                "daily",
                daily_config(),
                true,
                Some("2026-06-26 09:00:00"),
            ))
            .expect("create blocked");
        store
            .add_workflow_automation_run(&scheduled_run("a-blocked", "run-nr", "needs_reconcile"))
            .expect("add run");
        // Enabled but slot-less (expired configuration): not effective.
        store
            .create_workflow_automation(&upsert("a-none", "daily", daily_config(), true, None))
            .expect("create none");
        // A historical terminal run contributes nothing.
        seed_session(&store, "session-old");
        let mut terminal = scheduled_run("a-none", "run-term", "completed");
        terminal.workflow_session_id = Some("session-old".to_string());
        terminal.dispatch_key = None;
        store
            .add_workflow_automation_run(&terminal)
            .expect("add terminal");

        let facade = facade_with_owner(&store, runtime_owner(None));
        let summary = facade
            .runtime_summary("2026-06-25 10:00:00")
            .expect("summary");
        assert_eq!(summary.effective_schedules, 0);
        assert_eq!(summary.live_automation_runs, 0);
        assert_eq!(summary.blocked_reconcile, 1);
        assert!(
            !summary.keep_alive,
            "blocked/unknown work must not keep the runtime alive alone"
        );
        assert_eq!(summary.health, AutomationRuntimeHealth::Degraded);
    }

    #[test]
    fn runtime_summary_is_unknown_without_a_runtime_owner() {
        let (_dir, store) = seeded_store(&["agent-main"]);
        store
            .create_workflow_automation(&upsert(
                "a-fut2",
                "daily",
                daily_config(),
                true,
                Some("2026-06-26 09:00:00"),
            ))
            .expect("create future");

        // A standalone facade has no live owner to observe, so its health is
        // `unknown` rather than a fabricated healthy/degraded.
        let facade = facade(&store);
        let summary = facade
            .runtime_summary("2026-06-25 10:00:00")
            .expect("summary");
        assert_eq!(summary.health, AutomationRuntimeHealth::Unknown);
        assert_eq!(summary.effective_schedules, 1);
    }

    #[test]
    fn runtime_summary_reports_an_error_when_the_manager_cannot_be_observed() {
        let (_dir, store) = seeded_store(&["agent-main"]);
        store
            .create_workflow_automation(&upsert("a-orphan", "daily", daily_config(), true, None))
            .expect("create");
        let mut run = scheduled_run("a-orphan", "run-orphan", "running");
        run.workflow_session_id = Some("session-orphan".to_string());
        seed_session(&store, "session-orphan");
        store.add_workflow_automation_run(&run).expect("add run");

        // The owner exists but its weak manager is gone: observation fails, and
        // that failure must surface as an error, never as a zero live count.
        let facade = facade_with_owner(&store, runtime_owner(None));
        assert!(facade.runtime_summary("2026-06-25 10:00:00").is_err());
    }

    /// A minimal executor so the workflow manager can register a live session.
    struct StubExecutor {
        session_id: String,
        state: crate::workflow::react::types::WorkflowState,
    }

    #[async_trait::async_trait]
    impl crate::workflow::react::engine::ReActExecutor for StubExecutor {
        async fn init(&mut self) -> Result<(), crate::workflow::react::error::WorkflowEngineError> {
            Ok(())
        }
        async fn run_loop(
            &mut self,
        ) -> Result<(), crate::workflow::react::error::WorkflowEngineError> {
            Ok(())
        }
        async fn begin_new_context_segment(
            &mut self,
        ) -> Result<(), crate::workflow::react::error::WorkflowEngineError> {
            Ok(())
        }
        async fn begin_manual_clear_context_segment(
            &mut self,
        ) -> Result<(), crate::workflow::react::error::WorkflowEngineError> {
            Ok(())
        }
        async fn prepare_completed_resume(
            &mut self,
        ) -> Result<(), crate::workflow::react::error::WorkflowEngineError> {
            Ok(())
        }
        async fn add_message_and_notify(
            &mut self,
            _role: String,
            _content: String,
            _attached_context: Option<String>,
            _reasoning: Option<String>,
            _step_type: Option<crate::workflow::react::types::StepType>,
            _is_error: bool,
            _error_type: Option<String>,
            _metadata: Option<Value>,
        ) -> Result<bool, crate::workflow::react::error::WorkflowEngineError> {
            Ok(true)
        }
        fn session_id(&self) -> String {
            self.session_id.clone()
        }
        fn state(&self) -> crate::workflow::react::types::WorkflowState {
            self.state.clone()
        }
        fn set_state(&mut self, state: crate::workflow::react::types::WorkflowState) {
            self.state = state;
        }
        fn attach_signal_rx(&mut self, _signal_rx: tokio::sync::mpsc::Receiver<String>) {}
        fn messages(&self) -> Vec<crate::db::WorkflowMessage> {
            vec![]
        }
    }

    fn register_session(
        manager: &Arc<crate::workflow::react::manager::WorkflowManager>,
        session_id: &str,
        state: crate::workflow::react::types::WorkflowState,
        status: ManagedSessionStatus,
    ) {
        let executor: Arc<tokio::sync::Mutex<dyn crate::workflow::react::engine::ReActExecutor>> =
            Arc::new(tokio::sync::Mutex::new(StubExecutor {
                session_id: session_id.to_string(),
                state,
            }));
        manager
            .register_session(session_id.to_string(), executor, status)
            .expect("register session");
    }

    #[test]
    fn a_live_waiting_owner_keeps_a_disabled_automations_run_live() {
        let (_dir, store) = seeded_store(&["agent-main"]);
        store
            .create_workflow_automation(&upsert(
                "a-dis",
                "interval",
                json!({"interval_minutes": 60}),
                true,
                None,
            ))
            .expect("create");
        let mut run = scheduled_run("a-dis", "run-dis", "running");
        run.workflow_session_id = Some("sess-wait".to_string());
        run.dispatch_key = None;
        seed_session(&store, "sess-wait");
        store.add_workflow_automation_run(&run).expect("add run");

        let manager = Arc::new(crate::workflow::react::manager::WorkflowManager::new());
        register_session(
            &manager,
            "sess-wait",
            crate::workflow::react::types::WorkflowState::AwaitingUser,
            ManagedSessionStatus::Waiting,
        );
        let facade = facade_with_owner(&store, runtime_owner(Some(&manager)));

        let summary = facade
            .runtime_summary("2026-06-25 10:00:00")
            .expect("summary");
        assert_eq!(
            summary.live_automation_runs, 1,
            "a waiting session is a real live owner"
        );
        assert_eq!(summary.blocked_reconcile, 0);
        assert!(summary.keep_alive);

        // The live run is not converged by a read.
        let views = facade.runs("a-dis").expect("runs");
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].status, "running");
    }

    #[test]
    fn a_terminal_manager_session_converges_a_run_instead_of_zero_live() {
        let (_dir, store) = seeded_store(&["agent-main"]);
        store
            .create_workflow_automation(&upsert(
                "a-done",
                "interval",
                json!({"interval_minutes": 60}),
                true,
                None,
            ))
            .expect("create");
        let mut run = scheduled_run("a-done", "run-done", "running");
        run.workflow_session_id = Some("sess-done".to_string());
        run.dispatch_key = None;
        seed_session(&store, "sess-done");
        store.add_workflow_automation_run(&run).expect("add run");

        let manager = Arc::new(crate::workflow::react::manager::WorkflowManager::new());
        // A completed session the manager still hot-caches is terminal, not live.
        register_session(
            &manager,
            "sess-done",
            crate::workflow::react::types::WorkflowState::Completed,
            ManagedSessionStatus::Completed,
        );
        let facade = facade_with_owner(&store, runtime_owner(Some(&manager)));

        let views = facade.runs("a-done").expect("runs");
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].status, "completed");

        let summary = facade
            .runtime_summary("2026-06-25 10:00:00")
            .expect("summary");
        assert_eq!(summary.live_automation_runs, 0);
        assert_eq!(summary.blocked_reconcile, 0);
    }

    #[test]
    fn startup_normalize_retry_after_a_failure_is_not_treated_as_success() {
        let (_dir, store) = seeded_store(&["agent-main"]);
        store
            .create_workflow_automation(&upsert(
                "a-retry",
                "daily",
                daily_config(),
                true,
                Some("2026-06-24 09:00:00"),
            ))
            .expect("create");
        let owner = runtime_owner(None);
        let facade = facade_with_owner(&store, owner.clone());

        // A pass that errors before finishing must not be recorded as done.
        assert!(facade
            .normalize_schedules_at_startup("not-a-datetime")
            .is_err());
        assert!(!owner.startup_pass_completed());
        assert!(owner.startup_cutoff().is_none());

        // The retried pass runs and only then marks itself complete.
        let report = facade
            .normalize_schedules_at_startup("2026-06-25 10:00:00")
            .expect("retry");
        assert_eq!(report.examined, 1);
        assert_eq!(report.advanced, 1);
        assert!(owner.startup_pass_completed());
        assert_eq!(
            owner.startup_cutoff(),
            Some(parse_local_datetime("2026-06-25 10:00:00").expect("cutoff"))
        );

        // A duplicate signal after a successful pass is a no-op.
        let again = facade
            .normalize_schedules_at_startup("2026-06-25 11:00:00")
            .expect("duplicate");
        assert_eq!(again.examined, 0);

        // A retried pass reuses the first recorded boundary instead of
        // advancing to a later cutoff.
        let other = runtime_owner(None);
        let first = parse_local_datetime("2026-06-25 10:00:00").expect("first");
        let second = parse_local_datetime("2026-06-25 12:00:00").expect("second");
        assert!(other.mark_startup(first));
        assert!(!other.mark_startup(second));
        assert_eq!(other.startup_cutoff(), Some(first));
    }

    #[test]
    fn project_run_prefers_a_live_manager_over_a_stale_terminal_snapshot() {
        let (_dir, store) = seeded_store(&["agent-main"]);
        store
            .create_workflow_automation(&upsert(
                "a-live",
                "interval",
                json!({"interval_minutes": 60}),
                true,
                None,
            ))
            .expect("create");
        seed_session(&store, "sess-live");
        // The snapshot still holds the previous run's terminal state.
        seed_snapshot(&store, "sess-live", "completed");
        let mut run = scheduled_run("a-live", "run-live", "running");
        run.workflow_session_id = Some("sess-live".to_string());
        run.dispatch_key = None;
        store.add_workflow_automation_run(&run).expect("add run");

        let manager = Arc::new(crate::workflow::react::manager::WorkflowManager::new());
        register_session(
            &manager,
            "sess-live",
            crate::workflow::react::types::WorkflowState::Executing,
            ManagedSessionStatus::Active,
        );
        let facade = facade_with_owner(&store, runtime_owner(Some(&manager)));

        // A hot-recovered live session must not be projected terminal by the
        // stale snapshot of the same (continuous-context) session.
        let views = facade.runs("a-live").expect("runs");
        assert_eq!(views[0].status, "running");

        let summary = facade
            .runtime_summary("2026-06-25 10:00:00")
            .expect("summary");
        assert_eq!(summary.live_automation_runs, 1);
        assert_eq!(summary.blocked_reconcile, 0);
        assert!(summary.keep_alive);
    }

    #[test]
    fn project_run_marks_an_unowned_running_snapshot_needs_reconcile() {
        let (_dir, store) = seeded_store(&["agent-main"]);
        store
            .create_workflow_automation(&upsert(
                "a-unowned",
                "interval",
                json!({"interval_minutes": 60}),
                true,
                None,
            ))
            .expect("create");
        seed_session(&store, "sess-unowned");
        seed_snapshot(&store, "sess-unowned", "running");
        let mut run = scheduled_run("a-unowned", "run-unowned", "running");
        run.workflow_session_id = Some("sess-unowned".to_string());
        run.dispatch_key = None;
        store.add_workflow_automation_run(&run).expect("add run");

        // With no live owner and a non-terminal snapshot, neither the workflow
        // row nor the run's own `running` status may be read as a live run.
        let facade = facade(&store);
        let views = facade.runs("a-unowned").expect("runs");
        assert_eq!(views[0].status, "needs_reconcile");

        let summary = facade
            .runtime_summary("2026-06-25 10:00:00")
            .expect("summary");
        assert_eq!(summary.live_automation_runs, 0);
        assert_eq!(summary.blocked_reconcile, 1);
        assert_eq!(summary.health, AutomationRuntimeHealth::Unknown);
        assert!(
            summary.keep_alive,
            "an unobservable state must not be reported as nothing to do"
        );
    }

    #[test]
    fn runtime_summary_recovers_a_future_slot_after_a_run_reaches_terminal() {
        let (_dir, store) = seeded_store(&["agent-main"]);
        store
            .create_workflow_automation(&upsert(
                "a-recover",
                "daily",
                daily_config(),
                true,
                Some("2026-06-26 09:00:00"),
            ))
            .expect("create");
        seed_session(&store, "sess-recover");
        seed_snapshot(&store, "sess-recover", "completed");
        let mut run = scheduled_run("a-recover", "run-recover", "running");
        run.workflow_session_id = Some("sess-recover".to_string());
        run.dispatch_key = None;
        store.add_workflow_automation_run(&run).expect("add run");

        // The bounded reconcile converges the run on its terminal snapshot, so
        // the future enabled slot becomes effective again instead of staying
        // suppressed by a stale blocking run.
        let facade = facade(&store);
        let summary = facade
            .runtime_summary("2026-06-25 10:00:00")
            .expect("summary");
        assert_eq!(summary.effective_schedules, 1);
        assert_eq!(summary.blocked_reconcile, 0);
        assert_eq!(
            store
                .get_workflow_automation_run("run-recover")
                .expect("read")
                .expect("row")
                .status,
            "completed"
        );
    }

    #[test]
    fn runtime_summary_downgrades_an_invalid_schedule_instead_of_failing() {
        let (_dir, store) = seeded_store(&["agent-main"]);
        store
            .create_workflow_automation(&upsert(
                "a-bad",
                "interval",
                json!({"interval_minutes": 60}),
                true,
                Some("2026-06-26 09:00:00"),
            ))
            .expect("create");
        // A user configuration that cannot be parsed is a diagnostic problem,
        // not a query failure.
        store
            .db_runtime()
            .expect("runtime")
            .write_blocking(|conn| {
                conn.execute(
                    "UPDATE workflow_automations SET schedule_config = 'not json' WHERE id = 'a-bad'",
                    [],
                )?;
                Ok(())
            })
            .expect("corrupt config");

        let facade = facade_with_owner(&store, runtime_owner(None));
        let summary = facade
            .runtime_summary("2026-06-25 10:00:00")
            .expect("an invalid schedule must not fail the summary");
        assert_eq!(summary.effective_schedules, 0);
        assert!(!summary.keep_alive);
        assert_eq!(summary.health, AutomationRuntimeHealth::Degraded);
    }

    #[test]
    fn runtime_summary_builds_the_blocking_set_from_the_active_runs_once() {
        let (_dir, store) = seeded_store(&["agent-main"]);
        for id in ["a-one", "a-two", "a-three"] {
            store
                .create_workflow_automation(&upsert(
                    id,
                    "daily",
                    daily_config(),
                    true,
                    Some("2026-06-26 09:00:00"),
                ))
                .expect("create");
        }
        // One unknown run blocks only its own automation.
        store
            .add_workflow_automation_run(&scheduled_run("a-two", "run-two", "needs_reconcile"))
            .expect("add run");

        let facade = facade_with_owner(&store, runtime_owner(None));
        let summary = facade
            .runtime_summary("2026-06-25 10:00:00")
            .expect("summary");
        assert_eq!(summary.effective_schedules, 2);
        assert_eq!(summary.blocked_reconcile, 1);
        assert!(summary.keep_alive);
        assert_eq!(summary.health, AutomationRuntimeHealth::Degraded);
    }

    #[test]
    fn admission_precedes_the_durable_claim_for_dispatched_and_manual_runs() {
        // The claim commits a run that another worker thread can observe before
        // the guard exists, so the admission must be recorded first. This locks
        // the ordering so it cannot silently regress.
        let app_src = include_str!("./application.rs");
        let dispatch_body = app_src
            .split("pub async fn automation_dispatch_due(")
            .nth(1)
            .and_then(|rest| {
                rest.split("/// The one-time startup schedule normalization")
                    .next()
            })
            .expect("automation_dispatch_due body");
        let admit = dispatch_body
            .find("begin_run(")
            .expect("dispatch must admit the run");
        let claim = dispatch_body
            .find("claim_due_automation_slot(")
            .expect("dispatch claims the slot");
        assert!(
            admit < claim,
            "dispatch must admit the run before claiming its slot"
        );

        let service_src = include_str!("./service.rs");
        let manual_body = service_src
            .split("pub(crate) async fn create_manual_run(")
            .nth(1)
            .and_then(|rest| rest.split("#[cfg(test)]").next())
            .expect("create_manual_run body");
        let admit = manual_body
            .find("begin_run(")
            .expect("manual must admit the run");
        let claim = manual_body
            .find("claim_manual_run(")
            .expect("manual claims the run");
        assert!(
            admit < claim,
            "manual must admit the run before claiming it"
        );
    }

    /// Polls a pid fixture written by an owned pre-workflow shell.
    #[cfg(unix)]
    async fn wait_for_owned_pid(path: &std::path::Path) -> i32 {
        for _ in 0..500 {
            if let Ok(text) = std::fs::read_to_string(path) {
                if let Ok(pid) = text.trim().parse::<i32>() {
                    return pid;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("owned pid fixture {} was never written", path.display());
    }

    #[cfg(unix)]
    fn owned_process_alive(pid: i32) -> bool {
        pid > 0 && unsafe { libc::kill(pid, 0) == 0 }
    }

    /// A run whose pre-workflow shell is cancelled by the coordinated shutdown
    /// must project terminal `cancelled` — never a deterministic-effect `failed`
    /// — and its whole owned tree must be gone.
    #[cfg(unix)]
    #[tokio::test]
    async fn runtime_shutdown_owned_cancel_marks_run_cancelled() {
        let (dir, store) = seeded_store(&["agent-main"]);
        let svc = full_service(dir.path(), store.clone());

        let fixture = dir.path().join("owned-grandchild.pid");
        let command = format!("sleep 30 & echo $! > {}; sleep 30", fixture.display());
        store
            .create_workflow_automation(&WorkflowAutomationUpsert {
                id: "a-cancel".to_string(),
                title: "Cancel".to_string(),
                prompt: Some("do work".to_string()),
                prompt_file_path: None,
                agent_id: "agent-main".to_string(),
                agent_config: Some("{}".to_string()),
                allowed_paths: "[]".to_string(),
                shell_config: Some(json!({ "command": command }).to_string()),
                schedule_kind: "once".to_string(),
                schedule_config: json!({ "run_at": "2999-01-01 00:00" }).to_string(),
                continuous_context: false,
                current_workflow_session_id: None,
                self_review: false,
                enabled: false,
                next_run_at: None,
            })
            .expect("create automation");

        let task = tokio::spawn({
            let svc = svc.clone();
            async move {
                crate::workflow::automation::service::create_manual_run(&svc, "a-cancel").await
            }
        });

        let grandchild = wait_for_owned_pid(&fixture).await;
        assert!(owned_process_alive(grandchild));

        let cancelled = svc.cancel_owned_automation_shells().await;
        assert_eq!(cancelled, 1, "the in-flight pre-shell must be cancelled");

        let (_automation, run) = task.await.expect("task").expect("manual run");
        assert_eq!(
            run.status, "cancelled",
            "a cancelled shell must not report a deterministic `failed`"
        );

        let runs = svc.automation().runs("a-cancel").expect("runs");
        let projected = runs
            .iter()
            .find(|view| view.run_id == run.id)
            .expect("projected run");
        assert_eq!(projected.status, "cancelled");

        assert!(
            !owned_process_alive(grandchild),
            "the owned tree must be terminated"
        );
    }
}
