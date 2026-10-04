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
    McpClient, McpProtocolType, McpServerConfig, McpStatus, StdioClient, StreamableHttpClient,
};
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
        crate::tools::TOOL_WEB_SEARCH,
        crate::tools::TOOL_WEB_FETCH,
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
                ToolError::ExecutionFailed(format!(
                    "MCP call to server '{}' tool '{}' failed: {}",
                    self.server_name, self.tool_decl.name, e
                ))
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

#[cfg(not(feature = "desktop"))]
#[derive(Clone)]
pub struct McpToolSpec {
    pub canonical_name: String,
    pub declaration: MCPToolDeclaration,
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
            let _generations = self.mcp_registration_generations.lock().await;
            self.mcp_tools.write().await.clear();
            self.mcp_servers.write().await.clear();
            self.notify_mcp_tools_changed();
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
        .map_err(|error| ToolError::Initialization(error.to_string()))?;
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
            if tool.category() == ToolCategory::Mcp && tool.tool_calling_spec().disabled {
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
                Err(ToolError::ExecutionFailed(format!(
                    "Tool '{}' panicked: {}",
                    name, panic_message
                )))
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

    // =================================================
    // MCP tools
    // =================================================

    pub async fn start_mcp_server(
        self: Arc<Self>, // Changed to take Arc<Self>
        config: McpServerConfig,
    ) -> Result<(), ToolError> {
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

    /// Registers a new MCP (Message Communication Protocol) server with the given configuration.
    /// This involves creating the appropriate client, starting it, and then spawning a task
    /// to list its tools and register them internally.
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

        // Invalidate any older attempt for the same server before starting this one.
        // A detached tool-list task from that attempt may still complete later.
        let registration_generation = {
            let mut generations = self.mcp_registration_generations.lock().await;
            let generation = generations.entry(server_name_for_log.clone()).or_insert(0);
            *generation = generation.saturating_add(1);
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
        let client_arc: Arc<dyn McpClient> = match mcp_server_config.protocol_type {
            McpProtocolType::Sse => Err(crate::mcp::McpError::ClientConfigError(
                t!("mcp.config.sse_removed_in_rmcp_v1").to_string(),
            )),
            McpProtocolType::Stdio => {
                StdioClient::new(mcp_server_config.clone()) // Clone for the client
                    .map(|c| Arc::new(c) as Arc<dyn McpClient>)
            }
            McpProtocolType::StreamableHttp => match provider_max_retries {
                // The dedicated desktop provider is reached once: a dead or
                // released provider must fail closed, never reconnect.
                Some(max_retries) => StreamableHttpClient::with_retry(
                    mcp_server_config.clone(),
                    max_retries,
                    std::time::Duration::from_millis(250),
                )
                .map(|c| Arc::new(c) as Arc<dyn McpClient>),
                None => StreamableHttpClient::new(mcp_server_config.clone()) // Clone for the client
                    .map(|c| Arc::new(c) as Arc<dyn McpClient>),
            },
        }
        .map_err(|e_mcp| {
            ToolError::Config(
                t!(
                    "mcp.client.config_error_for_server",
                    server_name = &server_name_for_log, // Use cloned name
                    error = e_mcp.to_string()
                )
                .to_string(),
            )
        })?;

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
        client_arc
            .start()
            .await
            .map_err(|e_mcp_start| ToolError::Initialization(e_mcp_start.to_string()))?;
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
            let _ = client_arc.stop().await;
            return Err(error);
        }

        // 4. Each MCP finishes independently: list its tools and register the
        // completed snapshot as soon as this server is ready. The generation guard
        // prevents a late result from a stopped/restarted server from being applied.
        let tool_manager_arc = self.clone();
        let client_arc_for_task = client_arc.clone();
        let server_name_for_task = name.clone();
        let config_for_task = client_arc.config().await.clone();

        tokio::spawn(async move {
            let status = client_arc_for_task.status().await;
            if status != McpStatus::Connected && status != McpStatus::Running {
                log::warn!(
                    "MCP server {} is not running (status: {:?}) after start attempt. Skipping tool listing.",
                    server_name_for_task,
                    status
                );
                return;
            }

            let tools_result = client_arc_for_task.list_tools().await;
            let declarations = match tools_result {
                Ok(tools) => {
                    let disabled_tool_names = config_for_task.disabled_tools.unwrap_or_default();
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
                    log::error!(
                        "Failed to list tools for MCP server {}: {}",
                        server_name_for_task,
                        error
                    );
                    None
                }
            };

            if let Err(error) = tool_manager_arc
                .register_mcp_server_inner(
                    client_arc_for_task,
                    registration_generation,
                    declarations,
                )
                .await
            {
                log::debug!(
                    "Discarded MCP server {} discovery result for generation {}: {}",
                    server_name_for_task,
                    registration_generation,
                    error
                );
            } else {
                log::info!(
                    "MCP server {} tools registered for generation {}",
                    server_name_for_task,
                    registration_generation
                );
            }
        });

        // 5. Return after this server is connected and its independent discovery task is queued.
        Ok(())
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

        let client = match self.get_mcp_server(name).await {
            Ok(client) => client,
            Err(e) => {
                self.mcp_status_event_sender
                    .send((name.to_string(), McpStatus::Error(e.to_string())))
                    .ok();
                return Err(e);
            }
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

                {
                    let mut mcp_tools_guard = self.mcp_tools.write().await;
                    mcp_tools_guard.insert(name.to_string(), tools_with_disabled_flag);
                }
                self.rebuild_mcp_wrappers().await;
                self.notify_mcp_tools_changed();

                // On success, notify that it's "Running" again.
                self.mcp_status_event_sender
                    .send((name.to_string(), McpStatus::Running))
                    .ok();
                log::info!("Successfully refreshed tools for MCP server: {}", name);
                Ok(())
            }
            Err(e) => {
                log::error!(
                    "Failed to list tools for MCP server {} during refresh: {}",
                    name,
                    e.to_string()
                );
                {
                    let mut mcp_tools_guard = self.mcp_tools.write().await;
                    mcp_tools_guard.insert(name.to_string(), Vec::new());
                }
                self.rebuild_mcp_wrappers().await;
                self.notify_mcp_tools_changed();

                // On failure, notify "Error" status.
                self.mcp_status_event_sender
                    .send((name.to_string(), McpStatus::Error(e.to_string())))
                    .ok();
                Err(ToolError::ExecutionFailed(e.to_string()))
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
        let mut status = HashMap::new();

        // Step 1: Collect the Arcs of the McpClient while holding the read lock.
        // This minimizes the time the lock is held.
        let server_arcs_to_check: Vec<(String, Arc<dyn McpClient>)> = {
            let servers_guard = self.mcp_servers.read().await;
            servers_guard
                .iter()
                .map(|(name, server_arc)| (name.clone(), server_arc.clone()))
                .collect()
        };

        // Step 2: Iterate over the collected Arcs and await their status.
        // These .await calls now happen *without* holding the self.mcp_servers lock.
        for (name, server_arc) in server_arcs_to_check {
            status.insert(name.clone(), server_arc.status().await);
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

#[cfg(all(test, not(feature = "desktop")))]
mod tests {
    use super::*;
    use crate::tools::{ToolCallResult, ToolCategory, ToolScope};
    use serde_json::json;

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
                format!("{}{split}web_fetch", chatspeed_contracts::WEB_MCP_SERVER_NAME)
            );
            assert_eq!(
                search,
                format!("{}{split}web_search", chatspeed_contracts::WEB_MCP_SERVER_NAME)
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
}

/// A default implementation of `FunctionManager`.
#[cfg(not(feature = "desktop"))]
impl Default for ToolManager {
    fn default() -> Self {
        Self::new()
    }
}
