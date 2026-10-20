//! The single runtime work-admission gate (U-4).
//!
//! One gate instance fences *every* canonical entry point that can start or
//! resume work: lease registration, workflow start/resume, automation dispatch
//! and manual claims, client dispatch, chat turns, terminal work and capability
//! MCP starts. The gate is a backend type so both the daemon (which drives the
//! lifecycle state machine) and the backend application service (which fences
//! the work it starts) share one admission decision; the backend never depends
//! on the daemon.
//!
//! The gate is a short critical section plus an RAII ticket:
//!
//! - admission takes the gate mutex only long enough to read the phase, bump the
//!   active count and release the guard; a long-running work item holds the
//!   returned [`AdmissionTicket`] and never holds the mutex across an `await`,
//! - the cancel/epoch decision is taken under the same mutex as the count, so a
//!   quiesce transition cannot race a concurrent admission,
//! - every admission and release also bumps a separate *activity* epoch, and the
//!   watch count is published under the same mutex, so an idle decision can be
//!   re-validated atomically instead of racing a late arrival,
//! - the ticket decrements the count on drop, which is what lets the drain phase
//!   observe the runtime becoming idle without any lock held across an await.

pub mod gate;
pub mod traits;
pub mod types;

pub use gate::{AdmissionTicket, PermissiveAdmission, RuntimeLifecycleGate};
pub use traits::WorkAdmission;
pub use types::{AdmissionDenied, GateSnapshot, ShutdownHandoff, WorkKind};
