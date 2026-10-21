use async_trait::async_trait;
#[cfg(not(feature = "desktop"))]
use futures::FutureExt;
#[cfg(not(feature = "desktop"))]
use rust_i18n::t;
#[cfg(not(feature = "desktop"))]
use serde_json::json;
use serde_json::Value;
#[cfg(not(feature = "desktop"))]
use std::collections::{HashMap, HashSet};
#[cfg(not(feature = "desktop"))]
use std::panic::AssertUnwindSafe;
#[cfg(not(feature = "desktop"))]
use std::sync::Arc;
#[cfg(not(feature = "desktop"))]
use tokio::sync::{broadcast, Mutex, RwLock};

use crate::ai::traits::chat::MCPToolDeclaration;
#[cfg(not(feature = "desktop"))]
use crate::db::MainStore;
#[cfg(not(feature = "desktop"))]
use crate::mcp::client::{
    McpClient, McpProtocolType, McpServerConfig, McpStatus, McpStopOutcome, StdioClient,
    StreamableHttpClient,
};
#[cfg(not(feature = "desktop"))]
use crate::mcp::McpError;
use crate::tools::error::ToolError;
#[cfg(not(feature = "desktop"))]
use crate::tools::MCP_TOOL_NAME_SPLIT;
use crate::tools::{ToolCallResult, ToolCategory, ToolScope};

// use super::tools::SearchDedup;
// use super::tools::{ChatCompletion, ModelName};

#[cfg(not(feature = "desktop"))]
const DEFAULT_BROADCAST_CAPACITY: usize = 100;

/// Per-name registration generations prevent detached discovery tasks from
/// restoring state after a stop or same-name restart.
#[cfg(not(feature = "desktop"))]
type McpRegistrationGenerations = HashMap<String, u64>;

/// The result type of a function call.
pub type NativeToolResult = Result<ToolCallResult, ToolError>;
#[cfg(not(feature = "desktop"))]
pub type ToolResult = Result<Value, ToolError>;

/// A trait defining the characteristics of a function.
#[async_trait]
pub trait ToolDefinition: Send + Sync {
    /// Gets the public name exposed to model-facing tool declarations.
    fn name(&self) -> &str;

    /// Gets the stable internal registry name. Native tools use their public name.
    fn registry_name(&self) -> &str {
        self.name()
    }

    /// Gets the description of the function.
    fn description(&self) -> &str;

    fn category(&self) -> ToolCategory;

    /// Gets the intended scope of this tool.
    /// Default is Both (Chat and Workflow).
    fn scope(&self) -> ToolScope {
        ToolScope::Both
    }

    /// Returns the function calling specification in JSON format.
    ///
    /// This method provides detailed information about the function
    /// in a format compatible with function calling APIs.
    ///
    /// # Returns
    /// * `Value` - The function specification in JSON format.
    fn tool_calling_spec(&self) -> MCPToolDeclaration;

    /// Executes the function.
    ///
    /// # Arguments
    /// * `params` - The parameters to pass to the function.
    ///
    /// # Returns
    /// * `ToolResult` - The result of the function execution.
    async fn call(&self, params: Value) -> NativeToolResult;
}

/// A wrapper that adapts an MCP tool to the ToolDefinition trait.
/// This allows MCP tools to be registered and called just like native tools.
#[cfg(not(feature = "desktop"))]
pub(crate) struct McpToolWrapper {
    pub(crate) server_name: String,
    pub(crate) tool_decl: MCPToolDeclaration,
    pub(crate) client: Arc<dyn McpClient>,
    pub(crate) canonical_name: String,
    pub(crate) public_name: String,
}

#[cfg(not(feature = "desktop"))]
#[derive(Default)]
struct McpAliasRegistry {
    alias_to_canonical: HashMap<String, String>,
    canonical_to_alias: HashMap<String, String>,
}

#[cfg(not(feature = "desktop"))]
impl McpAliasRegistry {
    fn resolve(&self, name: &str) -> Option<String> {
        self.alias_to_canonical.get(name).cloned()
    }

    fn alias_for(&self, canonical_name: &str) -> Option<String> {
        self.canonical_to_alias.get(canonical_name).cloned()
    }
}

#[cfg(not(feature = "desktop"))]
#[derive(Clone)]
struct McpAliasInput {
    canonical_name: String,
    server_name: String,
    tool_name: String,
}

#[cfg(not(feature = "desktop"))]
fn normalize_mcp_alias(value: &str) -> String {
    let normalized: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect();
    let normalized = normalized.trim_matches('_');
    if normalized.is_empty() {
        "mcp_tool".to_string()
    } else {
        normalized.to_string()
    }
}

#[cfg(not(feature = "desktop"))]
fn reserved_mcp_aliases() -> HashSet<String> {
    [
        crate::tools::TOOL_BASH,
        crate::tools::TOOL_READ_FILE,
        crate::tools::TOOL_WRITE_FILE,
        crate::tools::TOOL_EDIT_FILE,
        crate::tools::TOOL_LIST_DIR,
        crate::tools::TOOL_GLOB,
        crate::tools::TOOL_GREP,
        crate::tools::TOOL_GIT_DIFF,
        crate::tools::TOOL_GIT_INSPECT,
        chatspeed_contracts::WEB_SEARCH_TOOL,
        chatspeed_contracts::WEB_FETCH_TOOL,
        crate::tools::TOOL_SUB_AGENT_RUN,
        crate::tools::TOOL_SUB_AGENT_OUTPUT,
        crate::tools::TOOL_TODO_CREATE,
        crate::tools::TOOL_TODO_LIST,
        crate::tools::TOOL_TODO_UPDATE,
        crate::tools::TOOL_SKILL,
        crate::tools::TOOL_ASK_USER,
        crate::tools::TOOL_COMPLETE_WORKFLOW,
        crate::tools::TOOL_SUBMIT_RESULT,
        crate::tools::TOOL_SUBMIT_PLAN,
        crate::tools::TOOL_MCP_TOOL_EXPAND,
        crate::tools::TOOL_MCP_TOOL_EXECUTE,
        crate::tools::TOOL_MCP_TOOL_LOAD_LEGACY,
        crate::tools::TOOL_READ_HISTORY_MESSAGE,
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

#[cfg(not(feature = "desktop"))]
fn allocate_mcp_aliases(
    mut inputs: Vec<McpAliasInput>,
    native_names: impl IntoIterator<Item = String>,
) -> McpAliasRegistry {
    inputs.sort_by(|left, right| left.canonical_name.cmp(&right.canonical_name));
    let mut occupied = reserved_mcp_aliases();
    occupied.extend(native_names);
    let mut registry = McpAliasRegistry::default();

    for input in inputs {
        let tool_name = normalize_mcp_alias(&input.tool_name);
        // The dedicated desktop Web MCP provider owns the two exact web aliases
        // (`web_fetch`, `web_search`); `reserved_mcp_aliases` keeps every other
        // server out of them, so this assignment cannot shadow or be shadowed.
        let is_reserved_web_alias = input.server_name == chatspeed_contracts::WEB_MCP_SERVER_NAME
            && chatspeed_contracts::WEB_MCP_ALIASES.contains(&tool_name.as_str());
        let server_tool_name = format!("{}_{}", normalize_mcp_alias(&input.server_name), tool_name);
        let alias = if is_reserved_web_alias {
            tool_name
        } else if !occupied.contains(&tool_name) {
            tool_name
        } else if !occupied.contains(&server_tool_name) {
            server_tool_name
        } else {
            let mut index = 2;
            loop {
                let candidate = format!("{}_{}", server_tool_name, index);
                if !occupied.contains(&candidate) {
                    break candidate;
                }
                index += 1;
            }
        };
        occupied.insert(alias.clone());
        registry
            .alias_to_canonical
            .insert(alias.clone(), input.canonical_name.clone());
        registry
            .canonical_to_alias
            .insert(input.canonical_name, alias);
    }

    registry
}

#[cfg(not(feature = "desktop"))]
#[async_trait]
impl ToolDefinition for McpToolWrapper {
    fn name(&self) -> &str {
        &self.public_name
    }

    fn registry_name(&self) -> &str {
        &self.canonical_name
    }

    fn description(&self) -> &str {
        &self.tool_decl.description
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Mcp
    }

    fn scope(&self) -> ToolScope {
        ToolScope::Both
    }

    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        let mut spec = self.tool_decl.clone();
        spec.name = self.public_name.clone();
        spec
    }

    async fn call(&self, params: Value) -> NativeToolResult {
        let res = self
            .client
            .call(&self.tool_decl.name, params)
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(
                    crate::error_diagnostics::FailureMessage::leaf(format!(
                        "MCP call to server '{}' tool '{}' failed: {}",
                        self.server_name, self.tool_decl.name, e
                    ))
                    .with_origin(e.error_origin()),
                )
            })?;

        // MCP results often come back as a JSON object with a 'content' field for display
        // We extract that if present for high-signal text observations.
        Ok(ToolCallResult::success(
            res.get("content")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            Some(res),
        ))
    }
}

#[cfg(not(feature = "desktop"))]
fn scope_allows(tool_scope: ToolScope, scope_filter: Option<ToolScope>) -> bool {
    match scope_filter {
        Some(ToolScope::Chat) => tool_scope == ToolScope::Chat || tool_scope == ToolScope::Both,
        Some(ToolScope::Both) => tool_scope == ToolScope::Both,
        Some(ToolScope::Workflow) | None => true,
    }
}

/// Refuses a disabled tool and otherwise returns a copy of its declaration.
///
/// Disabled MCP tools stay canonically addressable for management, so the
/// registry lookup alone does not prove a tool may run; this is the gate that
/// keeps a manual invocation from running one the user turned off.
#[cfg(not(feature = "desktop"))]
fn owned_enabled(
    server_name: &str,
    declaration: &MCPToolDeclaration,
) -> Result<MCPToolDeclaration, ToolError> {
    if declaration.disabled {
        return Err(ToolError::Security(format!(
            "MCP tool '{}' on server '{}' is disabled",
            declaration.name, server_name
        )));
    }
    Ok(declaration.clone())
}

#[cfg(not(feature = "desktop"))]
#[derive(Clone)]
pub struct McpToolSpec {
    pub canonical_name: String,
    pub declaration: MCPToolDeclaration,
}

/// Per-server budget for one bounded MCP teardown during shutdown.
///
/// Kept well below the daemon's shared coordinated-shutdown budget so a stuck
/// server cannot consume the whole drain window; the whole cancel pass is
/// additionally bounded by the caller.
#[cfg(not(feature = "desktop"))]
const MCP_SHUTDOWN_STOP_BUDGET: std::time::Duration = std::time::Duration::from_secs(3);

/// What one [`ToolManager::shutdown_all_mcp_servers`] pass proved.
#[cfg(not(feature = "desktop"))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct McpShutdownReport {
    /// Live server handles taken for teardown.
    pub servers: usize,
    /// Handles whose bounded `stop` completed (transport released, or already
    /// stopped). This counts only what is provable; descendant reaping is never
    /// claimed.
    pub stopped: usize,
    /// Handles whose teardown could not be proven within the budget, or that
    /// reported an error. A non-zero count means the caller must not record a
    /// clean stop.
    pub unproven: usize,
}

/// Manages the registration and execution of workflow functions.
///
/// This struct is responsible for maintaining a collection of functions
#[cfg(not(feature = "desktop"))]
pub struct ToolManager {
    /// A map of registered functions.
    tools: RwLock<HashMap<String, Arc<dyn ToolDefinition>>>,
    /// A map of registered MCP servers.
    mcp_servers: RwLock<HashMap<String, Arc<dyn McpClient>>>,
    /// A map of registered MCP tools. The key is the server name, and the value is a vector of declarations.
    mcp_tools: RwLock<HashMap<String, Vec<MCPToolDeclaration>>>,
    /// A registry mapping public MCP aliases to canonical internal tool IDs.
    mcp_alias_registry: RwLock<McpAliasRegistry>,
    /// A channel for sending MCP status events.
    mcp_status_event_sender: broadcast::Sender<(String, McpStatus)>,
    /// A channel for notifying consumers that the externally visible MCP tool list changed.
    mcp_tool_change_event_sender: broadcast::Sender<()>,
    /// Registration generations prevent stale discovery tasks from restoring old state.
    mcp_registration_generations: Mutex<McpRegistrationGenerations>,
    /// Overrides client state until discovery commits or an attempt fails.
    /// Access is serialized by `mcp_registration_generations`.
    mcp_registration_status: RwLock<HashMap<String, McpStatus>>,
    /// The one runtime work-admission gate, when the runtime installed one.
    ///
    /// Starting an MCP server is canonical runtime work, so it is fenced here,
    /// at the manager the capability orchestrator and any direct caller reach.
    admission: std::sync::OnceLock<Arc<dyn crate::lifecycle::WorkAdmission>>,
}

