//! Transport-neutral headless runtime bootstrap (Phase 2H).
//!
//! `chatspeed-headless` owns exactly one ChatSpeed runtime for its experiment
//! domain: one `MainStore`, one workflow runtime hub, one session lifecycle
//! manager, one sub-agent factory, one `WorkflowApplicationService` and one
//! loopback control plane. It starts *no* window, tray, updater, static server
//! or desktop automation, and it never falls back to an in-memory database
//! (AC-1, INV-1).
//!
//! Everything enforceable before a runtime exists is enforced first:
//!
//! 1. the data directory must be an explicit, usable experiment domain;
//! 2. an optional config package is imported for explicitly selected
//!    categories only, never wholesale;
//! 3. an optional API key file is activated;
//! 4. the credential state must not be locked, otherwise startup fails closed
//!    instead of starting a runtime that cannot reach a provider.
//!
//! Only after all four succeed are the store, runtime and control plane built.

use crate::ai::interaction::chat_completion::ChatState;
use crate::db::config_transfer::{import_config_package, ConfigCategory};
use crate::db::experiment_schedule::ExperimentScheduleStore;
use crate::db::MainStore;
use crate::headless::domain::{now_ms, ExperimentDomain, ExperimentDomainLease};
use crate::headless::scheduler_runtime::{DomainSchedulerResources, ScheduledCampaignKernel};
use crate::libs::tsid::TsidGenerator;
use crate::libs::window_channels::WindowChannels;
use crate::workflow::react::application::WorkflowApplicationService;
use crate::workflow::react::client::http::server::ControlPlaneHandle;
use crate::workflow::react::client::hub::{NoWindowTransport, WorkflowRuntimeHub};
use crate::workflow::react::experiment_schedule::scheduler::{
    CampaignScheduler, SchedulerConfig, TickOutcome,
};
use crate::workflow::react::experiment_schedule::types::{ScheduleError, ScheduleErrorCode};
use crate::workflow::react::manager::WorkflowManager;
use crate::workflow::react::orchestrator::{DefaultSubAgentFactory, SubAgentFactory};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// How often the process renews its experiment-domain lease. It is well inside
/// `DEFAULT_DOMAIN_LEASE_MS` so a healthy process never loses the domain.
pub const DOMAIN_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

/// Stable machine codes for headless bootstrap failures.
pub mod code {
    /// The data directory or its layout cannot be used.
    pub const DOMAIN_LAYOUT_UNSAFE: &str = "experiment_domain_layout_unsafe";
    /// The database is not a marked experiment domain.
    pub const DOMAIN_UNMARKED: &str = "experiment_domain_unmarked";
    /// Another live instance holds the domain.
    pub const DOMAIN_LOCKED: &str = "experiment_domain_locked";
    /// The config package could not be inspected or imported.
    pub const CONFIG_IMPORT_FAILED: &str = "config_import_failed";
    /// A config package was given without an explicit category selection.
    pub const CONFIG_CATEGORY_REQUIRED: &str = "config_category_required";
    /// The API key file could not be activated.
    pub const API_KEY_FILE_FAILED: &str = "api_key_file_failed";
    /// Stored API keys cannot be decrypted in this environment.
    pub const API_KEY_LOCKED: &str = "api_key_locked";
    /// The runtime could not be assembled.
    pub const RUNTIME_UNAVAILABLE: &str = "runtime_unavailable";
    /// The control plane could not be started.
    pub const CONTROL_PLANE_FAILED: &str = "control_plane_failed";
    /// The instance's own loopback chat-completion proxy could not be started.
    pub const CCPROXY_FAILED: &str = "ccproxy_failed";
    /// An agent in this domain requires a capability a headless instance does
    /// not provide (the Tauri-bound web tools).
    pub const WEB_TOOLS_UNSUPPORTED: &str = "web_tools_unsupported";
}

/// A fail-closed headless bootstrap error carrying a stable machine code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadlessError {
    pub code: &'static str,
    pub message: String,
}

