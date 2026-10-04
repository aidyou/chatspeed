//! The single canonical runtime owner.
//!
//! The standalone `chatspeed-runtime` process must not construct a second copy
//! of the database, the chat/tool state, the workflow hub, the session manager,
//! the sub-agent factory or the application service. [`RuntimeOwner`] builds
//! exactly one of each, wired to each other the same way the desktop
//! `tauri::setup` does, so the runtime serves the same canonical workflow,
//! capability and automation paths the desktop already trusts.
//!
//! Assembly is deliberately fail-closed: a database or migration failure is a
//! hard error the caller must surface, never a silent `:memory:` fallback. The
//! bundled built-in agents are synchronized into that one store during assembly,
//! taking over the `tauri::setup` call the runtime used to rely on the desktop
//! for, so the agents table is populated before the runtime is discoverable.

use crate::ai::interaction::chat_completion::ChatState;
use crate::capability::CapabilityRecoveryReport;
use crate::db::MainStore;
use crate::libs::tsid::TsidGenerator;
use crate::libs::window_channels::WindowChannels;
use crate::terminal::{TerminalError, TerminalManager, TerminalOwner, TerminalSubscription};
use crate::tools::ToolError;
use crate::workflow::react::application::WorkflowApplicationService;
use crate::workflow::react::client::http::server::{RuntimeChatPlane, RuntimeTerminalPlane};
use crate::workflow::react::client::hub::{NoWindowTransport, WorkflowRuntimeHub};
use crate::workflow::react::manager::WorkflowManager;
use crate::workflow::react::orchestrator::{DefaultSubAgentFactory, SubAgentFactory};
use chatspeed_contracts::{
    TerminalCreateRequest, TerminalResizeRequest, TerminalSessionMetadataDto, TerminalShellDto,
    TerminalWriteRequest,
};
use std::path::PathBuf;
use std::sync::Arc;

// The built-in agent synchronization is canonical shared source and is
// transport-neutral: it only reads the resource directory and writes through
// `MainStore`. Including it here keeps a single implementation shared with the
// desktop crate instead of a second copy the runtime would have to maintain.
// The module is public so the whole shared surface stays reachable from outside
// this crate rather than being reported as dead code.
#[path = "../../src/builtin_agents.rs"]
pub mod builtin_agents;

/// Inputs required to assemble the runtime owner.
///
/// `db_path` is the persistent SQLite file the runtime owns; it is never
/// `:memory:`. `instance_id` is the identity the runtime already claimed with
/// its runtime-directory lock, so the workflow hub's live cursors and the
/// control plane's `/meta` document report the same instance.
#[derive(Debug, Clone)]
pub struct RuntimeOwnerConfig {
    pub instance_id: String,
    pub db_path: PathBuf,
    pub app_data_dir: PathBuf,
}

/// Failures that prevent the runtime from owning its backend.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeOwnerError {
    /// The persistent database could not be opened or migrated.
    #[error("failed to open the runtime database at {}: {source}", path.display())]
    Database {
        path: PathBuf,
        #[source]
        source: crate::db::StoreError,
    },
    /// The TSID generator could not be initialized.
    #[error("failed to initialize the TSID generator: {0}")]
    Tsid(String),
    /// The bundled built-in agents could not be synchronized into the store.
    ///
    /// This is fatal rather than a logged warning: a client must never observe a
    /// runtime whose agents table silently lacks the bundled agents.
    #[error("failed to synchronize built-in agents: {0}")]
    BuiltinAgentSync(String),
}

/// Exactly one canonical owner of the runtime backend.
///
/// All fields are the same instances the application service transitively holds;
/// the owner keeps them alive so the runtime can expose them to later units
/// (Tauri client switch, maintenance, diagnostics) without a second construction.
pub struct RuntimeOwner {
    main_store: Arc<MainStore>,
    chat_state: Arc<ChatState>,
    gateway: Arc<WorkflowRuntimeHub>,
    workflow_manager: Arc<WorkflowManager>,
    service: Arc<WorkflowApplicationService>,
    /// The one interactive user terminal PTY owner (U-7).
    ///
    /// These are direct user terminals, not AI shell-tool executions, so the
    /// manager is deliberately independent of the tool manager and
    /// `shell_policy`.
    terminal_manager: Arc<TerminalManager>,
}

