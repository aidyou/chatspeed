use async_trait::async_trait;
use rust_i18n::t;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::ai::interaction::chat_completion::ChatState;
use crate::commands::chat::setup_chat_proxy;
use crate::constants::CFG_CHP_SERVER;
use crate::db::{AiModel, MainStore};
use crate::workflow::error::WorkflowError;

use super::tools::Crawler;
use super::tools::Plot;
use super::tools::Search;
use super::tools::SearchDedup;
use super::tools::{ChatCompletion, ModelName};

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
    CHP,
}

impl fmt::Display for FunctionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FunctionType::Native => write!(f, "Native"),
            FunctionType::Http => write!(f, "Http"),
            FunctionType::CHP => write!(f, "CHP"),
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
}

impl FunctionManager {
    /// Creates a new instance of `FunctionManager`.
    pub fn new() -> Self {
        Self {
            functions: RwLock::new(HashMap::new()),
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
    pub async fn get_registered_functions(&self) -> Vec<String> {
        let functions = self.functions.read().await;
        functions.keys().cloned().collect()
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
        for function in self.get_registered_functions().await {
            if let Ok(function) = self.get_function(&function).await {
                if !excluded.contains(function.name()) {
                    specs.push(function.function_calling_spec());
                }
            }
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
}

/// A default implementation of `FunctionManager`.
impl Default for FunctionManager {
    fn default() -> Self {
        Self::new()
    }
}
