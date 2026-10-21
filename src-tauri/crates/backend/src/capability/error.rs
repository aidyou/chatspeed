//! Stable, transport-neutral capability error contract.
//!
//! Every capability adapter (Tauri command, `/control/v1` handler, `cs` CLI)
//! maps this type into its own wire form, but the `code` is the canonical
//! machine contract: a client must be able to branch on `code` without
//! matching localized text. Messages are always non-secret.

use serde::{Deserialize, Serialize};

use crate::error_diagnostics::{ErrorDiagnostic, ErrorOrigin, ErrorProjection, FailureMessage};
use chatspeed_contracts::error::UserErrorPresentation;

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
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn legacy_envelope_omits_origin_and_typed_store_cause_survives() {
        let error = CapabilityError::from(crate::db::StoreError::Query(
            FailureMessage::leaf("legacy query wrapper").with_origin(ErrorOrigin::leaf(
                "root token=CANARY",
                None,
                None,
                Some(503),
            )),
        ));
        assert_eq!(error.code(), code::STORE_ERROR);
        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            json!({"code":"store_error","message":"legacy query wrapper"})
        );
        assert_eq!(error.user_error().upstream_status, Some(503));
        assert!(error.user_error().message.starts_with("root"));
        assert!(!error.user_error().message.contains("CANARY"));
        let restored: CapabilityError =
            serde_json::from_value(json!({"code":"store_error", "message":"old"})).unwrap();
        assert!(restored.error_origin().is_none());
        assert_eq!(restored.to_string(), "store_error: old");
    }

    #[test]
    fn typed_io_and_json_keep_existing_classification() {
        let io = CapabilityError::from(std::io::Error::other("token=CANARY"));
        assert_eq!(io.code(), code::INTERNAL);
        assert!(!io.user_error().message.contains("CANARY"));
        let json_error = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
        let error = CapabilityError::from(json_error);
        assert_eq!(error.code(), code::INVALID_REQUEST);
        assert!(error.origin.is_some());
    }
}

/// A capability failure with a stable code and a non-secret message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityError {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation: Option<UserErrorPresentation>,
    #[serde(skip, default)]
    pub(crate) origin: Option<ErrorOrigin>,
}

impl CapabilityError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            presentation: None,
            origin: None,
        }
    }

    /// Attaches the runtime's safe presentation plus the explicit origin it
    /// describes. The origin is built only from structured fields, never from
    /// message text, so a keyword in the legacy message cannot invent a reason.
    pub fn with_user_error(mut self, presentation: UserErrorPresentation) -> Self {
        self.origin = Some(ErrorOrigin::leaf(
            presentation.message.clone(),
            presentation.reason,
            Some(self.code.clone()),
            presentation.upstream_status,
        ));
        self.presentation = Some(presentation);
        self
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

impl ErrorProjection for CapabilityError {
    fn user_error(&self) -> UserErrorPresentation {
        let presentation = self.origin.as_ref().map(|origin| {
            let mut value = origin.clone();
            value.message = redaction::redact_text(&value.message);
            value
        });
        let message = presentation
            .as_ref()
            .map(|origin| origin.message.clone())
            .unwrap_or_else(|| self.redacted_message());
        UserErrorPresentation {
            schema_version: 1,
            message: crate::error_diagnostics::sanitize_user_message(&message),
            reason: presentation.as_ref().and_then(|origin| origin.reason),
            upstream_status: presentation.as_ref().and_then(|origin| origin.status),
        }
    }

    fn diagnostic(&self) -> ErrorDiagnostic {
        let mut diagnostic = self
            .origin
            .as_ref()
            .map(|origin| {
                FailureMessage::leaf(self.message.clone())
                    .with_origin(origin.clone())
                    .diagnostic()
            })
            .unwrap_or_else(|| FailureMessage::leaf(self.message.clone()).diagnostic());
        diagnostic.code = Some(self.code.clone());
        diagnostic.error = redaction::redact_text(&diagnostic.error);
        diagnostic
    }
}

impl CapabilityError {
    pub fn error_origin(&self) -> Option<&ErrorOrigin> {
        self.origin.as_ref()
    }
}

impl From<crate::db::StoreError> for CapabilityError {
    fn from(error: crate::db::StoreError) -> Self {
        let mut result = Self::store(redaction::redact_text(&error.to_string()));
        result.origin = Some(error.error_origin());
        result
    }
}

impl From<std::io::Error> for CapabilityError {
    fn from(error: std::io::Error) -> Self {
        let mut result = Self::internal(redaction::redact_text(&error.to_string()));
        result.origin = Some(ErrorOrigin::from_error(&error));
        result
    }
}

impl From<serde_json::Error> for CapabilityError {
    fn from(error: serde_json::Error) -> Self {
        let mut result = Self::invalid_request(redaction::redact_text(&error.to_string()));
        result.origin = Some(ErrorOrigin::from_error(&error));
        result
    }
}
