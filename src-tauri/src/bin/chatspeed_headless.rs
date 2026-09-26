//! `chatspeed-headless` — the windowless ChatSpeed runtime owner.
//!
//! It owns exactly one ChatSpeed runtime for one explicit experiment data
//! directory: one database, one workflow runtime, one loopback control plane.
//! It never opens the desktop database, never starts a window/tray/updater, and
//! never falls back to an in-memory database.
//!
//! Usage:
//!
//! ```text
//! chatspeed-headless --data-dir <dir> [--config-package <file> --config-category <name>]...
//!                                    [--api-key-file <file>]
//! ```
//!
//! Point a client at the instance with `--discovery-file
//! <data-dir>/runtime/control-plane-v1.json` (see `cs --help`).
//!
//! The process runs until it receives `SIGINT`/`SIGTERM`, then shuts the
//! control plane down and releases its experiment-domain lease.

use chatspeed_lib::headless::{self, ConfigCategory, HeadlessError, HeadlessOptions};
use clap::{Parser, ValueEnum};
use std::path::PathBuf;

/// Config-package categories an operator may import explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ConfigCategoryArg {
    AiModels,
    Skills,
    Mcp,
    Proxy,
    Agents,
    Sandbox,
}

impl ConfigCategoryArg {
    fn to_category(self) -> ConfigCategory {
        match self {
            ConfigCategoryArg::AiModels => ConfigCategory::AiModels,
            ConfigCategoryArg::Skills => ConfigCategory::Skills,
            ConfigCategoryArg::Mcp => ConfigCategory::Mcp,
            ConfigCategoryArg::Proxy => ConfigCategory::Proxy,
            ConfigCategoryArg::Agents => ConfigCategory::Agents,
            ConfigCategoryArg::Sandbox => ConfigCategory::Sandbox,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "chatspeed-headless",
    about = "Windowless ChatSpeed runtime owner for an isolated experiment data domain",
    version
)]
struct Args {
    /// Explicit experiment data directory. Created and marked on first use;
    /// an existing database without the experiment marker is refused.
    #[arg(long, value_name = "DIR")]
    data_dir: PathBuf,

    /// Optional config package to import before the runtime starts.
    #[arg(long, value_name = "FILE", requires = "config_category")]
    config_package: Option<PathBuf>,

    /// Config-package category to import. Repeat to select several; the import
    /// is never wholesale.
    #[arg(long = "config-category", value_name = "CATEGORY")]
    config_category: Vec<ConfigCategoryArg>,

    /// Optional permission-restricted API key file to activate.
    #[arg(long, value_name = "FILE")]
    api_key_file: Option<PathBuf>,
    /// Operator-configured base repository the scheduled execution owners use.
    #[arg(long)]
    base_repo: Option<PathBuf>,
}

fn main() {
    let args = Args::parse();
    let options = build_options(args);

    // A windowless instance never reaches the desktop's file-backed logger, so
    // the runtime installs its own stderr sink first: without it a scheduler or
    // owner failure would be invisible (CHATSPEED_LOG selects the level).
    chatspeed_lib::headless::logging::install_stderr_logger();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap_or_else(|error| {
            fail(&HeadlessError::new(
                "runtime_unavailable",
                error.to_string(),
            ))
        });

    match runtime.block_on(run(options)) {
        Ok(()) => {}
        Err(error) => fail(&error),
    }
}

/// Maps the parsed command line onto the headless startup options.
///
/// Every operator-visible flag must appear here: a flag the parser accepts but
/// this mapping drops would silently change runtime behaviour (an ignored
/// `--base-repo`, for instance, makes the scheduler fail every job closed).
fn build_options(args: Args) -> HeadlessOptions {
    let mut options = HeadlessOptions::new(args.data_dir);
    options.config_package = args.config_package;
    options.config_categories = args
        .config_category
        .into_iter()
        .map(ConfigCategoryArg::to_category)
        .collect();
    options.api_key_file = args.api_key_file;
    options.base_repo = args.base_repo;
    options
}

async fn run(options: HeadlessOptions) -> Result<(), HeadlessError> {
    let instance = headless::start(options).await?;
    eprintln!(
        "chatspeed-headless: domain {} listening on 127.0.0.1:{} (instance {})",
        instance.domain_id(),
        instance.control_plane_port(),
        instance.server_instance_id()
    );
    eprintln!(
        "chatspeed-headless: discovery file {}",
        instance.discovery_file().display()
    );

    shutdown_signal().await;
    eprintln!("chatspeed-headless: shutting down");
    instance.shutdown().await;
    Ok(())
}

/// Waits for `SIGINT` or `SIGTERM`.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(stream) => stream,
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

fn fail(error: &HeadlessError) -> ! {
    eprintln!("chatspeed-headless: {}: {}", error.code, error.message);
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// Every operator-visible flag must survive the mapping onto the startup
    /// options; a dropped flag silently changes runtime behaviour.
    #[test]
    fn the_command_line_flags_reach_the_startup_options() {
        let args = Args::try_parse_from([
            "chatspeed-headless",
            "--data-dir",
            "domain-data",
            "--api-key-file",
            "key.json",
            "--base-repo",
            "base-repo",
        ])
        .expect("parse");
        let options = build_options(args);
        assert_eq!(options.data_dir, PathBuf::from("domain-data"));
        assert_eq!(options.api_key_file, Some(PathBuf::from("key.json")));
        assert_eq!(
            options.base_repo,
            Some(PathBuf::from("base-repo")),
            "an ignored --base-repo makes the scheduler fail every job closed"
        );
        assert!(options.config_package.is_none());

        // The default is an explicit absence, never an inferred path.
        let options = build_options(
            Args::try_parse_from(["chatspeed-headless", "--data-dir", "domain-data"])
                .expect("parse"),
        );
        assert!(options.base_repo.is_none());
        assert!(options.api_key_file.is_none());
    }

    /// A config package cannot be imported without an explicit category.
    #[test]
    fn a_config_package_requires_an_explicit_category() {
        let error = Args::try_parse_from([
            "chatspeed-headless",
            "--data-dir",
            "domain-data",
            "--config-package",
            "package.json",
        ])
        .expect_err("category required");
        let rendered = error.to_string();
        assert!(rendered.contains("config-category") || rendered.contains("config_category"));
    }
}
