use async_trait::async_trait;
use futures::FutureExt;
use rust_i18n::t;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tokio::sync::{broadcast, RwLock};

use crate::ai::traits::chat::MCPToolDeclaration;
use crate::constants::CFG_SEARCH_ENGINE;
use crate::db::MainStore;
use crate::mcp::client::{
    McpClient, McpProtocolType, McpServerConfig, McpStatus, StdioClient, StreamableHttpClient,
};
use crate::tools::error::ToolError;
use crate::tools::{ToolCallResult, ToolCategory, ToolScope, MCP_TOOL_NAME_SPLIT};

// use super::tools::SearchDedup;
// use super::tools::{ChatCompletion, ModelName};

const DEFAULT_BROADCAST_CAPACITY: usize = 100;

/// The result type of a function call.
pub type NativeToolResult = Result<ToolCallResult, ToolError>;
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
pub struct McpToolWrapper {
    pub server_name: String,
    pub tool_decl: MCPToolDeclaration,
    pub client: Arc<dyn McpClient>,
    pub canonical_name: String,
    pub public_name: String,
}

#[derive(Default)]
struct McpAliasRegistry {
    alias_to_canonical: HashMap<String, String>,
    canonical_to_alias: HashMap<String, String>,
}

impl McpAliasRegistry {
    fn resolve(&self, name: &str) -> Option<String> {
        self.alias_to_canonical.get(name).cloned()
    }

    fn alias_for(&self, canonical_name: &str) -> Option<String> {
        self.canonical_to_alias.get(canonical_name).cloned()
    }
}

