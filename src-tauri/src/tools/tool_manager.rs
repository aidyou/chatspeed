use async_trait::async_trait;
use rust_i18n::t;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tokio::sync::{broadcast, RwLock};

// use crate::commands::chat::setup_chat_proxy;
// use crate::constants::CFG_CHP_SERVER;
use crate::ai::traits::chat::MCPToolDeclaration;
use crate::constants::CFG_SEARCH_ENGINE;
use crate::db::MainStore;
use crate::mcp::client::{
    McpClient, McpProtocolType, McpServerConfig, McpStatus, SseClient, StdioClient,
    StreamableHttpClient,
};
use crate::tools::error::ToolError;
use crate::tools::{ToolCallResult, ToolCategory};

// use super::tools::SearchDedup;
// use super::tools::{ChatCompletion, ModelName};

pub const MCP_TOOL_NAME_SPLIT: &str = "__MCP__";
const DEFAULT_BROADCAST_CAPACITY: usize = 100;

/// The result type of a function call.
pub type NativeToolResult = Result<ToolCallResult, ToolError>;
pub type ToolResult = Result<Value, ToolError>;

/// A trait defining the characteristics of a function.
#[async_trait]
pub trait ToolDefinition: Send + Sync {
    /// Gets the name of the function.
    fn name(&self) -> &str;

    /// Gets the description of the function.
    fn description(&self) -> &str;

    fn category(&self) -> ToolCategory;

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

/// Manages the registration and execution of workflow functions.
///
/// This struct is responsible for maintaining a collection of functions
pub struct ToolManager {
    /// A map of registered functions.
    tools: RwLock<HashMap<String, Arc<dyn ToolDefinition>>>,
    /// A map of registered MCP servers.
    mcp_servers: RwLock<HashMap<String, Arc<dyn McpClient>>>,
    /// A map of registered MCP tools. The key is the server name, and the value is a vector of ToolDeclaration.
    mcp_tools: RwLock<HashMap<String, Vec<MCPToolDeclaration>>>,
    /// A channel for sending MCP status events.
    mcp_status_event_sender: broadcast::Sender<(String, McpStatus)>,
    /// A set to track MCP server IDs with ongoing operations (start, stop, restart, refresh).
    /// This is used to prevent race conditions from rapid UI clicks.
    pub ops_in_progress: tokio::sync::Mutex<HashSet<i64>>,
}

impl ToolManager {
    /// Creates a new instance of `FunctionManager`.
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(DEFAULT_BROADCAST_CAPACITY);
        Self {
            tools: RwLock::new(HashMap::new()),
            mcp_servers: RwLock::new(HashMap::new()),
            mcp_tools: RwLock::new(HashMap::new()),
            mcp_status_event_sender: sender,
            ops_in_progress: tokio::sync::Mutex::new(HashSet::new()),
        }
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
        let main_store = app_handle
            .state::<Arc<std::sync::RwLock<MainStore>>>()
            .inner();

        // =================================================
        // Built-in tools
        // =================================================

        // Register search tool
        let search_engine = main_store
            .read()
            .map_err(|e| {
                ToolError::Store(
                    t!("db.failed_to_lock_main_store", error = e.to_string()).to_string(),
                )
            })?
            .get_config(CFG_SEARCH_ENGINE, "bing".to_string());
        if !search_engine.is_empty() {
            let ws = crate::tools::WebSearch::new(app_handle.clone());
            self.register_tool(ws).await?;
        }

        // Register web fetch tool
        self.register_tool(Arc::new(crate::tools::WebFetch::new(app_handle.clone())))
            .await?;

        // =================================================
        // Chat completion
        // =================================================
        // register chat completion tool
        // let chat_completion = ChatCompletion::new();
        // // add reasoning and general models to chat completion tool
        // let reasoning_model = Self::get_model(&main_store, ModelName::Reasoning.as_ref())?;
        // chat_completion
        //     .add_model(ModelName::Reasoning, reasoning_model)
        //     .await;
        // let general_model = Self::get_model(&main_store, ModelName::General.as_ref())?;
        // chat_completion
        //     .add_model(ModelName::General, general_model)
        //     .await;
        // self.register_tool(Arc::new(chat_completion)).await?;

