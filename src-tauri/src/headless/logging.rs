//! A dependency-free stderr logger for the headless runtime (Phase 2H).
//!
//! The desktop application installs a file-backed logger from its Tauri setup,
//! which a windowless instance never reaches: without a sink the scheduler's
//! `warn!`/`info!` records are dropped, and a headless operator has no way to
//! observe a dispatch failure beyond the durable `error_code`. This module
//! installs the smallest possible `log` sink on stderr, so the headless
//! runtime is observable without adding a logging crate.

use log::{LevelFilter, Log, Metadata, Record};
use std::io::Write;

/// How loud the headless stderr sink is by default.
const DEFAULT_LEVEL: LevelFilter = LevelFilter::Info;

struct StderrLogger {
    level: LevelFilter,
}

impl Log for StderrLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        // Best effort: a closed stderr must never panic a runtime worker.
        let _ = writeln!(
            std::io::stderr(),
            "{} [{}] {}: {}",
            crate::headless::domain::now_ms(),
            record.level(),
            record.target(),
            record.args()
        );
    }

    fn flush(&self) {
        let _ = std::io::stderr().flush();
    }
}

/// Parses a level name, accepting the usual `error|warn|info|debug|trace`.
fn parse_level(raw: &str) -> Option<LevelFilter> {
    Some(match raw.trim().to_ascii_lowercase().as_str() {
        "error" => LevelFilter::Error,
        "warn" | "warning" => LevelFilter::Warn,
        "info" => LevelFilter::Info,
        "debug" => LevelFilter::Debug,
        "trace" => LevelFilter::Trace,
        "off" => LevelFilter::Off,
        _ => return None,
    })
}

/// Installs the stderr sink, honouring `CHATSPEED_LOG` when it names a level.
///
/// It is idempotent-safe: a second call is simply ignored, so a caller never
/// replaces a logger another part of the process already installed.
pub fn install_stderr_logger() {
    let level = std::env::var("CHATSPEED_LOG")
        .ok()
        .and_then(|raw| parse_level(&raw))
        .unwrap_or(DEFAULT_LEVEL);
    let logger = Box::new(StderrLogger { level });
    if log::set_boxed_logger(logger).is_ok() {
        log::set_max_level(level);
    }
}

/// The level names [`install_stderr_logger`] accepts, for the CLI help text.
pub const LEVEL_NAMES: &str = "error|warn|info|debug|trace|off";

#[cfg(test)]
mod tests {
    use super::*;
    use log::Level;

    #[test]
    fn level_names_are_parsed_and_unknown_names_are_refused() {
        assert_eq!(parse_level("WARN"), Some(LevelFilter::Warn));
        assert_eq!(parse_level(" debug "), Some(LevelFilter::Debug));
        assert_eq!(parse_level("off"), Some(LevelFilter::Off));
        assert_eq!(parse_level("chatty"), None);
    }

    #[test]
    fn the_default_level_keeps_the_scheduler_visible() {
        // A dispatch failure is reported with `warn!`, so the default must not
        // filter it out.
        assert!(Level::Warn <= DEFAULT_LEVEL);
    }
}
