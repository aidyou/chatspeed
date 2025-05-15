use async_trait::async_trait;
use rust_i18n::t;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::ai::interaction::chat_completion::ChatState;
use crate::ai::traits::chat::MCPToolDeclaration;
use crate::commands::chat::setup_chat_proxy;
use crate::constants::CFG_CHP_SERVER;
use crate::db::{AiModel, MainStore};
use crate::mcp::client::sse::SseClient;
use crate::mcp::client::stdio::StdioClient;
use crate::mcp::client::{McpClient, McpProtocolType, McpStatus};
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

    /// Gets the type of the function.
    fn function_type(&self) -> FunctionType;

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

/// An enumeration of function types.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FunctionType {
    /// A native function.
    Native,
    /// An HTTP protocol function.
    Http,
    /// A Chatspeed HTTP protocol function.
    MCP,
}

impl fmt::Display for FunctionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FunctionType::Native => write!(f, "Native"),
            FunctionType::Http => write!(f, "Http"),
            FunctionType::MCP => write!(f, "MCP"),
        }
    }
}

/// Manages the registration and execution of workflow functions.
///
/// This struct is responsible for maintaining a collection of functions
/// that can be executed within a workflow.
pub struct FunctionManager {
    /// A map of registered functions.
    functions: RwLock<HashMap<String, Arc<dyn FunctionDefinition>>>,
    /// A map of registered MCP tools.
    mcp_servers: RwLock<HashMap<String, Arc<dyn McpClient>>>,
    /// A map of registered MCP tools. The key is the tool name, and the value is a vector of ToolDeclaration.
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
    /// * `chat_state` - The chat state
    /// * `main_store` - The main store
    ///
    /// # Returns
    /// * `Result<(), WorkflowError>` - The result of the registration
    pub async fn register_workflow_tools(
        &self,
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
                WorkflowError::Store(
                    t!("workflow.failed_to_lock_main_store", error = e).to_string(),
                )
            })?;
            main_store_guard
                .config
                .get_mcps()
                .into_iter()
                .filter(|mcp_db_config| !mcp_db_config.disable)
                .map(|mcp_db_config| mcp_db_config.config.clone()) // Clone the config
                .collect()
        }; // main_store_guard is dropped here

        for mcp_server_config_from_list in mcp_configs_to_process {
            let mcp_server_config = mcp_server_config_from_list; // Use the cloned config
            let server_name_for_log = mcp_server_config.name.clone(); // Clone for logging in case of early error

            // We'll create an inner async block to handle each MCP server's setup.
            // The result of this block will be logged, but won't stop the outer loop.
            let setup_result =
                async {
                    // 1. Create the MCP client (SseClient or StdioClient)
                    let client_arc: Arc<dyn McpClient> = match mcp_server_config.protocol_type {
                    McpProtocolType::Sse => SseClient::new(mcp_server_config.clone()) // Clone for the client
                        .map(|c| Arc::new(c) as Arc<dyn McpClient>),
                    McpProtocolType::Stdio => StdioClient::new(mcp_server_config.clone()) // Clone for the client
                        .map(|c| Arc::new(c) as Arc<dyn McpClient>),
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

                    // 2. Start the client
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

                    // 3. Register the MCP server
                    self.register_mcp_server(client_arc.clone()).await?;
                    log::info!(
                        "MCP server '{}' registered successfully.",
                        client_arc.name()
                    );

                    Result::<(), WorkflowError>::Ok(()) // Explicit type for Ok
                }
                .await;

            if let Err(e) = setup_result {
                log::error!(
                    "Failed to setup MCP server '{}': {}",
                    server_name_for_log, // Use cloned name for logging
                    e
                );
                // Optionally, you could update the McpClient's status to Error here
                // if the client was created but failed to start or register.
                // However, if client creation itself failed, there's no client object yet.
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
        &self,
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
        for server_name in self.get_registered_mcp_server_names().await {
            self.mcp_tools.read().await.get(&server_name).map(|tools| {
                for tool in tools {
                    if !excluded.contains(&tool.name) {
                        specs.push(json!({
                            "name": format!("{}{MCP_TOOL_NAME_SPLIT}{}", server_name, tool.name),
                            "description": tool.description,
                            "parameters": tool.get_input_schema()}));
                    }
                }
            });
        }
        serde_json::to_string(&specs).map_err(|e| WorkflowError::Serialization(e.to_string()))
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
                WorkflowError::Store(
                    t!("workflow.failed_to_lock_main_store", error = e).to_string(),
                )
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
                WorkflowError::Store(
                    t!("workflow.failed_to_lock_main_store", error = e).to_string(),
                )
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

    /// Registers a new MCP tool with the manager.
    ///
    /// # Arguments
    /// * `tool` - The MCP tool to register.
    ///
    /// # Returns
    /// * `Result<(), WorkflowError>` - The result of the registration.
    pub async fn register_mcp_server(&self, tool: Arc<dyn McpClient>) -> Result<(), WorkflowError> {
        let mut servers = self.mcp_servers.write().await;
        let name = tool.name();
        if servers.contains_key(&name) {
            return Err(WorkflowError::FunctionAlreadyExists(name));
        }
        servers.insert(name.clone(), tool.clone());

        // Register MCP tools
        if tool.status().await == McpStatus::Running {
            if let Ok(tool_declarations) = tool.list_tools().await {
                let tool_declarations = match tool.config().disabled_tools {
                    Some(disabled_tools) => tool_declarations
                        .into_iter()
                        .filter(|tool_declaration| !disabled_tools.contains(&tool_declaration.name))
                        .collect(),
                    None => tool_declarations,
                };
                let mut mcp_tools = self.mcp_tools.write().await;
                mcp_tools.insert(name, tool_declarations);
            }
        }
        Ok(())
    }

    /// unregisters a MCP server with the manager.
    ///
    /// # Arguments
    /// * `name` - The name of the MCP server to unregister.
    ///
    /// # Returns
    /// * `Result<(), WorkflowError>` - The result of the unregistration.
    pub async fn unregister_mcp_server(&self, name: &str) -> Result<(), WorkflowError> {
        // Remove MCP tools
        let mut mcp_tools = self.mcp_tools.write().await;
        mcp_tools.remove(name);

        // Remove MCP server
        let mut servers = self.mcp_servers.write().await;
        servers.remove(name);
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
        let servers = self.mcp_servers.read().await;
        for (name, server) in servers.iter() {
            status.insert(name.clone(), server.status().await);
        }
        Ok(status)
    }

    /// Gets a MCP server by its name.
    ///
    /// # Arguments
    /// * `name` - The name of the MCP server to get.
    ///
    /// # Returns
    /// * `Result<Arc<dyn McpClient>, WorkflowError>` - The result of the server retrieval.
    pub async fn get_mcp_server(&self, name: &str) -> Result<Arc<dyn McpClient>, WorkflowError> {
        let tools: tokio::sync::RwLockReadGuard<'_, HashMap<String, Arc<dyn McpClient + 'static>>> =
            self.mcp_servers.read().await;
        tools
            .get(name)
            .cloned()
            .ok_or_else(|| WorkflowError::FunctionNotFound(name.to_string()))
    }

    /// Gets the names of all registered MCP server.
    ///
    /// # Returns
    /// * `Vec<String>` - A vector of function names.
    async fn get_registered_mcp_server_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        {
            let tools = self.mcp_servers.read().await;
            names.extend(tools.keys().cloned());
        }
        names
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

        let tool = self.get_mcp_server(mcp_server_name).await?;
        tool.call(tool_name, params.clone()).await.map_err(|e| {
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
