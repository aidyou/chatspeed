//! Stable, transport-neutral error codes for the automation facade.
//!
//! Every adapter maps these codes to its own wire representation (Tauri string,
//! HTTP status + code, CLI exit code) without re-deriving the semantics, so the
//! same failed mutation reports identically everywhere (AC-9). The message is a
//! human diagnostic that the caller may surface; it must never carry raw shell
//! output, secrets or full prompts — those are redacted by the run projection,
//! not by this error type (INV-8).

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
#[cfg(not(feature = "desktop"))]
    pub const INTERNAL: &str = "internal";
}

/// A typed automation error carrying a stable code and a human message.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(not(feature = "desktop"))]
pub struct AutomationError {
    code: &'static str,
    message: String,
}

#[cfg(not(feature = "desktop"))]
impl AutomationError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
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

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(code::INTERNAL, message)
    }

    pub fn to_tauri_string(&self) -> String {
        serde_json::json!({
            "module": "automation",
            "message": self.message,
            "details": {
                "kind": self.code,
                "message": self.message,
            }
        })
        .to_string()
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
