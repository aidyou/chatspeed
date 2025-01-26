use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::error::Error;
use std::fmt;

/// Plugin type enumeration
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub enum PluginType {
    Native,
    Python,
    JavaScript,
}

impl fmt::Display for PluginType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PluginType::Native => write!(f, "Native"),
            PluginType::Python => write!(f, "Python"),
            PluginType::JavaScript => write!(f, "JavaScript"),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone)]
pub struct RuntimePluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub plugin_type: PluginType,
}

/// Universal plugin trait for all plugin types
#[async_trait]
pub trait Plugin: Send + Sync {
    /// Get the plugin info
    fn plugin_info(&self) -> &PluginInfo;

    fn plugin_type(&self) -> &PluginType;

    /// Initialize the plugin with workflow context
    async fn init(&mut self) -> Result<(), Box<dyn Error + Send + Sync>>;

    /// Destroy the plugin and perform necessary cleanup
    async fn destroy(&mut self) -> Result<(), Box<dyn Error + Send + Sync>>;

    /// Execute the plugin with input and return output
    ///
    /// # Arguments
    ///
    /// * `plugin_id` - Optional plugin ID for runtime plugins
    /// * `input` - Optional input data as JSON value
    async fn execute(
        &mut self,
        input: Option<Value>,
        plugin_info: Option<PluginInfo>,
    ) -> Result<Value, Box<dyn Error + Send + Sync>>;

    /// Returns the input parameter schema
    fn input_schema(&self) -> Value;

    /// Returns the output parameter schema
    fn output_schema(&self) -> Value;

    /// Validates the input against the schema
    fn validate_input(&self, _input: &Value) -> Result<(), Box<dyn Error + Send + Sync>> {
        Ok(())
    }

    /// Validates the output against the schema
    fn validate_output(&self, _output: &Value) -> Result<(), Box<dyn Error + Send + Sync>> {
        Ok(())
    }
}

/// Plugin factory trait for creating plugin instances
pub trait PluginFactory: Send + Sync {
    /// Creates a new instance of the plugin
    ///
    /// # Arguments
    ///
    /// * `init_options` - Required initialization options (e.g., database path for store plugin)
    fn create_instance(
        &self,
        init_options: Option<&Value>,
    ) -> Result<Box<dyn Plugin>, Box<dyn Error + Send + Sync>>;
}
