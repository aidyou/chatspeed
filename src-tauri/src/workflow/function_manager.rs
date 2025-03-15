use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::workflow::context::Context;
use crate::workflow::error::WorkflowError;

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
    /// * `context` - The context in which the function is executed.
    ///
    /// # Returns
    /// * `FunctionResult` - The result of the function execution.
    async fn execute(&self, params: Value, context: &Context) -> FunctionResult;
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

    /// Unregisters a function from the manager.
    ///
    /// # Arguments
    /// * `name` - The name of the function to unregister.
    ///
    /// # Returns
    /// * `Result<(), WorkflowError>` - The result of the unregistration.
    pub async fn unregister_function(&self, name: &str) -> Result<(), WorkflowError> {
        let mut functions = self.functions.write().await;

        if !functions.contains_key(name) {
            return Err(WorkflowError::FunctionNotFound(name.to_string()));
        }

        functions.remove(name);
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
    /// * `context` - The context in which the function is executed.
    ///
    /// # Returns
    /// * `FunctionResult` - The result of the function execution.
    pub async fn execute_function(
        &self,
        name: &str,
        params: Value,
        context: &Context,
    ) -> FunctionResult {
        let function = self.get_function(name).await?;
        function.execute(params, context).await
    }

    /// Gets the names of all registered functions.
    ///
    /// # Returns
    /// * `Vec<String>` - A vector of function names.
    pub async fn get_registered_functions(&self) -> Vec<String> {
        let functions = self.functions.read().await;
        functions.keys().cloned().collect()
    }

    /// Checks if a function is registered.
    ///
    /// # Arguments
    /// * `name` - The name of the function to check.
    ///
    /// # Returns
    /// * `bool` - Whether the function is registered.
    pub async fn has_function(&self, name: &str) -> bool {
        let functions = self.functions.read().await;
        functions.contains_key(name)
    }
}

/// A default implementation of `FunctionManager`.
impl Default for FunctionManager {
    fn default() -> Self {
        Self::new()
    }
}