impl HeadlessError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Maps a domain/schedule failure onto the headless surface, keeping the
    /// stable code so an operator sees one vocabulary end to end.
    pub fn from_schedule(error: ScheduleError) -> Self {
        let code = match error.code {
            ScheduleErrorCode::DomainLayoutUnsafe => code::DOMAIN_LAYOUT_UNSAFE,
            ScheduleErrorCode::DomainUnmarked => code::DOMAIN_UNMARKED,
            ScheduleErrorCode::DomainLocked => code::DOMAIN_LOCKED,
            _ => code::RUNTIME_UNAVAILABLE,
        };
        Self::new(code, error.message)
    }
}

impl std::fmt::Display for HeadlessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for HeadlessError {}

/// Startup options, resolved from the command line by the binary.
#[derive(Debug, Clone)]
pub struct HeadlessOptions {
    /// Explicit experiment data directory. Never inferred, never `:memory:`.
    pub data_dir: PathBuf,
    /// Optional checked-in config package to import.
    pub config_package: Option<PathBuf>,
    /// Explicit category selection for the config package. Required whenever a
    /// package is supplied, so an import is never wholesale by accident.
    pub config_categories: Vec<ConfigCategory>,
    /// Optional permission-restricted API key file to activate.
    pub api_key_file: Option<PathBuf>,
    /// Operator-configured base repository for the filesystem and container
    /// execution owners.
    ///
    /// It is server-side configuration: a job, a candidate or a CLI caller can
    /// never supply a host path, and a domain without it fails scheduled jobs
    /// closed instead of falling back to the host working tree.
    pub base_repo: Option<PathBuf>,
}

impl HeadlessOptions {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
            config_package: None,
            config_categories: Vec::new(),
            api_key_file: None,
            base_repo: None,
        }
    }
}

/// A running headless instance.
pub struct HeadlessRuntime {
    domain: ExperimentDomain,
    application: Arc<WorkflowApplicationService>,
    schedule_store: ExperimentScheduleStore,
    control_plane: ControlPlaneHandle,
    /// This instance's own loopback chat-completion proxy. `group@alias` models
    /// are resolved through it, so it is part of the runtime, not an add-on.
    ccproxy: crate::ccproxy::launcher::CcproxyServer,
    heartbeat: tokio::task::JoinHandle<()>,
    scheduler: SchedulerSupervisor,
}

/// The supervised durable-schedule loop.
struct SchedulerSupervisor {
    shutdown: tokio::sync::watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl HeadlessRuntime {
    /// The loopback control-plane port this instance publishes.
    pub fn control_plane_port(&self) -> u16 {
        self.control_plane.port
    }

    /// The instance id used by the discovery document.
    pub fn server_instance_id(&self) -> &str {
        &self.control_plane.server_instance_id
    }

    /// The discovery document path a client must be pointed at.
    pub fn discovery_file(&self) -> PathBuf {
        self.domain.paths().discovery_file()
    }

    pub fn domain_id(&self) -> &str {
        self.domain.domain_id()
    }

    /// The durable schedule store of this domain.
    pub fn schedule_store(&self) -> &ExperimentScheduleStore {
        &self.schedule_store
    }

    /// The loopback address of this instance's own chat-completion proxy.
    ///
    /// A headless instance resolves `group@alias` models through its *own*
    /// proxy: the proxy address and its internal key are process-local, so
    /// sharing another instance's listener would authenticate with a key that
    /// listener never issued.
    pub fn ccproxy_addr(&self) -> std::net::SocketAddr {
        self.ccproxy.addr()
    }

    /// Stops the control plane, stops the lease heartbeat and releases the
    /// domain lease. A superseded instance cannot release a newer one's lease.
    pub async fn shutdown(self) {
        let HeadlessRuntime {
            domain,
            application,
            control_plane,
            ccproxy,
            heartbeat,
            scheduler,
            ..
        } = self;
        // Stop admitting new scheduled work, then release everything else.
        let _ = scheduler.shutdown.send(true);
        scheduler.task.abort();
        heartbeat.abort();
        control_plane.shutdown();
        ccproxy.shutdown().await;
        // Give the server a moment to remove its own discovery document before
        // the store is dropped.
        tokio::time::sleep(Duration::from_millis(50)).await;
        match domain.lease().release(domain.store()) {
            Ok(()) => log::info!("[Headless] Released experiment domain lease"),
            Err(error) => log::warn!("[Headless] Failed to release domain lease: {error}"),
        }
        drop(application);
        log::info!("[Headless] Stopped");
    }
}

impl std::fmt::Debug for HeadlessRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The runtime owns live handles (store, control plane, heartbeat task);
        // only the identity that is meaningful to an operator is rendered.
        f.debug_struct("HeadlessRuntime")
            .field("domain_id", &self.domain.domain_id())
            .field("discovery_file", &self.discovery_file())
            .field("control_plane_port", &self.control_plane.port)
            .field("server_instance_id", &self.control_plane.server_instance_id)
            .finish_non_exhaustive()
    }
}