        // let chp_server = if let Ok(store) = &main_store.lock() {
        //     store.get_config(CFG_CHP_SERVER, String::new())
        // } else {
        //     String::new()
        // };

        // =================================================
        // ReAct
        // =================================================
        // if ChatSpeedBot server is available, register search and fetch tools
        // if !chp_server.is_empty() {
        //     // register search dedup tool
        //     self.register_tool(Arc::new(SearchDedup::new())).await?;

        //     // register search tool
        //     self.register_tool(Arc::new(Search::new(chp_server.clone())))
        //         .await?;

        //     // register fetch tool
        //     self.register_tool(Arc::new(Crawler::new(chp_server.clone())))
        //         .await?;

        //     // register plot tool
        //     self.register_tool(Arc::new(Plot::new(chp_server.clone())))
        //         .await?;
        // }

        // =================================================
        // MCP Tools
        // =================================================
        // Collect MCP configurations first to release the lock on main_store
        let mcp_configs_to_process: Vec<_> = {
            let main_store_guard = main_store.read().map_err(|e| {
                ToolError::Store(t!("db.failed_to_lock_main_store", error = e).to_string())
            })?;
            main_store_guard
                .config
                .get_mcps()
                .into_iter()
                .filter(|mcp_db_config| !mcp_db_config.disabled)
                .map(|mcp_db_config| mcp_db_config.config.clone()) // Clone the config
                .collect()
        };

        for mcp_server_config in mcp_configs_to_process {
            // Clone self (Arc<FunctionManager>) for the call
            if let Err(e) = self
                .clone()
                .register_mcp_server(mcp_server_config.clone())
                .await
            {
                log::error!(
                    "Failed to register MCP server: {:?}. Error: {}",
                    mcp_server_config,
                    e
                );
            }
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
        let name = tool.name().to_string();
        let mut tools = self.tools.write().await;

        if tools.contains_key(&name) {
            return Err(ToolError::FunctionAlreadyExists(name));
        }

        tools.insert(name, tool);
        Ok(())
    }

    /// Gets a tool by its name.
    ///
    /// # Arguments
    /// * `name` - The name of the tool to get.
    ///
    /// # Returns
    /// * `Result<Arc<dyn FunctionDefinition>, ToolError>` - The result of the function retrieval.
    pub async fn get_tool(&self, name: &str) -> Result<Arc<dyn ToolDefinition>, ToolError> {
        let tools = self.tools.read().await;

        tools
            .get(name)
            .cloned()
            .ok_or_else(|| ToolError::FunctionNotFound(name.to_string()))
    }

    pub async fn get_native_tools(&self) -> Vec<Arc<dyn ToolDefinition>> {
        self.tools.read().await.values().cloned().collect()
    }

    pub async fn get_all_mcp_tools(&self) -> HashMap<String, Vec<MCPToolDeclaration>> {
        self.mcp_tools.read().await.clone()
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
        tool.call(params).await
    }

    /// Call a native tool or mcp tool by its name.
    /// if the tool name contains <MCP_TOOL_NAME_SPLIT>, it is a mcp tool, will call mcp_tool_call,
    /// otherwise, call native_tool_call.
    ///
    /// # Arguments
    /// * `name` - The name of the function to execute.
    /// * `params` - The parameters to pass to the function.
    ///
    /// # Returns
    /// * `ToolResult` - The result of the function execution.
    pub async fn tool_call(&self, name: &str, params: Value) -> ToolResult {
        if name.contains(MCP_TOOL_NAME_SPLIT) {
            self.call_mcp_tool_by_combined_name(name, params).await
        } else {
            self.native_tool_call(name, params).await.map(|v| v.into())
        }
    }

    /// Gets the names of all registered tools.
    ///
    /// # Returns
    /// * `Vec<String>` - A vector of function names.
    async fn get_registered_tools(&self) -> Vec<String> {
        let mut names = Vec::new();
        {
            let tools = self.tools.read().await;
            names.extend(tools.keys().cloned());
        }
        names
    }