impl RuntimeOwner {
    /// Assembles the one owner for this process.
    ///
    /// Must be called inside a tokio runtime: `ChatState` starts its dispatcher
    /// loop during construction. Built-in agent synchronization runs here, so an
    /// assembled owner always has the bundled agents in its store or the
    /// assembly fails.
    pub fn assemble(config: RuntimeOwnerConfig) -> Result<Self, RuntimeOwnerError> {
        let main_store = Arc::new(MainStore::new(&config.db_path).map_err(|source| {
            RuntimeOwnerError::Database {
                path: config.db_path.clone(),
                source,
            }
        })?);

        // Synchronize the bundled built-in agents before the rest of the owner
        // is built. The desktop used to do this from `tauri::setup`; the runtime
        // has no such hook and must own it, otherwise the agents table is never
        // populated. It is transport-neutral (store plus resource directory
        // only) and fail-closed: a malformed bundle or store failure aborts
        // assembly instead of being downgraded to a logged warning, so a client
        // can never observe an owner whose built-in agents were silently left
        // out of the database.
        builtin_agents::sync_builtin_agents_if_needed(main_store.clone())
            .map_err(RuntimeOwnerError::BuiltinAgentSync)?;

        // A runtime process has no window, so the chat registry is inert and the
        // hub uses the no-window output transport: the SSE broker is the single
        // observation path.
        let chat_state =
            ChatState::runtime_new(Arc::new(WindowChannels::new()), main_store.clone());
        let tsid_generator = Arc::new(TsidGenerator::new(1).map_err(RuntimeOwnerError::Tsid)?);
        let gateway = Arc::new(WorkflowRuntimeHub::with_transport(
            Arc::new(NoWindowTransport),
            config.instance_id.clone(),
        ));
        let workflow_manager = Arc::new(WorkflowManager::new());

        let factory: Arc<dyn SubAgentFactory> = Arc::new(DefaultSubAgentFactory {
            main_store: main_store.clone(),
            chat_state: chat_state.clone(),
            gateway: gateway.clone(),
            workflow_manager: workflow_manager.clone(),
            app_data_dir: config.app_data_dir.clone(),
            tsid_generator: tsid_generator.clone(),
        });

        let service = Arc::new(WorkflowApplicationService::new(
            main_store.clone(),
            chat_state.clone(),
            tsid_generator,
            gateway.clone(),
            factory,
            workflow_manager.clone(),
            config.app_data_dir.clone(),
        ));

        // The one interactive user terminal owner. It is created here, not
        // lazily, so the runtime always owns exactly one PTY registry and its
        // Drop releases every session on shutdown.
        let terminal_manager = Arc::new(TerminalManager::new());

        Ok(Self {
            main_store,
            chat_state,
            gateway,
            workflow_manager,
            service,
            terminal_manager,
        })
    }

    /// The canonical transport-neutral application service.
    pub fn service(&self) -> &Arc<WorkflowApplicationService> {
        &self.service
    }

    /// The single database authority owned by this process.
    pub fn main_store(&self) -> &Arc<MainStore> {
        &self.main_store
    }

    /// The runtime chat/tool execution state.
    pub fn chat_state(&self) -> &Arc<ChatState> {
        &self.chat_state
    }

    /// The unique workflow runtime hub (input registry + SSE broker).
    pub fn gateway(&self) -> &Arc<WorkflowRuntimeHub> {
        &self.gateway
    }

    /// The unique session lifecycle manager.
    pub fn workflow_manager(&self) -> &Arc<WorkflowManager> {
        &self.workflow_manager
    }

    /// Classifies the operations a previous process left in flight.
    ///
    /// The runtime runs this before discovery is published and before any lease
    /// is accepted, so a client can never observe an owner that still holds a
    /// crash-interrupted capability operation. It mirrors the desktop gate: an
    /// operation interrupted before its first effect is failed (retryable),
    /// while one with an unproven effect moves to `needs_reconcile` and is never
    /// blindly retried. A failure is logged rather than fatal, because the
    /// durable journal keeps the operation repairable on the next start.
    pub fn recover_capability_state(&self) -> CapabilityRecoveryReport {
        match self.service.capability().recover_interrupted_operations() {
            Ok(report) => {
                if !report.is_empty() {
                    log::warn!(
                        "[Runtime][capability][recovery] {} operation(s) failed before any effect, {} need reconcile",
                        report.failed_before_effect.len(),
                        report.needs_reconcile.len()
                    );
                }
                report
            }
            Err(error) => {
                log::error!(
                    "[Runtime][capability][recovery] startup recovery failed: {}",
                    error.redacted_message()
                );
                CapabilityRecoveryReport::default()
            }
        }
    }

