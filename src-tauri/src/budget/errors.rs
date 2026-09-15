//! Stable machine errors for the budget admission ledger.
//!
//! Every rejection carries a stable machine-readable code so callers
//! (ccproxy admission gate, tool owners, future 2C experiment service) can
//! branch on the failure class without parsing messages. Rejections are
//! never expressed as `Option`, `0` or free-form strings.

use crate::budget::types::ResourceDimension;

/// Stable machine error codes for admission decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AdmissionErrorCode {
    /// A hard cap would be exceeded by this admission.
    BudgetExceeded,
    /// Money-budget mode could not obtain a valid, bounded price for the
    /// resolved model (missing, mismatched, non-finite, negative or
    /// unbounded worst-case). Fail closed: no outbound effect.
    UnpricedModel,
    /// A required bound (e.g. max output tokens) could not be established.
    MissingBound,
    /// The target scope (or an ancestor) is paused or closed.
    ScopePaused,
    /// The scope chain is malformed or does not exist.
    InvalidScopeChain,
    /// The same idempotency key was reused with conflicting parameters.
    IdempotencyConflict,
    /// The operation is not valid for the current reservation state.
    InvalidTransition,
    /// The ledger could not be durably updated; the whole operation was
    /// rolled back and no partial balance change remains.
    AdmissionPersistenceFailure,
    /// A required resource dimension cannot be reliably measured by the
    /// effect owner; admission is refused instead of assuming zero usage.
    ResourceUnobservable,
}

impl AdmissionErrorCode {
    /// Stable snake_case machine code used in logs and typed responses.
    pub fn as_str(&self) -> &'static str {
        match self {
            AdmissionErrorCode::BudgetExceeded => "budget_exceeded",
            AdmissionErrorCode::UnpricedModel => "unpriced_model",
            AdmissionErrorCode::MissingBound => "missing_bound",
            AdmissionErrorCode::ScopePaused => "scope_paused",
            AdmissionErrorCode::InvalidScopeChain => "invalid_scope_chain",
            AdmissionErrorCode::IdempotencyConflict => "idempotency_conflict",
            AdmissionErrorCode::InvalidTransition => "invalid_transition",
            AdmissionErrorCode::AdmissionPersistenceFailure => "admission_persistence_failure",
            AdmissionErrorCode::ResourceUnobservable => "resource_unobservable",
        }
    }
}

/// A typed admission error: stable machine code plus a human-readable
/// message and, when applicable, the offending resource dimension.
#[derive(Debug, Clone)]
pub struct AdmissionError {
    pub code: AdmissionErrorCode,
    pub message: String,
    pub dimension: Option<ResourceDimension>,
}

impl AdmissionError {
    pub fn new(code: AdmissionErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            dimension: None,
        }
    }

    pub fn with_dimension(mut self, dimension: ResourceDimension) -> Self {
        self.dimension = Some(dimension);
        self
    }

    pub fn budget_exceeded(message: impl Into<String>) -> Self {
        Self::new(AdmissionErrorCode::BudgetExceeded, message)
    }

    pub fn unpriced_model(message: impl Into<String>) -> Self {
        Self::new(AdmissionErrorCode::UnpricedModel, message)
    }

    pub fn missing_bound(message: impl Into<String>) -> Self {
        Self::new(AdmissionErrorCode::MissingBound, message)
    }

    pub fn scope_paused(message: impl Into<String>) -> Self {
        Self::new(AdmissionErrorCode::ScopePaused, message)
    }

    pub fn invalid_scope_chain(message: impl Into<String>) -> Self {
        Self::new(AdmissionErrorCode::InvalidScopeChain, message)
    }

    pub fn idempotency_conflict(message: impl Into<String>) -> Self {
        Self::new(AdmissionErrorCode::IdempotencyConflict, message)
    }

    pub fn invalid_transition(message: impl Into<String>) -> Self {
        Self::new(AdmissionErrorCode::InvalidTransition, message)
    }

    pub fn persistence_failure(message: impl Into<String>) -> Self {
        Self::new(AdmissionErrorCode::AdmissionPersistenceFailure, message)
    }

    pub fn resource_unobservable(message: impl Into<String>) -> Self {
        Self::new(AdmissionErrorCode::ResourceUnobservable, message)
    }
}

impl std::fmt::Display for AdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for AdmissionError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_codes_are_stable() {
        assert_eq!(
            AdmissionErrorCode::BudgetExceeded.as_str(),
            "budget_exceeded"
        );
        assert_eq!(AdmissionErrorCode::UnpricedModel.as_str(), "unpriced_model");
        assert_eq!(AdmissionErrorCode::MissingBound.as_str(), "missing_bound");
        assert_eq!(AdmissionErrorCode::ScopePaused.as_str(), "scope_paused");
        assert_eq!(
            AdmissionErrorCode::InvalidScopeChain.as_str(),
            "invalid_scope_chain"
        );
        assert_eq!(
            AdmissionErrorCode::IdempotencyConflict.as_str(),
            "idempotency_conflict"
        );
        assert_eq!(
            AdmissionErrorCode::InvalidTransition.as_str(),
            "invalid_transition"
        );
        assert_eq!(
            AdmissionErrorCode::AdmissionPersistenceFailure.as_str(),
            "admission_persistence_failure"
        );
        assert_eq!(
            AdmissionErrorCode::ResourceUnobservable.as_str(),
            "resource_unobservable"
        );
    }

    #[test]
    fn display_includes_machine_code() {
        let error = AdmissionError::unpriced_model("no price for model");
        assert_eq!(error.to_string(), "unpriced_model: no price for model");
    }

    #[test]
    fn dimension_is_attached_to_error() {
        let error = AdmissionError::budget_exceeded("output tokens")
            .with_dimension(ResourceDimension::OutputTokens);
        assert_eq!(error.dimension, Some(ResourceDimension::OutputTokens));
    }
}