#[cfg(not(feature = "desktop"))]
impl ToolManager {
    /// Creates a new instance of `FunctionManager`.
    pub fn new() -> Self {
        let (mcp_status_event_sender, _) = broadcast::channel(DEFAULT_BROADCAST_CAPACITY);
        let (mcp_tool_change_event_sender, _) = broadcast::channel(DEFAULT_BROADCAST_CAPACITY);
        Self {
            tools: RwLock::new(HashMap::new()),
            mcp_servers: RwLock::new(HashMap::new()),
            mcp_tools: RwLock::new(HashMap::new()),
            mcp_alias_registry: RwLock::new(McpAliasRegistry::default()),
            mcp_status_event_sender,
            mcp_tool_change_event_sender,
            mcp_registration_generations: Mutex::new(HashMap::new()),
            mcp_registration_status: RwLock::new(HashMap::new()),
            admission: std::sync::OnceLock::new(),
        }
    }

    /// Installs the one runtime work-admission gate. First-wins and idempotent.
    pub fn install_admission(&self, gate: Arc<dyn crate::lifecycle::WorkAdmission>) {
        let _ = self.admission.set(gate);
    }

    /// Fences one capability work start; a closed gate refuses with the stable
    /// `runtime_stopping` tool error.
    fn admit_work(
        &self,
        kind: crate::lifecycle::WorkKind,
    ) -> Result<crate::lifecycle::AdmissionTicket, ToolError> {
        match self.admission.get() {
            Some(gate) => gate
                .admit(kind)
                .map_err(|denied| ToolError::RuntimeStopping(denied.message)),
            None => Ok(crate::lifecycle::AdmissionTicket::permissive()),
        }
    }

    pub async fn clear(&self, clear_mcp: bool) {
        self.tools.write().await.clear();
        self.mcp_alias_registry
            .write()
            .await
            .alias_to_canonical
            .clear();
        self.mcp_alias_registry
            .write()
            .await
            .canonical_to_alias
            .clear();
        if clear_mcp {
            let mut generations = self.mcp_registration_generations.lock().await;
            for generation in generations.values_mut() {
                *generation = generation.saturating_add(1);
            }
            self.mcp_registration_status.write().await.clear();
            self.mcp_tools.write().await.clear();
            self.mcp_servers.write().await.clear();
            self.notify_mcp_tools_changed();
        }
    }

    /// Stops every registered MCP server with a bounded, explicit teardown and
    /// reports what was proven.
    ///
    /// A runtime-owned MCP server is a child process reached through its client.
    /// The handles and the tool cache are taken under a short lock and that lock
    /// is released *before* any `stop` is awaited, so a slow server can neither
    /// hold the registry lock across the await nor block a concurrent
    /// registration. Only the MCP state is touched: the native tool surface and
    /// the alias registry are left for the owner teardown.
    ///
    /// Each client is stopped through [`McpClient::stop_bounded`]; an unproven
    /// teardown is counted in [`McpShutdownReport::unproven`] instead of being
    /// hidden, so the coordinated shutdown can refuse to record a clean stop.
    /// Re-invoking it is idempotent: the map is already empty, so the report is
    /// all zeroes. rmcp's `RunningService::cancel` is canonical and drops the
    /// child transport, but it holds no child *tree* handle, so the report claims
    /// only what is provable (the transport/direct child), never a tree-wide reap.
    ///
    /// The lock order matches [`Self::clear`] (`mcp_tools` before `mcp_servers`),
    /// so it can never deadlock against a concurrent registration.
    pub async fn shutdown_all_mcp_servers(&self) -> McpShutdownReport {
        // Take every handle under the lock, then release it before awaiting any
        // teardown. The lock order matches `clear`/`rebuild_mcp_wrappers`.
        let handles: Vec<Arc<dyn McpClient>> = {
            let mut generations = self.mcp_registration_generations.lock().await;
            for generation in generations.values_mut() {
                *generation = generation.saturating_add(1);
            }
            self.mcp_registration_status.write().await.clear();
            self.mcp_tools.write().await.clear();
            let mut mcp_servers = self.mcp_servers.write().await;
            mcp_servers.drain().map(|(_, client)| client).collect()
        };
        self.notify_mcp_tools_changed();

        let servers = handles.len();
        let mut stopped = 0usize;
        let mut unproven = 0usize;
        for client in handles {
            match client.stop_bounded(MCP_SHUTDOWN_STOP_BUDGET).await {
                McpStopOutcome::AlreadyStopped | McpStopOutcome::TransportStopped => stopped += 1,
                McpStopOutcome::Unproven(error) => {
                    unproven += 1;
                    log::warn!(
                        "[Runtime][cancel] an owned MCP server teardown is unproven: {error}"
                    );
                }
            }
        }
        McpShutdownReport {
            servers,
            stopped,
            unproven,
        }
    }

    async fn rebuild_mcp_wrappers(&self) {
        // Keep the established lock order: mcp_tools -> mcp_servers -> tools -> aliases.
        let mcp_tools = self.mcp_tools.read().await;
        let mcp_servers = self.mcp_servers.read().await;
        let mut tools = self.tools.write().await;
        let native_names = tools
            .iter()
            .filter(|(_, tool)| tool.category() != ToolCategory::Mcp)
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();
        let inputs = mcp_tools
            .iter()
            .flat_map(|(server_name, declarations)| {
                declarations
                    .iter()
                    .filter(|declaration| !declaration.disabled)
                    .map(move |declaration| McpAliasInput {
                        canonical_name: format!(
                            "{}{}{}",
                            server_name, MCP_TOOL_NAME_SPLIT, declaration.name
                        ),
                        server_name: server_name.clone(),
                        tool_name: declaration.name.clone(),
                    })
            })
            .collect::<Vec<_>>();
        let registry = allocate_mcp_aliases(inputs, native_names);

        tools.retain(|_, tool| tool.category() != ToolCategory::Mcp);
        for (server_name, declarations) in mcp_tools.iter() {
            let Some(client) = mcp_servers.get(server_name) else {
                continue;
            };
            for declaration in declarations {
                let canonical_name =
                    format!("{}{}{}", server_name, MCP_TOOL_NAME_SPLIT, declaration.name);
                // Disabled MCP tools stay canonically addressable for management and security
                // checks, but must not reserve a model-facing alias.
                let public_name = registry
                    .alias_for(&canonical_name)
                    .unwrap_or_else(|| canonical_name.clone());
                let wrapper = Arc::new(McpToolWrapper {
                    server_name: server_name.clone(),
                    tool_decl: declaration.clone(),
                    client: client.clone(),
                    canonical_name: canonical_name.clone(),
                    public_name,
                });
                tools.insert(canonical_name, wrapper);
            }
        }
        *self.mcp_alias_registry.write().await = registry;
    }

    fn notify_mcp_tools_changed(&self) {
        let _ = self.mcp_tool_change_event_sender.send(());
    }

    /// Registers every tool that needs no Tauri/window state.
    ///
    /// This is the AppHandle-free core of the tool surface: the file-system and
    /// search tools. The desktop app calls it after the Tauri-bound web tools; a
    /// headless process calls it directly, so it never has to fabricate a window
    /// handle (INV-1/INV-4). Web tools are deliberately *not* part of this set: a
    /// headless instance must refuse a web-tool requirement up front rather
    /// than silently run without it.
    ///
    /// The system/workflow/interaction tools (shell execute, todo, skills,
    /// task orchestration) remain unregistered for both runtimes, exactly as
    /// before this split; enabling them is a separate change.
    #[cfg(not(feature = "desktop"))]
    pub async fn register_core_tools(
        self: Arc<Self>,
        _main_store: Arc<MainStore>,
    ) -> Result<(), ToolError> {
        // =================================================
        // FileSystem & Search tools
        // =================================================
        self.register_tool(Arc::new(crate::tools::ReadFile::default()))
            .await?;
        self.register_tool(Arc::new(crate::tools::WriteFile::default()))
            .await?;
        self.register_tool(Arc::new(crate::tools::EditFile::default()))
            .await?;
        self.register_tool(Arc::new(crate::tools::ListDir::default()))
            .await?;
        self.register_tool(Arc::new(crate::tools::Grep::default()))
            .await?;

        // =================================================
        // System & Workflow tools
        // =================================================
        // let tsid = app_handle
        //     .state::<Arc<crate::libs::tsid::TsidGenerator>>()
        //     .inner()
        //     .clone();
        // let path_guard = Arc::new(std::sync::RwLock::new(
        //     crate::workflow::react::security::PathGuard::new(vec![], vec![], vec![]),
        // ));
        // self.register_tool(Arc::new(crate::tools::ShellExecute::new(
        //     path_guard,
        //     tsid.clone(),
        //     vec![],
        //     false,
        // )))
        // .await?;

        // self.register_tool(Arc::new(crate::tools::TodoCreateTool {
        //     session_id: "".into(),
        //     main_store: main_store.clone(),
        // }))
        // .await?;
        // self.register_tool(Arc::new(crate::tools::TodoListTool {
        //     session_id: "".into(),
        //     main_store: main_store.clone(),
        // }))
        // .await?;
        // self.register_tool(Arc::new(crate::tools::TodoUpdateTool {
        //     session_id: "".into(),
        //     main_store: main_store.clone(),
        // }))
        // .await?;

        // let app_data_dir = app_handle.path().app_data_dir().unwrap_or_default();
        // let scanner = crate::workflow::react::skills::SkillScanner::new(app_data_dir);
        // let skills = scanner.scan().unwrap_or_default();
        // self.register_tool(Arc::new(crate::workflow::react::skills::SkillExecute::new(skills)))
        //     .await?;

        // let factory = app_handle
        //     .state::<Arc<dyn crate::workflow::react::orchestrator::SubAgentFactory>>()
        //     .inner()
        //     .clone();
        // self.register_tool(Arc::new(
        //     crate::workflow::react::orchestrator::TaskTool::new(factory, tsid),
        // ))
        // .await?;
        // self.register_tool(Arc::new(
        //     crate::workflow::react::orchestrator::TaskOutputTool,
        // ))
        // .await?;

        // // Interaction tools
        // self.register_tool(Arc::new(crate::tools::AskUser)).await?;
        // self.register_tool(Arc::new(crate::tools::FinishTask))
        //     .await?;

        Ok(())
    }

    /// Registers the fixed desktop-free web tools that execute only through the
    /// live client WebView capability bridge.
    ///
    /// The runtime never links a WebView, so `web_fetch`/`web_search` reach a
    /// desktop-backed capability exclusively by dispatching a typed invocation
    /// on the same registry the control plane's bridge routes own. With no live
    /// bridge each call fails closed with a structured result, so the runtime
    /// never advertises a capability it cannot prove.
    #[cfg(all(test, not(feature = "desktop")))]
    pub async fn register_client_bridge_web_tools(
        &self,
        registry: Arc<crate::workflow::react::client::http::client_bridge::ClientBridgeRegistry>,
    ) -> Result<(), ToolError> {
        self.register_tool(Arc::new(
            crate::tools::client_bridge_web::ClientBridgeWebTool::fetch(registry.clone()),
        ))
        .await?;
        self.register_tool(Arc::new(
            crate::tools::client_bridge_web::ClientBridgeWebTool::search(registry),
        ))
        .await
    }

    #[cfg(not(feature = "desktop"))]
    pub async fn register_available_mcp_tools(
        self: Arc<Self>,
        main_store: Arc<MainStore>,
    ) -> Result<(), ToolError> {
        // Each server gets an independent startup task. A slow or failed server
        // must not delay the others or the first window paint.
        let mcp_configs_to_process: Vec<_> = main_store
            .config
            .get_mcps()
            .into_iter()
            .filter(|mcp_db_config| !mcp_db_config.disabled)
            .map(|mcp_db_config| mcp_db_config.config.clone())
            .collect();

        for mcp_server_config in mcp_configs_to_process {
            let tool_manager = self.clone();
            tokio::spawn(async move {
                let server_name = mcp_server_config.name.clone();
                if let Err(error) = tool_manager.register_mcp_server(mcp_server_config).await {
                    log::error!(
                        "Failed to register MCP server '{}' during startup: {}",
                        server_name,
                        error
                    );
                }
            });
        }
        Ok(())
    }

    /// Registers a new tool with the manager.
    ///
    /// # Arguments
    /// * `tool` - The tool to register.
    ///
    /// # Returns
    /// * `Result<(), ToolError>` - The result of the registration.
    pub async fn register_tool(
        &self, // This can remain &self as it doesn't spawn long tasks
        tool: Arc<dyn ToolDefinition>,
    ) -> Result<(), ToolError> {
        let registry_name = tool.registry_name().to_string();
        let public_name = tool.name().to_string();
        let is_mcp = tool.category() == ToolCategory::Mcp;
        {
            let mut tools = self.tools.write().await;
            if tools.contains_key(&registry_name) {
                return Err(ToolError::FunctionAlreadyExists(registry_name));
            }
            tools.insert(registry_name.clone(), tool);
        }

        if is_mcp {
            let mut aliases = self.mcp_alias_registry.write().await;
            aliases
                .alias_to_canonical
                .insert(public_name.clone(), registry_name.clone());
            aliases
                .canonical_to_alias
                .insert(registry_name, public_name);
        } else {
            // A newly registered native tool must not be shadowed by an existing MCP alias.
            self.rebuild_mcp_wrappers().await;
        }
        Ok(())
    }

