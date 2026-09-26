//! Stable, transport-neutral capability error contract.
//!
//! Every capability adapter (Tauri command, `/control/v1` handler, `cs` CLI)
//! maps this type into its own wire form, but the `code` is the canonical
//! machine contract: a client must be able to branch on `code` without
//! matching localized text. Messages are always non-secret.

use serde::{Deserialize, Serialize};

use super::redaction;

/// Canonical machine-readable error codes.
pub mod code {
    /// The request itself is malformed or violates the descriptor contract.
    pub const INVALID_REQUEST: &str = "invalid_request";
    /// A mutation arrived without an `Idempotency-Key`.
    pub const IDEMPOTENCY_KEY_REQUIRED: &str = "idempotency_key_required";
    /// The same key was reused with a different canonical request hash.
    pub const IDEMPOTENCY_KEY_CONFLICT: &str = "idempotency_key_conflict";
    /// The referenced operation does not exist.
    pub const OPERATION_NOT_FOUND: &str = "operation_not_found";
    /// A read model exists but the referenced capability instance does not.
    pub const NOT_FOUND: &str = "not_found";
    /// The requested target/adapter is not verifiably supported.
    pub const UNSUPPORTED_TARGET: &str = "unsupported_target";
    /// The requested MCP transport/runner has no verified adapter.
    pub const UNSUPPORTED_ADAPTER: &str = "unsupported_adapter";
    /// No MCP runtime exists in this process, so an effect cannot be performed
    /// or observed. Refused rather than reported as a silent success.
    pub const RUNTIME_UNAVAILABLE: &str = "runtime_unavailable";
    /// The deterministic checker rejected the source.
    pub const CHECK_BLOCKED: &str = "check_blocked";
    /// The deterministic checker could not prove the source safe.
    pub const CHECK_INCONCLUSIVE: &str = "check_inconclusive";
    /// The mutation is intentionally refused (non-managed, drifted, skip).
    pub const REFUSED: &str = "refused";
    /// The caller may not perform this mutation.
    pub const FORBIDDEN: &str = "forbidden";
    /// The effect may or may not have happened; reconciliation is required.
    pub const NEEDS_RECONCILE: &str = "needs_reconcile";
    /// An operation was interrupted before any effect was attempted, so it is
    /// safe to retry.
    pub const INTERRUPTED_BEFORE_EFFECT: &str = "interrupted_before_effect";
    /// An operation was interrupted with an unproven effect.
    pub const EFFECT_STATE_UNKNOWN: &str = "effect_state_unknown";
    /// A resource is already locked by an in-flight mutation.
    pub const BUSY: &str = "busy";
    /// The operation completed but the runtime state is only partially known.
    pub const PARTIAL: &str = "partial";
    /// Unclassified internal failure. Never carries a secret.
    pub const INTERNAL: &str = "internal";
    /// The durable store rejected the operation.
    pub const STORE_ERROR: &str = "store_error";
}

/// A capability failure with a stable code and a non-secret message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityError {
    pub code: String,
    pub message: String,
}

impl CapabilityError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(code::INVALID_REQUEST, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(code::NOT_FOUND, message)
    }

    pub fn refused(message: impl Into<String>) -> Self {
        Self::new(code::REFUSED, message)
    }

    pub fn unsupported_target(message: impl Into<String>) -> Self {
        Self::new(code::UNSUPPORTED_TARGET, message)
    }

    pub fn needs_reconcile(message: impl Into<String>) -> Self {
        Self::new(code::NEEDS_RECONCILE, message)
    }

    pub fn busy(message: impl Into<String>) -> Self {
        Self::new(code::BUSY, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(code::INTERNAL, message)
    }

    pub fn store(message: impl Into<String>) -> Self {
        Self::new(code::STORE_ERROR, message)
    }

    /// The canonical code, for adapters that need to branch on it.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// The message after passing through the redaction filter, so an adapter
    /// can never leak a secret even if an upstream error interpolated one.
    pub fn redacted_message(&self) -> String {
        redaction::redact_text(&self.message)
    }

    /// The i18n key a localized adapter should render instead of `message`.
    pub fn i18n_key(&self) -> String {
        format!("capability.error.{}", self.code)
    }
}

impl std::fmt::Display for CapabilityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.redacted_message())
    }
}

impl std::error::Error for CapabilityError {}

impl From<crate::db::StoreError> for CapabilityError {
    fn from(error: crate::db::StoreError) -> Self {
        // Store errors can embed a JSON payload or a database message; both
        // are filtered before they can reach a log line, DTO or journal row.
        Self::store(redaction::redact_text(&error.to_string()))
    }
}

impl From<std::io::Error> for CapabilityError {
    fn from(error: std::io::Error) -> Self {
        Self::internal(redaction::redact_text(&error.to_string()))
    }
}

impl From<serde_json::Error> for CapabilityError {
    fn from(error: serde_json::Error) -> Self {
        Self::invalid_request(redaction::redact_text(&error.to_string()))
    }
}
