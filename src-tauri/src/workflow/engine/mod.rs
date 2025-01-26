//! Workflow engine module
//!
//! This module provides the workflow engine implementation for executing workflow configurations.
//! The engine manages plugin registration, workflow state, and node execution.

mod core;

pub use core::WorkflowEngine;