    /// Copies an MCP wrapper from another manager without rebuilding it from this manager's MCP
    /// server cache. Session-local workflow managers use this for model-visible autoExpand tools.
    #[cfg(not(feature = "desktop"))]
    pub(crate) async fn register_mcp_tool_wrapper(
        &self,
        tool: Arc<dyn ToolDefinition>,
    ) -> Result<(), ToolError> {
        if tool.category() != ToolCategory::Mcp {
            return Err(ToolError::InvalidParams(
                "register_mcp_tool_wrapper requires an MCP tool".to_string(),
            ));
        }
        self.register_tool(tool).await
    }

    #[cfg(all(test, not(feature = "desktop")))]
    pub(crate) async fn register_test_mcp_tool(
        &self,
        server_name: &str,
        public_name: &str,
        input_schema: Value,
    ) -> Result<String, ToolError> {
        let canonical_name = format!("{}{}{}", server_name, MCP_TOOL_NAME_SPLIT, public_name);
        let client = StdioClient::new(McpServerConfig {
            name: server_name.to_string(),
            protocol_type: McpProtocolType::Stdio,
            command: Some("ls".to_string()),
            args: Some(vec!["-la".to_string()]),
            ..Default::default()
        })
        .map_err(|error| {
            ToolError::Initialization(
                crate::error_diagnostics::FailureMessage::leaf(error.to_string())
                    .with_origin(error.error_origin()),
            )
        })?;
        client.set_test_status(McpStatus::Connected).await;
        let client: Arc<dyn McpClient> = Arc::new(client);

        self.mcp_servers
            .write()
            .await
            .insert(server_name.to_string(), client);
        self.mcp_tools
            .write()
            .await
            .entry(server_name.to_string())
            .or_default()
            .push(MCPToolDeclaration {
                name: public_name.to_string(),
                description: format!("Test MCP tool {}", public_name),
                input_schema,
                output_schema: None,
                disabled: false,
                scope: Some(ToolScope::Both),
            });
        self.rebuild_mcp_wrappers().await;
        self.notify_mcp_tools_changed();
        Ok(canonical_name)
    }

    pub async fn resolve_tool_name(&self, name: &str) -> String {
        let name = if name == crate::tools::TOOL_MCP_TOOL_LOAD_LEGACY
            && self
                .tools
                .read()
                .await
                .contains_key(crate::tools::TOOL_MCP_TOOL_EXPAND)
        {
            crate::tools::TOOL_MCP_TOOL_EXPAND
        } else {
            name
        };

        if self.tools.read().await.contains_key(name) {
            return name.to_string();
        }
        self.mcp_alias_registry
            .read()
            .await
            .resolve(name)
            .unwrap_or_else(|| name.to_string())
    }

    pub async fn resolve_mcp_tool_name(&self, name: &str) -> Option<String> {
        let canonical_name = self.resolve_tool_name(name).await;
        let tools = self.tools.read().await;
        tools
            .get(&canonical_name)
            .filter(|tool| tool.category() == ToolCategory::Mcp)
            .map(|_| canonical_name)
    }

    /// Gets a tool by its public alias or canonical registry name.
    pub async fn get_tool(&self, name: &str) -> Result<Arc<dyn ToolDefinition>, ToolError> {
        let canonical_name = self.resolve_tool_name(name).await;
        let tools = self.tools.read().await;
        tools
            .get(&canonical_name)
            .cloned()
            .ok_or_else(|| ToolError::FunctionNotFound(name.to_string()))
    }

    /// Checks whether a public alias or canonical registry name exists.
    pub async fn has_tool(&self, name: &str) -> bool {
        self.get_tool(name).await.is_ok()
    }

    /// Returns metadata for all registered native tools.
    /// This is used by the UI to discover available capabilities and their scopes.
    pub async fn get_all_native_tool_metadata(&self) -> Vec<Value> {
        let tools = self.tools.read().await;
        let mut meta = Vec::new();
        for (registry_name, tool) in tools.iter() {
            if tool.category() == ToolCategory::Mcp {
                continue;
            }
            meta.push(json!({
                "id": registry_name,
                "name": tool.name(),
                "category": tool.category().to_string(),
                "scope": tool.scope()
            }));
        }
        meta
    }

    /// Call a native tool by its name.
    ///
    /// # Arguments
    /// * `name` - The name of the function to execute.
    /// * `params` - The parameters to pass to the function.
    ///
    /// # Returns
    /// * `ToolResult` - The result of the function execution.
    pub async fn native_tool_call(&self, name: &str, params: Value) -> NativeToolResult {
        self.tool_call_with_dispatch(name, params, None).await.0
    }

    /// Call a tool and report whether the physical owner
    /// (`ToolDefinition::call`) was actually entered.
    ///
    /// `dispatched == false` means the failure happened before owner entry
    /// (registry lookup miss, disabled MCP tool) — a proven no-effect
    /// outcome. When `owner_entered` is supplied, it is set to `true`
    /// exactly at the owner boundary so callers can classify cancellations
    /// that race with dispatch.
    pub async fn tool_call_with_dispatch(
        &self,
        name: &str,
        params: Value,
        owner_entered: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ) -> (NativeToolResult, bool) {
        use std::sync::atomic::Ordering;
        let tool = match self.get_tool(name).await {
            Ok(tool) => tool,
            Err(error) => return (Err(error), false),
        };
        if tool.category() == ToolCategory::Mcp && tool.tool_calling_spec().disabled {
            return (
                Err(ToolError::Security(format!(
                    "MCP tool '{}' is disabled",
                    name
                ))),
                false,
            );
        }
        if let Some(flag) = &owner_entered {
            flag.store(true, Ordering::SeqCst);
        }
        let result = match AssertUnwindSafe(tool.call(params)).catch_unwind().await {
            Ok(result) => result,
            Err(payload) => {
                let panic_message = if let Some(message) = payload.downcast_ref::<&str>() {
                    (*message).to_string()
                } else if let Some(message) = payload.downcast_ref::<String>() {
                    message.clone()
                } else {
                    "unknown panic payload".to_string()
                };
                log::error!("Native tool '{}' panicked: {}", name, panic_message);
                Err(ToolError::ExecutionFailed(
                    format!("Tool '{}' panicked: {}", name, panic_message).into(),
                ))
            }
        };
        (result, true)
    }

    /// Call a native tool or mcp tool by its name.
    /// Since all tools are unified in the tools map, this directly delegates to native_tool_call.
    pub async fn tool_call(&self, name: &str, params: Value) -> ToolResult {
        self.native_tool_call(name, params).await.map(|v| v.into())
    }

    /// Get the calling spec of all registered tools, filtered by scope and exclusions.
    /// This includes both native tools and MCP tools (via wrappers).
    pub async fn get_tool_calling_spec(
        &self,
        scope_filter: Option<ToolScope>,
        exclude: Option<HashSet<String>>,
    ) -> Result<Vec<MCPToolDeclaration>, ToolError> {
        let mut specs = Vec::new();
        let excluded: HashSet<String> = exclude.unwrap_or_default();
        let resolved_excluded = {
            let aliases = self.mcp_alias_registry.read().await;
            excluded
                .iter()
                .map(|name| aliases.resolve(name).unwrap_or_else(|| name.clone()))
                .collect::<HashSet<_>>()
        };

        // Collect all tools from the unified map
        {
            let tools = self.tools.read().await;
            for (registry_name, tool) in tools.iter() {
                // Apply scope filter if provided
                if let Some(filter) = scope_filter {
                    match filter {
                        ToolScope::Chat => {
                            // Chat only sees Chat or Both
                            if tool.scope() != ToolScope::Chat && tool.scope() != ToolScope::Both {
                                continue;
                            }
                        }
                        ToolScope::Both => {
                            // Strictly Both
                            if tool.scope() != ToolScope::Both {
                                continue;
                            }
                        }
                        ToolScope::Workflow => {
                            // Workflow can see Chat + Workflow + Both
                        }
                    }
                }

                // Apply exclusion filter
                if !resolved_excluded.contains(registry_name) {
                    let spec = tool.tool_calling_spec();
                    if !spec.disabled {
                        specs.push(spec);
                    }
                }
            }
        }

        specs.sort_by(|left, right| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
                .then_with(|| left.name.cmp(&right.name))
        });