#[derive(Clone)]
struct McpAliasInput {
    canonical_name: String,
    server_name: String,
    tool_name: String,
}

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
        crate::tools::TOOL_SUB_AGENT_STOP,
        crate::tools::TOOL_TODO_CREATE,
        crate::tools::TOOL_TODO_LIST,
        crate::tools::TOOL_TODO_UPDATE,
        crate::tools::TOOL_TODO_GET,
        crate::tools::TOOL_SKILL,
        crate::tools::TOOL_ASK_USER,
        crate::tools::TOOL_COMPLETE_WORKFLOW,
        crate::tools::TOOL_SUBMIT_RESULT,
        crate::tools::TOOL_SUBMIT_PLAN,
        crate::tools::TOOL_MCP_TOOL_LOAD,
        crate::tools::TOOL_READ_HISTORY_MESSAGE,
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

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
        let server_tool_name = format!("{}_{}", normalize_mcp_alias(&input.server_name), tool_name);
        let alias = if !occupied.contains(&tool_name) {
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

fn scope_allows(tool_scope: ToolScope, scope_filter: Option<ToolScope>) -> bool {
    match scope_filter {
        Some(ToolScope::Chat) => tool_scope == ToolScope::Chat || tool_scope == ToolScope::Both,
        Some(ToolScope::Both) => tool_scope == ToolScope::Both,
        Some(ToolScope::Workflow) | None => true,
    }
}

#[derive(Clone)]
pub struct McpToolSpec {
    pub canonical_name: String,
    pub declaration: MCPToolDeclaration,
}

/// Manages the registration and execution of workflow functions.
///
/// This struct is responsible for maintaining a collection of functions
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
    /// A set to track MCP server IDs with ongoing operations (start, stop, restart, refresh).
    /// This is used to prevent race conditions from rapid UI clicks.
    pub ops_in_progress: tokio::sync::Mutex<HashSet<i64>>,
}

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
            ops_in_progress: tokio::sync::Mutex::new(HashSet::new()),
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
            self.mcp_servers.write().await.clear();
            self.mcp_tools.write().await.clear();
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

    /// Register tools for DAG Workflow
    ///
    /// # Arguments
    /// * `self` - An Arc pointing to the ToolManager instance.
    /// * `chat_state` - The chat state.
    /// * `main_store` - The main store.
    ///
    /// # Returns
    /// * `Result<(), ToolError>` - The result of the registration.
    pub async fn register_available_tools(
        self: Arc<Self>, // Changed to take Arc<Self>
        app_handle: AppHandle,
    ) -> Result<(), ToolError> {
        let main_store = app_handle.state::<Arc<MainStore>>().inner();

        // =================================================
        // Built-in tools
        // =================================================

        // Register search tool
        let search_engine = main_store.get_config(CFG_SEARCH_ENGINE, "bing".to_string());
        if !search_engine.is_empty() {
            let ws = crate::tools::WebSearch::new(app_handle.clone());
            self.register_tool(ws).await?;
        }

        // Register web fetch tool
        self.register_tool(Arc::new(crate::tools::WebFetch::new(app_handle.clone())))
            .await?;

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
        self.register_tool(Arc::new(crate::tools::Glob::default()))
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
        // self.register_tool(Arc::new(crate::tools::TodoGetTool {
        //     session_id: "".into(),
        //     main_store: main_store.clone(),
        // }))
        // .await?;

        // let app_data_dir = app_handle.path().app_data_dir().unwrap_or_default();
        // let scanner = crate::workflow::react::skills::SkillScanner::new(app_data_dir);
        // let skills = scanner.scan().unwrap_or_default();
        // self.register_tool(Arc::new(crate::tools::SkillExecute::new(skills)))
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
        // self.register_tool(Arc::new(crate::workflow::react::orchestrator::TaskStopTool))
        //     .await?;

        // // Interaction tools
        // self.register_tool(Arc::new(crate::tools::AskUser)).await?;
        // self.register_tool(Arc::new(crate::tools::FinishTask))
        //     .await?;

        Ok(())
    }

    pub async fn register_available_mcp_tools(
        self: Arc<Self>, // Changed to take Arc<Self>
        main_store: Arc<MainStore>,
    ) -> Result<(), ToolError> {
        // Collect MCP configurations first to release the lock on main_store
        let mcp_configs_to_process: Vec<_> = {
            main_store
                .config
                .get_mcps()
                .into_iter()
                .filter(|mcp_db_config| !mcp_db_config.disabled)
                .map(|mcp_db_config| mcp_db_config.config.clone()) // Clone the config
                .collect()
        };

        for mcp_server_config in mcp_configs_to_process {
            let tool_manager = self.clone();
            tokio::spawn(async move {
                let server_name = mcp_server_config.name.clone();
                if let Err(e) = tool_manager.register_mcp_server(mcp_server_config).await {
                    log::error!(
                        "Failed to register MCP server '{}' during startup: {}",
                        server_name,
                        e
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

    pub async fn resolve_tool_name(&self, name: &str) -> String {
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
        let tool = self.get_tool(name).await?;
        if tool.category() == ToolCategory::Mcp && tool.tool_calling_spec().disabled {
            return Err(ToolError::Security(format!(
                "MCP tool '{}' is disabled",
                name
            )));
        }
        match AssertUnwindSafe(tool.call(params)).catch_unwind().await {
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
        }
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
        #[cfg(debug_assertions)]
        {
            log::debug!("Register MCP server {} ... ", &mcp_server_config.name,);
            log::debug!("MCP server config: {:?}", &mcp_server_config);
        }

        // Clone for logging in case of early error
        let server_name_for_log = mcp_server_config.name.clone();

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
            McpProtocolType::StreamableHttp => {
                StreamableHttpClient::new(mcp_server_config.clone()) // Clone for the client
                    .map(|c| Arc::new(c) as Arc<dyn McpClient>)
            }
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

        // 3. Spawn a task to wait for status, list tools, and register
        // This allows the main registration flow to return quickly, while the tool discovery
        // and registration happens in the background.
        let tool_manager_arc = self.clone(); // Clone the Arc<Self> for the spawned task
        let client_arc_for_task = client_arc.clone(); // Clone the client Arc for the spawned task
        let server_name_for_task = name.clone(); // Clone name for logging in task
        let config_for_task = client_arc.config().await.clone(); // Clone config for disabled_tools check in task

        tokio::spawn(async move {
            // Check status again within the task. Could add retries/timeout here if needed.
            let status = client_arc_for_task.status().await;
            if status == McpStatus::Connected || status == McpStatus::Running {
                let tools_result = client_arc_for_task.list_tools().await;
                match tools_result {
                    Ok(tools) => {
                        // #[cfg(debug_assertions)]
                        // {
                        //     log::debug!("MCP server {} tools: {:?}", server_name_for_task, tools);
                        // }

                        // Get the set of disabled tool names from the config
                        let disabled_tool_names: HashSet<String> =
                            config_for_task.disabled_tools.unwrap_or_default();

                        // Add a 'disabled' flag to each tool based on the config
                        let tools_with_disabled_flag: Vec<MCPToolDeclaration> = tools
                            .into_iter()
                            .map(|mut tool_decl| {
                                tool_decl.disabled = disabled_tool_names.contains(&tool_decl.name);
                                tool_decl
                            })
                            .collect();

                        // 4. Register the MCP server and tools in internal state
                        // We register the server and its tools (with disabled flags) regardless of
                        // whether all tools are disabled, so the frontend can see them.
                        if let Err(e) = tool_manager_arc
                            .register_mcp_server_inner(
                                client_arc_for_task.clone(),
                                Some(tools_with_disabled_flag), // Pass the list with disabled flags
                            )
                            .await
                        {
                            log::error!(
                                "Failed to register MCP server {} tools internally: {}",
                                server_name_for_task,
                                e
                            );
                        } else {
                            log::info!("MCP server {} tools (with disabled flags) registered successfully.",server_name_for_task );
                        }
                    }
                    Err(e) => {
                        log::error!(
                            "Failed to list tools for MCP server {}: {}",
                            server_name_for_task,
                            e
                        );
                        // If listing tools failed, we still register the server itself (without tools)
                        // so the frontend knows the server exists but couldn't fetch tools.
                        if let Err(e_inner) = tool_manager_arc
                            .register_mcp_server_inner(client_arc_for_task.clone(), None)
                            .await
                        {
                            log::error!(
                                "Failed to register MCP server {} (without tools due to list_tools error) internally: {}",
                                server_name_for_task,
                                e_inner
                            );
                        }
                    }
                }
            } else {
                log::warn!(
                    "MCP server {} is not running (status: {:?}) after start attempt. Skipping tool listing and registration.",
                    server_name_for_task,
                    status
                );
            }
        }); // Task spawned

        // 5. Return immediately, the rest happens in the spawned task
        Ok(())
    }

    /// Registers a new MCP server and its tools with the manager's internal state.
    /// This method should be called *after* the client has been created and started,
    /// and its tools have been fetched. It primarily handles updating the internal HashMaps.
    /// This is called from within the spawned task in `register_mcp_server`.
    ///
    /// # Arguments
    /// * `client` - The Arc to the started McpClient instance.
    /// * `tools_declarations` - An optional vector of tool declarations fetched from the client.
    ///
    /// # Returns
    /// * `Result<(), ToolError>` - The result of the registration.
    async fn register_mcp_server_inner(
        &self, // This function's signature remains &self as it's called by the Arc<Self> holding task
        client: Arc<dyn McpClient>,
        tools_declarations: Option<Vec<MCPToolDeclaration>>,
    ) -> Result<(), ToolError> {
        let name = client.name().await;
        #[cfg(debug_assertions)]
        {
            log::debug!("register_mcp_server_inner: client.name() = {}", &name);
        }

        // Update server/declaration state, then rebuild all MCP wrappers and aliases as one set.
        {
            let mut mcp_tools_guard = self.mcp_tools.write().await;
            let mut servers_guard = self.mcp_servers.write().await;
            servers_guard.insert(name.clone(), client);
            if let Some(declarations) = tools_declarations {
                mcp_tools_guard.insert(name.clone(), declarations);
            }
        }
        self.rebuild_mcp_wrappers().await;
        self.notify_mcp_tools_changed();

        #[cfg(debug_assertions)]
        {
            log::debug!("MCP server {} inner registration process completed.", name);
        }
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
        // Scope the locks to ensure they are released before awaiting .stop()
        {
            let mut mcp_tools = self.mcp_tools.write().await;
            mcp_tools.remove(name);
        }
        let server_to_stop = {
            let mut servers_guard = self.mcp_servers.write().await;
            servers_guard.remove(name)
        };
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
    pub async fn get_mcp_server(&self, name: &str) -> Result<Arc<dyn McpClient>, ToolError> {
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

#[cfg(test)]
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

        let loader = crate::tools::McpToolLoad {
            tool_manager: manager.clone(),
            allowed_tools: Some(HashSet::from(["beta__MCP__search".to_string()])),
        };
        let loaded = loader
            .call(json!({ "tool_name": "search" }))
            .await
            .expect("public MCP alias must resolve through mcp_tool_load");
        let declaration = loaded
            .structured_content
            .expect("mcp_tool_load must return a declaration");
        assert_eq!(declaration["name"], "search");
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
}

/// A default implementation of `FunctionManager`.
impl Default for ToolManager {
    fn default() -> Self {
        Self::new()
    }
}
