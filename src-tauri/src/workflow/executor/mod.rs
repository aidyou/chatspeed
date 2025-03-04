//! Workflow executor module
//!
//! This module provides the core functionality for executing workflows,
//! including parallel execution, state management, and error handling.

mod channel;
mod core;

pub use core::WorkflowExecutor;
