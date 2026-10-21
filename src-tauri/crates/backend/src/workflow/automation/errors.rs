//! Stable, transport-neutral error codes for the automation facade.
//!
//! Every adapter maps these codes to its own wire representation (Tauri string,
//! HTTP status + code, CLI exit code) without re-deriving the semantics, so the
//! same failed mutation reports identically everywhere (AC-9). The message is a
//! human diagnostic that the caller may surface; it must never carry raw shell
//! output, secrets or full prompts — those are redacted by the run projection,
//! not by this error type (INV-8).

#[cfg(not(feature = "desktop"))]
use crate::error_diagnostics::{ErrorDiagnostic, ErrorOrigin, ErrorProjection, FailureMessage};
#[cfg(not(feature = "desktop"))]
use chatspeed_contracts::error::{ErrorReason, UserErrorPresentation};

/// The set of stable automation error codes. Kept as `&'static str` constants so
/// the HTTP envelope can reuse them verbatim as its `code` field.
pub mod code {
    #[cfg(not(feature = "desktop"))]
    pub const INVALID_REQUEST: &str = "invalid_request";
    #[cfg(not(feature = "desktop"))]
    pub const NOT_FOUND: &str = "not_found";
    #[cfg(not(feature = "desktop"))]
    pub const CONFLICT: &str = "conflict";
    #[cfg(not(feature = "desktop"))]
    pub const REVISION_CONFLICT: &str = "revision_conflict";
    #[cfg(not(feature = "desktop"))]
    pub const PLAN_EXPIRED: &str = "plan_expired";
    #[cfg(not(feature = "desktop"))]
    pub const PERMISSION_EXPANSION: &str = "permission_expansion";
    #[cfg(not(feature = "desktop"))]
    pub const BUSY: &str = "busy";
    #[cfg(not(feature = "desktop"))]
    pub const CONFIRMATION_REQUIRED: &str = "confirmation_required";
    #[cfg(not(feature = "desktop"))]
    pub const NEEDS_RECONCILE: &str = "needs_reconcile";
    /// The runtime is quiescing and refuses new automation work. Reuses the one
    /// canonical wire code so every transport branches on the same token.
    #[cfg(not(feature = "desktop"))]
    pub const STOPPING: &str = chatspeed_contracts::RUNTIME_CODE_RUNTIME_STOPPING;
    #[cfg(not(feature = "desktop"))]
    pub const INTERNAL: &str = "internal";
}

/// A typed automation error carrying a stable code and a human message.
///
/// `origin` is an in-memory carrier only. It is populated from a real typed
/// cause before that cause is stringified, so the legacy `message`, `Display`
/// and `code` stay byte-for-byte compatible while a safe user projection and a
/// local diagnostic chain remain available.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(not(feature = "desktop"))]
pub struct AutomationError {
    code: &'static str,
    message: String,
    origin: Option<ErrorOrigin>,
}

#[cfg(not(feature = "desktop"))]
impl AutomationError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            origin: None,
        }
    }

    /// Attaches a typed origin captured before the caller flattened the cause.
    pub fn with_origin(mut self, origin: ErrorOrigin) -> Self {
        self.origin = Some(origin);
        self
    }

    /// Converts a typed error without losing its cause chain.
    ///
    /// `StoreError` already carries a structured origin (including the module
    /// context and any upstream root), so it is reused directly instead of
    /// being re-derived from its display text. Every other error contributes
    /// only its real standard-error chain; no cause is inferred from prose.
    pub fn internal_from<E>(error: E) -> Self
    where
        E: std::error::Error + 'static,
    {
        let legacy = error.to_string();
        let any: &dyn std::any::Any = &error;
        let origin = match any.downcast_ref::<crate::db::StoreError>() {
            Some(store) => store.error_origin(),
            None => match FailureMessage::from_error(&error).origin {
                Some(origin) => origin,
                None => ErrorOrigin::leaf(legacy.clone(), None, None, None),
            },
        };
        Self {
            code: code::INTERNAL,
            message: legacy,
            origin: Some(origin),
        }
    }

    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(code::INVALID_REQUEST, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(code::NOT_FOUND, message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(code::CONFLICT, message)
    }

    pub fn revision_conflict(message: impl Into<String>) -> Self {
        Self::new(code::REVISION_CONFLICT, message)
    }

    pub fn plan_expired(message: impl Into<String>) -> Self {
        Self::new(code::PLAN_EXPIRED, message)
    }

    pub fn permission_expansion(message: impl Into<String>) -> Self {
        Self::new(code::PERMISSION_EXPANSION, message)
    }

    pub fn busy(message: impl Into<String>) -> Self {
        Self::new(code::BUSY, message)
    }

    pub fn confirmation_required(message: impl Into<String>) -> Self {
        Self::new(code::CONFIRMATION_REQUIRED, message)
    }

    pub fn stopping(message: impl Into<String>) -> Self {
        let message = message.into();
        // A closed admission gate is a real classification, not a text guess:
        // the stable code and reason travel with the legacy message.
        Self::new(code::STOPPING, message.clone()).with_origin(ErrorOrigin::leaf(
            message,
            Some(ErrorReason::RuntimeStopping),
            Some(code::STOPPING.to_string()),
            None,
        ))
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(code::INTERNAL, message)
    }

    pub fn to_tauri_string(&self) -> String {
        let mut wire = serde_json::json!({
            "module": "automation",
            "message": self.message,
            "details": {
                "kind": self.code,
                "message": self.message,
            }
        });
        // Only a real typed cause contributes a presentation; a plain string
        // leaf keeps the historical shape unchanged.
        if self.origin.is_some() {
            if let Some(object) = wire.as_object_mut() {
                if let Ok(presentation) = serde_json::to_value(self.user_error()) {
                    object.insert("presentation".to_string(), presentation);
                }
            }
        }
        wire.to_string()
    }
}

