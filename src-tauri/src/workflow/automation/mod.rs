pub mod application;
pub mod errors;
// The automation tick is runtime-owned: the standalone runtime drives the
// canonical `WorkflowApplicationService::automation_dispatch_due` path directly,
// so the desktop has no scheduler module of its own.
pub mod service;
pub mod types;
