//! Transport-neutral workflow application surface.
//!
//! The workflow request DTOs moved to the shared `chatspeed-contracts` crate;
//! this always-compiled module re-exports them so the existing
//! `application_types::*` and `application::*` paths keep working for both the
//! desktop and runtime crates. What stays here is the runtime-only domain error,
//! which is not a transport-neutral wire type.

#[cfg(not(feature = "desktop"))]
pub use chatspeed_contracts::workflow::WorkflowEventsQuery;
pub use chatspeed_contracts::workflow::{WorkflowCreateRequest, WorkflowStartRequest};

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
    /// The runtime is quiescing and refuses new work (`runtime_stopping`).
    Stopping,
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
    pub origin: Option<crate::error_diagnostics::ErrorOrigin>,
}

#[cfg(not(feature = "desktop"))]
impl ApplicationError {
    pub fn new(kind: ApplicationErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            origin: None,
        }
    }

    pub fn with_origin(mut self, origin: crate::error_diagnostics::ErrorOrigin) -> Self {
        self.origin = Some(origin);
        self
    }

    pub fn from_failure(
        kind: ApplicationErrorKind,
        failure: crate::error_diagnostics::FailureMessage,
    ) -> Self {
        Self {
            kind,
            message: failure.legacy,
            origin: failure.origin,
        }
    }

    pub fn with_reason(self, reason: chatspeed_contracts::ErrorReason) -> Self {
        let mut origin = self.origin.clone().unwrap_or_else(|| {
            crate::error_diagnostics::ErrorOrigin::leaf(&self.message, None, None, None)
        });
        origin.reason = Some(reason);
        self.with_origin(origin)
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

    pub fn stopping(message: impl Into<String>) -> Self {
        Self::new(ApplicationErrorKind::Stopping, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ApplicationErrorKind::Internal, message)
    }
}

#[cfg(not(feature = "desktop"))]
impl crate::error_diagnostics::ErrorProjection for ApplicationError {
    fn user_error(&self) -> chatspeed_contracts::UserErrorPresentation {
        let failure = crate::error_diagnostics::FailureMessage {
            legacy: self.message.clone(),
            origin: self.origin.clone(),
        };
        failure.user_error()
    }

    fn diagnostic(&self) -> crate::error_diagnostics::ErrorDiagnostic {
        let failure = crate::error_diagnostics::FailureMessage {
            legacy: self.message.clone(),
            origin: self.origin.clone(),
        };
        let mut diagnostic = failure.diagnostic();
        diagnostic.module = "application".into();
        diagnostic.operation = "request".into();
        diagnostic
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

#[cfg(all(test, not(feature = "desktop")))]
mod tests {
    use super::*;
    use crate::error_diagnostics::ErrorProjection;

    #[test]
    fn with_reason_preserves_legacy_error_shape_and_projects_source_reason() {
        let error = ApplicationError::state("Session is stopping: session-1")
            .with_reason(chatspeed_contracts::ErrorReason::SessionStopping);

        assert_eq!(error.kind, ApplicationErrorKind::State);
        assert_eq!(error.message, "Session is stopping: session-1");
        assert_eq!(
            error.user_error().reason,
            Some(chatspeed_contracts::ErrorReason::SessionStopping)
        );
    }
}