        Ok(specs)
    }

    /// Returns enabled MCP declarations together with their canonical identities.
    pub async fn get_mcp_tool_specs(&self, scope_filter: Option<ToolScope>) -> Vec<McpToolSpec> {
        let tools = self.tools.read().await;
        let mut specs = tools
            .iter()
            .filter_map(|(canonical_name, tool)| {
                (tool.category() == ToolCategory::Mcp
                    && !tool.tool_calling_spec().disabled
                    && scope_allows(tool.scope(), scope_filter))
                .then(|| McpToolSpec {
                    canonical_name: canonical_name.clone(),
                    declaration: tool.tool_calling_spec(),
                })
            })
            .collect::<Vec<_>>();
        specs.sort_by(|left, right| left.canonical_name.cmp(&right.canonical_name));
        specs
    }

    /// Get the complete public declaration for a MCP alias or canonical name.
    pub async fn get_mcp_tool_declaration(
        &self,
        tool_name: &str,
    ) -> Result<MCPToolDeclaration, ToolError> {
        let canonical_name = self
            .resolve_mcp_tool_name(tool_name)
            .await
            .ok_or_else(|| ToolError::InvalidParams("Not an MCP tool".to_string()))?;
        let tool = self.get_tool(&canonical_name).await?;
        let declaration = tool.tool_calling_spec();
        if declaration.disabled {
            return Err(ToolError::Security(format!(
                "MCP tool '{}' is disabled",
                tool_name
            )));
        }
        Ok(declaration)
    }

    /// Invokes one tool owned by `server_name` and returns the MCP runtime's
    /// serialized tool result unchanged.
    ///
    /// The requested name may be the internal tool name or one of the public
    /// aliases the manager assigned. Ownership is enforced here: a name that
    /// does not resolve to a tool of `server_name`, an alias owned by another
    /// server, and a disabled tool are all refused before the MCP client is
    /// reached, so a manual invocation can never run the wrong server's tool
    /// (INV-1/AC-11).
    pub(crate) async fn invoke_mcp_tool(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: Value,
    ) -> Result<Value, ToolError> {
        let declaration = self.resolve_owned_mcp_tool(server_name, tool_name).await?;
        let client = self.get_mcp_server(server_name).await?;
        client
            .call(&declaration.name, arguments)
            .await
            .map_err(|error| {
                // The client folds a tool's own error content and a transport
                // failure into one message: it is redacted before it can be
                // logged and reduced to an allowlisted code for the caller, so
                // neither the provider message nor any raw content is exposed.
                log::warn!(
                    "MCP tool call on server '{}' failed: {}",
                    server_name,
                    crate::capability::redaction::redact_text(&error.to_string())
                );
                ToolError::ExecutionFailed(
                    crate::error_diagnostics::FailureMessage::leaf(mcp_call_error_detail(&error))
                        .with_origin(error.error_origin()),
                )
            })
    }

    /// Resolves a requested tool name to a declaration owned by `server_name`.
    ///
    /// An exact internal name of the server wins; otherwise a public alias or
    /// canonical name is resolved and its owning server is checked against
    /// `server_name`. A name that belongs elsewhere is refused rather than
    /// silently invoked on the requested server (INV-1).
    async fn resolve_owned_mcp_tool(
        &self,
        server_name: &str,
        tool_name: &str,
    ) -> Result<MCPToolDeclaration, ToolError> {
        let declarations = self
            .mcp_tools
            .read()
            .await
            .get(server_name)
            .cloned()
            .ok_or_else(|| ToolError::McpServerNotFound(server_name.to_string()))?;

        if let Some(declaration) = declarations.iter().find(|decl| decl.name == tool_name) {
            return owned_enabled(server_name, declaration);
        }

        if let Some(canonical) = self.resolve_mcp_tool_name(tool_name).await {
            let prefix = format!("{server_name}{MCP_TOOL_NAME_SPLIT}");
            if let Some(internal) = canonical.strip_prefix(&prefix) {
                if let Some(declaration) = declarations.iter().find(|decl| decl.name == internal) {
                    return owned_enabled(server_name, declaration);
                }
            }
            return Err(ToolError::Config(format!(
                "the MCP tool '{tool_name}' does not belong to server '{server_name}'"
            )));
        }

        Err(ToolError::FunctionNotFound(tool_name.to_string()))
    }

    // =================================================
    // MCP tools
    // =================================================

    pub async fn start_mcp_server(
        self: Arc<Self>, // Changed to take Arc<Self>
        config: McpServerConfig,
    ) -> Result<(), ToolError> {
        // Starting an MCP server is canonical runtime work: fence it at this
        // lowest layer so a direct, non-HTTP caller is refused on the one gate.
        let _ticket = self.admit_work(crate::lifecycle::WorkKind::CapabilityStart)?;
        // Check if the server is already registered and running *without* holding the main lock
        // This avoids holding the main lock while potentially waiting for status()
        // Note: self.get_mcp_server doesn't need Arc<Self> if it only reads.
        let is_running = match self.get_mcp_server(config.name.as_str()).await {
            Ok(mcp_server) => mcp_server.status().await == McpStatus::Running,
            Err(_) => false, // Not found, so not running
        };

        if is_running {
            log::info!("MCP server {} is already running.", config.name);
            return Ok(());
        }

        // If not running or not found, proceed with registration which includes starting
        // Use self.clone() because register_mcp_server now takes Arc<Self>
        self.clone().register_mcp_server(config).await
    }

    /// Alias for `unregister_mcp_server`
    pub async fn stop_mcp_server(&self, name: &str) -> Result<(), ToolError> {
        self.unregister_mcp_server(name).await
    }

    async fn set_registration_status(&self, name: &str, generation: u64, status: McpStatus) {
        let generations = self.mcp_registration_generations.lock().await;
        if generations.get(name).copied() != Some(generation) {
            return;
        }
        let mut registration_status = self.mcp_registration_status.write().await;
        let status = match status {
            McpStatus::Running => {
                // Discovery has committed; future reads must observe live disconnects.
                registration_status.remove(name);
                McpStatus::Running
            }
            McpStatus::Error(_) => {
                let status = McpStatus::Error("registration_failed".to_string());
                registration_status.insert(name.to_string(), status.clone());
                status
            }
            other => {
                registration_status.insert(name.to_string(), other.clone());
                other
            }
        };
        let _ = self
            .mcp_status_event_sender
            .send((name.to_string(), status));
    }

    /// Registers a server and asynchronously commits its generation-fenced tool list.
    ///
    /// # Arguments
    /// * `self` - An Arc pointing to the FunctionManager instance.
    /// * `mcp_server_config` - Configuration for the MCP server to be registered.
    ///
    /// # Returns
    /// Result indicating success of initiating the registration process.
    /// The actual tool listing and internal registration happen asynchronously.
    pub async fn register_mcp_server(
        self: Arc<Self>, // Changed to take Arc<Self>
        mcp_server_config: McpServerConfig,
    ) -> Result<(), ToolError> {
        // The reserved server name belongs to the dedicated desktop Web MCP
        // provider. An ordinary, user-configured server may never occupy it, so
        // the fixed web tools can never be shadowed by a user MCP server.
        if mcp_server_config.name == chatspeed_contracts::WEB_MCP_SERVER_NAME {
            return Err(ToolError::Config(format!(
                "MCP server name `{}` is reserved for the desktop Web MCP provider",
                mcp_server_config.name
            )));
        }
        self.register_mcp_server_generic(mcp_server_config, None)
            .await
    }

    /// Registers the dedicated desktop loopback Web MCP provider.
    ///
    /// Only the reserved server name is accepted, and the streamable-HTTP client
    /// is built with a single bounded connect attempt so a released or dead
    /// provider fails its in-flight call immediately instead of silently
    /// reconnecting.
    pub async fn register_web_mcp_provider(
        self: Arc<Self>,
        mcp_server_config: McpServerConfig,
    ) -> Result<(), ToolError> {
        if mcp_server_config.name != chatspeed_contracts::WEB_MCP_SERVER_NAME {
            return Err(ToolError::Config(format!(
                "Web MCP provider must use the reserved server name `{}`",
                chatspeed_contracts::WEB_MCP_SERVER_NAME
            )));
        }
        self.register_mcp_server_generic(mcp_server_config, Some(1))
            .await
    }

    async fn register_mcp_server_generic(
        self: Arc<Self>, // Changed to take Arc<Self>
        mcp_server_config: McpServerConfig,
        provider_max_retries: Option<usize>,
    ) -> Result<(), ToolError> {
        #[cfg(debug_assertions)]
        {
            log::debug!("Register MCP server {} ... ", &mcp_server_config.name);
        }

        // Clone for logging in case of early error
        let server_name_for_log = mcp_server_config.name.clone();

        // Registration starts asynchronously and publishes a generation-fenced discovery result.
        let registration_generation = {
            let mut generations = self.mcp_registration_generations.lock().await;
            let mut statuses = self.mcp_registration_status.write().await;
            if matches!(
                statuses.get(&server_name_for_log),
                Some(McpStatus::Starting)
            ) {
                return Err(ToolError::StateChangeFailed(format!(
                    "MCP server '{}' registration is already in progress",
                    server_name_for_log
                )));
            }
            let generation = generations.entry(server_name_for_log.clone()).or_insert(0);
            *generation = generation.saturating_add(1);
            statuses.insert(server_name_for_log.clone(), McpStatus::Starting);
            *generation
        };
        // Immediately broadcast "Starting" status to provide user feedback
        if let Err(e) = self
            .mcp_status_event_sender
            .send((server_name_for_log.clone(), McpStatus::Starting))
        {
            log::warn!(
                "Failed to broadcast MCP starting status for server {}: {}",
                server_name_for_log,
                e
            );
        }

        // 1. Create the MCP client
        // This happens without holding FunctionManager's locks
        let client_result: Result<Arc<dyn McpClient>, crate::mcp::McpError> =
            match mcp_server_config.protocol_type {
                McpProtocolType::Sse => Err(crate::mcp::McpError::ClientConfigError(
                    t!("mcp.config.sse_removed_in_rmcp_v1").to_string().into(),
                )),
                McpProtocolType::Stdio => StdioClient::new(mcp_server_config.clone())
                    .map(|c| Arc::new(c) as Arc<dyn McpClient>),
                McpProtocolType::StreamableHttp => match provider_max_retries {
                    // The dedicated Web provider must fail closed without reconnecting.
                    Some(max_retries) => StreamableHttpClient::with_retry(
                        mcp_server_config.clone(),
                        max_retries,
                        std::time::Duration::from_millis(250),
                    )
                    .map(|c| Arc::new(c) as Arc<dyn McpClient>),
                    None => StreamableHttpClient::new(mcp_server_config.clone())
                        .map(|c| Arc::new(c) as Arc<dyn McpClient>),
                },
            };
        let client_arc: Arc<dyn McpClient> = match client_result {
            Ok(client) => client,
            Err(error) => {
                self.set_registration_status(
                    &server_name_for_log,
                    registration_generation,
                    McpStatus::Error(error.to_string()),
                )
                .await;
                return Err(ToolError::Config(
                    t!(
                        "mcp.client.config_error_for_server",
                        server_name = &server_name_for_log,
                        error = error.to_string()
                    )
                    .to_string(),
                ));
            }
        };

        // Set a status change callback for the client to broadcast its status changes.
        // This callback is invoked when the client's internal status changes
        // (through McpClientCore::set_status -> notify_status_change).
        // We set it before start() to ensure status changes during the start process
        // (such as becoming Running or Error) can be captured.
        let sender_for_callback = self.mcp_status_event_sender.clone();
        client_arc
            .on_status_change(Box::new(move |name, new_status| {
                if let Err(e) = sender_for_callback.send((name.clone(), new_status.clone())) {
                    log::error!(
                        "Failed to broadcast MCP status change for server {}: {}",
                        name,
                        e
                    );
                }
            }))
            .await;

        let name = client_arc.name().await;
        #[cfg(debug_assertions)]
        {
            log::debug!("MCP server {} created successfully.", &name);
        }

        // 2. Start the client
        // This .await happens without holding FunctionManager's locks
        if let Err(error) = client_arc.start().await {
            self.set_registration_status(
                &server_name_for_log,
                registration_generation,
                McpStatus::Error(error.to_string()),
            )
            .await;
            return Err(ToolError::Initialization(
                crate::error_diagnostics::FailureMessage::leaf(error.to_string())
                    .with_origin(error.error_origin()),
            ));
        }
        log::info!("MCP client {} started successfully.", &name);

        // 3. Publish the connected client before its tool list is read.
        // Registration used to wait for the listing, which left the runtime unable to
        // report a server that was up and still listing. The read projection reports a
        // server the runtime does not know as a proven stop, so a healthy cold start
        // showed as "enabled but not running" with zero tools for the whole listing.
        if let Err(error) = self
            .publish_connected_mcp_client(client_arc.clone(), registration_generation)
            .await
        {
            self.set_registration_status(
                &server_name_for_log,
                registration_generation,
                McpStatus::Error(error.to_string()),
            )
            .await;
            let _ = client_arc.stop().await;
            return Err(error);
        }

        // 4. The dedicated desktop Web MCP provider must finish its first tool
        // discovery before registration returns: desktop startup can create the
        // Vue windows immediately afterwards, and the first available-tools read
        // must not race an event emitted before the UI listener exists.
        // Ordinary configured MCP servers retain asynchronous discovery so one
        // slow server never delays the control plane.
        let tool_manager_arc = self.clone();
        let client_arc_for_task = client_arc.clone();
        let server_name_for_task = name.clone();
        let config_for_task = client_arc.config().await.clone();
        if provider_max_retries.is_some() {
            if let Err(error) = tool_manager_arc
                .discover_and_register_mcp_tools(
                    client_arc_for_task,
                    registration_generation,
                    config_for_task,
                    true,
                )
                .await
            {
                self.set_registration_status(
                    &server_name_for_task,
                    registration_generation,
                    McpStatus::Error(error.to_string()),
                )
                .await;
                return Err(error);
            }
            self.set_registration_status(
                &server_name_for_task,
                registration_generation,
                McpStatus::Running,
            )
            .await;
        } else {
            tokio::spawn(async move {
                if let Err(error) = tool_manager_arc
                    .discover_and_register_mcp_tools(
                        client_arc_for_task,
                        registration_generation,
                        config_for_task,
                        false,
                    )
                    .await
                {
                    tool_manager_arc
                        .set_registration_status(
                            &server_name_for_task,
                            registration_generation,
                            McpStatus::Error(error.to_string()),
                        )
                        .await;
                    log::debug!(
                        "Discarded MCP server {} discovery result for generation {}: {}",
                        server_name_for_task,
                        registration_generation,
                        error
                    );
                } else {
                    tool_manager_arc
                        .set_registration_status(
                            &server_name_for_task,
                            registration_generation,
                            McpStatus::Running,
                        )
                        .await;
                }
            });
        }

        // 5. Return after this server is connected and, for the reserved Web
        // provider, its initial tool snapshot has been committed.
        Ok(())
    }

    /// Lists and commits one MCP server's tool snapshot.
    async fn discover_and_register_mcp_tools(
        &self,
        client: Arc<dyn McpClient>,
        generation: u64,
        config: McpServerConfig,
        fail_closed: bool,
    ) -> Result<(), ToolError> {
        let server_name = client.name().await;
        let status = client.status().await;
        if status != McpStatus::Connected && status != McpStatus::Running {
            let error = ToolError::Initialization(
                format!(
                    "MCP server {} is not running (status: {:?})",
                    server_name, status
                )
                .into(),
            );
            if fail_closed {
                return Err(error);
            }
            log::warn!("{}", error);
            return Err(error);
        }

        let declarations = match client.list_tools().await {
            Ok(tools) => {
                let disabled_tool_names = config.disabled_tools.unwrap_or_default();
                Some(
                    tools
                        .into_iter()
                        .map(|mut tool_decl| {
                            tool_decl.disabled = disabled_tool_names.contains(&tool_decl.name);
                            tool_decl
                        })
                        .collect::<Vec<_>>(),
                )
            }
            Err(error) => {
                if fail_closed {
                    return Err(ToolError::Initialization(
                        crate::error_diagnostics::FailureMessage::leaf(format!(
                            "Failed to list tools for MCP server {}: {}",
                            server_name, error
                        ))
                        .with_origin(error.error_origin()),
                    ));
                }
                log::error!(
                    "Failed to list tools for MCP server {}: {}",
                    server_name,
                    error
                );
                return Err(ToolError::from(error));
            }
        };

        self.register_mcp_server_inner(client, generation, declarations)
            .await
    }

    /// Makes a started client visible to the runtime before its tool list is read.
    ///
    /// The client is published before listing so status observers can distinguish a
    /// connected/loading server from a server that is absent.
    async fn publish_connected_mcp_client(
        &self,
        client: Arc<dyn McpClient>,
        generation: u64,
    ) -> Result<(), ToolError> {
        let name = client.name().await;
        let generations = self.mcp_registration_generations.lock().await;
        if generations.get(&name).copied() != Some(generation) {
            return Err(ToolError::StateChangeFailed(format!(
                "Stale MCP registration for server '{}'",
                name
            )));
        }
        let mut servers_guard = self.mcp_servers.write().await;
        servers_guard.insert(name, client);
        drop(servers_guard);
        drop(generations);
        Ok(())
    }

    /// The server itself is published by `publish_connected_mcp_client`; this method
    /// commits the completed tool snapshot for the matching generation.
    ///
    /// # Arguments
    /// * `client` - The Arc to the started McpClient instance.
    /// * `tools_declarations` - An optional vector of tool declarations fetched from the client.
    ///
    /// # Returns
    /// * `Result<(), ToolError>` - The result of the registration.
    async fn register_mcp_server_inner(
        &self,
        client: Arc<dyn McpClient>,
        generation: u64,
        tools_declarations: Option<Vec<MCPToolDeclaration>>,
    ) -> Result<(), ToolError> {
        let name = client.name().await;
        // Keep the generation lock first, matching unregister/clear. This makes
        // invalidation and snapshot commit atomic with respect to each other.
        let generations = self.mcp_registration_generations.lock().await;
        if generations.get(&name).copied() != Some(generation) {
            return Err(ToolError::StateChangeFailed(format!(
                "Stale MCP registration for server '{}'",
                name
            )));
        }

        let mut mcp_tools_guard = self.mcp_tools.write().await;
        let servers_guard = self.mcp_servers.write().await;
        if !servers_guard
            .get(&name)
            .is_some_and(|registered| Arc::ptr_eq(registered, &client))
        {
            return Err(ToolError::StateChangeFailed(format!(
                "MCP registration changed for server '{}'",
                name
            )));
        }
        if let Some(declarations) = tools_declarations {
            mcp_tools_guard.insert(name.clone(), declarations);
        } else {
            mcp_tools_guard.insert(name.clone(), Vec::new());
        }
        drop(servers_guard);
        drop(mcp_tools_guard);
        drop(generations);

        self.rebuild_mcp_wrappers().await;
        self.notify_mcp_tools_changed();
        Ok(())
    }

    /// unregisters a MCP server with the manager.
    /// This involves removing it from internal state and stopping the client.
    ///
    /// # Arguments
    /// * `name` - The name of the MCP server to unregister.
    ///
    /// # Returns
    /// * `Result<(), ToolError>` - The result of the unregistration.
    pub async fn unregister_mcp_server(&self, name: &str) -> Result<(), ToolError> {
        let mut generations = self.mcp_registration_generations.lock().await;
        let generation = generations.entry(name.to_string()).or_insert(0);
        *generation = generation.saturating_add(1);

        // Scope the locks to ensure they are released before awaiting .stop()
        {
            let mut mcp_tools = self.mcp_tools.write().await;
            mcp_tools.remove(name);
        }
        self.mcp_registration_status.write().await.remove(name);
        let server_to_stop = {
            let mut servers_guard = self.mcp_servers.write().await;
            servers_guard.remove(name)
        };
        drop(generations);
        self.rebuild_mcp_wrappers().await;
        self.notify_mcp_tools_changed();

        // Stop the server client if it was found
        if let Some(server_arc) = server_to_stop {
            // Now call stop() on the Arc. FunctionManager's locks are released.
            // This .await happens without holding FunctionManager's locks.
            server_arc.stop().await.map_err(|e| {
                log::error!("Failed to stop MCP server {}: {}", name, e);
                // Construct the full message here, as StateChangeFailed is now #[error("{0}")]
                // McpClientError's Display impl is already clean.
                ToolError::StateChangeFailed(
                    t!(
                        "tools.mcp_stop_failed_details",
                        server_name = name,
                        details = e.to_string()
                    )
                    .to_string(),
                )
            })?;
        } else {
            // Server was not found in the map, maybe it wasn't registered or already removed.
            // Log a warning but don't necessarily return an error, as the goal (unregistering the name) is achieved.
            log::warn!(
                "Attempted to unregister MCP server {} but it was not found in the manager.",
                name
            );
        }
        Ok(())
    }

    /// Refreshes the tool list for a specific MCP server.
    ///
    /// This function will contact the specified MCP server, fetch its current list of tools,
    /// and update the in-memory cache (`mcp_tools`) with the new list.
    /// It uses status notifications to inform the frontend about its progress.
    ///
    /// # Arguments
    /// * `name` - The name of the MCP server to refresh.
    ///
    /// # Returns
    /// * `Result<(), ToolError>` - Ok on success, or an error if the server is not found or fetching tools fails.
    pub async fn refresh_mcp_server_tools(&self, name: &str) -> Result<(), ToolError> {
        // Use "Starting" status to indicate a refresh is in progress.
        self.mcp_status_event_sender
            .send((name.to_string(), McpStatus::Starting))
            .ok();

        let (client, generation) = {
            let mut generations = self.mcp_registration_generations.lock().await;
            let client = self.get_mcp_server(name).await?;
            // A newer explicit refresh owns discovery; an earlier startup list cannot overwrite it.
            let generation = generations.entry(name.to_string()).or_insert(0);
            *generation = generation.saturating_add(1);
            self.mcp_registration_status
                .write()
                .await
                .insert(name.to_string(), McpStatus::Starting);
            (client, *generation)
        };

        let tools_result = client.list_tools().await;

        match tools_result {
            Ok(tools) => {
                log::debug!(
                    "Successfully fetched {} tools for MCP server {} during refresh.",
                    tools.len(),
                    name
                );
                let config = client.config().await;
                let disabled_tool_names: HashSet<String> =
                    config.disabled_tools.unwrap_or_default();

                let tools_with_disabled_flag: Vec<MCPToolDeclaration> = tools
                    .into_iter()
                    .map(|mut tool_decl| {
                        tool_decl.disabled = disabled_tool_names.contains(&tool_decl.name);
                        tool_decl
                    })
                    .collect();

                self.register_mcp_server_inner(
                    client.clone(),
                    generation,
                    Some(tools_with_disabled_flag),
                )
                .await?;
                self.set_registration_status(name, generation, McpStatus::Running)
                    .await;
                log::info!("Successfully refreshed tools for MCP server: {}", name);
                Ok(())
            }
            Err(e) => {
                log::error!(
                    "Failed to list tools for MCP server {} during refresh: {}",
                    name,
                    e.to_string()
                );
                self.register_mcp_server_inner(client.clone(), generation, Some(Vec::new()))
                    .await?;
                self.set_registration_status(
                    name,
                    generation,
                    McpStatus::Error("list_failed".into()),
                )
                .await;
                Err(ToolError::from(e))
            }
        }
    }

    /// Gets the status of all registered MCP servers.
    ///
    /// # Returns
    /// * `Result<HashMap<String, McpStatus>, ToolError>` - The result of the status retrieval.
    ///   The key is the name of the MCP server, and the value is its status.
    ///   If no MCP servers are registered, an empty map is returned.
    ///   If an error occurs during the status retrieval, an error is returned.
    ///   The error message will indicate the specific error that occurred.
    pub async fn get_mcp_serves_status(&self) -> Result<HashMap<String, McpStatus>, ToolError> {
        let mut status = self.mcp_registration_status.read().await.clone();
        let server_arcs_to_check: Vec<(String, Arc<dyn McpClient>)> = {
            let servers_guard = self.mcp_servers.read().await;
            servers_guard
                .iter()
                .map(|(name, server_arc)| (name.clone(), server_arc.clone()))
                .collect()
        };
        for (name, server_arc) in server_arcs_to_check {
            if !status.contains_key(&name) {
                status.insert(name, server_arc.status().await);
            }
        }
        Ok(status)
    }

    /// Gets a MCP server in manager by its name.
    ///
    /// # Arguments
    /// * `name` - The name of the MCP server to get.
    ///
    /// # Returns
    /// * `Result<Arc<dyn McpClient>, ToolError>` - The result of the server retrieval.
    pub(crate) async fn get_mcp_server(&self, name: &str) -> Result<Arc<dyn McpClient>, ToolError> {
        let servers_guard: tokio::sync::RwLockReadGuard<
            '_,
            HashMap<String, Arc<dyn McpClient + 'static>>,
        > = self.mcp_servers.read().await;
        servers_guard
            .get(name)
            .cloned()
            .ok_or_else(|| ToolError::McpServerNotFound(name.to_string()))
    }

    /// Gets the list of tools declared by a specified MCP server.
    ///
    /// # Arguments
    /// * `name` - The name of the MCP server whose tools are to be retrieved.
    ///
    /// # Returns
    /// * `Result<Vec<MCPToolDeclaration>, ToolError>` - The result of the tool retrieval.
    pub async fn get_mcp_server_tools(
        &self,
        name: &str,
    ) -> Result<Vec<MCPToolDeclaration>, ToolError> {
        let tools_guard = self.mcp_tools.read().await;
        tools_guard
            .get(name)
            .cloned()
            .ok_or_else(|| ToolError::McpServerNotFound(name.to_string()))
    }

    /// Disables or enables a specific MCP tool in the in-memory cache.
    ///
    /// This function updates the `disabled` flag of a tool within the `mcp_tools` cache.
    /// NOTE: This only affects the in-memory state and does NOT persist the change
    /// to the database configuration. A separate mechanism (e.g., a Tauri command
    /// that updates the database and triggers server re-registration) is required
    /// for persistent changes and full state synchronization.
    ///
    /// # Arguments
    /// * `mcp_server_name` - The name of the MCP server.
    /// * `mcp_tool_name` - The name of the tool on the MCP server.
    /// * `is_disabled` - `true` to disable the tool, `false` to enable it.
    ///
    /// # Returns
    /// * `Result<(), ToolError>` - Ok on success, or an error if the server or tool is not found in the cache.
    pub async fn disable_mcp_tool(
        &self,
        mcp_server_name: &str,
        mcp_tool_name: &str,
        is_disabled: bool,
    ) -> Result<(), ToolError> {
        // Phase 1: Update the McpClient's internal config copy.
        // Get the client Arc first, then release the lock before awaiting the update.
        let server_client_to_update = {
            let mcp_servers_read_guard = self.mcp_servers.read().await;
            mcp_servers_read_guard.get(mcp_server_name).cloned()
        };

        if let Some(server_client) = server_client_to_update {
            server_client
                .update_disabled_tools(mcp_tool_name, is_disabled)
                .await
                .map_err(|e| {
                    ToolError::Config(format!(
                        "Failed to update McpClient internal config for tool {} on server {}: {}",
                        mcp_tool_name, mcp_server_name, e
                    ))
                })?;
            log::debug!(
                "Internal McpClient config updated for tool {} on server {}, disabled={}",
                mcp_tool_name,
                mcp_server_name,
                is_disabled
            );
        } else {
            // Log if server not found in mcp_servers, but proceed to update mcp_tools cache if possible.
            log::warn!("Server {} not found in mcp_servers cache while trying to update its internal config for tool {}.", mcp_server_name, mcp_tool_name);
        }

        // Phase 2: Update the in-memory declaration cache, then rebuild the wrappers.
        let result = {
            let mut mcp_tools_guard = self.mcp_tools.write().await;
            if let Some(tools) = mcp_tools_guard.get_mut(mcp_server_name) {
                if let Some(tool_decl) = tools.iter_mut().find(|td| td.name == mcp_tool_name) {
                    tool_decl.disabled = is_disabled;
                    Ok(())
                } else {
                    Err(ToolError::FunctionNotFound(format!(
                        "Tool {} not found on server {} in mcp_tools cache.",
                        mcp_tool_name, mcp_server_name
                    )))
                }
            } else {
                Err(ToolError::McpServerNotFound(format!(
                    "Server {} not found in mcp_tools cache.",
                    mcp_server_name
                )))
            }
        };
        if result.is_ok() {
            self.rebuild_mcp_wrappers().await;
            self.notify_mcp_tools_changed();
        }
        result
    }

    pub fn subscribe_mcp_status_events(&self) -> broadcast::Receiver<(String, McpStatus)> {
        self.mcp_status_event_sender.subscribe()
    }

    pub fn subscribe_mcp_tool_change_events(&self) -> broadcast::Receiver<()> {
        self.mcp_tool_change_event_sender.subscribe()
    }
}

