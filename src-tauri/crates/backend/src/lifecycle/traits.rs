//! The runtime work-admission trait.
//!
//! Kept separate from the concrete gate so the daemon can drive a
//! [`RuntimeLifecycleGate`] while the backend holds only the trait object it
//! fences work with.

use crate::lifecycle::gate::AdmissionTicket;
use crate::lifecycle::types::{AdmissionDenied, GateSnapshot, WorkKind};
use chatspeed_contracts::RuntimeLifecyclePhase;

/// The one runtime work-admission gate.
pub trait WorkAdmission: Send + Sync + 'static {
    /// Admits `kind`, returning the RAII ticket that keeps the runtime draining
    /// or refuses with a stable [`AdmissionDenied`].
    fn admit(&self, kind: WorkKind) -> Result<AdmissionTicket, AdmissionDenied>;

    /// Point-in-time gate view.
    fn snapshot(&self) -> GateSnapshot;

    /// Current lifecycle phase.
    fn phase(&self) -> RuntimeLifecyclePhase;

    /// Monotonic gate epoch.
    fn epoch(&self) -> u64;

    /// Work admissions currently held.
    fn active_admissions(&self) -> usize;
}