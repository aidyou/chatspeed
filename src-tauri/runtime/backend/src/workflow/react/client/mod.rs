//! Transport adapters for the workflow runtime.
//!
//! The shared runtime hub (`hub`) and the loopback HTTP control plane (`http`)
//! are the runtime's in-process adapters. The desktop reaches the runtime
//! through the `RuntimeSupervisor` client, not through these in-process
//! adapters; the former Tauri output adapter (`TauriGateway`) now lives in the
//! desktop crate as a retained desktop-only source
//! (`src/workflow_event_gateway.rs`).

pub mod http;
pub mod hub;