/// Builds a bounded, redacted caller-facing detail for a failed MCP tool call.
///
/// Provider diagnostics are useful for repairing a custom MCP server, so keep
/// the provider's actual detail instead of collapsing every failure to one
/// generic sentence. JSON diagnostics go through the shared recursive redactor;
/// plain text goes through the text redactor. Both forms are length-bounded.
#[cfg(not(feature = "desktop"))]
fn mcp_call_error_detail(error: &McpError) -> String {
    const MAX_DETAIL_CHARS: usize = 1600;
    let raw_detail = match error {
        McpError::ClientCallError(detail)
        | McpError::ClientConfigError(detail)
        | McpError::ClientStartError(detail)
        | McpError::ClientStopError(detail)
        | McpError::ServerInitializationError(detail)
        | McpError::Io(detail)
        | McpError::Serialization(detail)
        | McpError::Timeout(detail)
        | McpError::General(detail) => detail.as_str(),
        McpError::ClientStatusError(detail)
        | McpError::ServerToolExecutionError(detail)
        | McpError::ServerInternalError(detail)
        | McpError::ServerUnknownError(detail)
        | McpError::Store(detail)
        | McpError::StateChangeFailed(detail) => detail.as_str(),
        McpError::ServerToolNotFound(name) | McpError::NotFound(name) => name,
    };
    let detail = serde_json::from_str::<Value>(raw_detail)
        .map(|value| {
            crate::capability::redaction::bounded_redacted_json(&value, MAX_DETAIL_CHARS)
                .to_string()
        })
        .unwrap_or_else(|_| crate::capability::redaction::redact_text(raw_detail));
    let bounded: String = detail.chars().take(MAX_DETAIL_CHARS).collect();
    format!("MCP tool call failed: {bounded}")
}

#[cfg(all(test, not(feature = "desktop")))]
mod tests {
    use super::*;
    use crate::tools::{ToolCallResult, ToolCategory, ToolScope};
    use serde_json::json;