    /// Get the calling spec of all registered tools, it's contained all native tools and MCP tools.
    ///
    /// # Returns
    /// * `Result<String, ToolError>` - The calling spec of all registered tools
    pub async fn get_tool_calling_spec(
        &self,
        exclude: Option<HashSet<String>>,
    ) -> Result<Vec<MCPToolDeclaration>, ToolError> {
        let mut specs = Vec::new();
        let excluded: HashSet<String> = exclude.unwrap_or_default();
        for tool_names in self.get_registered_tools().await {
            if let Ok(tool) = self.get_tool(&tool_names).await {
                if !excluded.contains(tool.name()) {
                    specs.push(tool.tool_calling_spec());
                }
            }
        }

        // Collect MCP tool specs *after* getting core function specs
        // This involves reading mcp_tools, which is fine after releasing the tools lock
        // The MCP tool will be remove after the MCP server is disabled. So we don't need to filter it here.
        let mcp_tool_specs: Vec<MCPToolDeclaration> = {
            let mcp_tools_guard = self.mcp_tools.read().await;
            mcp_tools_guard
                .iter()
                .flat_map(|(server_name, tools_vec)| {
                    tools_vec
                        .iter()
                        .filter(|td| !td.disabled)
                        .map(move |original_td| {
                            let mut new_td = original_td.clone(); // Clone the original MCPToolDeclaration
                            new_td.name = format!(
                                "{}{}{}",
                                &server_name, MCP_TOOL_NAME_SPLIT, original_td.name
                            ); // Modify the name of the clone
                            new_td // Return the modified clone
                        })
                })
                .collect()
        };

        specs.extend(mcp_tool_specs);

        Ok(specs)
    }

    /// Get an AI model by its type
    ///
    /// Retrieves an AI model by its type from the configuration store.
    ///
    /// # Arguments
    /// * `main_store` - The main store containing the configuration
    /// * `model_type` - The type of the AI model to retrieve
    ///
    /// # Returns
    /// * `Result<AiModel, ToolError>` - The AI model or an error
    // pub fn get_model(
    //     main_store: Arc<std::sync::RwLock<MainStore>>,
    //     model_type: &str,
    // ) -> Result<AiModel, ToolError> {
    //     let model_name = format!("workflow_{}_model", model_type);
    //     // Get the model configured for the workflow
    //     let reasoning_model = main_store
    //         .read()
    //         .map_err(|e| {
    //             ToolError::Store(
    //                 t!("db.failed_to_lock_main_store", error = e.to_string()).to_string(),
    //             )
    //         })?
    //         .get_config(&model_name, Value::Null);
    //     if reasoning_model.is_null() {
    //         return Err(ToolError::Config(
    //             t!("tools.failed_to_get_model", model_type = model_type).to_string(),
    //         ));
    //     }

    //     let model_id = reasoning_model["id"].as_i64().unwrap_or_default();
    //     if model_id < 1 {
    //         return Err(ToolError::Config(
    //             t!(
    //                 "tools.model_type_not_found",
    //                 model_type = model_type,
    //                 id = model_id,
    //                 error = ""
    //             )
    //             .to_string(),
    //         ));
    //     }

    //     // Get model detail by id
    //     let mut ai_model = main_store
    //         .read()
    //         .map_err(|e| {
    //             ToolError::Store(
    //                 t!("db.failed_to_lock_main_store", error = e.to_string()).to_string(),
    //             )
    //         })?
    //         .config
    //         .get_ai_model_by_id(model_id)
    //         .map_err(|e| {
    //             ToolError::Config(
    //                 t!(
    //                     "tools.model_type_not_found",
    //                     model_type = model_type,
    //                     id = model_id,
    //                     error = e
    //                 )
    //                 .to_string(),
    //             )
    //         })
    //         .map(|mut m| {
    //             match reasoning_model["model"].as_str() {
    //                 Some(md) => m.default_model = md.to_string(),
    //                 None => {}
    //             };
    //             m
    //         })?;

    //     setup_chat_proxy(main_store.clone(), &mut ai_model.metadata)
    //         .map_err(|e| ToolError::Initialization(e.to_string()))?;

    //     Ok(ai_model)
    // }

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

