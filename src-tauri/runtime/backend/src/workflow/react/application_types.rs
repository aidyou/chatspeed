//! Transport-neutral workflow application surface.
//!
//! The workflow request DTOs moved to the shared `chatspeed-contracts` crate;
//! this always-compiled module re-exports them so the existing
//! `application_types::*` and `application::*` paths keep working for both the
//! desktop and runtime crates. What stays here is the runtime-only domain error,
//! which is not a transport-neutral wire type.

pub use chatspeed_contracts::workflow::{WorkflowCreateRequest, WorkflowStartRequest};
#[cfg(not(feature = "desktop"))]
pub use chatspeed_contracts::workflow::WorkflowEventsQuery;

/// Stable error classification for workflow application operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg(not(feature = "desktop"))]
pub enum ApplicationErrorKind {
    /// A referenced entity does not exist.
    NotFound,
    /// The request is malformed or references an unusable entity.
    InvalidInput,
    /// The request conflicts with current state (e.g. duplicate idempotency
    /// key with a different body).
    Conflict,
    /// The operation is not valid for the current workflow state.
    State,
    /// The runtime gateway rejected the operation (e.g. no live input route).
    Gateway,
    /// Any other failure (storage, tooling, unexpected errors).
    Internal,
}

/// A workflow application error with a stable kind and a human-readable
/// message. `Display` renders only the message so existing Tauri string errors
/// stay byte-identical.
#[derive(Debug, Clone)]
#[cfg(not(feature = "desktop"))]
pub struct ApplicationError {
    pub kind: ApplicationErrorKind,
    pub message: String,
}

#[cfg(not(feature = "desktop"))]
impl ApplicationError {
    pub fn new(kind: ApplicationErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ApplicationErrorKind::NotFound, message)
    }

    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(ApplicationErrorKind::InvalidInput, message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(ApplicationErrorKind::Conflict, message)
    }

    pub fn state(message: impl Into<String>) -> Self {
        Self::new(ApplicationErrorKind::State, message)
    }

    pub fn gateway(message: impl Into<String>) -> Self {
        Self::new(ApplicationErrorKind::Gateway, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ApplicationErrorKind::Internal, message)
    }
}

#[cfg(not(feature = "desktop"))]
impl std::fmt::Display for ApplicationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

#[cfg(not(feature = "desktop"))]
impl From<String> for ApplicationError {
    fn from(message: String) -> Self {
        Self::internal(message)
    }
}

#[cfg(not(feature = "desktop"))]
impl From<&str> for ApplicationError {
    fn from(message: &str) -> Self {
        Self::internal(message)
    }
}