    #[tokio::test]
    async fn configuration_failure_is_observable_before_a_client_is_published() {
        let manager = Arc::new(ToolManager::new());
        let result = manager
            .clone()
            .register_mcp_server(McpServerConfig {
                name: "invalid-transport".into(),
                protocol_type: McpProtocolType::Sse,
                ..Default::default()
            })
            .await;
        assert!(result.is_err());
        assert_eq!(
            manager
                .get_mcp_serves_status()
                .await
                .expect("statuses")
                .get("invalid-transport"),
            Some(&McpStatus::Error("registration_failed".into()))
        );
        assert!(manager.get_mcp_server("invalid-transport").await.is_err());
    }

    #[tokio::test]
    async fn failed_registration_is_observable_and_old_generations_cannot_restore_it() {
        let manager = ToolManager::new();
        manager
            .mcp_registration_generations
            .lock()
            .await
            .insert("test".into(), 1);
        manager
            .set_registration_status("test", 1, McpStatus::Error("secret".into()))
            .await;
        assert_eq!(
            manager
                .get_mcp_serves_status()
                .await
                .expect("statuses")
                .get("test"),
            Some(&McpStatus::Error("registration_failed".into()))
        );
        manager.unregister_mcp_server("test").await.expect("stop");
        manager
            .set_registration_status("test", 1, McpStatus::Starting)
            .await;
        assert!(!manager
            .get_mcp_serves_status()
            .await
            .expect("statuses")
            .contains_key("test"));
    }

    #[tokio::test]
    async fn completed_registration_follows_live_client_status() {
        let manager = ToolManager::new();
        manager
            .register_test_mcp_tool("test", "search", json!({}))
            .await
            .expect("tool");
        manager
            .mcp_registration_generations
            .lock()
            .await
            .insert("test".into(), 1);
        manager
            .set_registration_status("test", 1, McpStatus::Running)
            .await;
        assert_eq!(
            manager
                .get_mcp_serves_status()
                .await
                .expect("statuses")
                .get("test"),
            Some(&McpStatus::Connected)
        );
        manager
            .get_mcp_server("test")
            .await
            .expect("client")
            .set_status(McpStatus::Stopped)
            .await;
        assert_eq!(
            manager
                .get_mcp_serves_status()
                .await
                .expect("statuses")
                .get("test"),
            Some(&McpStatus::Stopped)
        );
    }

    #[tokio::test]
    async fn clear_invalidates_unpublished_registration() {
        let manager = ToolManager::new();
        manager
            .mcp_registration_generations
            .lock()
            .await
            .insert("test".into(), 1);
        manager
            .set_registration_status("test", 1, McpStatus::Starting)
            .await;
        manager.clear(true).await;
        manager
            .set_registration_status("test", 1, McpStatus::Error("late".into()))
            .await;
        assert!(!manager
            .get_mcp_serves_status()
            .await
            .expect("statuses")
            .contains_key("test"));
    }

    // A structured provider error is classified down to its allowlisted code,
    // and the provider's own message (which may carry a URL or credential) is
    // never echoed into the caller-facing detail.
    #[test]
    fn structured_provider_code_is_surfaced_without_provider_text() {
        let message = json!({
            "code": "config_unavailable",
            "message": "https://user:secret@example.com token=abc123",
        })
        .to_string();
        let detail = mcp_call_error_detail(&McpError::ClientCallError(message.into()));
        assert!(detail.starts_with("MCP tool call failed:"));
        assert!(detail.contains("config_unavailable"));
        assert!(detail.contains("[redacted]@example.com"));
        assert!(!detail.contains("secret"));
        assert!(!detail.contains("abc123"));
    }

    // Provider diagnostics remain visible, but are bounded and redacted.
    #[test]
    fn provider_error_detail_is_visible_without_secrets() {
        let detail = mcp_call_error_detail(&McpError::ClientCallError(
            json!({"code": "provider_private_code", "message": "raw provider failure"})
                .to_string()
                .into(),
        ));
        assert!(detail.contains("provider_private_code"));
        assert!(detail.contains("raw provider failure"));
        assert_eq!(
            mcp_call_error_detail(&McpError::Timeout("timed out".into())),
            "MCP tool call failed: timed out"
        );
    }

    #[test]
    fn provider_error_detail_is_bounded() {
        let detail = mcp_call_error_detail(&McpError::General("x".repeat(5000).into()));
        assert!(detail.chars().count() <= 1622);
    }

    // A mock tool for testing
    struct MockTool {
        name: String,
        scope: ToolScope,
    }

