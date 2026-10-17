//! Public lifecycle value types.
//!
//! This module owns the plain data the admission gate exposes: the [`WorkKind`]
//! labels, the stable [`AdmissionDenied`], the point-in-time [`GateSnapshot`]
//! and the [`ShutdownHandoff`] the coordinated-shutdown response carries. The
//! concrete gate, the RAII ticket and the trait live next to it so `mod.rs`
//! stays a pure declaration/re-export surface.

use chatspeed_contracts::{RuntimeLifecyclePhase, RUNTIME_CODE_RUNTIME_STOPPING};
use rust_i18n::t;
use std::sync::Mutex;

/// A canonical entry point that starts or resumes work.
///
/// The kind is recorded for diagnostics and denial messages; it is not part of
/// the admission decision, which depends only on the current phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkKind {
    /// A workflow session start.
    WorkflowStart,
    /// A workflow session resume (a signal that continues a wait).
    WorkflowResume,
    /// The runtime automation scheduler dispatching a due slot.
    AutomationDispatch,
    /// An explicitly requested automation run.
    ManualClaim,
    /// An automation definition mutation that may start or alter scheduled work
    /// (create, update, apply, enable).
    AutomationConfigure,
    /// A client bridge dispatch of web work.
    ClientDispatch,
    /// A runtime chat turn.
    ChatTurn,
    /// Runtime interactive terminal work.
    TerminalWork,
    /// A capability MCP server start.
    CapabilityStart,
    /// A client lease registration.
    LeaseRegister,
}

impl WorkKind {
    /// Stable diagnostic name.
    pub const fn as_str(self) -> &'static str {
        match self {
            WorkKind::WorkflowStart => "workflow_start",
            WorkKind::WorkflowResume => "workflow_resume",
            WorkKind::AutomationDispatch => "automation_dispatch",
            WorkKind::ManualClaim => "manual_claim",
            WorkKind::AutomationConfigure => "automation_configure",
            WorkKind::ClientDispatch => "client_dispatch",
            WorkKind::ChatTurn => "chat_turn",
            WorkKind::TerminalWork => "terminal_work",
            WorkKind::CapabilityStart => "capability_start",
            WorkKind::LeaseRegister => "lease_register",
        }
    }
}

/// Why the gate refused an admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmissionDenied {
    /// Stable wire code (`runtime_stopping`).
    pub code: &'static str,
    /// Human diagnostic, localized through the shared catalog.
    pub message: String,
}

impl AdmissionDenied {
    /// Builds the stable `runtime_stopping` refusal for a closed gate.
    pub(crate) fn stopping(phase: RuntimeLifecyclePhase, kind: WorkKind) -> Self {
        Self {
            code: RUNTIME_CODE_RUNTIME_STOPPING,
            message: t!(
                "runtime.work_refused_while_stopping",
                phase = phase_wire(phase),
                kind = kind.as_str()
            )
            .to_string(),
        }
    }
}

/// A point-in-time view of the gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateSnapshot {
    /// Current lifecycle phase.
    pub phase: RuntimeLifecyclePhase,
    /// Monotonic epoch, bumped once when the gate first closes.
    pub epoch: u64,
    /// Work admissions currently held.
    pub active: usize,
    /// Monotonic activity epoch, bumped on every admission and release.
    ///
    /// The idle supervisor captures it before it decides the runtime is idle and
    /// re-checks it under the gate lock, so an admission or release that lands
    /// between the summary read and the close is observed instead of lost.
    pub activity_epoch: u64,
}

/// Hand-off that fires when the response carrying it is dropped.
///
/// The coordinated shutdown handler closes the gate and returns `202` with this
/// guard attached to the response body stream. The response body is consumed or
/// dropped by the transport after its bytes have been handed off, which is the
/// point at which the shutdown worker is released. The guard is drop-safe: even
/// if the response is dropped early, or the guard is dropped directly, the
/// release fires exactly once, so the worker can never be stranded.
pub struct ShutdownHandoff {
    sender: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

impl ShutdownHandoff {
    /// Creates the hand-off and the receiver the shutdown worker awaits.
    pub fn channel() -> (Self, tokio::sync::oneshot::Receiver<()>) {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        (
            Self {
                sender: Mutex::new(Some(sender)),
            },
            receiver,
        )
    }

    /// Releases the worker without waiting for the response to be dropped.
    pub fn release(&self) {
        let sender = self
            .sender
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(sender) = sender {
            let _ = sender.send(());
        }
    }
}

impl Drop for ShutdownHandoff {
    fn drop(&mut self) {
        self.release();
    }
}

impl std::fmt::Debug for ShutdownHandoff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShutdownHandoff").finish_non_exhaustive()
    }
}

/// Stable wire name for a lifecycle phase.
pub(crate) fn phase_wire(phase: RuntimeLifecyclePhase) -> &'static str {
    match phase {
        RuntimeLifecyclePhase::Running => "running",
        RuntimeLifecyclePhase::Quiescing => "quiescing",
        RuntimeLifecyclePhase::Cancelling => "cancelling",
        RuntimeLifecyclePhase::Draining => "draining",
        RuntimeLifecyclePhase::Stopped => "stopped",
    }
}