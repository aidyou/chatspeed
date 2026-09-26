//! Phase 2G+2H: durable experiment schedule contract, shared benchmark fixture
//! resolution and (from U-5 onward) the isolated execution owner.
//!
//! Sub-modules:
//!
//! - [`fixture`]: the shared, compile-time-pinned `chatspeed-smoke` fixture
//!   resolver. Both the backend scheduler and the `cs` CLI resolve exactly the
//!   same task document and digests from here, so a durable schedule request
//!   can store refs only and recover the instruction at dispatch.
//! - [`types`]: the frozen strict documents and the pure job FSM / restart
//!   classification shared by the store, the scheduler, the owner saga and the
//!   control plane.
//!
//! Nothing in this module opens a database, spawns a process or talks to a
//! provider; the durable state lives in `db::experiment_schedule`, the
//! scheduling loop in `experiment_schedule::scheduler`, and the isolation in
//! `experiment_owner`.

pub mod fixture;
pub mod scheduler;
pub mod types;