    #[async_trait]
    impl ToolDefinition for MockTool {
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            "Mock"
        }
        fn category(&self) -> ToolCategory {
            ToolCategory::System
        }
        fn scope(&self) -> ToolScope {
            self.scope
        }
        fn tool_calling_spec(&self) -> MCPToolDeclaration {
            MCPToolDeclaration {
                name: self.name.clone(),
                description: "Mock".into(),
                input_schema: json!({}),
                output_schema: None,
                disabled: false,
                scope: Some(self.scope()),
            }
        }
        async fn call(&self, _params: Value) -> NativeToolResult {
            Ok(ToolCallResult::success(Some("ok".into()), None))
        }
    }

    #[test]
    fn allocates_mcp_aliases_deterministically_without_shadowing_reserved_names() {
        let inputs = vec![
            McpAliasInput {
                canonical_name: "beta__MCP__read file".into(),
                server_name: "beta".into(),
                tool_name: "read file".into(),
            },
            McpAliasInput {
                canonical_name: "alpha__MCP__read_file".into(),
                server_name: "alpha".into(),
                tool_name: "read_file".into(),
            },
            McpAliasInput {
                canonical_name: "gamma__MCP__bash".into(),
                server_name: "gamma".into(),
                tool_name: "bash".into(),
            },
        ];
        let registry = allocate_mcp_aliases(inputs.clone(), vec!["web_search".into()]);
        let reversed = allocate_mcp_aliases(
            inputs.into_iter().rev().collect(),
            vec!["web_search".into()],
        );

        assert_eq!(
            registry.alias_for("alpha__MCP__read_file").as_deref(),
            Some("alpha_read_file")
        );
        assert_eq!(
            registry.alias_for("beta__MCP__read file").as_deref(),
            Some("beta_read_file")
        );
        assert_eq!(
            registry.alias_for("gamma__MCP__bash").as_deref(),
            Some("gamma_bash")
        );
        assert_eq!(registry.alias_to_canonical, reversed.alias_to_canonical);
    }

    #[test]
    fn allocates_numeric_mcp_alias_suffixes_after_server_prefix_collisions() {
        let registry = allocate_mcp_aliases(
            vec![
                McpAliasInput {
                    canonical_name: "one__MCP__tool".into(),
                    server_name: "server".into(),
                    tool_name: "tool".into(),
                },
                McpAliasInput {
                    canonical_name: "two__MCP__tool".into(),
                    server_name: "server".into(),
                    tool_name: "tool".into(),
                },
                McpAliasInput {
                    canonical_name: "three__MCP__tool".into(),
                    server_name: "server".into(),
                    tool_name: "tool".into(),
                },
            ],
            Vec::new(),
        );

        assert_eq!(
            registry.alias_for("one__MCP__tool").as_deref(),
            Some("tool")
        );
        assert_eq!(
            registry.alias_for("three__MCP__tool").as_deref(),
            Some("server_tool")
        );
        assert_eq!(
            registry.alias_for("two__MCP__tool").as_deref(),
            Some("server_tool_2")
        );
    }

    #[test]
    fn reserved_web_provider_owns_the_exact_web_aliases() {
        let server = chatspeed_contracts::WEB_MCP_SERVER_NAME;
        let registry = allocate_mcp_aliases(
            vec![
                McpAliasInput {
                    canonical_name: format!("{server}__MCP__web_fetch"),
                    server_name: server.to_string(),
                    tool_name: "web_fetch".into(),
                },
                McpAliasInput {
                    canonical_name: format!("{server}__MCP__web_search"),
                    server_name: server.to_string(),
                    tool_name: "web_search".into(),
                },
            ],
            Vec::new(),
        );
        assert_eq!(
            registry
                .alias_for(&format!("{server}__MCP__web_fetch"))
                .as_deref(),
            Some("web_fetch")
        );
        assert_eq!(
            registry
                .alias_for(&format!("{server}__MCP__web_search"))
                .as_deref(),
            Some("web_search")
        );
    }

    #[test]
    fn an_ordinary_mcp_server_cannot_occupy_the_reserved_web_aliases() {
        let registry = allocate_mcp_aliases(
            vec![McpAliasInput {
                canonical_name: "evil__MCP__web_fetch".into(),
                server_name: "evil".into(),
                tool_name: "web_fetch".into(),
            }],
            Vec::new(),
        );
        assert_eq!(
            registry.alias_for("evil__MCP__web_fetch").as_deref(),
            Some("evil_web_fetch")
        );
    }

    #[tokio::test]
    async fn register_mcp_server_rejects_the_reserved_provider_name() {
        let manager = Arc::new(ToolManager::new());
        let config = McpServerConfig {
            name: chatspeed_contracts::WEB_MCP_SERVER_NAME.to_string(),
            protocol_type: McpProtocolType::Stdio,
            ..Default::default()
        };
        let error = manager
            .register_mcp_server(config)
            .await
            .expect_err("the reserved provider name must be rejected");
        assert!(matches!(error, ToolError::Config(_)), "{error}");
    }

    /// A manual invocation reaches the MCP client only for a tool that belongs
    /// to the requested server and is enabled; the exact internal name and the
    /// distinct public alias both resolve to that server's tool.
    #[cfg(not(feature = "desktop"))]
    #[tokio::test]
    async fn invoke_mcp_tool_reaches_the_client_only_for_an_owned_enabled_tool() {
        let manager = Arc::new(ToolManager::new());
        manager
            .register_test_mcp_tool("alpha", "get_weather", json!({ "type": "object" }))
            .await
            .expect("alpha tool");
        manager
            .register_test_mcp_tool("beta", "get_weather", json!({ "type": "object" }))
            .await
            .expect("beta tool");

        // The internal name of an owned tool passes validation. No live child is
        // connected in this test, so reaching the client is proven by the
        // execution error rather than by a successful result.
        let error = manager
            .invoke_mcp_tool("alpha", "get_weather", json!({ "city": "Kyiv" }))
            .await
            .expect_err("no live child is connected");
        assert!(
            matches!(error, ToolError::ExecutionFailed(_)),
            "an owned internal name must reach the client: {error}"
        );

        // beta's colliding tool gets the distinct `beta_get_weather` alias, which
        // must resolve to beta's tool and reach beta's client.
        let error = manager
            .invoke_mcp_tool("beta", "beta_get_weather", json!({}))
            .await
            .expect_err("no live child is connected");
        assert!(
            matches!(error, ToolError::ExecutionFailed(_)),
            "an owned alias must reach the client: {error}"
        );
    }

    /// A tool that belongs to another server, an unowned name and an unknown
    /// server are refused before any MCP client is reached (INV-1).
    #[cfg(not(feature = "desktop"))]
    #[tokio::test]
    async fn invoke_mcp_tool_refuses_a_tool_not_owned_by_the_requested_server() {
        let manager = Arc::new(ToolManager::new());
        manager
            .register_test_mcp_tool("alpha", "get_weather", json!({}))
            .await
            .expect("alpha tool");
        manager
            .register_test_mcp_tool("beta", "get_weather", json!({}))
            .await
            .expect("beta tool");

        // beta's canonical name may not be routed through alpha.
        let error = manager
            .invoke_mcp_tool("alpha", "beta__MCP__get_weather", json!({}))
            .await
            .expect_err("a foreign canonical name is refused");
        assert!(matches!(error, ToolError::Config(_)), "{error}");

        // beta's distinct public alias is foreign to alpha too.
        let error = manager
            .invoke_mcp_tool("alpha", "beta_get_weather", json!({}))
            .await
            .expect_err("a foreign alias is refused");
        assert!(matches!(error, ToolError::Config(_)), "{error}");

        // An unowned name has no canonical tool at all.
        let error = manager
            .invoke_mcp_tool("alpha", "nope", json!({}))
            .await
            .expect_err("an unowned name is refused");
        assert!(matches!(error, ToolError::FunctionNotFound(_)), "{error}");

        // A server the manager does not hold is refused, not guessed.
        let error = manager
            .invoke_mcp_tool("ghost", "get_weather", json!({}))
            .await
            .expect_err("an unknown server is refused");
        assert!(matches!(error, ToolError::McpServerNotFound(_)), "{error}");
    }

    /// A disabled tool stays addressable for management, so the invocation gate
    /// must refuse it explicitly rather than run it.
    #[cfg(not(feature = "desktop"))]
    #[tokio::test]
    async fn invoke_mcp_tool_refuses_a_disabled_tool() {
        let manager = Arc::new(ToolManager::new());
        manager
            .register_test_mcp_tool("alpha", "get_weather", json!({}))
            .await
            .expect("alpha tool");
        manager
            .disable_mcp_tool("alpha", "get_weather", true)
            .await
            .expect("disable the tool");

        let error = manager
            .invoke_mcp_tool("alpha", "get_weather", json!({}))
            .await
            .expect_err("a disabled tool is refused");
        assert!(matches!(error, ToolError::Security(_)), "{error}");
    }

    #[cfg(not(feature = "desktop"))]
    #[tokio::test]
    async fn disabled_mcp_tools_do_not_reserve_public_aliases() {
        let manager = Arc::new(ToolManager::new());
        let declaration = |disabled| MCPToolDeclaration {
            name: "search".into(),
            description: "Search".into(),
            input_schema: json!({}),
            output_schema: None,
            disabled,
            scope: Some(ToolScope::Both),
        };
        let client = |name: &str| {
            Arc::new(
                crate::mcp::client::StdioClient::new(crate::mcp::client::McpServerConfig {
                    name: name.to_string(),
                    protocol_type: crate::mcp::client::McpProtocolType::Stdio,
                    command: Some("ls".into()),
                    args: Some(vec!["-la".into()]),
                    ..Default::default()
                })
                .expect("test MCP client"),
            ) as Arc<dyn McpClient>
        };

        manager.mcp_servers.write().await.extend([
            (String::from("alpha"), client("alpha")),
            (String::from("beta"), client("beta")),
        ]);
        manager.mcp_tools.write().await.extend([
            (String::from("alpha"), vec![declaration(true)]),
            (String::from("beta"), vec![declaration(false)]),
        ]);

        manager.rebuild_mcp_wrappers().await;

        let specs = manager.get_mcp_tool_specs(None).await;
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].canonical_name, "beta__MCP__search");
        assert_eq!(specs[0].declaration.name, "search");
        assert_eq!(
            manager.resolve_mcp_tool_name("search").await.as_deref(),
            Some("beta__MCP__search")
        );
        assert_eq!(
            manager
                .resolve_mcp_tool_name("alpha__MCP__search")
                .await
                .as_deref(),
            Some("alpha__MCP__search")
        );
        assert!(manager
            .resolve_mcp_tool_name("alpha_search")
            .await
            .is_none());

        let loader = crate::tools::McpToolExpand {
            tool_manager: manager.clone(),
            allowed_tools: Some(HashSet::from(["beta__MCP__search".to_string()])),
        };
        let loaded = loader
            .call(json!({ "tool_name": "search" }))
            .await
            .expect("public MCP alias must resolve through mcp_tool_expand");
        let declaration = loaded
            .structured_content
            .expect("mcp_tool_expand must return a declaration");
        assert_eq!(declaration["name"], "search");
        assert!(loaded
            .content
            .as_deref()
            .is_some_and(|content| content.contains("Full MCP tool definition:")));
        assert!(loaded.content.as_deref().is_some_and(|content| {
            content.contains("Call `mcp_tool_execute` in your next tool action")
        }));
    }

    #[cfg(not(feature = "desktop"))]
    #[tokio::test]
    async fn legacy_mcp_expander_name_resolves_to_new_name() {
        let manager = Arc::new(ToolManager::new());
        manager
            .register_tool(Arc::new(crate::tools::McpToolExpand {
                tool_manager: manager.clone(),
                allowed_tools: None,
            }))
            .await
            .expect("MCP expander should register");

        assert!(
            manager
                .has_tool(crate::tools::TOOL_MCP_TOOL_LOAD_LEGACY)
                .await
        );
        let tool = manager
            .get_tool(crate::tools::TOOL_MCP_TOOL_LOAD_LEGACY)
            .await
            .expect("legacy MCP expander name should resolve");
        assert_eq!(tool.name(), crate::tools::TOOL_MCP_TOOL_EXPAND);
    }

    #[tokio::test]
    async fn test_tool_scope_filtering() {
        let manager = ToolManager::new();

        // Register a Chat tool
        manager
            .register_tool(Arc::new(MockTool {
                name: "chat_only".into(),
                scope: ToolScope::Chat,
            }))
            .await
            .unwrap();
        // Register a Workflow tool
        manager
            .register_tool(Arc::new(MockTool {
                name: "wf_only".into(),
                scope: ToolScope::Workflow,
            }))
            .await
            .unwrap();
        // Register a Both tool
        manager
            .register_tool(Arc::new(MockTool {
                name: "both".into(),
                scope: ToolScope::Both,
            }))
            .await
            .unwrap();

        // 1. Filter for Chat: Should get chat_only + both
        let chat_specs = manager
            .get_tool_calling_spec(Some(ToolScope::Chat), None)
            .await
            .unwrap();
        let names: HashSet<_> = chat_specs.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains("chat_only"));
        assert!(names.contains("both"));
        assert!(!names.contains("wf_only"));

        // 2. Filter for Workflow: Should get everything (WF + Chat + Both)
        let wf_specs = manager
            .get_tool_calling_spec(Some(ToolScope::Workflow), None)
            .await
            .unwrap();
        let names: HashSet<_> = wf_specs.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains("wf_only"));
        assert!(names.contains("chat_only"));
        assert!(names.contains("both"));
    }

    #[cfg(not(feature = "desktop"))]
    #[tokio::test]
    async fn native_registration_after_a_copied_mcp_wrapper_removes_the_wrapper() {
        let manager = ToolManager::new();
        let wrapper = Arc::new(McpToolWrapper {
            server_name: "test_server".into(),
            tool_decl: MCPToolDeclaration {
                name: "copied_tool".into(),
                description: "Copied MCP tool".into(),
                input_schema: json!({ "type": "object" }),
                output_schema: None,
                disabled: false,
                scope: Some(ToolScope::Both),
            },
            client: Arc::new(
                crate::mcp::client::StdioClient::new(crate::mcp::client::McpServerConfig {
                    name: "test_server".into(),
                    protocol_type: crate::mcp::client::McpProtocolType::Stdio,
                    command: Some("ls".into()),
                    args: Some(vec!["-la".into()]),
                    ..Default::default()
                })
                .expect("test client"),
            ),
            canonical_name: "test_server__MCP__copied_tool".into(),
            public_name: "copied_tool".into(),
        });

        manager
            .register_mcp_tool_wrapper(wrapper)
            .await
            .expect("copied wrapper must register");
        assert!(manager.has_tool("copied_tool").await);

        manager
            .register_tool(Arc::new(MockTool {
                name: "native_after_mcp".into(),
                scope: ToolScope::Both,
            }))
            .await
            .expect("native tool must register");

        assert!(manager.has_tool("native_after_mcp").await);
        assert!(
            !manager.has_tool("copied_tool").await,
            "native registration rebuilds from the local MCP cache and removes copied wrappers"
        );
    }

    #[cfg(not(feature = "desktop"))]
    #[tokio::test]
    async fn copied_mcp_wrapper_registered_last_remains_model_visible() {
        let manager = ToolManager::new();
        manager
            .register_tool(Arc::new(MockTool {
                name: "native_before_mcp".into(),
                scope: ToolScope::Both,
            }))
            .await
            .expect("native tool must register");
        let wrapper = Arc::new(McpToolWrapper {
            server_name: "test_server".into(),
            tool_decl: MCPToolDeclaration {
                name: "visible_tool".into(),
                description: "Visible MCP tool".into(),
                input_schema: json!({ "type": "object" }),
                output_schema: None,
                disabled: false,
                scope: Some(ToolScope::Both),
            },
            client: Arc::new(
                crate::mcp::client::StdioClient::new(crate::mcp::client::McpServerConfig {
                    name: "test_server".into(),
                    protocol_type: crate::mcp::client::McpProtocolType::Stdio,
                    command: Some("ls".into()),
                    args: Some(vec!["-la".into()]),
                    ..Default::default()
                })
                .expect("test client"),
            ),
            canonical_name: "test_server__MCP__visible_tool".into(),
            public_name: "visible_tool".into(),
        });
        manager
            .register_mcp_tool_wrapper(wrapper)
            .await
            .expect("copied wrapper must register last");

        let names = manager
            .get_tool_calling_spec(Some(ToolScope::Workflow), None)
            .await
            .expect("tool declarations")
            .into_iter()
            .map(|tool| tool.name)
            .collect::<HashSet<_>>();
        assert!(names.contains("native_before_mcp"));
        assert!(names.contains("visible_tool"));
    }

    #[tokio::test]
    async fn test_mcp_wrapper_integration() {
        let manager = ToolManager::new();

        // Mock data for registration
        let server_name = "test_server";
        let tool_decl = MCPToolDeclaration {
            name: "test_tool".into(),
            description: "Desc".into(),
            input_schema: json!({}),
            output_schema: None,
            disabled: false,
            scope: Some(ToolScope::Chat),
        };

        let canonical_name = format!("{}{}{}", server_name, MCP_TOOL_NAME_SPLIT, "test_tool");
        let public_name = "test_tool".to_string();

        // Manually register a wrapper to verify public declarations retain canonical keys.
        let wrapper = Arc::new(McpToolWrapper {
            server_name: server_name.into(),
            tool_decl: tool_decl.clone(),
            client: Arc::new(
                crate::mcp::client::StdioClient::new(crate::mcp::client::McpServerConfig {
                    name: "test".into(),
                    protocol_type: crate::mcp::client::McpProtocolType::Stdio,
                    command: Some("ls".into()),
                    args: Some(vec!["-la".into()]),
                    ..Default::default()
                })
                .unwrap(),
            ), // Dummy
            canonical_name: canonical_name.clone(),
            public_name: public_name.clone(),
        });

        manager.register_tool(wrapper).await.unwrap();

        let specs = manager.get_tool_calling_spec(None, None).await.unwrap();
        let names: HashSet<_> = specs.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(public_name.as_str()));
        assert!(manager.has_tool(&canonical_name).await);
        assert!(manager.has_tool(&public_name).await);
    }

    #[tokio::test]
    async fn broadcasts_mcp_tool_change_events() {
        let manager = ToolManager::new();
        let mut receiver = manager.subscribe_mcp_tool_change_events();

        manager.notify_mcp_tools_changed();

        receiver.recv().await.expect("tool change event");
    }

    #[tokio::test]
    async fn a_published_client_is_observable_before_its_tool_list_arrives() {
        let manager = Arc::new(ToolManager::new());
        let client: Arc<dyn McpClient> = Arc::new(
            StdioClient::new(McpServerConfig {
                name: "weather".into(),
                protocol_type: McpProtocolType::Stdio,
                command: Some("ls".into()),
                args: Some(vec!["-la".into()]),
                ..Default::default()
            })
            .expect("test MCP client"),
        );

        // Registration publishes the client before the tool listing, so the runtime
        // answers with the client's own status while the cache is still empty. Staying
        // absent until the listing finished is what made a cold start read as a proven
        // "stopped" server ("enabled but not running") with zero tools.
        let generation = {
            let mut generations = manager.mcp_registration_generations.lock().await;
            let generation = generations.entry("weather".to_string()).or_insert(0);
            *generation = generation.saturating_add(1);
            *generation
        };
        manager
            .publish_connected_mcp_client(client.clone(), generation)
            .await
            .expect("test client publication");

        let statuses = manager.get_mcp_serves_status().await.expect("status read");
        let published = statuses
            .get("weather")
            .expect("a published server must be observable");
        assert_eq!(published, &client.status().await);
        assert!(
            manager.get_mcp_server_tools("weather").await.is_err(),
            "the tool cache is filled by the listing, not by the publish"
        );
    }

    #[tokio::test]
    async fn stale_mcp_discovery_cannot_overwrite_a_same_name_restart() {
        let manager = Arc::new(ToolManager::new());
        let old_client: Arc<dyn McpClient> = Arc::new(
            StdioClient::new(McpServerConfig {
                name: "weather".into(),
                protocol_type: McpProtocolType::Stdio,
                command: Some("ls".into()),
                args: Some(vec!["-la".into()]),
                ..Default::default()
            })
            .expect("old test MCP client"),
        );
        let new_client: Arc<dyn McpClient> = Arc::new(
            StdioClient::new(McpServerConfig {
                name: "weather".into(),
                protocol_type: McpProtocolType::Stdio,
                command: Some("ls".into()),
                args: Some(vec!["-la".into()]),
                ..Default::default()
            })
            .expect("new test MCP client"),
        );

        let old_generation = {
            let mut generations = manager.mcp_registration_generations.lock().await;
            let generation = generations.entry("weather".to_string()).or_insert(0);
            *generation = generation.saturating_add(1);
            *generation
        };
        manager
            .publish_connected_mcp_client(old_client.clone(), old_generation)
            .await
            .expect("publish old client");

        manager
            .unregister_mcp_server("weather")
            .await
            .expect("stop old client");

        let new_generation = {
            let mut generations = manager.mcp_registration_generations.lock().await;
            let generation = generations.entry("weather".to_string()).or_insert(0);
            *generation = generation.saturating_add(1);
            *generation
        };
        manager
            .publish_connected_mcp_client(new_client.clone(), new_generation)
            .await
            .expect("publish new client");

        let stale = manager
            .register_mcp_server_inner(
                old_client,
                old_generation,
                Some(vec![MCPToolDeclaration {
                    name: "old_tool".into(),
                    description: "old".into(),
                    input_schema: json!({}),
                    output_schema: None,
                    disabled: false,
                    scope: Some(ToolScope::Both),
                }]),
            )
            .await;
        assert!(stale.is_err(), "old discovery must be rejected");
        assert!(manager.get_mcp_server_tools("weather").await.is_err());

        manager
            .register_mcp_server_inner(new_client, new_generation, Some(Vec::new()))
            .await
            .expect("new discovery should register");
        assert!(manager.get_mcp_server_tools("weather").await.is_ok());
    }

    #[tokio::test]
    async fn tool_call_with_dispatch_reports_owner_entry_fact() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let manager = ToolManager::new();
        manager
            .register_tool(Arc::new(MockTool {
                name: "ok_tool".into(),
                scope: ToolScope::Both,
            }))
            .await
            .expect("mock tool should register");

        // Registry lookup miss: failure happens before owner entry.
        let (result, dispatched) = manager
            .tool_call_with_dispatch("missing_tool", json!({}), None)
            .await;
        assert!(matches!(result, Err(ToolError::FunctionNotFound(_))));
        assert!(!dispatched);

        // Owner entered even when the tool itself fails; the flag is set
        // exactly at the owner boundary.
        let flag = Arc::new(AtomicBool::new(false));
        let (result, dispatched) = manager
            .tool_call_with_dispatch("ok_tool", json!({}), Some(Arc::clone(&flag)))
            .await;
        assert!(result.is_ok());
        assert!(dispatched);
        assert!(flag.load(Ordering::SeqCst));
    }

    // End-to-end coverage that the dedicated desktop Web MCP provider is
    // reached through the canonical `ToolManager` MCP client path: a real
    // streamable-HTTP MCP server exposes exactly `web_fetch`/`web_search`, the
    // manager allocates the exact public aliases, and a call executes.
    #[cfg(not(feature = "desktop"))]
    mod web_provider_e2e {
        use super::*;
        use axum::Router;
        use rmcp::model::{
            CallToolRequestParams, CallToolResponse, CallToolResult, ListToolsResult,
            PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
        };
        use rmcp::service::{RequestContext, RoleServer};
        use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
        use rmcp::transport::streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService,
        };
        use rmcp::{ErrorData, ServerHandler};

        /// A stub MCP server exposing exactly the two fixed web tools.
        #[derive(Clone)]
        struct StubWebProvider;

        impl ServerHandler for StubWebProvider {
            fn get_info(&self) -> ServerConfig {
                let mut info = ServerConfig::default();
                info.capabilities = ServerCapabilities::builder().enable_tools().build();
                info
            }

            async fn list_tools(
                &self,
                _request: Option<PaginatedRequestParams>,
                _context: RequestContext<RoleServer>,
            ) -> Result<ListToolsResult, ErrorData> {
                let schema = Arc::new(serde_json::Map::new());
                Ok(ListToolsResult::with_all_items(vec![
                    Tool::new("web_fetch", "fetch", schema.clone()),
                    Tool::new("web_search", "search", schema),
                ]))
            }

            async fn call_tool(
                &self,
                request: CallToolRequestParams,
                _context: RequestContext<RoleServer>,
            ) -> Result<CallToolResponse, ErrorData> {
                Ok(CallToolResponse::Complete(CallToolResult::structured(
                    json!({"tool": request.name, "ok": true}),
                )))
            }
        }

        async fn spawn_stub() -> (u16, tokio::task::JoinHandle<()>) {
            let factory = move || Ok(StubWebProvider);
            let service = StreamableHttpService::new(
                factory,
                Arc::new(LocalSessionManager::default()),
                StreamableHttpServerConfig::default(),
            );
            let router = Router::new().nest_service("/mcp", service);
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .expect("bind stub provider");
            let port = listener.local_addr().expect("addr").port();
            let task = tokio::spawn(async move {
                let _ = axum::serve(listener, router).await;
            });
            (port, task)
        }

        async fn await_alias(manager: &Arc<ToolManager>, alias: &str) -> String {
            for _ in 0..100 {
                if let Some(canonical) = manager.resolve_mcp_tool_name(alias).await {
                    return canonical;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            panic!("alias `{alias}` was never exposed");
        }

        #[tokio::test]
        async fn web_provider_is_registered_through_the_canonical_mcp_path() {
            let (port, task) = spawn_stub().await;
            let manager = Arc::new(ToolManager::new());
            let config = McpServerConfig {
                name: chatspeed_contracts::WEB_MCP_SERVER_NAME.to_string(),
                protocol_type: McpProtocolType::StreamableHttp,
                url: Some(chatspeed_contracts::web_mcp_endpoint(port)),
                bearer_token: Some("provider-proof".to_string()),
                timeout: Some(10),
                ..Default::default()
            };
            manager
                .clone()
                .register_web_mcp_provider(config)
                .await
                .expect("register the provider as an MCP server");

            let fetch = await_alias(&manager, "web_fetch").await;
            let search = await_alias(&manager, "web_search").await;
            let split = crate::tools::MCP_TOOL_NAME_SPLIT;
            assert_eq!(
                fetch,
                format!(
                    "{}{split}web_fetch",
                    chatspeed_contracts::WEB_MCP_SERVER_NAME
                )
            );
            assert_eq!(
                search,
                format!(
                    "{}{split}web_search",
                    chatspeed_contracts::WEB_MCP_SERVER_NAME
                )
            );

            let result = manager
                .tool_call(&fetch, json!({"url": "https://example.com"}))
                .await
                .expect("call the fixed web tool through the MCP path");
            assert!(
                result.to_string().contains("web_fetch"),
                "unexpected result: {result}"
            );

            manager
                .unregister_mcp_server(chatspeed_contracts::WEB_MCP_SERVER_NAME)
                .await
                .expect("unregister");
            assert!(manager.resolve_mcp_tool_name("web_fetch").await.is_none());
            task.abort();
        }
    }

    /// Focused owned-MCP-teardown tests (U-5 / AC-7). They use either the crate's
    /// deterministic, network-free stdio fixture or a mock canonical client, so
    /// no external service is involved.
    #[cfg(not(feature = "desktop"))]
    mod owned_mcp_shutdown {
        use super::*;
        use crate::mcp::client::{McpClientInternal, McpClientResult, StatusChangeCallback};
        use rmcp::model::InitializeRequestParams;
        use rmcp::service::RunningService;
        use rmcp::RoleClient;
        use std::path::PathBuf;
        use std::sync::Arc;
        use std::time::Duration;
        use tokio::sync::RwLock;

        fn declaration(name: &str) -> MCPToolDeclaration {
            MCPToolDeclaration {
                name: name.to_string(),
                description: format!("owned-teardown fixture tool {name}"),
                input_schema: json!({ "type": "object" }),
                output_schema: None,
                disabled: false,
                scope: Some(ToolScope::Both),
            }
        }

        async fn register_started_client(
            manager: &Arc<ToolManager>,
            server_name: &str,
            client: Arc<dyn McpClient>,
        ) {
            manager
                .mcp_servers
                .write()
                .await
                .insert(server_name.to_string(), client);
            manager
                .mcp_tools
                .write()
                .await
                .insert(server_name.to_string(), vec![declaration("echo")]);
            manager.rebuild_mcp_wrappers().await;
        }

        /// A real stdio server started from the crate's deterministic,
        /// network-free fixture. The canonical bounded stop must report a
        /// confirmed transport stop and release the running service, and a second
        /// pass must be a proved no-op (idempotent).
        #[cfg(unix)]
        #[tokio::test]
        async fn runtime_shutdown_owned_mcp_stop_is_bounded_confirmed_and_idempotent() {
            let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .find(|dir| dir.file_name() == Some(std::ffi::OsStr::new("src-tauri")))
                .expect("the backend crate lives under src-tauri")
                .join("fixtures/mcp_stdio.py");
            let client = crate::mcp::client::StdioClient::new(McpServerConfig {
                name: "owned-fixture".into(),
                protocol_type: McpProtocolType::Stdio,
                command: Some("python3".into()),
                args: Some(vec!["-u".into(), fixture.to_string_lossy().into_owned()]),
                timeout: Some(10),
                ..Default::default()
            })
            .expect("fixture config");
            client
                .start()
                .await
                .expect("start the owned fixture server");
            let client: Arc<dyn McpClient> = Arc::new(client);

            let manager = Arc::new(ToolManager::new());
            register_started_client(&manager, "owned-fixture", client.clone()).await;
            assert!(!manager.mcp_servers.read().await.is_empty());

            let report = manager.shutdown_all_mcp_servers().await;
            assert_eq!(
                report,
                McpShutdownReport {
                    servers: 1,
                    stopped: 1,
                    unproven: 0
                }
            );
            assert!(
                manager.mcp_servers.read().await.is_empty(),
                "the taken server handle must not be restored"
            );
            assert!(
                manager.mcp_tools.read().await.is_empty(),
                "the MCP tool cache must be cleared"
            );
            assert_eq!(
                client.status().await,
                McpStatus::Stopped,
                "the canonical bounded stop must mark the client stopped"
            );
            assert!(
                client.client().read().await.is_none(),
                "the running service handle must be released"
            );

            // A second pass finds no live server: it is a proved, idempotent no-op.
            let second = manager.shutdown_all_mcp_servers().await;
            assert_eq!(second, McpShutdownReport::default());
        }

        /// An unproven teardown must be reported as unproven, never hidden behind
        /// a clean count.
        #[tokio::test]
        async fn runtime_shutdown_owned_mcp_unproven_stop_is_reported_not_hidden() {
            let manager = Arc::new(ToolManager::new());
            let client: Arc<dyn McpClient> = Arc::new(MockStopClient {
                outcome: McpStopOutcome::Unproven(McpError::ClientStopError(
                    "mock cancel did not complete".into(),
                )),
            });
            register_started_client(&manager, "mock-unproven", client).await;

            let report = manager.shutdown_all_mcp_servers().await;
            assert_eq!(report.servers, 1);
            assert_eq!(report.stopped, 0);
            assert_eq!(
                report.unproven, 1,
                "an unproven teardown must be counted, not silently dropped"
            );

            // Still idempotent: the registry is empty afterwards.
            let second = manager.shutdown_all_mcp_servers().await;
            assert_eq!(second, McpShutdownReport::default());
        }

        /// A mock canonical client whose bounded stop reports a fixed outcome, so
        /// the shutdown aggregation is exercised without an external service.
        struct MockStopClient {
            outcome: McpStopOutcome,
        }

        #[async_trait]
        impl McpClientInternal for MockStopClient {
            async fn set_status(&self, _status: McpStatus) {}
            async fn notify_status_change(&self, _name: String, _status: McpStatus) {}
        }

        #[async_trait]
        impl McpClient for MockStopClient {
            async fn name(&self) -> String {
                "mock-stop-client".to_string()
            }

            async fn config(&self) -> McpServerConfig {
                McpServerConfig {
                    name: "mock-stop-client".to_string(),
                    ..Default::default()
                }
            }

            async fn update_disabled_tools(
                &self,
                _tool_name: &str,
                _is_disabled: bool,
            ) -> McpClientResult<()> {
                Ok(())
            }

            fn client(
                &self,
            ) -> Arc<RwLock<Option<RunningService<RoleClient, InitializeRequestParams>>>>
            {
                Arc::new(RwLock::new(None))
            }

            async fn status(&self) -> McpStatus {
                McpStatus::Stopped
            }

            async fn on_status_change(&self, _callback: StatusChangeCallback) {}

            async fn perform_connect(
                &self,
            ) -> McpClientResult<RunningService<RoleClient, InitializeRequestParams>> {
                Err(McpError::General("mock client cannot connect".into()))
            }

            async fn stop_bounded(&self, _budget: Duration) -> McpStopOutcome {
                self.outcome.clone()
            }
        }
    }

    /// Pins the boundary of the stop-gap tool surface: [`ToolManager::register_core_tools`]
    /// registers no subprocess-owning tool on the *global* surface. The runtime's
    /// owned shell children are created per workflow by the engine's
    /// [`crate::tools::ShellExecute`] and register on the one runtime-wide
    /// owned-process signal, so this asserts the global surface stays lean rather
    /// than claiming a missing tool-child owner.
    #[cfg(not(feature = "desktop"))]
    mod owned_tool_surface {
        use super::*;

        #[tokio::test]
        async fn runtime_core_tool_surface_registers_no_shell_tool() {
            let dir = tempfile::tempdir().expect("temp dir");
            let store =
                Arc::new(MainStore::new(dir.path().join("tool_surface.db")).expect("store"));
            let manager = Arc::new(ToolManager::new());
            manager
                .clone()
                .register_core_tools(store)
                .await
                .expect("register core tools");

            // The file/search surface is present ...
            assert!(manager.has_tool(crate::tools::TOOL_READ_FILE).await);
            // ... and the shell tool is created per workflow by the engine, not
            // registered here on the shared core surface.
            assert!(!manager.has_tool(crate::tools::TOOL_BASH).await);
        }
    }
}

/// A default implementation of `FunctionManager`.
#[cfg(not(feature = "desktop"))]
impl Default for ToolManager {
    fn default() -> Self {
        Self::new()
    }
}
