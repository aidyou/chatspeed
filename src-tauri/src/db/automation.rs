use crate::db::{MainStore, StoreError};
use rusqlite::{params, OptionalExtension, Row, Transaction};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowAutomation {
    pub id: String,
    pub title: String,
    pub prompt: Option<String>,
    pub prompt_file_path: Option<String>,
    pub agent_id: String,
    pub agent_config: Option<String>,
    pub allowed_paths: String,
    pub shell_config: Option<String>,
    pub schedule_kind: String,
    pub schedule_config: String,
    pub continuous_context: bool,
    pub current_workflow_session_id: Option<String>,
    pub self_review: bool,
    pub enabled: bool,
    pub next_run_at: Option<String>,
    pub last_run_at: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    /// Monotonic compare-and-set revision, bumped on every mutation. Historical
    /// rows are back-filled to 1 by the v21 migration default.
    pub revision: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowAutomationRun {
    pub id: String,
    pub automation_id: String,
    pub workflow_session_id: Option<String>,
    pub status: String,
    pub scheduled_for: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub error: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    /// What started this run: `manual` or `scheduled`. Historical rows default
    /// to `manual` so legacy runs never collide with a scheduled dispatch key.
    pub trigger: String,
    /// Stable scheduled-slot key, unique per `(automation_id, dispatch_key)`.
    /// `None` for manual and legacy runs.
    pub dispatch_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WorkflowAutomationUpsert {
    pub id: String,
    pub title: String,
    pub prompt: Option<String>,
    pub prompt_file_path: Option<String>,
    pub agent_id: String,
    pub agent_config: Option<String>,
    pub allowed_paths: String,
    pub shell_config: Option<String>,
    pub schedule_kind: String,
    pub schedule_config: String,
    pub continuous_context: bool,
    pub current_workflow_session_id: Option<String>,
    pub self_review: bool,
    pub enabled: bool,
    pub next_run_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WorkflowAutomationRunInsert {
    pub id: String,
    pub automation_id: String,
    pub workflow_session_id: Option<String>,
    pub status: String,
    pub scheduled_for: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub error: Option<String>,
    pub trigger: String,
    pub dispatch_key: Option<String>,
}

/// Outcome of a compare-and-set mutation that must observe a specific revision.
#[derive(Debug, Clone)]
pub enum CasOutcome {
    /// The row advanced and the new projection is returned.
    Updated(WorkflowAutomation),
    /// No automation with that id exists.
    NotFound,
    /// A row exists but its revision moved past the expected value.
    RevisionConflict,
}

/// Outcome of an atomic scheduled-slot claim.
#[derive(Debug, Clone)]
pub enum ClaimOutcome {
    /// This caller advanced the schedule and inserted the scheduled run.
    Claimed(WorkflowAutomationRun),
    /// The automation is disabled, not due, or its revision/`next_run_at` moved
    /// since it was read, so the slot no longer belongs to this caller.
    NotEligible,
    /// A scheduled run for this exact slot already exists (durable dedupe).
    SlotTaken,
    /// An active or not-yet-provably-terminal run already exists for this
    /// automation, so the scheduler must not start a second overlapping run.
    /// The schedule is left untouched (AC-6/INV-6); the caller skips this tick.
    ActiveRunExists,
}

/// Outcome of an atomic manual-run claim (AC-6). The active-run guard and the
/// pending-row insert share one write transaction, so two concurrent manual
/// requests can never both observe "no active run" and overlap.
#[derive(Debug, Clone)]
pub enum ManualClaimOutcome {
    /// No active run existed; this caller owns the newly inserted pending run.
    Claimed(WorkflowAutomationRun),
    /// An active/unknown run already exists; the caller must not start another.
    Busy,
}

/// Outcome of reserving a durable mutation receipt.
#[derive(Debug, Clone)]
pub enum ReceiptOutcome {
    /// No prior receipt: the caller must execute the mutation and complete it.
    Proceed,
    /// A completed identical mutation exists; replay this stored result.
    Replay(Option<String>),
    /// The key was used by a different request body.
    Conflict,
}

impl From<&Row<'_>> for WorkflowAutomation {
    fn from(row: &Row<'_>) -> Self {
        Self {
            id: row.get("id").unwrap_or_default(),
            title: row.get("title").unwrap_or_default(),
            prompt: row.get("prompt").ok(),
            prompt_file_path: row.get("prompt_file_path").ok(),
            agent_id: row.get("agent_id").unwrap_or_default(),
            agent_config: row.get("agent_config").ok(),
            allowed_paths: row
                .get("allowed_paths")
                .unwrap_or_else(|_| "[]".to_string()),
            shell_config: row.get("shell_config").ok(),
            schedule_kind: row.get("schedule_kind").unwrap_or_default(),
            schedule_config: row
                .get("schedule_config")
                .unwrap_or_else(|_| "{}".to_string()),
            continuous_context: row.get::<_, i64>("continuous_context").unwrap_or(0) != 0,
            current_workflow_session_id: row.get("current_workflow_session_id").ok(),
            self_review: row.get::<_, i64>("self_review").unwrap_or(0) != 0,
            enabled: row.get::<_, i64>("enabled").unwrap_or(1) != 0,
            next_run_at: row.get("next_run_at").ok(),
            last_run_at: row.get("last_run_at").ok(),
            created_at: row.get("created_at").ok(),
            updated_at: row.get("updated_at").ok(),
            revision: row.get::<_, i64>("revision").unwrap_or(1),
        }
    }
}

impl From<&Row<'_>> for WorkflowAutomationRun {
    fn from(row: &Row<'_>) -> Self {
        Self {
            id: row.get("id").unwrap_or_default(),
            automation_id: row.get("automation_id").unwrap_or_default(),
            workflow_session_id: row.get("workflow_session_id").ok(),
            status: row.get("status").unwrap_or_default(),
            scheduled_for: row.get("scheduled_for").unwrap_or_default(),
            started_at: row.get("started_at").ok(),
            finished_at: row.get("finished_at").ok(),
            error: row.get("error").ok(),
            created_at: row.get("created_at").ok(),
            updated_at: row.get("updated_at").ok(),
            trigger: row
                .get::<_, String>("trigger")
                .unwrap_or_else(|_| "manual".to_string()),
            dispatch_key: row.get("dispatch_key").ok().flatten(),
        }
    }
}

impl MainStore {
    fn delete_workflow_tree_tx(tx: &Transaction<'_>, id: &str) -> Result<(), StoreError> {
        let workflow_ids = {
            let mut stmt = tx.prepare(
                "WITH RECURSIVE workflow_tree(id, depth) AS (
                    SELECT id, 0 FROM workflows WHERE id = ?1
                    UNION ALL
                    SELECT workflows.id, workflow_tree.depth + 1
                    FROM workflows
                    JOIN workflow_tree ON workflows.parent_session_id = workflow_tree.id
                )
                SELECT id FROM workflow_tree ORDER BY depth DESC",
            )?;
            let rows = stmt.query_map(params![id], |row| row.get::<_, String>(0))?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row?);
            }
            ids
        };

        for workflow_id in &workflow_ids {
            tx.execute(
                "DELETE FROM workflow_context_messages WHERE session_id = ?1",
                params![workflow_id],
            )?;

            tx.execute(
                "DELETE FROM workflow_messages WHERE session_id = ?1",
                params![workflow_id],
            )?;

            tx.execute(
                "DELETE FROM workflow_snapshots WHERE session_id = ?1",
                params![workflow_id],
            )?;

            if let Err(e) = tx.execute(
                "DELETE FROM workflow_events WHERE session_id = ?1",
                params![workflow_id],
            ) {
                log::error!(
                    "[Workflow][session={}] Failed to delete workflow events (non-fatal, continuing): {}",
                    workflow_id,
                    e
                );
            }
        }

        for workflow_id in &workflow_ids {
            tx.execute("DELETE FROM workflows WHERE id = ?1", params![workflow_id])?;
        }

        Ok(())
    }

    pub fn list_workflow_automations(&self) -> Result<Vec<WorkflowAutomation>, StoreError> {
        self.db_runtime()?.read_blocking(|conn| {
            let mut statement = conn.prepare(
                "SELECT * FROM workflow_automations
                 ORDER BY updated_at DESC, created_at DESC",
            )?;
            let rows = statement.query_map([], |row| Ok(WorkflowAutomation::from(row)))?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
    }

    pub fn get_workflow_automation(
        &self,
        id: &str,
    ) -> Result<Option<WorkflowAutomation>, StoreError> {
        let id = id.to_string();
        self.db_runtime()?.read_blocking(move |conn| {
            conn.query_row(
                "SELECT * FROM workflow_automations WHERE id = ?1",
                params![id],
                |row| Ok(WorkflowAutomation::from(row)),
            )
            .optional()
            .map_err(StoreError::from)
        })
    }

    pub fn delete_workflow_automation(&self, id: &str) -> Result<(), StoreError> {
        let id = id.to_string();
        self.db_runtime()?.write_blocking(move |conn| {
            let transaction = conn.transaction()?;
            let mut workflow_ids = {
                let mut statement = transaction.prepare(
                    "SELECT DISTINCT workflow_session_id
                     FROM workflow_automation_runs
                     WHERE automation_id = ?1 AND workflow_session_id IS NOT NULL",
                )?;
                let rows = statement.query_map(params![id], |row| row.get::<_, String>(0))?;
                let ids = rows.collect::<Result<Vec<_>, _>>()?;
                ids
            };
            if let Some(current_workflow_id) = transaction
                .query_row(
                    "SELECT current_workflow_session_id FROM workflow_automations WHERE id = ?1",
                    params![id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()?
                .flatten()
            {
                workflow_ids.push(current_workflow_id);
            }
            let mut seen = HashSet::new();
            workflow_ids.retain(|workflow_id| seen.insert(workflow_id.clone()));
            transaction.execute(
                "DELETE FROM workflow_automation_runs WHERE automation_id = ?1",
                params![id],
            )?;
            for workflow_id in &workflow_ids {
                Self::delete_workflow_tree_tx(&transaction, workflow_id)?;
            }
            transaction.execute(
                "DELETE FROM workflow_automations WHERE id = ?1",
                params![id],
            )?;
            transaction.commit()?;
            Ok(())
        })
    }

    pub fn update_workflow_automation_run_after_start(
        &self,
        automation_id: &str,
        run_id: &str,
        workflow_session_id: &str,
        scheduled_for: &str,
        current_workflow_session_id: Option<&str>,
    ) -> Result<(), StoreError> {
        let automation_id = automation_id.to_string();
        let run_id = run_id.to_string();
        let workflow_session_id = workflow_session_id.to_string();
        let scheduled_for = scheduled_for.to_string();
        let current_workflow_session_id = current_workflow_session_id.map(ToString::to_string);
        self.db_runtime()?.write_blocking(move |conn| {
            let transaction = conn.transaction()?;
            transaction.execute(
                "UPDATE workflow_automations
                 SET last_run_at = ?2, current_workflow_session_id = ?3, updated_at = CURRENT_TIMESTAMP
                 WHERE id = ?1",
                params![automation_id, scheduled_for, current_workflow_session_id],
            )?;
            transaction.execute(
                "UPDATE workflow_automation_runs
                 SET workflow_session_id = ?2, status = 'running', started_at = CURRENT_TIMESTAMP,
                     updated_at = CURRENT_TIMESTAMP
                 WHERE id = ?1",
                params![run_id, workflow_session_id],
            )?;
            transaction.commit()?;
            Ok(())
        })
    }

    pub fn add_workflow_automation_run(
        &self,
        run: &WorkflowAutomationRunInsert,
    ) -> Result<WorkflowAutomationRun, StoreError> {
        let run = run.clone();
        self.db_runtime()?.write_blocking(move |conn| {
            conn.execute(
                "INSERT INTO workflow_automation_runs
                 (id, automation_id, workflow_session_id, status, scheduled_for,
                  started_at, finished_at, error, trigger, dispatch_key)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    run.id,
                    run.automation_id,
                    run.workflow_session_id,
                    run.status,
                    run.scheduled_for,
                    run.started_at,
                    run.finished_at,
                    run.error,
                    run.trigger,
                    run.dispatch_key,
                ],
            )?;
            conn.query_row(
                "SELECT * FROM workflow_automation_runs WHERE id = ?1",
                params![run.id],
                |row| Ok(WorkflowAutomationRun::from(row)),
            )
            .map_err(StoreError::from)
        })
    }

    pub fn list_workflow_automation_runs(
        &self,
        automation_id: &str,
    ) -> Result<Vec<WorkflowAutomationRun>, StoreError> {
        let automation_id = automation_id.to_string();
        self.db_runtime()?.read_blocking(move |conn| {
            let mut statement = conn.prepare(
                "SELECT * FROM workflow_automation_runs
                 WHERE automation_id = ?1
                 ORDER BY created_at DESC",
            )?;
            let rows = statement.query_map(params![automation_id], |row| {
                Ok(WorkflowAutomationRun::from(row))
            })?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
    }

    /// Inserts a brand-new automation. The caller must have verified the id is
    /// free; a primary-key collision surfaces as a store error the facade maps
    /// to a conflict. Revision starts at the migration default (1).
    pub fn create_workflow_automation(
        &self,
        automation: &WorkflowAutomationUpsert,
    ) -> Result<WorkflowAutomation, StoreError> {
        let automation = automation.clone();
        self.db_runtime()?.write_blocking(move |conn| {
            conn.execute(
                "INSERT INTO workflow_automations
                 (id, title, prompt, prompt_file_path, agent_id, agent_config, allowed_paths,
                  shell_config, schedule_kind, schedule_config, continuous_context,
                  current_workflow_session_id, self_review, enabled, next_run_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    automation.id,
                    automation.title,
                    automation.prompt,
                    automation.prompt_file_path,
                    automation.agent_id,
                    automation.agent_config,
                    automation.allowed_paths,
                    automation.shell_config,
                    automation.schedule_kind,
                    automation.schedule_config,
                    automation.continuous_context as i64,
                    automation.current_workflow_session_id,
                    automation.self_review as i64,
                    automation.enabled as i64,
                    automation.next_run_at,
                ],
            )?;
            conn.query_row(
                "SELECT * FROM workflow_automations WHERE id = ?1",
                params![automation.id],
                |row| Ok(WorkflowAutomation::from(row)),
            )
            .map_err(StoreError::from)
        })
    }

    /// Applies a full automation update only when the stored revision still
    /// equals `expected_revision`, advancing it by one (compare-and-set). An
    /// unknown id yields `NotFound`; a moved revision yields `RevisionConflict`
    /// rather than silently overwriting a concurrent write (AC-5).
    pub fn update_workflow_automation_cas(
        &self,
        automation: &WorkflowAutomationUpsert,
        expected_revision: i64,
    ) -> Result<CasOutcome, StoreError> {
        let automation = automation.clone();
        self.db_runtime()?.write_blocking(move |conn| {
            let current: Option<i64> = conn
                .query_row(
                    "SELECT revision FROM workflow_automations WHERE id = ?1",
                    params![automation.id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?;
            match current {
                None => Ok(CasOutcome::NotFound),
                Some(revision) if revision != expected_revision => {
                    Ok(CasOutcome::RevisionConflict)
                }
                Some(_) => {
                    conn.execute(
                        "UPDATE workflow_automations SET
                            title = ?2, prompt = ?3, prompt_file_path = ?4, agent_id = ?5,
                            agent_config = ?6, allowed_paths = ?7, shell_config = ?8,
                            schedule_kind = ?9, schedule_config = ?10, continuous_context = ?11,
                            current_workflow_session_id = ?12, self_review = ?13, enabled = ?14,
                            next_run_at = ?15, revision = ?16, updated_at = CURRENT_TIMESTAMP
                         WHERE id = ?1",
                        params![
                            automation.id,
                            automation.title,
                            automation.prompt,
                            automation.prompt_file_path,
                            automation.agent_id,
                            automation.agent_config,
                            automation.allowed_paths,
                            automation.shell_config,
                            automation.schedule_kind,
                            automation.schedule_config,
                            automation.continuous_context as i64,
                            automation.current_workflow_session_id,
                            automation.self_review as i64,
                            automation.enabled as i64,
                            automation.next_run_at,
                            expected_revision + 1,
                        ],
                    )?;
                    conn.query_row(
                        "SELECT * FROM workflow_automations WHERE id = ?1",
                        params![automation.id],
                        |row| Ok(WorkflowAutomation::from(row)),
                    )
                    .map(CasOutcome::Updated)
                    .map_err(StoreError::from)
                }
            }
        })
    }

    /// Compare-and-set for an enable/disable flip, so a concurrent schedule
    /// advance or edit that moved the revision is reported instead of lost.
    pub fn set_workflow_automation_enabled_cas(
        &self,
        id: &str,
        enabled: bool,
        next_run_at: Option<String>,
        expected_revision: i64,
    ) -> Result<CasOutcome, StoreError> {
        let id = id.to_string();
        self.db_runtime()?.write_blocking(move |conn| {
            let current: Option<i64> = conn
                .query_row(
                    "SELECT revision FROM workflow_automations WHERE id = ?1",
                    params![id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?;
            match current {
                None => Ok(CasOutcome::NotFound),
                Some(revision) if revision != expected_revision => {
                    Ok(CasOutcome::RevisionConflict)
                }
                Some(_) => {
                    conn.execute(
                        "UPDATE workflow_automations
                         SET enabled = ?2, next_run_at = ?3, revision = ?4,
                             updated_at = CURRENT_TIMESTAMP
                         WHERE id = ?1",
                        params![id, enabled as i64, next_run_at, expected_revision + 1],
                    )?;
                    conn.query_row(
                        "SELECT * FROM workflow_automations WHERE id = ?1",
                        params![id],
                        |row| Ok(WorkflowAutomation::from(row)),
                    )
                    .map(CasOutcome::Updated)
                    .map_err(StoreError::from)
                }
            }
        })
    }

    /// Advances an enabled automation's schedule and records the scheduled run
    /// in one transaction, so no half-advanced slot can be observed. The claim
    /// is conditional on `enabled`, the previously read `next_run_at` and
    /// `revision`; if any moved, another actor owns the slot (`NotEligible`).
    /// The durable `(automation_id, dispatch_key)` index makes a re-attempt of
    /// the same slot report `SlotTaken` instead of a second run (AC-6/INV-6).
    pub fn claim_due_automation_slot(
        &self,
        automation_id: &str,
        expected_revision: i64,
        expected_next_run_at: &str,
        new_next_run_at: Option<String>,
        dispatch_key: &str,
        run_id: &str,
        scheduled_for: &str,
    ) -> Result<ClaimOutcome, StoreError> {
        let automation_id = automation_id.to_string();
        let expected_next_run_at = expected_next_run_at.to_string();
        let dispatch_key = dispatch_key.to_string();
        let run_id = run_id.to_string();
        let scheduled_for = scheduled_for.to_string();
        self.db_runtime()?.write_blocking(move |conn| {
            let transaction = conn.transaction()?;
            let advanced = transaction.execute(
                "UPDATE workflow_automations
                 SET next_run_at = ?2, revision = revision + 1, updated_at = CURRENT_TIMESTAMP
                 WHERE id = ?1 AND enabled = 1 AND revision = ?3 AND next_run_at = ?4",
                params![automation_id, new_next_run_at, expected_revision, expected_next_run_at],
            )?;
            if advanced != 1 {
                // The slot no longer belongs to this caller; drop the tx (no
                // commit) so nothing is advanced or inserted.
                return Ok(ClaimOutcome::NotEligible);
            }
            // We won the slot, but a manual (or prior) run may already be active.
            // Because the CAS above and this count share one write transaction,
            // a run claimed in any other committed transaction is always visible
            // here. On overlap we return without committing, so the schedule
            // advance rolls back and this automation stays due for the next tick
            // (AC-6/INV-6): the scheduler never starts a second concurrent run.
            let active_runs: i64 = transaction.query_row(
                "SELECT COUNT(1) FROM workflow_automation_runs
                 WHERE automation_id = ?1
                   AND status IN ('pending','starting','running','needs_reconcile')",
                params![automation_id],
                |row| row.get(0),
            )?;
            if active_runs > 0 {
                return Ok(ClaimOutcome::ActiveRunExists);
            }
            match transaction.execute(
                "INSERT INTO workflow_automation_runs
                 (id, automation_id, status, scheduled_for, trigger, dispatch_key)
                 VALUES (?1, ?2, 'pending', ?3, 'scheduled', ?4)",
                params![run_id, automation_id, scheduled_for, dispatch_key],
            ) {
                Ok(_) => {}
                Err(error) if is_unique_constraint(&error) => {
                    // Scheduled run for this slot already exists; roll back the
                    // advance so the schedule is not double-advanced.
                    return Ok(ClaimOutcome::SlotTaken);
                }
                Err(error) => return Err(StoreError::from(error)),
            }
            let run = transaction.query_row(
                "SELECT * FROM workflow_automation_runs WHERE id = ?1",
                params![run_id],
                |row| Ok(WorkflowAutomationRun::from(row)),
            )?;
            transaction.commit()?;
            Ok(ClaimOutcome::Claimed(run))
        })
    }

    /// Atomically claims the right to start a manual run. In a single write
    /// transaction it counts active/unknown runs and, only when none exists,
    /// inserts the pending manual run. Concurrent manual requests therefore
    /// cannot both pass the guard (AC-6/INV-6); the loser receives `Busy`.
    pub fn claim_manual_run(
        &self,
        automation_id: &str,
        run_id: &str,
        scheduled_for: &str,
    ) -> Result<ManualClaimOutcome, StoreError> {
        let automation_id = automation_id.to_string();
        let run_id = run_id.to_string();
        let scheduled_for = scheduled_for.to_string();
        self.db_runtime()?.write_blocking(move |conn| {
            let transaction = conn.transaction()?;
            let active_runs: i64 = transaction.query_row(
                "SELECT COUNT(1) FROM workflow_automation_runs
                 WHERE automation_id = ?1
                   AND status IN ('pending','starting','running','needs_reconcile')",
                params![automation_id],
                |row| row.get(0),
            )?;
            if active_runs > 0 {
                return Ok(ManualClaimOutcome::Busy);
            }
            transaction.execute(
                "INSERT INTO workflow_automation_runs
                 (id, automation_id, status, scheduled_for, trigger, dispatch_key)
                 VALUES (?1, ?2, 'pending', ?3, 'manual', NULL)",
                params![run_id, automation_id, scheduled_for],
            )?;
            let run = transaction.query_row(
                "SELECT * FROM workflow_automation_runs WHERE id = ?1",
                params![run_id],
                |row| Ok(WorkflowAutomationRun::from(row)),
            )?;
            transaction.commit()?;
            Ok(ManualClaimOutcome::Claimed(run))
        })
    }

    /// Reserves (or replays) a durable mutation receipt for the given idempotency
    /// scope and key. A completed identical mutation replays without re-running;
    /// a different request hash under the same key conflicts (AC-5).
    pub fn reserve_automation_mutation(
        &self,
        actor_scope: &str,
        idempotency_key: &str,
        operation: &str,
        request_hash: &str,
    ) -> Result<ReceiptOutcome, StoreError> {
        let actor_scope = actor_scope.to_string();
        let idempotency_key = idempotency_key.to_string();
        let operation = operation.to_string();
        let request_hash = request_hash.to_string();
        self.db_runtime()?.write_blocking(move |conn| {
            let now = crate::capability::operation::now_ms();
            conn.execute(
                "INSERT OR IGNORE INTO automation_operations
                 (operation_id, actor_scope, operation, idempotency_key, request_hash,
                  status, created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'in_progress', ?6, ?6)",
                params![
                    format!("autom-op-{}", uuid::Uuid::now_v7()),
                    actor_scope,
                    operation,
                    idempotency_key,
                    request_hash,
                    now,
                ],
            )?;
            let (status, stored_hash, result_json): (String, String, Option<String>) = conn
                .query_row(
                    "SELECT status, request_hash, result_json FROM automation_operations
                     WHERE actor_scope = ?1 AND idempotency_key = ?2",
                    params![actor_scope, idempotency_key],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )?;
            if stored_hash != request_hash {
                return Ok(ReceiptOutcome::Conflict);
            }
            match status.as_str() {
                "completed" => Ok(ReceiptOutcome::Replay(result_json)),
                // An `in_progress` receipt with the same hash is either a live
                // concurrent attempt (guarded by the transport in-flight tracker)
                // or a crash leftover; the underlying writes are revision CAS
                // and dispatch-unique guarded, so re-running cannot duplicate an
                // effect. The caller proceeds and completes the receipt.
                _ => Ok(ReceiptOutcome::Proceed),
            }
        })
    }

    /// Marks a reserved mutation receipt completed with a redacted result
    /// reference so a later identical retry replays instead of re-executing.
    pub fn complete_automation_mutation(
        &self,
        actor_scope: &str,
        idempotency_key: &str,
        result_json: Option<&str>,
    ) -> Result<(), StoreError> {
        let actor_scope = actor_scope.to_string();
        let idempotency_key = idempotency_key.to_string();
        let result_json = result_json.map(str::to_string);
        self.db_runtime()?.write_blocking(move |conn| {
            conn.execute(
                "UPDATE automation_operations
                 SET status = 'completed', result_json = ?3, updated_at_ms = ?4
                 WHERE actor_scope = ?1 AND idempotency_key = ?2",
                params![actor_scope, idempotency_key, result_json, crate::capability::operation::now_ms()],
            )?;
            Ok(())
        })
    }

    /// Whether an automation currently owns a run whose effect is not yet
    /// provably terminal. Delete refuses these, and manual run refuses overlap
    /// (AC-6/AC-10). `needs_reconcile` blocks too because its outcome is unknown.
    pub fn automation_has_blocking_run(&self, automation_id: &str) -> Result<bool, StoreError> {
        let automation_id = automation_id.to_string();
        self.db_runtime()?.read_blocking(move |conn| {
            let count: i64 = conn.query_row(
                "SELECT COUNT(1) FROM workflow_automation_runs
                 WHERE automation_id = ?1
                   AND status IN ('pending','starting','running','needs_reconcile')",
                params![automation_id],
                |row| row.get(0),
            )?;
            Ok(count > 0)
        })
    }

    /// The enabled automations whose `next_run_at` is due at or before `now`.
    /// `dispatch_due` reads this to find candidate slots, then CAS-claims each.
    pub fn list_due_workflow_automations(
        &self,
        now: &str,
    ) -> Result<Vec<WorkflowAutomation>, StoreError> {
        let now = now.to_string();
        self.db_runtime()?.read_blocking(move |conn| {
            let mut statement = conn.prepare(
                "SELECT * FROM workflow_automations
                 WHERE enabled = 1 AND next_run_at IS NOT NULL AND next_run_at <= ?1
                 ORDER BY next_run_at ASC",
            )?;
            let rows = statement.query_map(params![now], |row| Ok(WorkflowAutomation::from(row)))?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
    }

    /// Fetches one run by id (for terminal projection).
    pub fn get_workflow_automation_run(
        &self,
        run_id: &str,
    ) -> Result<Option<WorkflowAutomationRun>, StoreError> {
        let run_id = run_id.to_string();
        self.db_runtime()?.read_blocking(move |conn| {
            conn.query_row(
                "SELECT * FROM workflow_automation_runs WHERE id = ?1",
                params![run_id],
                |row| Ok(WorkflowAutomationRun::from(row)),
            )
            .optional()
            .map_err(StoreError::from)
        })
    }

    /// The durable workflow snapshot lifecycle state for a session, or `None`
    /// when no snapshot exists yet. This is the structured terminal authority
    /// the automation run projection reads (INV-7): a snapshot row records
    /// `RuntimeState` as a snake_case string, and only `completed`/`failed`/
    /// `cancelled` are provably terminal. Transcript text is never consulted.
    pub fn workflow_snapshot_state(
        &self,
        session_id: &str,
    ) -> Result<Option<String>, StoreError> {
        let session_id = session_id.to_string();
        self.db_runtime()?.read_blocking(move |conn| {
            conn.query_row(
                "SELECT state FROM workflow_snapshots WHERE session_id = ?1",
                params![session_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map(|value| value.flatten())
            .map_err(StoreError::from)
        })
    }

    /// Whether a workflow row exists for the session. Used to tell a run whose
    /// session was deleted (unknown effect) from one merely awaiting start.
    pub fn workflow_session_exists(&self, session_id: &str) -> Result<bool, StoreError> {
        let session_id = session_id.to_string();
        self.db_runtime()?.read_blocking(move |conn| {
            let count: i64 = conn.query_row(
                "SELECT COUNT(1) FROM workflows WHERE id = ?1",
                params![session_id],
                |row| row.get(0),
            )?;
            Ok(count > 0)
        })
    }

    /// All not-yet-terminal runs, used for bounded reconciliation at startup
    /// and on read.
    pub fn list_unreconciled_automation_runs(&self) -> Result<Vec<WorkflowAutomationRun>, StoreError> {
        self.db_runtime()?.read_blocking(|conn| {
            let mut statement = conn.prepare(
                "SELECT * FROM workflow_automation_runs
                 WHERE status IN ('pending','starting','running')
                 ORDER BY created_at ASC",
            )?;
            let rows =
                statement.query_map([], |row| Ok(WorkflowAutomationRun::from(row)))?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
    }

    /// Sets a run's lifecycle status within the automation projection. Terminal
    /// transitions also stamp `finished_at`.
    pub fn update_workflow_automation_run_status(
        &self,
        run_id: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<(), StoreError> {
        let run_id = run_id.to_string();
        let status = status.to_string();
        let error = error.map(str::to_string);
        self.db_runtime()?.write_blocking(move |conn| {
            let terminal = matches!(status.as_str(), "completed" | "failed" | "cancelled");
            if terminal {
                conn.execute(
                    "UPDATE workflow_automation_runs
                     SET status = ?2, error = ?3, finished_at = CURRENT_TIMESTAMP,
                         updated_at = CURRENT_TIMESTAMP
                     WHERE id = ?1",
                    params![run_id, status, error],
                )?;
            } else {
                conn.execute(
                    "UPDATE workflow_automation_runs
                     SET status = ?2, error = ?3, updated_at = CURRENT_TIMESTAMP
                     WHERE id = ?1",
                    params![run_id, status, error],
                )?;
            }
            Ok(())
        })
    }
}

/// Whether a rusqlite error is a unique/primary-key constraint violation. Used
/// to distinguish a durable scheduled-slot collision (`SlotTaken`) from a real
/// store failure without matching on the full message.
fn is_unique_constraint(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(inner, _)
            if inner.code == rusqlite::ErrorCode::ConstraintViolation
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::workflow::WorkflowMessage;
    use crate::db::MainStore;
    use crate::db::StoreError;
    use tempfile::tempdir;

    fn create_test_store() -> (tempfile::TempDir, MainStore) {
        let dir = tempdir().expect("failed to create temp dir");
        let db_path = dir.path().join("automation_test.db");
        let store = MainStore::new(db_path).expect("failed to create MainStore");
        (dir, store)
    }

    fn seed_agent(store: &MainStore, id: &str) {
        let id = id.to_string();
        store
            .db_runtime()
            .expect("failed to obtain database runtime")
            .write_blocking(move |conn| {
                conn.execute(
                    "INSERT INTO agents (id, name, system_prompt, agent_type, max_contexts)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        id,
                        format!("Agent {}", id),
                        "You are a test agent.",
                        "autonomous",
                        20
                    ],
                )?;
                Ok(())
            })
            .expect("failed to seed agent");
    }

    #[test]
    fn test_delete_workflow_automation_removes_associated_workflows() -> Result<(), StoreError> {
        let (_temp_dir, store) = create_test_store();
        seed_agent(&store, "agent-main");
        seed_agent(&store, "agent-child");

        store.create_workflow("auto-session", "Auto task", "agent-main", None, None)?;
        store.create_workflow(
            "auto-child-session",
            "Auto child task",
            "agent-child",
            None,
            Some("auto-session"),
        )?;

        store.add_workflow_message(&WorkflowMessage {
            id: None,
            session_id: "auto-session".to_string(),
            role: "assistant".to_string(),
            message: "hello".to_string(),
            reasoning: None,
            message_kind: "message".to_string(),
            message_subtype: None,
            segment_id: 1,
            source_event_type: None,
            metadata: None,
            attached_context: None,
            step_type: None,
            step_index: 0,
            is_error: false,
            error_type: None,
            created_at: None,
        })?;

        let automation = WorkflowAutomationUpsert {
            id: "automation-1".to_string(),
            title: "Automation".to_string(),
            prompt: Some("run".to_string()),
            prompt_file_path: None,
            agent_id: "agent-main".to_string(),
            agent_config: Some("{}".to_string()),
            allowed_paths: "[]".to_string(),
            shell_config: None,
            schedule_kind: "daily".to_string(),
            schedule_config: "{}".to_string(),
            continuous_context: true,
            current_workflow_session_id: Some("auto-session".to_string()),
            self_review: false,
            enabled: true,
            next_run_at: None,
        };
        store.create_workflow_automation(&automation)?;
        store.add_workflow_automation_run(&WorkflowAutomationRunInsert {
            id: "run-1".to_string(),
            automation_id: "automation-1".to_string(),
            workflow_session_id: None,
            status: "completed".to_string(),
            scheduled_for: "2026-06-27 10:00:00".to_string(),
            started_at: None,
            finished_at: None,
            error: None,
            trigger: "manual".to_string(),
            dispatch_key: None,
        })?;

        store.delete_workflow_automation("automation-1")?;

        assert!(store.get_workflow("auto-session")?.is_none());
        assert!(store.get_workflow("auto-child-session")?.is_none());
        assert!(store.get_workflow_automation("automation-1")?.is_none());
        assert!(store
            .list_workflow_automation_runs("automation-1")?
            .is_empty());

        Ok(())
    }

    #[test]
    fn update_cas_advances_revision_and_rejects_stale() -> Result<(), StoreError> {
        let (_dir, store) = create_test_store();
        seed_agent(&store, "agent-main");
        let upsert = WorkflowAutomationUpsert {
            id: "a-cas".to_string(),
            title: "v1".to_string(),
            prompt: Some("p".to_string()),
            prompt_file_path: None,
            agent_id: "agent-main".to_string(),
            agent_config: Some("{}".to_string()),
            allowed_paths: "[]".to_string(),
            shell_config: None,
            schedule_kind: "interval".to_string(),
            schedule_config: "{\"interval_minutes\":60}".to_string(),
            continuous_context: false,
            current_workflow_session_id: None,
            self_review: false,
            enabled: false,
            next_run_at: None,
        };
        let created = store.create_workflow_automation(&upsert)?;
        assert_eq!(created.revision, 1);

        // Stale revision is rejected rather than silently overwriting.
        let mut stale = upsert.clone();
        stale.title = "stale".to_string();
        match store.update_workflow_automation_cas(&stale, 99)? {
            CasOutcome::RevisionConflict => {}
            other => panic!("expected revision conflict, got {other:?}"),
        }

        let mut current = upsert.clone();
        current.title = "v2".to_string();
        match store.update_workflow_automation_cas(&current, 1)? {
            CasOutcome::Updated(row) => {
                assert_eq!(row.revision, 2);
                assert_eq!(row.title, "v2");
            }
            other => panic!("expected update, got {other:?}"),
        }
        Ok(())
    }

    #[test]
    fn scheduled_slot_claim_is_atomic_and_unique() -> Result<(), StoreError> {
        let (_dir, store) = create_test_store();
        seed_agent(&store, "agent-main");
        let upsert = WorkflowAutomationUpsert {
            id: "a-slot".to_string(),
            title: "slot".to_string(),
            prompt: Some("p".to_string()),
            prompt_file_path: None,
            agent_id: "agent-main".to_string(),
            agent_config: Some("{}".to_string()),
            allowed_paths: "[]".to_string(),
            shell_config: None,
            schedule_kind: "interval".to_string(),
            schedule_config: "{\"interval_minutes\":60}".to_string(),
            continuous_context: false,
            current_workflow_session_id: None,
            self_review: false,
            enabled: true,
            next_run_at: Some("2026-06-25 09:00:00".to_string()),
        };
        store.create_workflow_automation(&upsert)?;

        // First claim of the due slot wins and inserts one scheduled run.
        match store.claim_due_automation_slot(
            "a-slot",
            1,
            "2026-06-25 09:00:00",
            Some("2026-06-25 10:00:00".to_string()),
            "2026-06-25 09:00:00",
            "run-1",
            "2026-06-25 09:00:00",
        )? {
            ClaimOutcome::Claimed(run) => {
                assert_eq!(run.trigger, "scheduled");
                assert_eq!(run.dispatch_key.as_deref(), Some("2026-06-25 09:00:00"));
            }
            other => panic!("expected claim, got {other:?}"),
        }

        // A stale re-attempt (same slot after the schedule advanced) is refused
        // and never double-advances the schedule or duplicates the run.
        let second = store.claim_due_automation_slot(
            "a-slot",
            1,
            "2026-06-25 09:00:00",
            Some("2026-06-25 10:00:00".to_string()),
            "2026-06-25 09:00:00",
            "run-2",
            "2026-06-25 09:00:00",
        )?;
        assert!(
            matches!(second, ClaimOutcome::NotEligible | ClaimOutcome::SlotTaken),
            "stale duplicate claim must be refused, got {second:?}"
        );
        assert_eq!(store.list_workflow_automation_runs("a-slot")?.len(), 1);
        Ok(())
    }

    #[test]
    fn mutation_receipt_replays_same_hash_and_conflicts_on_other() -> Result<(), StoreError> {
        let (_dir, store) = create_test_store();
        assert!(matches!(
            store.reserve_automation_mutation("control-plane", "key-1", "create", "hash-a")?,
            ReceiptOutcome::Proceed
        ));
        store.complete_automation_mutation("control-plane", "key-1", Some("{\"automation_id\":\"x\",\"revision\":1}"))?;
        assert!(matches!(
            store.reserve_automation_mutation("control-plane", "key-1", "create", "hash-a")?,
            ReceiptOutcome::Replay(_)
        ));
        assert!(matches!(
            store.reserve_automation_mutation("control-plane", "key-1", "create", "hash-b")?,
            ReceiptOutcome::Conflict
        ));
        Ok(())
    }

    #[test]
    fn blocking_run_detects_active_statuses() -> Result<(), StoreError> {
        let (_dir, store) = create_test_store();
        seed_agent(&store, "agent-main");
        store.create_workflow_automation(&WorkflowAutomationUpsert {
            id: "a-block".to_string(),
            title: "block".to_string(),
            prompt: Some("p".to_string()),
            prompt_file_path: None,
            agent_id: "agent-main".to_string(),
            agent_config: Some("{}".to_string()),
            allowed_paths: "[]".to_string(),
            shell_config: None,
            schedule_kind: "interval".to_string(),
            schedule_config: "{\"interval_minutes\":60}".to_string(),
            continuous_context: false,
            current_workflow_session_id: None,
            self_review: false,
            enabled: false,
            next_run_at: None,
        })?;
        assert!(!store.automation_has_blocking_run("a-block")?);
        store.add_workflow_automation_run(&WorkflowAutomationRunInsert {
            id: "run-active".to_string(),
            automation_id: "a-block".to_string(),
            workflow_session_id: None,
            status: "running".to_string(),
            scheduled_for: "2026-06-25 09:00:00".to_string(),
            started_at: None,
            finished_at: None,
            error: None,
            trigger: "manual".to_string(),
            dispatch_key: None,
        })?;
        assert!(store.automation_has_blocking_run("a-block")?);
        Ok(())
    }

    #[test]
    fn manual_claim_is_atomic_and_busy_on_overlap() -> Result<(), StoreError> {
        let (_dir, store) = create_test_store();
        seed_agent(&store, "agent-main");
        store.create_workflow_automation(&WorkflowAutomationUpsert {
            id: "a-manual".to_string(),
            title: "manual".to_string(),
            prompt: Some("p".to_string()),
            prompt_file_path: None,
            agent_id: "agent-main".to_string(),
            agent_config: Some("{}".to_string()),
            allowed_paths: "[]".to_string(),
            shell_config: None,
            schedule_kind: "interval".to_string(),
            schedule_config: "{\"interval_minutes\":60}".to_string(),
            continuous_context: true,
            current_workflow_session_id: None,
            self_review: false,
            enabled: false,
            next_run_at: None,
        })?;

        // First manual claim wins and inserts one pending manual run.
        match store.claim_manual_run("a-manual", "run-1", "2026-06-25 09:00:00")? {
            ManualClaimOutcome::Claimed(run) => {
                assert_eq!(run.status, "pending");
                assert_eq!(run.trigger, "manual");
                assert!(run.dispatch_key.is_none());
            }
            other => panic!("expected claim, got {other:?}"),
        }

        // A concurrent second manual request sees the still-active run and is
        // refused; only one run exists (AC-6/INV-6).
        assert!(matches!(
            store.claim_manual_run("a-manual", "run-2", "2026-06-25 09:00:05")?,
            ManualClaimOutcome::Busy
        ));
        assert_eq!(store.list_workflow_automation_runs("a-manual")?.len(), 1);

        // Once the prior run is provably terminal a new manual run may claim.
        store.update_workflow_automation_run_status("run-1", "completed", None)?;
        assert!(matches!(
            store.claim_manual_run("a-manual", "run-3", "2026-06-25 09:30:00")?,
            ManualClaimOutcome::Claimed(_)
        ));
        Ok(())
    }

    #[test]
    fn scheduler_claim_skips_when_active_run_exists() -> Result<(), StoreError> {
        let (_dir, store) = create_test_store();
        seed_agent(&store, "agent-main");
        store.create_workflow_automation(&WorkflowAutomationUpsert {
            id: "a-race".to_string(),
            title: "race".to_string(),
            prompt: Some("p".to_string()),
            prompt_file_path: None,
            agent_id: "agent-main".to_string(),
            agent_config: Some("{}".to_string()),
            allowed_paths: "[]".to_string(),
            shell_config: None,
            schedule_kind: "interval".to_string(),
            schedule_config: "{\"interval_minutes\":60}".to_string(),
            continuous_context: true,
            current_workflow_session_id: None,
            self_review: false,
            enabled: true,
            next_run_at: Some("2026-06-25 09:00:00".to_string()),
        })?;

        // A manual run is already active for this automation.
        store.add_workflow_automation_run(&WorkflowAutomationRunInsert {
            id: "run-active".to_string(),
            automation_id: "a-race".to_string(),
            workflow_session_id: None,
            status: "running".to_string(),
            scheduled_for: "2026-06-25 09:00:00".to_string(),
            started_at: None,
            finished_at: None,
            error: None,
            trigger: "manual".to_string(),
            dispatch_key: None,
        })?;

        // The scheduler's due claim must refuse to overlap and must not advance
        // the schedule or insert a second run (AC-6/INV-6).
        let outcome = store.claim_due_automation_slot(
            "a-race",
            1,
            "2026-06-25 09:00:00",
            Some("2026-06-25 10:00:00".to_string()),
            "2026-06-25 09:00:00",
            "run-scheduled",
            "2026-06-25 09:00:00",
        )?;
        assert!(
            matches!(outcome, ClaimOutcome::ActiveRunExists),
            "expected active-run skip, got {outcome:?}"
        );
        assert_eq!(store.list_workflow_automation_runs("a-race")?.len(), 1);
        let automation = store
            .get_workflow_automation("a-race")?
            .expect("automation exists");
        assert_eq!(automation.next_run_at.as_deref(), Some("2026-06-25 09:00:00"));
        assert_eq!(automation.revision, 1);
        Ok(())
    }
}
