//! Explicit diagnostic sink integration.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error_diagnostics::{ErrorProjection, FailureMessage};
    use std::sync::Mutex;

    struct Capture(Mutex<Vec<String>>);
    impl log::Log for Capture {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.target() == "error_diagnostic"
        }
        fn log(&self, record: &log::Record<'_>) {
            if self.enabled(record.metadata()) {
                self.0.lock().unwrap().push(record.args().to_string());
            }
        }
        fn flush(&self) {}
    }
    static CAPTURE: Capture = Capture(Mutex::new(Vec::new()));

    #[test]
    fn existing_log_facade_receives_one_safe_json_report() {
        log::set_logger(&CAPTURE).expect("install focused test logger");
        log::set_max_level(log::LevelFilter::Error);
        log_error_diagnostic(&FailureMessage::leaf("Authorization: Bearer CANARY").diagnostic());
        let lines = CAPTURE.0.lock().unwrap();
        assert_eq!(lines.len(), 1);
        assert!(!lines[0].contains("CANARY"));
        let event: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(event["event"], "error_diagnostic");
    }
}
use super::types::ErrorDiagnostic;

/// Sanitizes and emits one bounded diagnostic event through the log facade.
pub fn log_error_diagnostic(diagnostic: &ErrorDiagnostic) {
    log::error!(target: "error_diagnostic", "{}", diagnostic.to_json_line());
}
