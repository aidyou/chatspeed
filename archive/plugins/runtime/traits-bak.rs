use async_trait::async_trait;
use serde_json::Value;
use std::error::Error;

/// Runtime trait, defines the basic interface for runtime environments
#[async_trait]
pub trait Runtime: Send + Sync {
    /// Initialize the runtime environment
    async fn init(&mut self) -> Result<(), Box<dyn Error>>;

    /// Execute a file in the runtime environment
    ///
    /// For module-based files (e.g., .js, .py), this will load and execute the module.
    /// The module should export a handler function that handles its own input/output.
    ///
    /// # Arguments
    ///
    /// * `plugin_id` - Directory path of the plugin to execute
    ///
    /// # Returns
    ///
    /// Returns the execution result as a JSON value or an error
    async fn execute_plugin(&self, plugin_id: &str) -> Result<Value, Box<dyn Error + Send>>;

    /// Cleanup the runtime environment
    async fn cleanup(&mut self) -> Result<(), Box<dyn Error>>;
}