/// The per-instance owner id: stable for the process lifetime, unique per
/// process, and never derived from user data.
fn owner_id() -> String {
    use rand::Rng;
    let mut bytes = [0u8; 8];
    rand::rng().fill_bytes(&mut bytes);
    format!("headless-{}-{}", std::process::id(), hex::encode(bytes))
}

/// Refuses to start when an agent in this domain explicitly requires a tool a
/// headless runtime does not provide.
///
/// The desktop app registers `WebSearch`/`WebFetch`, but both read Tauri
/// webview state, so a headless process cannot provide them. The contract says
/// the gap must fail closed with an explicit rejection rather than silently
/// dropping the tools (INV-4): an operator who asked for a web-capable agent
/// gets a clear startup error instead of a quietly less capable run.
fn preflight_capabilities(store: &Arc<MainStore>) -> Result<(), HeadlessError> {
    use crate::tools::{TOOL_WEB_FETCH, TOOL_WEB_SEARCH};
    let agents = store.get_all_agents().map_err(|error| {
        HeadlessError::new(
            code::RUNTIME_UNAVAILABLE,
            format!("failed to read the agents of this domain: {error}"),
        )
    })?;
    for agent in agents {
        let requested: Vec<String> = agent
            .available_tools
            .as_deref()
            .and_then(|raw| serde_json::from_str(raw).ok())
            .unwrap_or_default();
        if let Some(tool) = requested
            .iter()
            .find(|tool| tool.as_str() == TOOL_WEB_SEARCH || tool.as_str() == TOOL_WEB_FETCH)
        {
            return Err(HeadlessError::new(
                code::WEB_TOOLS_UNSUPPORTED,
                format!(
                    "agent '{}' requires the web tool '{}', which a headless instance does not \
                     provide; remove it from the agent or run this domain on the desktop app",
                    agent.id, tool
                ),
            ));
        }
    }
    Ok(())
}

