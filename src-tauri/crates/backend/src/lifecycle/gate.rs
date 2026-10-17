//! The admission gate and its RAII ticket.
//!
//! See [`crate::lifecycle`] for the design contract. This file owns the concrete
//! forward-only lifecycle state machine, the two epochs (the close epoch and the
//! activity epoch), the active-admission counter and the watch channels the
//! daemon observes.

use crate::lifecycle::traits::WorkAdmission;
use crate::lifecycle::types::{AdmissionDenied, GateSnapshot, WorkKind};
use chatspeed_contracts::RuntimeLifecyclePhase;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

struct GateInner {
    phase: RuntimeLifecyclePhase,
    /// Bumped once when the gate first leaves `running`.
    epoch: u64,
    /// Bumped on every admission and every release.
    ///
    /// The idle supervisor captures this before deciding the runtime is idle and
    /// re-checks it inside [`RuntimeLifecycleGate::try_begin_idle`], so an
    /// admission or release that lands between the summary read and the close is
    /// observed and the idle close is refused instead of racing it.
    activity_epoch: u64,
    active: usize,
}

struct GateShared {
    inner: Mutex<GateInner>,
    active_tx: watch::Sender<usize>,
    phase_tx: watch::Sender<RuntimeLifecyclePhase>,
}

impl GateShared {
    fn lock(&self) -> std::sync::MutexGuard<'_, GateInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The concrete admission gate the daemon drives.
#[derive(Clone)]
pub struct RuntimeLifecycleGate {
    shared: Arc<GateShared>,
}

impl Default for RuntimeLifecycleGate {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeLifecycleGate {
    /// Creates a gate in the `running` phase with zero epochs.
    pub fn new() -> Self {
        let (active_tx, _) = watch::channel(0usize);
        let (phase_tx, _) = watch::channel(RuntimeLifecyclePhase::Running);
        Self {
            shared: Arc::new(GateShared {
                inner: Mutex::new(GateInner {
                    phase: RuntimeLifecyclePhase::Running,
                    epoch: 0,
                    activity_epoch: 0,
                    active: 0,
                }),
                active_tx,
                phase_tx,
            }),
        }
    }

    /// Current lifecycle phase.
    pub fn phase(&self) -> RuntimeLifecyclePhase {
        self.shared.lock().phase
    }

    /// Monotonic close epoch, bumped once when the gate first closes.
    pub fn epoch(&self) -> u64 {
        self.shared.lock().epoch
    }

    /// Monotonic activity epoch, bumped on every admission and every release.
    pub fn activity_epoch(&self) -> u64 {
        self.shared.lock().activity_epoch
    }

    /// Work admissions currently held.
    pub fn active_admissions(&self) -> usize {
        self.shared.lock().active
    }

    /// Whether the gate has left the `running` phase.
    pub fn is_quiesced(&self) -> bool {
        self.phase() != RuntimeLifecyclePhase::Running
    }

    /// Point-in-time gate view.
    pub fn snapshot(&self) -> GateSnapshot {
        let inner = self.shared.lock();
        GateSnapshot {
            phase: inner.phase,
            epoch: inner.epoch,
            active: inner.active,
            activity_epoch: inner.activity_epoch,
        }
    }

    /// Closes the gate: `running -> quiescing`, bumping the close epoch once.
    ///
    /// Returns `true` when this call performed the transition, `false` when the
    /// gate was already closed. Idempotent, so the signal listener, the idle
    /// supervisor and an explicit request share one close.
    pub fn begin_quiesce(&self) -> bool {
        let mut inner = self.shared.lock();
        if inner.phase != RuntimeLifecyclePhase::Running {
            return false;
        }
        inner.phase = RuntimeLifecyclePhase::Quiescing;
        inner.epoch += 1;
        let phase = inner.phase;
        self.shared.phase_tx.send_replace(phase);
        true
    }

