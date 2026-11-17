//! Read-only observer model and transport, independent of the GUI backend.
pub mod icons;
pub mod metrics;
pub mod model;
pub mod presentation;
pub mod types;
pub mod worker;

rust_i18n::i18n!("locales", fallback = "en");

#[cfg(test)]
mod locale_tests {
    #[test]
    fn observer_labels_resolve_in_both_languages() {
        assert_eq!(
            rust_i18n::t!("sessions", locale = "en"),
            "Workflow sessions"
        );
        assert_eq!(rust_i18n::t!("sessions", locale = "zh-CN"), "工作流会话");
        assert_ne!(
            rust_i18n::t!("metrics_error", locale = "en"),
            "metrics_error"
        );
    }
}