        // 1. Create the MCP client (SseClient or StdioClient)
        // This happens without holding FunctionManager's locks
        let client_arc: Arc<dyn McpClient> = match mcp_server_config.protocol_type {
            McpProtocolType::Sse => {
                SseClient::new(mcp_server_config.clone()) // Clone for the client
                    .map(|c| Arc::new(c) as Arc<dyn McpClient>)
            }
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

        // Adhere to a consistent locking order (mcp_tools -> mcp_servers) to prevent deadlocks.
        let mut mcp_tools_guard = self.mcp_tools.write().await;
        let mut servers_guard = self.mcp_servers.write().await;

        if servers_guard.contains_key(&name) {
            // This case might happen if registration is triggered multiple times quickly
            // before the first one completes its async tool listing.
            // Depending on desired behavior, could log a warning and update, or return error.
            // For now, let's log and allow update (though client Arc should be the same).
            log::warn!("MCP server {} is already in mcp_servers map during inner registration. Overwriting.", &name);
        }
        servers_guard.insert(name.clone(), client.clone()); // client.clone() is cheap (Arc clone)

        #[cfg(debug_assertions)]
        {
            log::debug!("MCP server {} added/updated in mcp_servers.", &name);
        }

        // Register MCP tools if available
        if let Some(declarations) = tools_declarations {
            #[cfg(debug_assertions)]
            {
                let tool_count = declarations.len();
                log::debug!(
                    "MCP tools for server {} registered (count: {}).",
                    name,
                    tool_count
                );
            }
            // Always insert if Some, even if 'declarations' is an empty Vec.
            // This allows the frontend to distinguish between "server not found in mcp_tools"
            // and "server found, but has no tools".
            mcp_tools_guard.insert(name.clone(), declarations);
        } else {
            // tools_declarations was None, e.g. if list_tools failed.
            #[cfg(debug_assertions)]
            {
                log::debug!("No tool declarations (Option was None) provided for MCP server {}. Not updating mcp_tools.", name);
            }
        }

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
            // Remove MCP tools from internal state first
            let mut mcp_tools = self.mcp_tools.write().await;
            mcp_tools.remove(name);
        } // mcp_tools write lock is released here

        // Get the server Arc to stop it *after* removing it from the map
        // and releasing the FunctionManager's mcp_servers lock.
        let server_to_stop = {
            // Acquire write lock for mcp_servers
            let mut servers_guard = self.mcp_servers.write().await;
            servers_guard.remove(name) // This removes and returns the Arc<dyn McpClient>
        }; // servers_guard (write lock on mcp_servers) is released here

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

                let mut mcp_tools_guard = self.mcp_tools.write().await;
                mcp_tools_guard.insert(name.to_string(), tools_with_disabled_flag);

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
                let mut mcp_tools_guard = self.mcp_tools.write().await;
                mcp_tools_guard.insert(name.to_string(), Vec::new());

                // On failure, notify "Error" status.
                self.mcp_status_event_sender
                    .send((name.to_string(), McpStatus::Error(e.to_string())))
                    .ok();
                Err(ToolError::Execution(e.to_string()))
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

        // Phase 2: Update the in-memory cache for mcp_tools.
        let mut mcp_tools_guard = self.mcp_tools.write().await;
        if let Some(tools) = mcp_tools_guard.get_mut(mcp_server_name) {
            // find the tool in the list by name
            if let Some(tool_decl) = tools.iter_mut().find(|td| td.name == mcp_tool_name) {
                // set the disabled flag
                tool_decl.disabled = is_disabled;
                log::debug!(
                    "In-memory mcp_tools cache updated: tool {} on server {} set to disabled={}",
                    mcp_tool_name,
                    mcp_server_name,
                    is_disabled
                );
                Ok(())
            } else {
                // tool not found in the list
                Err(ToolError::FunctionNotFound(format!(
                    "Tool {} not found on server {} in mcp_tools cache.",
                    mcp_tool_name, mcp_server_name
                )))
            }
        } else {
            // server not found in the cache
            Err(ToolError::McpServerNotFound(format!(
                "Server {} not found in mcp_tools cache.",
                mcp_server_name
            )))
        }
    }