    /// Atomically closes the gate for the idle supervisor.
    ///
    /// Succeeds only when the gate is still `running`, the activity epoch is
    /// exactly `expected_activity_epoch` and the held admission count is exactly
    /// `expected_active`. The comparison and the close happen under one guard,
    /// so a lease registration, a claim or any other admission that lands
    /// between the caller's summary read and this call changes the activity epoch
    /// and refuses the close instead of racing it. On success the gate is
    /// `quiescing` and this returns `true`.
    pub fn try_begin_idle(&self, expected_activity_epoch: u64, expected_active: usize) -> bool {
        let mut inner = self.shared.lock();
        if inner.phase != RuntimeLifecyclePhase::Running
            || inner.activity_epoch != expected_activity_epoch
            || inner.active != expected_active
        {
            return false;
        }
        inner.phase = RuntimeLifecyclePhase::Quiescing;
        inner.epoch += 1;
        let phase = inner.phase;
        self.shared.phase_tx.send_replace(phase);
        true
    }

    /// Advances the lifecycle phase forward-only to `phase`.
    ///
    /// A no-op when the gate is already at or past `phase`, so the shutdown
    /// worker can drive every stage without ordering hazards. Returns the phase
    /// in effect afterwards.
    pub fn advance(&self, phase: RuntimeLifecyclePhase) -> RuntimeLifecyclePhase {
        let mut inner = self.shared.lock();
        if phase_rank(phase) > phase_rank(inner.phase) {
            inner.phase = phase;
        }
        let current = inner.phase;
        self.shared.phase_tx.send_replace(current);
        current
    }

    /// Subscribes to active-admission changes.
    pub fn subscribe_active(&self) -> watch::Receiver<usize> {
        self.shared.active_tx.subscribe()
    }

    /// Subscribes to lifecycle phase changes.
    pub fn subscribe_phase(&self) -> watch::Receiver<RuntimeLifecyclePhase> {
        self.shared.phase_tx.subscribe()
    }

    /// Resolves once no work admission is held.
    ///
    /// Takes no lock across the await: the watch channel carries the count.
    pub async fn wait_drained(&self) {
        let mut rx = self.shared.active_tx.subscribe();
        while *rx.borrow() > 0 {
            if rx.changed().await.is_err() {
                return;
            }
        }
    }

    /// Admits one work item against *this* gate.
    ///
    /// Every canonical entry point admits for real against the one gate it
    /// holds; there is deliberately no ambient (task- or call-scoped) bypass. A
    /// task-identity scope would be neither unforgeable nor bound to a single
    /// gate: a Tokio task is reused by HTTP keep-alive and by `join!`, so a
    /// ticket held on one gate could silently satisfy an admission on another
    /// gate, and dropping an outer ticket could release the count an inner
    /// ticket still needs. A nested canonical layer therefore admits a second
    /// time, which only over-counts the work (the drain simply waits longer);
    /// after a quiesce it is refused, which is exactly the stop contract.
    fn admit_inner(&self, kind: WorkKind) -> Result<AdmissionTicket, AdmissionDenied> {
        // The phase check, the count bump and the publication happen under one
        // mutex guard, so a concurrent `begin_quiesce` can never admit work
        // after closing, and no stale count can be published last. No `await`
        // runs under the guard.
        let mut inner = self.shared.lock();
        if inner.phase != RuntimeLifecyclePhase::Running {
            return Err(AdmissionDenied::stopping(inner.phase, kind));
        }
        inner.active += 1;
        inner.activity_epoch += 1;
        self.shared.active_tx.send_replace(inner.active);
        drop(inner);
        Ok(AdmissionTicket {
            kind: Some(kind),
            hold: Some((self.shared.clone(), kind)),
        })
    }
}

impl WorkAdmission for RuntimeLifecycleGate {
    fn admit(&self, kind: WorkKind) -> Result<AdmissionTicket, AdmissionDenied> {
        self.admit_inner(kind)
    }

    fn snapshot(&self) -> GateSnapshot {
        RuntimeLifecycleGate::snapshot(self)
    }

    fn phase(&self) -> RuntimeLifecyclePhase {
        RuntimeLifecycleGate::phase(self)
    }

    fn epoch(&self) -> u64 {
        RuntimeLifecycleGate::epoch(self)
    }

