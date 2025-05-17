use async_trait::async_trait;
use rust_i18n::t;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::ai::interaction::chat_completion::ChatState;
use crate::ai::traits::chat::MCPToolDeclaration;
use crate::commands::chat::setup_chat_proxy;
use crate::constants::CFG_CHP_SERVER;
use crate::db::{AiModel, MainStore};
use crate::mcp::client::sse::SseClient;
use crate::mcp::client::stdio::StdioClient;
use crate::mcp::client::{McpClient, McpProtocolType, McpServerConfig, McpStatus};
use crate::workflow::error::WorkflowError;

use super::tools::Crawler;
use super::tools::Plot;
use super::tools::Search;
use super::tools::SearchDedup;
use super::tools::{ChatCompletion, ModelName};

pub static MCP_TOOL_NAME_SPLIT: &str = "__M_C_P_S_P__";

/// The result type of a function call.
pub type FunctionResult = Result<Value, WorkflowError>;

/// A trait defining the characteristics of a function.
#[async_trait]
pub trait FunctionDefinition: Send + Sync {
    /// Gets the name of the function.
    fn name(&self) -> &str;

    /// Gets the description of the function.
    fn description(&self) -> &str;

    /// Returns the function calling specification in JSON format.
    ///
    /// This method provides detailed information about the function
    /// in a format compatible with function calling APIs.
    ///
    /// # Returns
    /// * `Value` - The function specification in JSON format.
    fn function_calling_spec(&self) -> Value;

    /// Executes the function.
    ///
    /// # Arguments
    /// * `params` - The parameters to pass to the function.
    ///
    /// # Returns
    /// * `FunctionResult` - The result of the function execution.
    async fn execute(&self, params: Value) -> FunctionResult;
}

/// Manages the registration and execution of workflow functions.
///
/// This struct is responsible for maintaining a collection of functions
/// that can be executed within a workflow.
pub struct FunctionManager {
    /// A map of registered functions.
    functions: RwLock<HashMap<String, Arc<dyn FunctionDefinition>>>,
    /// A map of registered MCP servers.
    mcp_servers: RwLock<HashMap<String, Arc<dyn McpClient>>>,
    /// A map of registered MCP tools. The key is the server name, and the value is a vector of ToolDeclaration.
    mcp_tools: RwLock<HashMap<String, Vec<MCPToolDeclaration>>>,
}

impl FunctionManager {
    /// Creates a new instance of `FunctionManager`.
    pub fn new() -> Self {
        Self {
            functions: RwLock::new(HashMap::new()),
            mcp_servers: RwLock::new(HashMap::new()),
            mcp_tools: RwLock::new(HashMap::new()),
        }
    }