#[cfg(not(feature = "desktop"))]
impl std::fmt::Display for AutomationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

#[cfg(not(feature = "desktop"))]
impl std::error::Error for AutomationError {}

#[cfg(not(feature = "desktop"))]
impl AutomationError {
    /// The in-memory typed origin, when the cause was captured before
    /// stringification.
    pub fn error_origin(&self) -> Option<&ErrorOrigin> {
        self.origin.as_ref()
    }
}

#[cfg(not(feature = "desktop"))]
impl ErrorProjection for AutomationError {
    fn user_error(&self) -> UserErrorPresentation {
        let origin = self.origin.as_ref();
        let message = origin
            .map(|value| value.message.as_str())
            .unwrap_or(self.message.as_str());
        UserErrorPresentation {
            schema_version: 1,
            message: crate::error_diagnostics::sanitize_user_message(message),
            reason: origin.and_then(|value| value.reason),
            upstream_status: origin.and_then(|value| value.status),
        }
    }

    fn diagnostic(&self) -> ErrorDiagnostic {
        let mut diagnostic = match &self.origin {
            Some(origin) => FailureMessage::leaf(self.message.clone())
                .with_origin(origin.clone())
                .diagnostic(),
            None => FailureMessage::leaf(self.message.clone()).diagnostic(),
        };
        diagnostic.module = "automation".to_string();
        diagnostic.operation = self.code.to_string();
        diagnostic.code = Some(self.code.to_string());
        diagnostic
    }
}

#[cfg(all(test, not(feature = "desktop")))]
mod tests {
    use super::*;

    #[test]
    fn plain_leaf_keeps_legacy_display_tauri_and_code() {
        let error = AutomationError::invalid_request("bad input");
        assert_eq!(error.to_string(), "bad input");
        assert_eq!(error.code(), code::INVALID_REQUEST);
        let wire: serde_json::Value =
            serde_json::from_str(&error.to_tauri_string()).expect("wire json");
        assert_eq!(wire["module"], "automation");
        assert_eq!(wire["message"], "bad input");
        assert_eq!(wire["details"]["kind"], code::INVALID_REQUEST);
        assert!(wire.get("presentation").is_none());
        assert!(error.error_origin().is_none());
    }

    #[test]
    fn typed_store_cause_preserves_legacy_message_and_projects_safe_origin() {
        let store = crate::db::StoreError::Query(
            FailureMessage::leaf("legacy query wrapper").with_origin(ErrorOrigin::leaf(
                "Authorization: Bearer CANARY",
                Some(ErrorReason::SessionStopping),
                Some("internal_error".to_string()),
                Some(503),
            )),
        );
        let error = AutomationError::internal_from(store);
        assert_eq!(error.code(), code::INTERNAL);
        assert_eq!(error.to_string(), "legacy query wrapper");
        let projection = error.user_error();
        assert_eq!(projection.reason, Some(ErrorReason::SessionStopping));
        assert_eq!(projection.upstream_status, Some(503));
        assert!(!projection.message.contains("CANARY"));
        assert!(!projection.message.is_empty());
        let diagnostic = error.diagnostic();
        assert_eq!(diagnostic.module, "automation");
        assert_eq!(diagnostic.code.as_deref(), Some(code::INTERNAL));
        assert_eq!(diagnostic.http_status, Some(503));
        assert_eq!(diagnostic.source_chain, Vec::<String>::new());
        let wire: serde_json::Value =
            serde_json::from_str(&error.to_tauri_string()).expect("wire json");
        assert_eq!(wire["message"], "legacy query wrapper");
        assert_eq!(wire["presentation"]["schema_version"], 1);
        assert!(!wire.to_string().contains("CANARY"));
    }

    #[test]
    fn stopping_projects_the_real_admission_classification() {
        let error = AutomationError::stopping("runtime is stopping");
        assert_eq!(error.code(), code::STOPPING);
        assert_eq!(error.to_string(), "runtime is stopping");
        let projection = error.user_error();
        assert_eq!(projection.reason, Some(ErrorReason::RuntimeStopping));
        assert_eq!(projection.message, "runtime is stopping");
        let wire: serde_json::Value =
            serde_json::from_str(&error.to_tauri_string()).expect("wire json");
        assert_eq!(
            wire["details"]["kind"],
            chatspeed_contracts::RUNTIME_CODE_RUNTIME_STOPPING
        );
        assert_eq!(wire["presentation"]["reason"], "runtime_stopping");
    }
}
