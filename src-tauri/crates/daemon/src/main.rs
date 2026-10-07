//! `chatspeed-runtime` binary entry point.
//!
//! Runs the standalone runtime until the idle grace period expires with no
//! client leases or a termination signal arrives. The runtime owns exactly one
//! canonical backend (database, workflow hub, application service) and serves
//! the single canonical `/control/v1` HTTP/JSON + SSE control plane, so this
//! binary is the production desktop-free runtime rather than a meta-only stub.

use std::process::ExitCode;
use std::time::Duration;

use chatspeed_runtime::{start_runtime, RuntimeConfig};

/// Minimal stderr logger so the runtime needs no logging framework dependency.
struct StderrLogger;

impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!(
                "[{}] {}: {}",
                record.level(),
                record.target(),
                record.args()
            );
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

#[tokio::main]
async fn main() -> ExitCode {
    init_logging();

    let config = match RuntimeConfig::from_env() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("chatspeed-runtime: {error}");
            return ExitCode::FAILURE;
        }
    };

    let handle = match start_runtime(config).await {
        Ok(handle) => handle,
        Err(error) => {
            eprintln!("chatspeed-runtime: {error}");
            return ExitCode::FAILURE;
        }
    };

    tokio::select! {
        _ = handle.wait() => {}
        _ = termination_signal() => {
            log::info!("[Runtime] termination signal received; shutting down");
            handle.shutdown();
            let _ = tokio::time::timeout(Duration::from_secs(5), handle.wait()).await;
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
