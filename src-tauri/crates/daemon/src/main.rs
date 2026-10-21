//! `chatspeed-runtime` binary entry point.
//!
//! Runs the standalone runtime until the idle grace period expires with no
//! client leases or a termination signal arrives. The runtime owns exactly one
//! canonical backend (database, workflow hub, application service) and serves
//! the single canonical `/control/v1` HTTP/JSON + SSE control plane, so this
//! binary is the production desktop-free runtime rather than a meta-only stub.

use std::io::Write;
use std::process::ExitCode;
use std::sync::Arc;

use chatspeed_runtime::{start_runtime, ProcessExitTerminationHook, RuntimeConfig};
use chatspeed_runtime_backend::environment;
use chatspeed_runtime_backend::error_diagnostics::{ErrorProjection, FailureMessage};
use std::error::Error;

/// Minimal stderr logger so the runtime needs no logging framework dependency.
struct StderrLogger;

/// Writes one sanitized diagnostic line to the existing stderr sink.
///
/// Exit codes and normal stdout stay untouched: only the final failure report
/// becomes structured, bounded JSON instead of an unstructured display string.
fn write_failure(
    writer: &mut impl Write,
    prefix: &str,
    error: &(dyn Error + 'static),
) -> std::io::Result<()> {
    let failure = FailureMessage::from_error(error);
    let mut diagnostic = failure.diagnostic();
    diagnostic.module = prefix.to_string();
    diagnostic.operation = "startup".to_string();
    writeln!(writer, "{}", diagnostic.to_json_line())
}

fn print_failure(prefix: &str, error: &(dyn Error + 'static)) {
    let stderr = std::io::stderr();
    let mut handle = stderr.lock();
    let _ = write_failure(&mut handle, prefix, error);
}

/// Project-owned targets keep the level they emitted; everything else is
/// treated as third-party noise.
///
/// The boundary check matters because the log facade reports crate paths, so a
/// crate merely *starting* with "chatspeed" (for example an unrelated
/// "chatspeedish" helper) must not inherit the project's verbosity.
fn is_project_target(target: &str) -> bool {
    target == "error_diagnostic"
        || target == "chatspeed"
        || target
            .strip_prefix("chatspeed")
            .is_some_and(|rest| rest.starts_with("::") || rest.starts_with('_'))
}

/// Third-party diagnostics are excluded from the runtime log below warning
/// level, so the desktop can preserve the child's level without admitting
/// dependency lifecycle noise to the application log file.
fn admitted_level(metadata: &log::Metadata<'_>, max_level: log::LevelFilter) -> Option<log::Level> {
    let level = metadata.level();
    if !is_project_target(metadata.target()) && level > log::Level::Warn {
        return None;
    }
    (level.to_level_filter() <= max_level).then_some(level)
}

// Keep the target in the transport header for filtering, not in the file message.
fn runtime_log_line(record: &log::Record<'_>, level: log::Level) -> String {
    match (record.file(), record.line()) {
        (Some(file), Some(line)) => format!(
            "[{}] {} [{}:{}]: {}",
            level,
            record.target(),
            file,
            line,
            record.args()
        ),
        _ => format!("[{}] {}: {}", level, record.target(), record.args()),
    }
}

impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        admitted_level(metadata, log::max_level()).is_some()
    }

    fn log(&self, record: &log::Record<'_>) {
        if let Some(level) = admitted_level(record.metadata(), log::max_level()) {
            eprintln!("{}", runtime_log_line(record, level));
        }
    }

    fn flush(&self) {}
}

fn init_logging() {
    let level = std::env::var("CHATSPEED_RUNTIME_LOG")
        .ok()
        .and_then(|value| value.parse::<log::LevelFilter>().ok())
        .unwrap_or(log::LevelFilter::Info);
    let logger: &'static StderrLogger = Box::leak(Box::new(StderrLogger));
    if log::set_logger(logger).is_ok() {
        log::set_max_level(level);
    }
}

fn main() -> ExitCode {
    init_logging();

    // The runtime owns subprocesses now; desktop PATH updates after a spawn
    // cannot reach it. Resolve the user environment before starting any worker
    // threads or publishing a control plane that can accept shell commands.
    let started = std::time::Instant::now();
    log::info!(
        "[Runtime] initializing process environment; pid={}",
        std::process::id()
    );
    environment::init_environment();
    log::info!(
        "[Runtime] process environment ready after {:?}",
        started.elapsed()
    );

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            print_failure("chatspeed-runtime", &error);
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(run())
}

async fn run() -> ExitCode {
    let config = match RuntimeConfig::from_env() {
        // Only the real binary installs the last-resort self-exit fallback. The
        // library default never terminates the host, so an embedding caller (or a
        // test) is never killed; here a forced shutdown that exhausts both budgets
        // ends with this instance exiting rather than lingering.
        Ok(config) => config.with_termination_hook(Arc::new(ProcessExitTerminationHook)),
        Err(error) => {
            print_failure("chatspeed-runtime", &error);
            return ExitCode::FAILURE;
        }
    };

    let handle = match start_runtime(config).await {
        Ok(handle) => handle,
        Err(error) => {
            print_failure("chatspeed-runtime", &error);
            return ExitCode::FAILURE;
        }
    };

    tokio::select! {
        _ = handle.wait() => {}
        _ = termination_signal() => {
            log::info!("[Runtime] termination signal received; shutting down");
            // The signal funnels into the same coordinated exit the idle
            // supervisor and an explicit request use: close the gate, drain
            // admitted work, then stop. It is not a fixed-timeout exit.
            handle.shutdown();
            handle.wait().await;
        }
    }

    ExitCode::SUCCESS
}

