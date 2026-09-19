//! MCP domain sub-modules.
//!
//! - [`repository`] is the persisted desired state (the only DB seam),
//! - [`runtime`] is the observation and effect ports (the only runtime seam),
//! - [`descriptor`] validates a strict install request,
//! - [`orchestrator`] is the lifecycle state machine every adapter delegates to.

pub mod descriptor;
pub mod orchestrator;
pub mod repository;
pub mod runtime;

#[cfg(test)]
mod tests;
