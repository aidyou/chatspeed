//! Long-lived background work owned by one runtime instance.
//!
//! The desktop wires these tasks inside `tauri::setup`; a desktop-free runtime
//! has to own them itself instead of leaning on a Tauri process. Every task here
//! drives the same canonical path the desktop uses:
//!
//! - capability reconcile, the evidence-driven convergence that runs after the
//!   synchronous startup recovery `RuntimeOwner::recover_capability_state`
//!   already classified;
//! - the configured MCP servers, registered through the AppHandle-free
//!   `ToolManager::register_available_mcp_tools`;
//! - the 60-second automation tick, which calls
//!   `WorkflowApplicationService::automation_dispatch_due` directly instead of
//!   reaching the service through an `AppHandle`;
//! - the runtime-neutral chat-completion proxy launcher, so the runtime owns the
//!   model backend its own sessions resolve against.
//!
//! Nothing here links Tauri, Wry or a WebView. The desktop-only `WebSearch` and
//! `WebFetch` tools are absent from the compiled runtime sources, so the runtime
//! cannot advertise a WebView tool it is unable to run.

use crate::ai::interaction::chat_completion::ChatState;
use crate::capability::CapabilityApplicationService;
use crate::ccproxy::launcher::{self, CcproxyServer};
use crate::db::MainStore;
use crate::owner::RuntimeOwner;
use crate::workflow::automation::service::normalize_datetime_for_db;
use crate::workflow::react::application::WorkflowApplicationService;
use chrono::Local;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// Cadence of the runtime automation tick, matching the desktop scheduler.
const AUTOMATION_TICK: Duration = Duration::from_secs(60);

/// Owns every long-lived task the runtime starts after its owner is assembled.
///
/// The runtime starts this *after* the fail-closed startup recovery and core
/// tool registration, and shuts it down before the owner and the
/// runtime-directory lock are released, so a successor instance can never
/// observe a half-stopped set of runtime-owned tasks.
pub struct RuntimeBackground {
    shutdown_tx: watch::Sender<bool>,
    handles: Vec<JoinHandle<()>>,
    ccproxy: Option<CcproxyServer>,
}

impl RuntimeBackground {
    /// Starts capability reconcile, MCP registration, the automation tick and
    /// the chat-completion proxy for `owner`.
    pub async fn start(owner: &RuntimeOwner, package_version: String) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let handles = vec![
            spawn_capability_reconcile(owner.service().capability().clone()),
            spawn_mcp_registration(owner.chat_state().clone(), owner.main_store().clone()),
            spawn_automation_scheduler(owner.service().clone(), shutdown_rx),
        ];

        // The proxy is the runtime's own model backend. A bind failure is
        // reported but must not stop the control plane from serving, because
        // only `group@alias` model resolution depends on the proxy.
        let ccproxy = match launcher::start(
            owner.main_store().clone(),
            owner.chat_state().clone(),
            package_version,
        )
        .await
        {
            Ok(server) => Some(server),
            Err(error) => {
                log::warn!("[Runtime][ccproxy] chat completion proxy unavailable: {error}");
                None
            }
        };

        Self {
            shutdown_tx,
            handles,
            ccproxy,
        }
    }

    /// Signals the automation tick, drains every spawned task and stops the
    /// chat-completion proxy, so shutdown leaves no runtime-owned task behind.
    pub async fn shutdown(mut self) {
        let _ = self.shutdown_tx.send_replace(true);
        for handle in std::mem::take(&mut self.handles) {
            let _ = handle.await;
        }
        if let Some(proxy) = self.ccproxy.take() {
            proxy.shutdown().await;
        }
    }
}

/// Runs one evidence-driven reconcile pass, mirroring the desktop's post-boot
/// convergence. Anything it cannot prove is left as `needs_reconcile` rather
/// than blind-retried.
fn spawn_capability_reconcile(capability: Arc<CapabilityApplicationService>) -> JoinHandle<()> {
    tokio::spawn(async move {
        match capability.reconcile().await {
            Ok(report) if !report.is_noop() => log::info!(
                "[Runtime][capability][reconcile] converged {} quarantine(s), {} install(s), {} mcp effect(s), removed {} staging residue, left {} needing reconcile",
                report.quarantines_finalized.len(),
                report.installs_recovered.len(),
                report.mcp_effects_recovered.len(),
                report.staging_residue_removed,
                report.still_needs_reconcile.len()
            ),
            Ok(_) => {}
            Err(error) => log::error!(
                "[Runtime][capability][reconcile] startup reconcile failed: {}",
                error.redacted_message()
            ),
        }
    })
}

/// Registers every configured, enabled MCP server. Registration spawns one task
/// per server and returns immediately, so a slow server never delays the
/// control plane.
fn spawn_mcp_registration(
    chat_state: Arc<ChatState>,
    main_store: Arc<MainStore>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(error) = chat_state
            .tool_manager
            .clone()
            .register_available_mcp_tools(main_store)
            .await
        {
            log::error!("[Runtime][mcp] failed to register configured MCP servers: {error}");
        }
    })
}

/// Spawns the runtime automation tick.
///
/// The tick advances the clock and hands every due slot to the single canonical
/// `automation_dispatch_due` facade path, so claim, dedupe, next-slot advance
/// and run creation stay in one durable transaction and no second scheduler is
/// introduced. It sleeps before the first tick to match the desktop cadence and
/// stops promptly when `shutdown` flips.
fn spawn_automation_scheduler(
    service: Arc<WorkflowApplicationService>,
    mut shutdown: watch::Receiver<bool>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(AUTOMATION_TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // `interval` fires immediately; consume that tick so the first dispatch
        // happens after a full cadence, exactly like the desktop scheduler.
        ticker.tick().await;
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    let now = normalize_datetime_for_db(Local::now());
                    if let Err(error) = service.automation_dispatch_due(&now).await {
                        log::error!("[Runtime][automation][scheduler] dispatch_due failed: {error}");
                    }
                }
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break;
                    }
                }
            }
        }
    })
}