    /// Register functions for DAG Workflow
    ///
    /// # Arguments
    /// * `self` - An Arc pointing to the FunctionManager instance.
    /// * `chat_state` - The chat state.
    /// * `main_store` - The main store.
    ///
    /// # Returns
    /// * `Result<(), WorkflowError>` - The result of the registration.
    pub async fn register_workflow_tools(
        self: Arc<Self>, // Changed to take Arc<Self>
        chat_state: Arc<ChatState>,
        main_store: Arc<std::sync::Mutex<MainStore>>,
    ) -> Result<(), WorkflowError> {
        // =================================================
        // Chat completion
        // =================================================
        // register chat completion tool
        let chat_completion = ChatCompletion::new(chat_state.clone());
        // add reasoning and general models to chat completion tool
        let reasoning_model = Self::get_model(&main_store, ModelName::Reasoning.as_ref())?;
        chat_completion
            .add_model(ModelName::Reasoning, reasoning_model)
            .await;
        let general_model = Self::get_model(&main_store, ModelName::General.as_ref())?;
        chat_completion
            .add_model(ModelName::General, general_model)
            .await;
        self.register_function(Arc::new(chat_completion)).await?;

        let chp_server = if let Ok(store) = &main_store.lock() {
            store.get_config(CFG_CHP_SERVER, String::new())
        } else {
            String::new()
        };

        // =================================================
        // ReAct
        // =================================================
        // if ChatSpeedBot server is available, register search and fetch tools
        if !chp_server.is_empty() {
            // register search dedup tool
            self.register_function(Arc::new(SearchDedup::new())).await?;

            // register search tool
            self.register_function(Arc::new(Search::new(chp_server.clone())))
                .await?;

            // register fetch tool
            self.register_function(Arc::new(Crawler::new(chp_server.clone())))
                .await?;

            // register plot tool
            self.register_function(Arc::new(Plot::new(chp_server.clone())))
                .await?;
        }

        // =================================================
        // MCP Tools
        // =================================================
        // Collect MCP configurations first to release the lock on main_store
        let mcp_configs_to_process: Vec<_> = {
            let main_store_guard = main_store.lock().map_err(|e| {
                WorkflowError::Store(t!("db.failed_to_lock_main_store", error = e).to_string())
            })?;
            main_store_guard
                .config
                .get_mcps()
                .into_iter()
                .filter(|mcp_db_config| !mcp_db_config.disabled)
                .map(|mcp_db_config| mcp_db_config.config.clone()) // Clone the config
                .collect()
        }; // main_store_guard is dropped here

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

    /// Registers a new function with the manager.
    ///
    /// # Arguments
    /// * `function` - The function to register.
    ///
    /// # Returns
    /// * `Result<(), WorkflowError>` - The result of the registration.
    pub async fn register_function(
        &self, // This can remain &self as it doesn't spawn long tasks
        function: Arc<dyn FunctionDefinition>,
    ) -> Result<(), WorkflowError> {
        let name = function.name().to_string();
        let mut functions = self.functions.write().await;

        if functions.contains_key(&name) {
            return Err(WorkflowError::FunctionAlreadyExists(name));
        }

        functions.insert(name, function);
        Ok(())
    }

    /// Gets a function by its name.
    ///
    /// # Arguments
    /// * `name` - The name of the function to get.
    ///
    /// # Returns
    /// * `Result<Arc<dyn FunctionDefinition>, WorkflowError>` - The result of the function retrieval.
    pub async fn get_function(
        &self,
        name: &str,
    ) -> Result<Arc<dyn FunctionDefinition>, WorkflowError> {
        let functions = self.functions.read().await;

        functions
            .get(name)
            .cloned()
            .ok_or_else(|| WorkflowError::FunctionNotFound(name.to_string()))
    }

    /// Executes a function by its name.
    ///
    /// # Arguments
    /// * `name` - The name of the function to execute.
    /// * `params` - The parameters to pass to the function.
    ///
    /// # Returns
    /// * `FunctionResult` - The result of the function execution.
    pub async fn execute_function(&self, name: &str, params: Value) -> FunctionResult {
        let function = self.get_function(name).await?;
        function.execute(params).await
    }

    /// Gets the names of all registered functions.
    ///
    /// # Returns
    /// * `Vec<String>` - A vector of function names.
    async fn get_registered_functions(&self) -> Vec<String> {
        let mut names = Vec::new();
        {
            let functions = self.functions.read().await;
            names.extend(functions.keys().cloned());
        }
        names
    }

    /// Get the calling spec of all registered functions
    ///
    /// # Returns
    /// * `Result<String, WorkflowError>` - The calling spec of all registered functions
    pub async fn get_function_calling_spec(
        &self,
        exclude: Option<HashSet<String>>,
    ) -> Result<String, WorkflowError> {
        let mut specs = Vec::new();
        let excluded: HashSet<String> = exclude.unwrap_or_default();
        for function_name in self.get_registered_functions().await {
            if let Ok(function) = self.get_function(&function_name).await {
                if !excluded.contains(function.name()) {
                    specs.push(function.function_calling_spec());
                }
            }
        }
        // Collect MCP tool specs *after* getting core function specs
        // This involves reading mcp_tools, which is fine after releasing the functions lock
        // The MCP tool will be remove after the MCP server is disabled. So we don't need to filter it here.
        let mcp_tool_specs: Vec<Value> = {
            let mcp_tools_guard = self.mcp_tools.read().await;
            mcp_tools_guard
                .iter()
                .flat_map(|(server_name, tools)| {
                    tools.iter().map(move |tool| {
                        json!({
                            "name": format!("{}{MCP_TOOL_NAME_SPLIT}{}", server_name, tool.name),
                            "description": tool.description,
                            "parameters": tool.get_input_schema()
                        })
                    })
                })
                .collect()
        }; // mcp_tools_guard is released here

        specs.extend(mcp_tool_specs);

        serde_json::to_string(&specs).map_err(|e| {
            WorkflowError::Serialization(
                t!(
                    "workflow.serialization_error_details",
                    details = e.to_string()
                )
                .to_string(),
            )
        })
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
    /// * `Result<AiModel, WorkflowError>` - The AI model or an error
    pub fn get_model(
        main_store: &Arc<std::sync::Mutex<MainStore>>,
        model_type: &str,
    ) -> Result<AiModel, WorkflowError> {
        let model_name = format!("workflow_{}_model", model_type);
        // Get the model configured for the workflow
        let reasoning_model = main_store
            .lock()
            .map_err(|e| {
                WorkflowError::Store(t!("db.failed_to_lock_main_store", error = e).to_string())
            })?
            .get_config(&model_name, Value::Null);
        if reasoning_model.is_null() {
            return Err(WorkflowError::Config(
                t!("workflow.failed_to_get_model", model_type = model_type).to_string(),
            ));
        }

        let model_id = reasoning_model["id"].as_i64().unwrap_or_default();
        if model_id < 1 {
            return Err(WorkflowError::Config(
                t!(
                    "workflow.model_type_not_found",
                    model_type = model_type,
                    id = model_id,
                    error = ""
                )
                .to_string(),
            ));
        }

        // Get model detail by id
        let mut ai_model = main_store
            .lock()
            .map_err(|e| {
                WorkflowError::Store(t!("db.failed_to_lock_main_store", error = e).to_string())
            })?
            .config
            .get_ai_model_by_id(model_id)
            .map_err(|e| {
                WorkflowError::Config(
                    t!(
                        "workflow.model_type_not_found",
                        model_type = model_type,
                        id = model_id,
                        error = e
                    )
                    .to_string(),
                )
            })
            .map(|mut m| {
                match reasoning_model["model"].as_str() {
                    Some(md) => m.default_model = md.to_string(),
                    None => {}
                };
                m
            })?;

        setup_chat_proxy(&main_store, &mut ai_model.metadata)?;

        Ok(ai_model)
    }

    // =================================================
    // MCP tools
    // =================================================

    pub async fn start_mcp_server(
        self: Arc<Self>, // Changed to take Arc<Self>
        config: McpServerConfig,
    ) -> Result<(), WorkflowError> {
        // Check if the server is already registered and running *without* holding the main lock
        // This avoids holding the main lock while potentially waiting for status()
        // Note: self.get_mcp_server doesn't need Arc<Self> if it only reads.
        let is_running = match self.get_mcp_server(config.name.as_str()).await {
            Ok(mcp_server) => mcp_server.status().await == McpStatus::Running,
            Err(_) => false, // Not found, so not running
        };

        if is_running {
            log::info!("MCP server '{}' is already running.", config.name);
            return Ok(());
        }

        // If not running or not found, proceed with registration which includes starting
        // Use self.clone() because register_mcp_server now takes Arc<Self>
        self.clone().register_mcp_server(config).await
    }

    /// Alias for `unregister_mcp_server`
    pub async fn stop_mcp_server(&self, name: &str) -> Result<(), WorkflowError> {
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
    ) -> Result<(), WorkflowError> {
        // Clone for logging in case of early error
        let server_name_for_log = mcp_server_config.name.clone();

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
        }
        .map_err(|e_mcp| {
            WorkflowError::Config(
                t!(
                    "mcp.client.config_error_for_server",
                    server_name = &server_name_for_log, // Use cloned name
                    error = e_mcp.to_string()
                )
                .to_string(),
            )
        })?;

        #[cfg(debug_assertions)]
        {
            log::debug!("MCP server '{}' created successfully.", client_arc.name(),);
        }

        // 2. Start the client
        // This .await happens without holding FunctionManager's locks
        client_arc.start().await.map_err(|e_mcp_start| {
            WorkflowError::Initialization(
                t!(
                    "mcp.client.failed_to_start_server",
                    server_name = client_arc.name(),
                    error = e_mcp_start.to_string()
                )
                .to_string(),
            )
        })?;
        log::info!("MCP client '{}' started successfully.", client_arc.name());

        // 3. Spawn a task to wait for status, list tools, and register
        // This allows the main registration flow to return quickly, while the tool discovery
        // and registration happens in the background.
        let function_manager_arc = self.clone(); // Clone the Arc<Self> for the spawned task
        let client_arc_for_task = client_arc.clone(); // Clone the client Arc for the spawned task
        let server_name_for_task = client_arc.name().to_string(); // Clone name for logging in task
        let config_for_task = client_arc.config().clone(); // Clone config for disabled_tools check

        tokio::spawn(async move {
            // Check status again within the task. Could add retries/timeout here if needed.
            let status = client_arc_for_task.status().await;

            if status == McpStatus::Running {
                let tools_result = client_arc_for_task.list_tools().await;
                match tools_result {
                    Ok(tools) => {
                        #[cfg(debug_assertions)]
                        {
                            log::debug!("MCP server '{}' tools: {:?}", server_name_for_task, tools);
                        }

                        let filtered_tools = match config_for_task.disabled_tools {
                            Some(disabled_tools) => {
                                if disabled_tools.is_empty() {
                                    tools
                                } else {
                                    tools
                                        .into_iter()
                                        .filter(|td| !disabled_tools.contains(&td.name))
                                        .collect()
                                }
                            }
                            None => tools, // No disabled tools specified
                        };

                        // 4. Register the MCP server and tools in internal state
                        // If the MCP has no tools after filtering, we don't need to register it.
                        if !filtered_tools.is_empty() {
                            // Pass the original client_arc_for_task here
                            if let Err(e) = function_manager_arc
                                .register_mcp_server_inner(
                                    client_arc_for_task.clone(),
                                    Some(filtered_tools),
                                )
                                .await
                            {
                                log::error!(
                                    "Failed to register MCP server '{}' tools internally: {}",
                                    server_name_for_task,
                                    e
                                );
                            } else {
                                log::info!(
                                    "MCP server '{}' tools registered successfully.",
                                    server_name_for_task
                                );
                            }
                        } else {
                            log::warn!(
                                "MCP server '{}' reported no tools or all tools are disabled. Skipping internal registration.",
                                server_name_for_task
                            );
                        }
                    }
                    Err(e) => {
                        log::error!(
                            "Failed to list tools for MCP server '{}': {}",
                            server_name_for_task,
                            e
                        );
                        // Do not register if listing tools failed
                    }
                }
            } else {
                log::warn!(
                    "MCP server '{}' is not running (status: {:?}) after start attempt. Skipping tool listing and registration.",
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
    /// * `Result<(), WorkflowError>` - The result of the registration.
    async fn register_mcp_server_inner(
        &self, // This function's signature remains &self as it's called by the Arc<Self> holding task
        client: Arc<dyn McpClient>,
        tools_declarations: Option<Vec<MCPToolDeclaration>>,
    ) -> Result<(), WorkflowError> {
        #[cfg(debug_assertions)]
        {
            log::debug!(
                "register_mcp_server_inner: client.name() = {}",
                client.name(),
            );
        }

        let name = {
            let mut servers = self.mcp_servers.write().await;
            let name = client.name();
            if servers.contains_key(&name) {
                // This case might happen if registration is triggered multiple times quickly
                // before the first one completes its async tool listing.
                // Depending on desired behavior, could log a warning and update, or return error.
                // For now, let's log and allow update (though client Arc should be the same).
                log::warn!("MCP server '{}' is already in mcp_servers map during inner registration. Overwriting.", name);
            }
            servers.insert(name.clone(), client.clone()); // client.clone() is cheap (Arc clone)

            #[cfg(debug_assertions)]
            {
                log::debug!("MCP server '{}' added/updated in mcp_servers.", name);
            }

            name
        };

        // servers write lock is released here

        // Register MCP tools if available
        if let Some(declarations) = tools_declarations {
            if !declarations.is_empty() {
                // Only lock and insert if there are tools
                // Acquire write lock for mcp_tools
                let mut mcp_tools_guard = self.mcp_tools.write().await;
                mcp_tools_guard.insert(name.clone(), declarations);
                #[cfg(debug_assertions)]
                {
                    log::debug!("MCP tools for server '{}' registered.", name);
                }
                // mcp_tools write lock is released here
            } else {
                #[cfg(debug_assertions)]
                {
                    log::debug!("No tools to register for MCP server '{}'.", name);
                }
            }
        }

        #[cfg(debug_assertions)]
        {
            log::debug!(
                "MCP server '{}' inner registration process completed.",
                name
            );
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
    /// * `Result<(), WorkflowError>` - The result of the unregistration.
    pub async fn unregister_mcp_server(&self, name: &str) -> Result<(), WorkflowError> {
        // Remove MCP tools from internal state first
        let mut mcp_tools = self.mcp_tools.write().await;
        mcp_tools.remove(name);
        // mcp_tools write lock is released here

        // Get the server Arc to stop it *after* removing it from the map
        // and releasing the FunctionManager's mcp_servers lock.
        let server_to_stop = {
            // Acquire write lock for mcp_servers
            let mut servers_guard = self.mcp_servers.write().await;
            servers_guard.remove(name) // This removes and returns the Arc<dyn McpClient>
        }; // servers_guard (write lock on mcp_servers) is released here explicitly by scope end

        // Stop the server client if it was found
        if let Some(server_arc) = server_to_stop {
            // Now call stop() on the Arc. FunctionManager's locks are released.
            // This .await happens without holding FunctionManager's locks.
            server_arc.stop().await.map_err(|e| {
                log::error!("Failed to stop MCP server '{}': {}", name, e);
                // Construct the full message here, as StateChangeFailed is now #[error("{0}")]
                // McpClientError's Display impl is already clean.
                WorkflowError::StateChangeFailed(
                    t!(
                        "workflow.mcp_stop_failed_details",
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
                "Attempted to unregister MCP server '{}' but it was not found in the manager.",
                name
            );
        }
        Ok(())
    }

    /// Gets the status of all registered MCP servers.
    ///
    /// # Returns
    /// * `Result<HashMap<String, McpStatus>, WorkflowError>` - The result of the status retrieval.
    ///   The key is the name of the MCP server, and the value is its status.
    ///   If no MCP servers are registered, an empty map is returned.
    ///   If an error occurs during the status retrieval, an error is returned.
    ///   The error message will indicate the specific error that occurred.
    pub async fn get_mcp_serves_status(&self) -> Result<HashMap<String, McpStatus>, WorkflowError> {
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
    /// * `Result<Arc<dyn McpClient>, WorkflowError>` - The result of the server retrieval.
    pub async fn get_mcp_server(&self, name: &str) -> Result<Arc<dyn McpClient>, WorkflowError> {
        let servers_guard: tokio::sync::RwLockReadGuard<
            '_,
            HashMap<String, Arc<dyn McpClient + 'static>>,
        > = self.mcp_servers.read().await;
        servers_guard
            .get(name)
            .cloned()
            .ok_or_else(|| WorkflowError::McpServerNotFound(name.to_string()))
    }

    /// Gets the list of tools declared by a specified MCP server.
    ///
    /// # Arguments
    /// * `name` - The name of the MCP server whose tools are to be retrieved.
    ///
    /// # Returns
    /// * `Result<Vec<MCPToolDeclaration>, WorkflowError>` - The result of the tool retrieval.
    pub async fn get_mcp_server_tools(
        &self,
        name: &str,
    ) -> Result<Vec<MCPToolDeclaration>, WorkflowError> {
        let tools_guard = self.mcp_tools.read().await;
        tools_guard
            .get(name)
            .cloned()
            .ok_or_else(|| WorkflowError::McpServerNotFound(name.to_string()))
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
    /// * `FunctionResult` - `Ok(Value)` containing the result of the tool execution on success.
    ///
    /// # Errors
    /// * `WorkflowError::Execution`:
    ///   - If `name_with_split` is improperly formatted (e.g., cannot be split correctly, or parts are empty),
    ///     an error with a message like "mcp.client.invalid_tool_name" is returned.
    ///   - If the underlying `McpClient::call` method fails (e.g., network issues, tool execution error on the server),
    ///     an error with a message like "mcp.client.failed_to_call_tool" is returned. This error message
    ///     will include the server name, tool name, parameters, and the underlying error details.
    /// * `WorkflowError::FunctionNotFound` - If the specified `mcp_server_name` cannot be found.
    pub async fn mcp_tool_call(&self, name_with_split: &str, params: Value) -> FunctionResult {
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
                WorkflowError::Execution(
                    t!("mcp.client.invalid_tool_name", name = name_with_split).to_string(),
                )
            })?;

        // Get the server Arc *before* calling the tool
        let client_arc = self.get_mcp_server(mcp_server_name).await?; // Renamed to client_arc for clarity

        // Call the tool on the server Arc *without* holding FunctionManager's locks
        client_arc
            .call(tool_name, params.clone())
            .await
            .map_err(|e| {
                WorkflowError::Execution(
                    t!(
                        "mcp.client.failed_to_call_tool",
                        server_name = mcp_server_name,
                        tool_name = tool_name,
                        args = params.to_string(),
                        error = e
                    )
                    .to_string(),
                )
            })
    }
}

/// A default implementation of `FunctionManager`.
impl Default for FunctionManager {
    fn default() -> Self {
        Self::new()
    }
}