    pub async fn mcp_tool_call(
        &self,
        mcp_name: &str,
        tool_name: &str,
        params: Value,
    ) -> ToolResult {
        let client_arc = self.get_mcp_server(mcp_name).await?;
        client_arc
            .call(tool_name, params.clone())
            .await
            .map_err(|e| {
                ToolError::Execution(
                    t!(
                        "mcp.client.failed_to_call_tool",
                        server_name = mcp_name,
                        tool_name = tool_name,
                        args = params.to_string(),
                        error = e
                    )
                    .to_string(),
                )
            })
    }

    /// Calls a tool on a specified MCP (Model Control Protocol) server.
    ///
    /// This function is designed to locate and execute an MCP tool using a composite name string.
    /// This mechanism simplifies calling MCP tools from external systems, such as Chat Completion.
    ///
    /// # Arguments
    /// * `name_with_split` - A specially formatted string to uniquely identify an MCP server and a tool on it.
    ///   The format is `mcp_server_name<MCP_TOOL_NAME_SPLIT>tool_name`.
    ///   For example: "my_mcp_server__MCP_SPLIT__specific_tool".
    ///   - `mcp_server_name` is the name of a registered MCP server.
    ///   - `<MCP_TOOL_NAME_SPLIT>` is the delimiter defined by the `MCP_TOOL_NAME_SPLIT` constant.
    ///   - `specific_tool` is the name of the tool on the target MCP server.
    /// * `params` - The parameters to pass to the target tool, as a `serde_json::Value`.
    ///
    /// # Returns
    /// * `ToolResult` - `Ok(Value)` containing the result of the tool execution on success.
    ///
    /// # Errors
    /// * `ToolError::Execution`:
    ///   - If `name_with_split` is improperly formatted (e.g., cannot be split correctly, or parts are empty),
    ///     an error with a message like "mcp.client.invalid_tool_name" is returned.
    ///   - If the underlying `McpClient::call` method fails (e.g., network issues, tool execution error on the server),
    ///     an error with a message like "mcp.client.failed_to_call_tool" is returned. This error message
    ///     will include the server name, tool name, parameters, and the underlying error details.
    /// * `ToolError::FunctionNotFound` - If the specified `mcp_server_name` cannot be found.
    async fn call_mcp_tool_by_combined_name(
        &self,
        name_with_split: &str,
        params: Value,
    ) -> ToolResult {
        // Unlike regular functions that are directly looked up and invoked by name,
        // MCP (Model Control Protocol) tool calls require a two-step process:
        // 1. Locate the MCP server within the FunctionManager.
        // 2. Invoke the specific tool on that server.
        // To simplify invoking MCP tools, especially from external systems like Chat Completion,
        // a composite name format is used: `mcp_server_name<MCP_TOOL_NAME_SPLIT>tool_name`.
        // The `<MCP_TOOL_NAME_SPLIT>` is a defined constant (MCP_TOOL_NAME_SPLIT).
        // This allows callers to use a single string, e.g., "cstoolbox__MCP_SPLIT__web_search",
        // to specify both the MCP server ("cstoolbox") and the tool ("web_search").
        // This function parses this composite name and dispatches the call accordingly.
        let (mcp_server_name, tool_name) = name_with_split
            .split_once(MCP_TOOL_NAME_SPLIT)
            .filter(|(server_part, tool_part)| !server_part.is_empty() && !tool_part.is_empty())
            .ok_or_else(|| {
                ToolError::Execution(
                    t!("mcp.client.invalid_tool_name", name = name_with_split).to_string(),
                )
            })?;

        self.mcp_tool_call(mcp_server_name, tool_name, params).await
    }

    /// Subscribes to MCP status events.
    ///
    /// This function returns a broadcast receiver that can be used to listen for status events.
    pub fn subscribe_mcp_status_events(&self) -> broadcast::Receiver<(String, McpStatus)> {
        self.mcp_status_event_sender.subscribe()
    }
}

/// A default implementation of `FunctionManager`.
impl Default for ToolManager {
    fn default() -> Self {
        Self::new()
    }
}
