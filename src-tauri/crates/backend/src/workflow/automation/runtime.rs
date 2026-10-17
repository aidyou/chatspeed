//! Short-lived, in-memory execution evidence for the running automation owner.
//!
//! The durable `workflow_automation_runs` row plus the workflow snapshot is the
//! recovery authority, but between the durable claim and the moment the workflow
//! session is linked there is a window in which a run has no session yet. A
//! concurrent facade read (`runs`) or `reconcile_all` that observed only the
//! durable state would wrongly converge such a run to `needs_reconcile`.
//!
//! This module is the one place that records "this instance is executing run X
//! right now" as *short-lived* evidence: the `AutomationRuntimeOwner` keeps the
//! set of in-flight run ids and a weak handle to the live [`WorkflowManager`]
//! sessions. It never copies the manager, is never persisted, and never becomes
//! a second session registry — it only lets the facade verify the current guard
//! and the live manager before it declares an effect unknown.
//!
//! Admission is recorded *before* the durable slot claim and released by an RAII
//! guard, so a run stays protected across the whole claim -> shell -> session
//! link -> start window. Recording it after the claim would leave a window in
//! which a concurrent read on another worker thread observes the durable run
//! before the guard exists and wrongly converges it to `needs_reconcile`.

use crate::owned_process::OwnedProcessCancellation;
use crate::workflow::react::manager::{ManagedSessionStatus, WorkflowManager};
use chrono::{DateTime, Local};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

/// The runtime instance's short-lived automation execution evidence.
pub struct AutomationRuntimeOwner {
    /// Run ids admitted by this instance whose session has not been linked yet.
    in_flight: Mutex<HashSet<String>>,
    /// The live workflow manager, observed weakly so the owner never keeps it
    /// alive and a standalone facade can run without a manager at all.
    manager: Weak<WorkflowManager>,
    /// The instance start boundary; `None` until the daemon marks startup. A due
    /// slot at or after this boundary became due while this instance was online,
    /// so it may be consumed; an earlier slot was missed while offline.
    startup_cutoff: Mutex<Option<DateTime<Local>>>,
    /// Whether the one-time startup normalization finished successfully. A failed
    /// pass leaves this unset so it can be retried with the same boundary.
    startup_pass_done: AtomicBool,
    /// The one runtime-wide cancellation signal for owned child processes: the
    /// pre-workflow automation shells this instance started. It is short-lived
    /// and never persisted, and each shell holds an RAII ticket on it, so the
    /// coordinated shutdown can cancel every in-flight tree and *await* the
    /// teardown.
    owned_processes: OwnedProcessCancellation,
}

impl AutomationRuntimeOwner {
    pub fn new(manager: Weak<WorkflowManager>) -> Self {
        Self {
            in_flight: Mutex::new(HashSet::new()),
            manager,
            startup_cutoff: Mutex::new(None),
            startup_pass_done: AtomicBool::new(false),
            owned_processes: OwnedProcessCancellation::new(),
        }
    }

    /// Registers `run_id` as in flight; the returned guard clears it on drop, so
    /// the evidence never outlives the executing call.
    pub fn begin_run(self: &Arc<Self>, run_id: &str) -> AutomationRunGuard {
        lock_in_flight(&self.in_flight).insert(run_id.to_string());
        AutomationRunGuard {
            owner: Arc::clone(self),
            run_id: run_id.to_string(),
            active: true,
        }
    }

    pub fn is_in_flight(&self, run_id: &str) -> bool {
        lock_in_flight(&self.in_flight).contains(run_id)
    }

    pub fn in_flight_count(&self) -> usize {
        lock_in_flight(&self.in_flight).len()
    }

    /// The live manager status for a session. A missing manager is an observation
    /// failure, not "not live": the caller must not read it as a zero count.
    pub fn manager_status(&self, session_id: &str) -> Result<Option<ManagedSessionStatus>, String> {
        let manager = self
            .manager
            .upgrade()
            .ok_or_else(|| "workflow manager is unavailable".to_string())?;
        Ok(manager.get_session_status(session_id))
    }

