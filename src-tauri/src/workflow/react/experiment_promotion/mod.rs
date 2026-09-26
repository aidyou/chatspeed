//! Phase 2I promotion: strict contracts, the server-owned target registry, the
//! automatic policy gate, the local checkpoint owner, the paired canary runner
//! and the durable promotion supervisor.
//!
//! The module is split so that the *pure* half (`types`, `policy`) can be
//! linked into the `cs` CLI without pulling in a database, an owner or a
//! runtime, while the *effectful* half (`scheduler`, and the owner/canary
//! modules under `experiment_owner`) is only reachable from the headless
//! backend.

pub mod binding;
pub mod policy;
pub mod scheduler;
pub mod types;

#[cfg(test)]
pub mod sigkill;
#[cfg(test)]
pub mod smoke;
