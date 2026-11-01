//! Runtime-owned budgets shared across host shell executions.

mod controller;
mod platform;
mod types;

pub use controller::{check_admission, install, register, reset};
pub use types::{ResourceLimits, ResourceViolation, TaskGuard, CONFIG_KEY};