    fn active_admissions(&self) -> usize {
        RuntimeLifecycleGate::active_admissions(self)
    }
}

/// An RAII hold on one admission.
///
/// Dropping the ticket releases the hold and wakes any drain observer. The
/// ticket never holds the gate mutex across an `await`, so a long-running work
/// item cannot block a quiesce transition or the shutdown drain.
#[must_use = "dropping the ticket releases the admission and may let the runtime stop"]
pub struct AdmissionTicket {
    kind: Option<WorkKind>,
    /// The gate hold, present only when this ticket performed a real admission.
    hold: Option<(Arc<GateShared>, WorkKind)>,
}

impl AdmissionTicket {
    /// A no-op ticket for an ungated build (desktop/tests without a gate).
    pub fn permissive() -> Self {
        Self {
            kind: None,
            hold: None,
        }
    }

    /// The kind this ticket was admitted for, if it is a real admission.
    pub fn kind(&self) -> Option<WorkKind> {
        self.kind
    }
}

impl std::fmt::Debug for AdmissionTicket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmissionTicket")
            .field("kind", &self.kind())
            .finish()
    }
}

impl Drop for AdmissionTicket {
    fn drop(&mut self) {
        if let Some((shared, _)) = self.hold.take() {
            // Decrement, bump the activity epoch and publish the fresh count
            // under one guard so a concurrent admission cannot publish a stale
            // count last.
            let mut inner = shared.lock();
            if inner.active > 0 {
                inner.active -= 1;
            }
            inner.activity_epoch += 1;
            shared.active_tx.send_replace(inner.active);
        }
    }
}

/// A gate that admits everything, for a process that owns no runtime lifecycle.
///
/// The desktop in-process control plane and the backend unit tests use this so
/// no caller needs a closure or an `Option` in the hot path.
pub struct PermissiveAdmission;

impl WorkAdmission for PermissiveAdmission {
    fn admit(&self, _kind: WorkKind) -> Result<AdmissionTicket, AdmissionDenied> {
        Ok(AdmissionTicket::permissive())
    }

    fn snapshot(&self) -> GateSnapshot {
        GateSnapshot {
            phase: RuntimeLifecyclePhase::Running,
            epoch: 0,
            active: 0,
            activity_epoch: 0,
        }
    }

    fn phase(&self) -> RuntimeLifecyclePhase {
        RuntimeLifecyclePhase::Running
    }

    fn epoch(&self) -> u64 {
        0
    }

    fn active_admissions(&self) -> usize {
        0
    }
}

