// The automation application service and its scheduler tick are runtime-owned:
// the desktop reaches them through the control plane, so only the shared wire
// types and errors are compiled into the desktop crate.
#[cfg(not(feature = "desktop"))]
pub mod application;
pub mod errors;
// The automation tick is runtime-owned: the standalone runtime drives the
// canonical `WorkflowApplicationService::automation_dispatch_due` path directly,
// so the desktop has no scheduler module of its own.
#[cfg(not(feature = "desktop"))]
pub mod service;
pub mod types;
