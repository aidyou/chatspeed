//! Error projection contract.
use super::types::ErrorDiagnostic;
use chatspeed_contracts::error::UserErrorPresentation;

/// Separates safe user output from local diagnostics.
pub trait ErrorProjection {
    fn user_error(&self) -> UserErrorPresentation;
    fn diagnostic(&self) -> ErrorDiagnostic;
}