fn phase_rank(phase: RuntimeLifecyclePhase) -> u8 {
    match phase {
        RuntimeLifecyclePhase::Running => 0,
        RuntimeLifecyclePhase::Quiescing => 1,
        RuntimeLifecyclePhase::Cancelling => 2,
        RuntimeLifecyclePhase::Draining => 3,
        RuntimeLifecyclePhase::Stopped => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate() -> RuntimeLifecycleGate {
        RuntimeLifecycleGate::new()
    }

    #[test]
    fn running_gate_admits_and_ticket_releases_on_drop() {
        let gate = gate();
        let ticket = gate.admit(WorkKind::WorkflowStart).expect("admit");
        assert_eq!(gate.active_admissions(), 1);
        assert_eq!(ticket.kind(), Some(WorkKind::WorkflowStart));
        drop(ticket);
        assert_eq!(gate.active_admissions(), 0);
    }

    #[test]
    fn quiesce_closes_the_gate_and_refuses_with_a_stable_code() {
        let gate = gate();
        assert!(gate.begin_quiesce());
        // A second close is a no-op and does not bump the epoch again.
        assert!(!gate.begin_quiesce());
        assert_eq!(gate.epoch(), 1);

        let denied = gate.admit(WorkKind::ChatTurn).expect_err("refused");
        assert_eq!(
            denied.code,
            chatspeed_contracts::RUNTIME_CODE_RUNTIME_STOPPING
        );
        assert!(denied.message.contains("chat_turn"), "{}", denied.message);
        assert!(denied.message.contains("quiescing"), "{}", denied.message);
        assert_eq!(gate.active_admissions(), 0);
    }

    #[test]
    fn advance_is_forward_only_and_idempotent() {
        let gate = gate();
        assert_eq!(
            gate.advance(RuntimeLifecyclePhase::Draining),
            RuntimeLifecyclePhase::Draining
        );
        // A backward request never regresses the phase.
        assert_eq!(
            gate.advance(RuntimeLifecyclePhase::Quiescing),
            RuntimeLifecyclePhase::Draining
        );
        assert_eq!(
            gate.advance(RuntimeLifecyclePhase::Stopped),
            RuntimeLifecyclePhase::Stopped
        );
        // And a later forward request is still a no-op.
        assert_eq!(
            gate.advance(RuntimeLifecyclePhase::Stopped),
            RuntimeLifecyclePhase::Stopped
        );
    }

    #[test]
    fn every_admission_and_release_bumps_the_activity_epoch() {
        let gate = gate();
        assert_eq!(gate.activity_epoch(), 0);
        let ticket = gate.admit(WorkKind::TerminalWork).expect("admit");
        assert_eq!(gate.activity_epoch(), 1, "an admission is activity");
        drop(ticket);
        assert_eq!(gate.activity_epoch(), 2, "a release is activity");
    }

    #[test]
    fn try_begin_idle_refuses_when_activity_landed_after_the_epoch_was_captured() {
        let gate = gate();
        let captured = gate.activity_epoch();
        // Any admission or release after the capture invalidates the idle close.
        let ticket = gate.admit(WorkKind::ManualClaim).expect("admit");
        assert!(!gate.try_begin_idle(captured, 0));
        assert_eq!(gate.phase(), RuntimeLifecyclePhase::Running);
        drop(ticket);
        assert!(!gate.try_begin_idle(captured, 0));

        // With a fresh capture and no held work the close succeeds atomically.
        let captured = gate.activity_epoch();
        assert!(gate.try_begin_idle(captured, 0));
        assert_eq!(gate.phase(), RuntimeLifecyclePhase::Quiescing);
    }

    #[test]
    fn try_begin_idle_refuses_while_a_ticket_is_held() {
        let gate = gate();
        let ticket = gate.admit(WorkKind::ChatTurn).expect("admit");
        let captured = gate.activity_epoch();
        assert!(
            !gate.try_begin_idle(captured, 0),
            "active work keeps it up"
        );
        assert_eq!(gate.phase(), RuntimeLifecyclePhase::Running);
        drop(ticket);
    }

    #[tokio::test]
    async fn wait_drained_resolves_only_when_the_last_ticket_drops() {
        let gate = gate();
        let first = gate.admit(WorkKind::TerminalWork).expect("admit");
        let second = gate.admit(WorkKind::AutomationDispatch).expect("admit");

        let waiter = {
            let gate = gate.clone();
            tokio::spawn(async move { gate.wait_drained().await })
        };
        // Still draining: the waiter must not resolve while a ticket is held.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(!waiter.is_finished());

        drop(first);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(!waiter.is_finished());
        drop(second);
        tokio::time::timeout(std::time::Duration::from_secs(2), waiter)
            .await
            .expect("drain observer resolves")
            .expect("join");
    }

    #[tokio::test]
    async fn a_ticket_on_one_gate_never_lets_another_gate_bypass_its_own_quiesce() {
        // Two independent gates share one Tokio task, exactly as HTTP keep-alive
        // reuses a connection task. Admitting on the first gate must not satisfy
        // (or mask) an admission on the second: a task-scoped bypass would see
        // the task as already "in scope" and silently admit on the quiesced gate.
        let first = gate();
        let second = gate();
        tokio::spawn(async move {
            let held = first.admit(WorkKind::WorkflowStart).expect("first admits");
            assert_eq!(first.active_admissions(), 1);

            assert!(second.begin_quiesce());
            let denied = second
                .admit(WorkKind::ChatTurn)
                .expect_err("the second gate refuses independently");
            assert_eq!(
                denied.code,
                chatspeed_contracts::RUNTIME_CODE_RUNTIME_STOPPING
            );
            // The bypass would have left the second gate silently un-counted.
            assert_eq!(second.active_admissions(), 0);
            drop(held);
        })
        .await
        .expect("task joins");
    }

    #[tokio::test]
    async fn dropping_an_outer_ticket_still_counts_a_nested_ticket() {
        // A nested canonical admission is a real, independent hold on the same
        // gate. Dropping the outer ticket early must not release the count the
        // nested ticket still needs (the failure mode of a shared task scope).
        let gate = gate();
        tokio::spawn(async move {
            let outer = gate.admit(WorkKind::WorkflowStart).expect("outer admits");
            let inner = gate.admit(WorkKind::ChatTurn).expect("inner admits");
            assert_eq!(gate.active_admissions(), 2);

            drop(outer);
            assert_eq!(
                gate.active_admissions(),
                1,
                "the nested hold survives the outer drop"
            );
            drop(inner);
            assert_eq!(gate.active_admissions(), 0);
        })
        .await
        .expect("task joins");
    }

    #[tokio::test]
    async fn independent_futures_on_one_task_do_not_share_admission() {
        // `tokio::join!` polls both futures on the same task. Their admissions
        // must stay independent: a task-scoped bypass would let the second
        // future ride the first future's hold across a quiesce.
        let gate = gate();
        let first = {
            let gate = gate.clone();
            async move {
                let held = gate.admit(WorkKind::WorkflowStart).expect("running admits");
                tokio::task::yield_now().await;
                drop(held);
            }
        };
        let second = {
            let gate = gate.clone();
            async move {
                tokio::task::yield_now().await;
                assert!(gate.begin_quiesce());
                match gate.admit(WorkKind::ChatTurn) {
                    Ok(_) => panic!("the joined future must not bypass the quiesce"),
                    Err(denied) => assert_eq!(
                        denied.code,
                        chatspeed_contracts::RUNTIME_CODE_RUNTIME_STOPPING
                    ),
                }
            }
        };
        tokio::spawn(async move {
            tokio::join!(first, second);
        })
        .await
        .expect("task joins");
    }

    #[tokio::test]
    async fn a_nested_admission_after_quiesce_is_refused_inside_the_same_call() {
        // A canonical layer admits, then a quiesce lands, then the very same
        // call reaches a nested canonical layer. There is no ambient bypass, so
        // the nested admission is refused exactly as the stop contract requires
        // while the already-admitted outer hold stays counted for the drain.
        let gate = gate();
        tokio::spawn(async move {
            let outer = gate.admit(WorkKind::WorkflowStart).expect("outer admits");
            assert!(gate.begin_quiesce());
            let denied = gate
                .admit(WorkKind::ChatTurn)
                .expect_err("the nested admission refuses");
            assert_eq!(
                denied.code,
                chatspeed_contracts::RUNTIME_CODE_RUNTIME_STOPPING
            );
            assert_eq!(
                gate.active_admissions(),
                1,
                "the accepted outer hold is still counted"
            );
            drop(outer);
            assert_eq!(gate.active_admissions(), 0);
        })
        .await
        .expect("task joins");
    }

    #[test]
    fn permissive_gate_admits_without_state() {
        let gate = PermissiveAdmission;
        let ticket = gate.admit(WorkKind::LeaseRegister).expect("admit");
        assert_eq!(gate.phase(), RuntimeLifecyclePhase::Running);
        assert_eq!(gate.active_admissions(), 0);
        assert!(ticket.kind().is_none());
    }

    #[test]
    fn shutdown_handoff_fires_on_release_and_on_drop() {
        let (handoff, receiver) = crate::lifecycle::ShutdownHandoff::channel();
        handoff.release();
        // Releasing twice is a no-op.
        handoff.release();
        assert_eq!(receiver.blocking_recv(), Ok(()));

        let (handoff, receiver) = crate::lifecycle::ShutdownHandoff::channel();
        drop(handoff);
        assert_eq!(receiver.blocking_recv(), Ok(()));
    }
}