/// Imports the optional config package. The category selection is explicit, so
/// a package can never bring in categories the operator did not ask for.
fn import_config(options: &HeadlessOptions, store: &Arc<MainStore>) -> Result<(), HeadlessError> {
    let Some(path) = options.config_package.as_deref() else {
        if !options.config_categories.is_empty() {
            log::warn!("[Headless] --config-category was given without --config-package");
        }
        return Ok(());
    };
    if !path.exists() {
        return Err(HeadlessError::new(
            code::CONFIG_IMPORT_FAILED,
            format!("config package '{}' does not exist", path.display()),
        ));
    }
    if options.config_categories.is_empty() {
        return Err(HeadlessError::new(
            code::CONFIG_CATEGORY_REQUIRED,
            "a config package must be imported with an explicit --config-category selection",
        ));
    }
    let result =
        import_config_package(store, path, options.config_categories.clone()).map_err(|error| {
            HeadlessError::new(
                code::CONFIG_IMPORT_FAILED,
                format!("failed to import '{}': {error}", path.display()),
            )
        })?;
    // Only counts are logged: a package may carry profile metadata, and no
    // credential value may ever reach a log line.
    log::info!(
        "[Headless] Imported config package categories: {}",
        result
            .imported_counts
            .iter()
            .map(|(category, count)| format!("{category:?}={count}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(())
}

/// Activates the optional API key file, then refuses to continue when the
/// stored keys still cannot be decrypted in this environment.
fn activate_credentials(
    options: &HeadlessOptions,
    store: &Arc<MainStore>,
) -> Result<(), HeadlessError> {
    if let Some(path) = options.api_key_file.as_deref() {
        if !path.exists() {
            return Err(HeadlessError::new(
                code::API_KEY_FILE_FAILED,
                format!("api key file '{}' does not exist", path.display()),
            ));
        }
        store.activate_api_key_file(path).map_err(|error| {
            HeadlessError::new(
                code::API_KEY_FILE_FAILED,
                format!("failed to activate '{}': {error}", path.display()),
            )
        })?;
        log::info!("[Headless] Activated API key file from the configured path");
    }
    let status = store.api_key_encryption_status().map_err(|error| {
        HeadlessError::new(
            code::API_KEY_LOCKED,
            format!("failed to inspect API key encryption status: {error}"),
        )
    })?;
    if status.is_locked() {
        return Err(HeadlessError::new(
            code::API_KEY_LOCKED,
            format!(
                "stored API keys cannot be decrypted in this environment; provide --api-key-file \
                 ({})",
                status
                    .reason
                    .unwrap_or_else(|| "no reason reported".to_string())
            ),
        ));
    }
    Ok(())
}

/// Starts a headless runtime for the given options.
///
/// The order is the safety contract: validate the domain, then credentials,
/// then build the runtime, then publish the control plane. Any failure before
/// the last step leaves no runtime, no discovery document and — critically —
/// no held domain lease behind, so a corrected retry can take the domain.
pub async fn start(options: HeadlessOptions) -> Result<HeadlessRuntime, HeadlessError> {
    if options.data_dir.as_os_str().is_empty() {
        return Err(HeadlessError::new(
            code::DOMAIN_LAYOUT_UNSAFE,
            "chatspeed-headless requires an explicit --data-dir",
        ));
    }

    let owner = owner_id();
    let domain =
        ExperimentDomain::open(&options.data_dir, &owner).map_err(HeadlessError::from_schedule)?;
    let store = domain.store().clone();

    let application = match prepare(&options, &domain).await {
        Ok(application) => application,
        Err(error) => {
            // A half-started instance must not hold the domain: release the
            // lease we just took before reporting the failure.
            release_domain_lease(&domain);
            return Err(error);
        }
    };

    let control_plane =
        match crate::workflow::react::client::http::server::start_with_discovery_dir(
            application.clone(),
            Some(domain.paths().runtime.clone()),
        )
        .await
        {
            Ok(control_plane) => control_plane,
            Err(error) => {
                release_domain_lease(&domain);
                return Err(HeadlessError::new(code::CONTROL_PLANE_FAILED, error));
            }
        };

    let heartbeat = spawn_domain_heartbeat(domain.store().clone(), domain.lease().clone());
    let scheduler = spawn_scheduler(&options, domain.paths().root.clone(), application.clone());

    // The instance's own loopback chat-completion proxy. A `group@alias` model is
    // resolved through this listener with a process-local key, so a runtime that
    // does not own its proxy cannot run those models at all.
    let ccproxy = match crate::ccproxy::launcher::start(
        store.clone(),
        application.chat_state.clone(),
        env!("CARGO_PKG_VERSION").to_string(),
    )
    .await
    {
        Ok(ccproxy) => ccproxy,
        Err(error) => {
            let _ = scheduler.shutdown.send(true);
            scheduler.task.abort();
            heartbeat.abort();
            control_plane.shutdown();
            release_domain_lease(&domain);
            return Err(HeadlessError::new(code::CCPROXY_FAILED, error));
        }
    };
    log::info!(
        "[Headless] Chat-completion proxy on {} (published to the in-process client)",
        ccproxy.base_url()
    );

    log::info!(
        "[Headless] Running domain {} for data dir {} on port {} (instance {})",
        domain.domain_id(),
        domain.paths().root.display(),
        control_plane.port,
        control_plane.server_instance_id
    );

    let schedule_store = ExperimentScheduleStore::new(store);
    Ok(HeadlessRuntime {
        domain,
        application,
        schedule_store,
        control_plane,
        ccproxy,
        heartbeat,
        scheduler,
    })
}

/// Spawns the durable-schedule loop for this domain.
///
/// One tick is synchronous filesystem/process work, so it runs on a blocking
/// thread and never stalls the async runtime. The scheduler's own `tick`
/// processes at most `max_jobs_per_tick` jobs and holds no job open across
/// ticks, so the cadence is what bounds this loop.
fn spawn_scheduler(
    options: &HeadlessOptions,
    domain_root: PathBuf,
    application: Arc<WorkflowApplicationService>,
) -> SchedulerSupervisor {
    let (shutdown, receiver) = tokio::sync::watch::channel(false);
    let store = ExperimentScheduleStore::new(application.main_store.clone());
    let resources = Arc::new(DomainSchedulerResources::new(
        domain_root,
        options.base_repo.clone(),
    ));
    let kernel = Arc::new(ScheduledCampaignKernel::new(
        application.clone(),
        tokio::runtime::Handle::current(),
    ));
    let config = SchedulerConfig::default();
    let poll_ms = config.poll_ms.max(MIN_SCHEDULER_POLL_MS);
    let scheduler = Arc::new(CampaignScheduler::new(
        store,
        owner_id(),
        config,
        resources,
        kernel,
    ));
    let tick = Arc::new(move || scheduler.tick(crate::headless::domain::now_ms()));
    let task = tokio::spawn(run_scheduler_loop(receiver, poll_ms, tick));
    SchedulerSupervisor { shutdown, task }
}

/// The shortest cadence a supervisor may tick at.
///
/// A tick that finds nothing to do returns immediately, so the interval — not
/// the tick's own duration — is the only thing that bounds this loop.
pub const MIN_SCHEDULER_POLL_MS: u64 = 50;

/// Drives one scheduler tick per `poll_ms` until the supervisor is shut down.
///
/// The cadence is enforced *after* each tick, never raced against it: a tick
/// that finds nothing to do returns in microseconds, and racing a sleep against
/// it would spin the blocking pool and the database instead of waiting. A
/// shutdown signal interrupts either phase, and the loop never leaves a job
/// half-processed across ticks.
async fn run_scheduler_loop<F>(
    mut receiver: tokio::sync::watch::Receiver<bool>,
    poll_ms: u64,
    tick: Arc<F>,
) where
    F: Fn() -> Result<TickOutcome, ScheduleError> + Send + Sync + 'static,
{
    loop {
        if *receiver.borrow() {
            break;
        }
        let tick_fn = tick.clone();
        let running = tokio::task::spawn_blocking(move || tick_fn());
        tokio::select! {
            result = running => match result {
                Ok(Ok(TickOutcome::Idle)) => {}
                Ok(Ok(outcomes)) => log::info!("[Headless][scheduler] tick: {outcomes:?}"),
                Ok(Err(error)) => log::warn!(
                    "[Headless][scheduler] tick failed ({}): {}",
                    error.code.as_str(),
                    error.message
                ),
                Err(error) => log::warn!("[Headless][scheduler] tick task failed: {error}"),
            },
            changed = receiver.changed() => {
                if changed.is_err() || *receiver.borrow() {
                    break;
                }
                continue;
            }
        }

        // One tick per interval, at most. This is what keeps an idle instance
        // idle instead of spinning as fast as empty ticks complete.
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(poll_ms)) => {}
            changed = receiver.changed() => {
                if changed.is_err() || *receiver.borrow() {
                    break;
                }
            }
        }
    }
}

/// Runs every pre-control-plane step: credential bootstrap, runtime assembly
/// and tool registration.
async fn prepare(
    options: &HeadlessOptions,
    domain: &ExperimentDomain,
) -> Result<Arc<WorkflowApplicationService>, HeadlessError> {
    let store = domain.store().clone();

    import_config(options, &store)?;
    activate_credentials(options, &store)?;
    preflight_capabilities(&store)?;

    let application = build_application(domain).map_err(|error| {
        HeadlessError::new(
            code::RUNTIME_UNAVAILABLE,
            format!("failed to assemble the workflow runtime: {error}"),
        )
    })?;

    // Register the AppHandle-free tool core. Web tools are intentionally absent
    // and are never faked with a window handle.
    let tool_manager = application.chat_state.tool_manager.clone();
    if let Err(error) = tool_manager.register_core_tools(store).await {
        return Err(HeadlessError::new(
            code::RUNTIME_UNAVAILABLE,
            format!("failed to register the headless tool core: {error}"),
        ));
    }

    Ok(application)
}

/// Best-effort release of a domain lease taken by a failed startup. A failure
/// here is logged, never retried: the lease expires on its own.
fn release_domain_lease(domain: &ExperimentDomain) {
    match domain.lease().release(domain.store()) {
        Ok(()) => log::warn!("[Headless] Startup failed; released the experiment domain lease"),
        Err(error) => log::warn!(
            "[Headless] Startup failed and the domain lease could not be released: {error}"
        ),
    }
}

type BuildError = Box<dyn std::error::Error + Send + Sync>;

/// Assembles the single runtime authority: store → chat state → hub →
/// lifecycle manager → sub-agent factory → application service.
///
/// This mirrors the desktop assembly exactly, with two differences: no
/// `AppHandle` is ever created, and the hub's output transport is the no-window
/// transport (INV-1/INV-2).
fn build_application(
    domain: &ExperimentDomain,
) -> Result<Arc<WorkflowApplicationService>, BuildError> {
    let store = domain.store().clone();
    let app_data_dir = domain.paths().root.clone();

    // `None` here is the whole point: a headless runtime has no window to
    // notify, and `ChatState` already supports that.
    let chat_state = ChatState::new(Arc::new(WindowChannels::new()), None, store.clone());

    let tsid_generator = Arc::new(TsidGenerator::new(1)?);

    let mut instance_bytes = [0u8; 16];
    {
        use rand::Rng;
        rand::rng().fill_bytes(&mut instance_bytes);
    }
    let server_instance_id = hex::encode(instance_bytes);
    let hub = Arc::new(WorkflowRuntimeHub::with_transport(
        Arc::new(NoWindowTransport),
        server_instance_id,
    ));

    let workflow_manager = Arc::new(WorkflowManager::new());
    let factory: Arc<dyn SubAgentFactory> = Arc::new(DefaultSubAgentFactory {
        main_store: store.clone(),
        chat_state: chat_state.clone(),
        gateway: hub.clone(),
        workflow_manager: workflow_manager.clone(),
        app_data_dir: app_data_dir.clone(),
        tsid_generator: tsid_generator.clone(),
    });

    Ok(Arc::new(WorkflowApplicationService::new(
        store,
        chat_state,
        tsid_generator,
        hub,
        factory,
        workflow_manager,
        app_data_dir,
    )))
}

/// Renews the domain lease on a bounded interval.
///
/// The heartbeat reuses the exact `(owner_id, generation)` the process was
/// granted, so it can only ever renew its own lease: once another instance
/// takes the domain over, the renewal fails and the loop stops instead of
/// stealing it back.
fn spawn_domain_heartbeat(
    store: Arc<MainStore>,
    lease: ExperimentDomainLease,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(DOMAIN_HEARTBEAT_INTERVAL);
        ticker.tick().await; // the lease was just taken
        loop {
            ticker.tick().await;
            if let Err(error) = lease.renew(
                &store,
                now_ms(),
                crate::headless::domain::DEFAULT_DOMAIN_LEASE_MS,
            ) {
                log::error!(
                    "[Headless] Failed to renew the domain lease; stopping the heartbeat: {error}"
                );
                return;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn temp_data_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp data dir")
    }

    /// An idle scheduler must wait between ticks.
    ///
    /// A tick that finds nothing to do returns immediately, so a supervisor that
    /// races its sleep against the tick spins the blocking pool and the database
    /// at thousands of ticks per second (the whole domain database was observed
    /// being hammered by an idle instance). The cadence is therefore a
    /// behavioural contract, not an implementation detail: this test fails if a
    /// 400 ms window ever admits substantially more ticks than the interval
    /// allows.
    #[tokio::test]
    async fn an_idle_supervisor_waits_between_ticks() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let (shutdown, receiver) = tokio::sync::watch::channel(false);
        let ticks = Arc::new(AtomicUsize::new(0));
        let counter = ticks.clone();
        let tick = Arc::new(move || {
            counter.fetch_add(1, Ordering::Relaxed);
            Ok(TickOutcome::Idle)
        });

        let loop_task = tokio::spawn(run_scheduler_loop(receiver, 100, tick));
        tokio::time::sleep(Duration::from_millis(400)).await;

        let observed = ticks.load(Ordering::Relaxed);
        assert!(
            observed <= 8,
            "an idle supervisor ticked {observed} times in 400 ms with a 100 ms interval"
        );
        assert!(
            observed >= 2,
            "the supervisor stopped ticking entirely ({observed} ticks); the schedule would \
             never be driven"
        );

        // A shutdown must end the loop promptly, even mid-interval.
        shutdown.send(true).expect("shutdown signal");
        tokio::time::timeout(Duration::from_secs(5), loop_task)
            .await
            .expect("the supervised loop must stop on shutdown")
            .expect("the supervised loop must not panic");
    }

    /// A fresh data directory becomes a marked domain, starts a windowless
    /// runtime, publishes its own discovery document and reports a live port.
    #[tokio::test]
    async fn a_fresh_domain_starts_a_windowless_runtime() {
        let directory = temp_data_dir();
        let instance = start(HeadlessOptions::new(directory.path()))
            .await
            .expect("headless start");

        assert!(instance.control_plane_port() > 0);
        assert!(!instance.server_instance_id().is_empty());
        assert!(instance.domain_id().starts_with("domain-"));
        assert_eq!(
            instance.discovery_file(),
            directory
                .path()
                .join("runtime")
                .join("control-plane-v1.json")
        );
        assert!(instance.discovery_file().exists());
        // The experiment layout exists and the database is a marked domain.
        assert!(directory.path().join("chatspeed.db").exists());
        for name in crate::headless::DOMAIN_DIRECTORIES {
            assert!(directory.path().join(name).is_dir(), "missing {name}");
        }

        instance.shutdown().await;
        // Shutdown releases the lease, so a new instance can take the domain.
        let second = start(HeadlessOptions::new(directory.path()))
            .await
            .expect("restart");
        second.shutdown().await;
    }

    /// Two headless instances never share a domain.
    #[tokio::test]
    async fn a_second_instance_on_the_same_domain_fails_closed() {
        let directory = temp_data_dir();
        let first = start(HeadlessOptions::new(directory.path()))
            .await
            .expect("first start");
        let error = start(HeadlessOptions::new(directory.path()))
            .await
            .expect_err("second start must fail");
        assert_eq!(error.code, code::DOMAIN_LOCKED);
        first.shutdown().await;
    }

    /// An existing database without the experiment marker is never adopted.
    #[tokio::test]
    async fn an_unmarked_database_is_never_adopted() {
        let directory = temp_data_dir();
        {
            let connection =
                Connection::open(directory.path().join("chatspeed.db")).expect("open db");
            connection
                .execute("CREATE TABLE conversations (id TEXT PRIMARY KEY)", [])
                .expect("create table");
        }
        let error = start(HeadlessOptions::new(directory.path()))
            .await
            .expect_err("must refuse");
        assert_eq!(error.code, code::DOMAIN_UNMARKED);
    }

    /// A config package is only ever imported with an explicit category
    /// selection, and a malformed package fails closed before the runtime
    /// starts.
    #[tokio::test]
    async fn config_package_import_is_explicit_and_fail_closed() {
        let directory = temp_data_dir();
        let package = directory.path().join("package.json");
        std::fs::write(&package, b"{}").expect("write package");

        let mut without_categories = HeadlessOptions::new(directory.path());
        without_categories.config_package = Some(package.clone());
        let error = start(without_categories)
            .await
            .expect_err("categories required");
        assert_eq!(error.code, code::CONFIG_CATEGORY_REQUIRED);

        let mut with_categories = HeadlessOptions::new(directory.path());
        with_categories.config_package = Some(package);
        with_categories.config_categories = vec![ConfigCategory::Agents];
        let error = start(with_categories).await.expect_err("bad package");
        assert_eq!(error.code, code::CONFIG_IMPORT_FAILED);

        let mut missing = HeadlessOptions::new(directory.path());
        missing.config_package = Some(directory.path().join("absent.json"));
        let error = start(missing).await.expect_err("missing package");
        assert_eq!(error.code, code::CONFIG_IMPORT_FAILED);
    }

    /// A named API key file that does not exist is a pre-runtime failure.
    #[tokio::test]
    async fn a_missing_api_key_file_fails_before_the_runtime_starts() {
        let directory = temp_data_dir();
        let mut options = HeadlessOptions::new(directory.path());
        options.api_key_file = Some(directory.path().join("absent-keyfile"));
        let error = start(options).await.expect_err("must refuse");
        assert_eq!(error.code, code::API_KEY_FILE_FAILED);
        // Nothing was published: no runtime discovery document, no lock.
        assert!(!directory
            .path()
            .join("runtime")
            .join("control-plane-v1.json")
            .exists());
    }

    /// An agent that explicitly requires the Tauri-bound web tools is rejected
    /// at startup instead of silently running without them (INV-4).
    #[tokio::test]
    async fn an_agent_requiring_web_tools_fails_closed_at_startup() {
        let directory = temp_data_dir();
        // Initialize the domain and seed an agent that asks for web_search.
        {
            let first = start(HeadlessOptions::new(directory.path()))
                .await
                .expect("initialize domain");
            first.shutdown().await;
        }
        {
            let store = Arc::new(
                MainStore::new(directory.path().join("chatspeed.db")).expect("main store"),
            );
            let agent = crate::db::Agent::new(
                "agent-web".to_string(),
                "Web agent".to_string(),
                None,
                Some("primary".to_string()),
                None,
                "prompt".to_string(),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(false),
                Some(false),
                None,
            );
            let mut agent = agent;
            agent.available_tools = Some(serde_json::json!(["web_search"]).to_string());
            store.add_agent(&agent).expect("seed agent");
        }

        let error = start(HeadlessOptions::new(directory.path()))
            .await
            .expect_err("must refuse a web-tool requirement");
        assert_eq!(error.code, code::WEB_TOOLS_UNSUPPORTED);
        assert!(error.message.contains("web_search"));
        assert!(error.message.contains("agent-web"));
        // The failed startup released the domain lease: a retry hits the same
        // capability rejection, not `experiment_domain_locked`.
        let retry = start(HeadlessOptions::new(directory.path()))
            .await
            .expect_err("still refuses the same agent");
        assert_eq!(retry.code, code::WEB_TOOLS_UNSUPPORTED);
    }
}