    /// Registers the AppHandle-free core tool surface.
    ///
    /// This is the file-system and search set. The fixed `web_fetch` /
    /// `web_search` tools are no longer registered here as native tools: they are
    /// exposed by the dedicated desktop loopback Web MCP provider, which the
    /// runtime installs through the canonical `ToolManager` MCP path when the
    /// desktop registers it against a live lease (AC-8). A runtime with no
    /// registered provider therefore advertises no web tools at all instead of a
    /// native tool that would have to fail closed. Configured MCP servers are
    /// registered separately by [`crate::background::RuntimeBackground`].
    pub async fn register_core_tools(&self) -> Result<(), ToolError> {
        let tool_manager = self.chat_state.tool_manager.clone();
        tool_manager
            .clone()
            .register_core_tools(self.main_store.clone())
            .await
    }
}

/// Exposes the owner's canonical chat/model handles to the control plane.
///
/// The control-plane router calls the same canonical `list_models_async` and
/// `start_new_chat_interaction` this owner runs, so mounting this adapter adds an
/// HTTP surface to the single owner instead of a second executor.
pub struct OwnerChatPlane {
    main_store: Arc<MainStore>,
    chat_state: Arc<ChatState>,
}

impl RuntimeChatPlane for OwnerChatPlane {
    fn main_store(&self) -> Arc<MainStore> {
        self.main_store.clone()
    }

    fn chat_state(&self) -> Arc<ChatState> {
        self.chat_state.clone()
    }
}

impl RuntimeOwner {
    /// The chat/model adapter the runtime mounts on its control plane.
    pub fn chat_plane(&self) -> Arc<OwnerChatPlane> {
        Arc::new(OwnerChatPlane {
            main_store: self.main_store.clone(),
            chat_state: self.chat_state.clone(),
        })
    }

    /// The interactive terminal adapter the runtime mounts on its control plane.
    pub fn terminal_plane(&self) -> Arc<OwnerTerminalPlane> {
        Arc::new(OwnerTerminalPlane {
            manager: self.terminal_manager.clone(),
        })
    }
}

/// Exposes the owner's single interactive terminal manager to the control plane.
///
/// The control-plane routes call the same PTY manager the owner assembled, so
/// the HTTP surface adds an observer/control path to the one owner instead of a
/// second PTY registry.
pub struct OwnerTerminalPlane {
    manager: Arc<TerminalManager>,
}

impl RuntimeTerminalPlane for OwnerTerminalPlane {
    fn list_shells(&self) -> Vec<TerminalShellDto> {
        self.manager.list_shells()
    }

    fn list_sessions(
        &self,
        client_id: &str,
        lease_id: &str,
    ) -> Vec<TerminalSessionMetadataDto> {
        self.manager.list_sessions(client_id, lease_id)
    }

    fn create(
        &self,
        client_id: &str,
        lease_id: &str,
        request: &TerminalCreateRequest,
    ) -> Result<TerminalSessionMetadataDto, TerminalError> {
        self.manager.create(
            TerminalOwner {
                client_id: client_id.to_string(),
                lease_id: lease_id.to_string(),
            },
            request.cwd.as_deref(),
            request.shell_path.as_deref(),
            request.cols,
            request.rows,
        )
    }

    fn write(
        &self,
        client_id: &str,
        lease_id: &str,
        session_id: &str,
        request: &TerminalWriteRequest,
    ) -> Result<(), TerminalError> {
        self.manager.write(client_id, lease_id, session_id, &request.input)
    }

    fn resize(
        &self,
        client_id: &str,
        lease_id: &str,
        session_id: &str,
        request: &TerminalResizeRequest,
    ) -> Result<(), TerminalError> {
        self.manager
            .resize(client_id, lease_id, session_id, request.cols, request.rows)
    }

    fn close(
        &self,
        client_id: &str,
        lease_id: &str,
        session_id: &str,
    ) -> Result<(), TerminalError> {
        self.manager.close(client_id, lease_id, session_id)
    }

    fn subscribe(
        &self,
        client_id: &str,
        lease_id: &str,
        session_id: &str,
    ) -> Result<Option<TerminalSubscription>, TerminalError> {
        self.manager.subscribe(client_id, lease_id, session_id)
    }

    fn sweep_invalid_leases(&self, is_valid: &(dyn Fn(&str, &str) -> bool + Send + Sync)) {
        self.manager.sweep_invalid_owners(is_valid);
    }
}