async fn termination_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut terminate = match signal(SignalKind::terminate()) {
            Ok(signal) => signal,
            Err(error) => {
                log::warn!("[Runtime] failed to install SIGTERM handler: {error}");
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests {
    use super::{admitted_level, is_project_target, runtime_log_line, write_failure};

    #[test]
    fn runtime_log_transport_preserves_source_and_target() {
        let record = log::Record::builder()
            .args(format_args!("[Workflow] completed: detail"))
            .level(log::Level::Info)
            .target("chatspeed_runtime_backend::workflow::react::manager")
            .file(Some("crates/backend/src/workflow/react/manager.rs"))
            .line(Some(42))
            .build();
        assert_eq!(
            runtime_log_line(&record, log::Level::Info),
            "[INFO] chatspeed_runtime_backend::workflow::react::manager [crates/backend/src/workflow/react/manager.rs:42]: [Workflow] completed: detail"
        );
        let without_source = log::Record::builder()
            .args(format_args!("diagnostic"))
            .level(log::Level::Warn)
            .target("rmcp::service")
            .build();
        assert_eq!(
            runtime_log_line(&without_source, log::Level::Warn),
            "[WARN] rmcp::service: diagnostic"
        );
    }

    fn metadata(level: log::Level, target: &'static str) -> log::Metadata<'static> {
        log::Metadata::builder().level(level).target(target).build()
    }

    #[test]
    fn info_max_keeps_project_levels_and_third_party_warning_errors() {
        use log::{Level, LevelFilter};

        assert_eq!(
            admitted_level(
                &metadata(Level::Info, "chatspeed_runtime_backend::mcp"),
                LevelFilter::Info
            ),
            Some(Level::Info)
        );
        assert_eq!(
            admitted_level(
                &metadata(Level::Info, "chatspeed::runtime"),
                LevelFilter::Info
            ),
            Some(Level::Info)
        );
        assert_eq!(
            admitted_level(
                &metadata(Level::Error, "error_diagnostic"),
                LevelFilter::Info
            ),
            Some(Level::Error)
        );
        assert_eq!(
            admitted_level(
                &metadata(Level::Debug, "chatspeed_runtime_backend::mcp"),
                LevelFilter::Info
            ),
            None
        );

        assert_eq!(
            admitted_level(&metadata(Level::Info, "rmcp::service"), LevelFilter::Info),
            None
        );
        assert_eq!(
            admitted_level(&metadata(Level::Debug, "rmcp::service"), LevelFilter::Info),
            None
        );
        assert_eq!(
            admitted_level(&metadata(Level::Warn, "rmcp::service"), LevelFilter::Info),
            Some(Level::Warn)
        );
        assert_eq!(
            admitted_level(
                &metadata(Level::Error, "hyper::proto::h1"),
                LevelFilter::Info
            ),
            Some(Level::Error)
        );
    }

    #[test]
    fn debug_max_still_rejects_third_party_info_and_lower() {
        use log::{Level, LevelFilter};

        assert_eq!(
            admitted_level(&metadata(Level::Info, "rmcp::service"), LevelFilter::Debug),
            None
        );
        assert_eq!(
            admitted_level(&metadata(Level::Debug, "tracing::span"), LevelFilter::Debug),
            None
        );
        assert_eq!(
            admitted_level(&metadata(Level::Trace, "rmcp::service"), LevelFilter::Debug),
            None
        );
        assert_eq!(
            admitted_level(&metadata(Level::Warn, "rmcp::service"), LevelFilter::Debug),
            Some(Level::Warn)
        );
        assert_eq!(
            admitted_level(&metadata(Level::Error, "rmcp::service"), LevelFilter::Debug),
            Some(Level::Error)
        );

        assert_eq!(
            admitted_level(
                &metadata(Level::Info, "chatspeed_runtime_backend::mcp"),
                LevelFilter::Debug
            ),
            Some(Level::Info)
        );
        assert_eq!(
            admitted_level(
                &metadata(Level::Trace, "chatspeed_runtime_backend::mcp"),
                LevelFilter::Debug
            ),
            None
        );
    }

    #[test]
    fn project_targets_stop_at_a_crate_boundary() {
        for target in [
            "chatspeed",
            "chatspeed::runtime",
            "chatspeed_runtime_backend::mcp",
            "error_diagnostic",
        ] {
            assert!(
                is_project_target(target),
                "{target} should be project-owned"
            );
        }
        for target in ["chatspeedish", "external::chatspeed", "rmcp::service"] {
            assert!(!is_project_target(target), "{target} should be third-party");
        }
    }

    #[test]
    fn final_failure_is_one_sanitized_json_line_on_the_stderr_sink() {
        let error = std::io::Error::other("Authorization: Bearer CANARY");
        let mut output = Vec::new();
        write_failure(&mut output, "chatspeed-runtime", &error).expect("write diagnostic");

        let text = String::from_utf8(output).expect("utf-8 diagnostic");
        assert_eq!(text.matches('\n').count(), 1);
        let line = text.trim_end_matches('\n');
        assert!(!line.contains('\n'));
        assert!(!line.contains("CANARY"));
        let event: serde_json::Value = serde_json::from_str(line).expect("json diagnostic");
        assert_eq!(event["event"], "error_diagnostic");
        assert_eq!(event["module"], "chatspeed-runtime");
        assert_eq!(event["operation"], "startup");
        assert!(event["error"]
            .as_str()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .contains("[redacted]"));
    }
}
