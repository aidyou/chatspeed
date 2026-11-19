//! Workflow window model and transport, independent of the GUI backend.
//!
//! Reads snapshots and live events and carries the user's actions — messages,
//! approvals and stops — back to the runtime.
pub mod chimes;
pub mod icons;
pub mod metrics;
pub mod model;
pub mod native;
pub mod prefs;
pub mod presentation;
pub mod types;
pub mod worker;

rust_i18n::i18n!("locales", fallback = "en");

#[cfg(test)]
mod locale_tests {
    #[test]
    fn observer_labels_resolve_in_every_interface_locale() {
        assert_eq!(
            rust_i18n::t!("sessions", locale = "en"),
            "Workflow sessions"
        );
        assert_eq!(rust_i18n::t!("sessions", locale = "zh-Hans"), "工作流会话");
        assert_eq!(rust_i18n::t!("sessions", locale = "zh-Hant"), "工作流會話");
        assert_ne!(
            rust_i18n::t!("metrics_error", locale = "en"),
            "metrics_error"
        );
    }
}