    /// The one runtime-wide cancellation signal for owned child processes. Each
    /// pre-workflow shell clones it, so a coordinated shutdown can reach every
    /// in-flight tree through this single handle.
    pub fn owned_process_cancellation(&self) -> OwnedProcessCancellation {
        self.owned_processes.clone()
    }

    /// Requests cancellation of every in-flight owned shell and awaits each
    /// tree's teardown, reporting how many were in flight. The caller's deadline
    /// bounds the wait, so a shell that cannot finish never blocks the shutdown
    /// past the shared budget.
    pub async fn cancel_owned_processes(&self) -> usize {
        let in_flight = self.owned_processes.in_flight();
        self.owned_processes.cancel_all_and_wait().await;
        in_flight
    }

    /// Records the instance start boundary once. The first cutoff wins, so a
    /// normalization pass retried after a failure keeps the original boundary
    /// instead of silently advancing to a later one.
    pub fn mark_startup(&self, cutoff: DateTime<Local>) -> bool {
        let mut guard = self
            .startup_cutoff
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if guard.is_some() {
            return false;
        }
        *guard = Some(cutoff);
        true
    }

    /// Whether the one-time startup normalization already completed. Only a
    /// successful pass sets this, so a failed pass is retried with the same
    /// boundary instead of being reported as an already-done no-op.
    pub fn startup_pass_completed(&self) -> bool {
        self.startup_pass_done.load(Ordering::SeqCst)
    }

    /// Marks the one-time startup normalization complete. Called only after the
    /// pass finished without error.
    pub fn complete_startup_pass(&self) {
        self.startup_pass_done.store(true, Ordering::SeqCst);
    }

    pub fn startup_cutoff(&self) -> Option<DateTime<Local>> {
        *self
            .startup_cutoff
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl std::fmt::Debug for AutomationRuntimeOwner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AutomationRuntimeOwner")
            .field("in_flight", &self.in_flight_count())
            .field(
                "owned_processes_in_flight",
                &self.owned_processes.in_flight(),
            )
            .field("manager_alive", &(self.manager.strong_count() > 0))
            .field("startup_cutoff", &self.startup_cutoff())
            .finish()
    }
}

/// RAII evidence that this instance is executing a run in its pre-session window.
pub struct AutomationRunGuard {
    owner: Arc<AutomationRuntimeOwner>,
    run_id: String,
    active: bool,
}

impl AutomationRunGuard {
    /// Releases the in-flight evidence without waiting for drop. Retained for a
    /// caller that hands execution off to a longer-lived owner.
    pub fn release(&mut self) {
        if self.active {
            lock_in_flight(&self.owner.in_flight).remove(&self.run_id);
            self.active = false;
        }
    }
}

impl Drop for AutomationRunGuard {
    fn drop(&mut self) {
        if self.active {
            lock_in_flight(&self.owner.in_flight).remove(&self.run_id);
        }
    }
}

fn lock_in_flight(set: &Mutex<HashSet<String>>) -> std::sync::MutexGuard<'_, HashSet<String>> {
    set.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Whether a managed session is genuinely live (actively running, stopping, or
/// waiting for a signal). A terminal manager status — including a completed
/// session the manager still hot-caches — is not live.
pub fn session_status_is_live(status: &ManagedSessionStatus) -> bool {
    matches!(
        status,
        ManagedSessionStatus::Active
            | ManagedSessionStatus::Waiting
            | ManagedSessionStatus::Stopping
    )
}

/// The automation-run lifecycle status a terminal managed session projects to.
pub fn manager_terminal_status(status: &ManagedSessionStatus) -> Option<&'static str> {
    match status {
        ManagedSessionStatus::Completed => Some("completed"),
        ManagedSessionStatus::Failed => Some("failed"),
        ManagedSessionStatus::Cancelled => Some("cancelled"),
        ManagedSessionStatus::Active
        | ManagedSessionStatus::Waiting
        | ManagedSessionStatus::Stopping => None,
    }
}
