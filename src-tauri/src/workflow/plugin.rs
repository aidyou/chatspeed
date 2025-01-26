use async_trait::async_trait;
use rust_i18n::t;
use serde_json::Value;

use super::config::NodeConfig;
use super::context::Context;
use super::error::WorkflowError;
use super::traits::Plugin;
use super::types::WorkflowResult;

/// Base plugin implementation providing common functionality
#[derive(Debug)]
pub struct BasePlugin {
    id: String,
    name: String,
    description: String,
    version: String,
}

impl BasePlugin {
    /// Create a new base plugin instance
    pub fn new(id: String, name: String, description: String, version: String) -> Self {
        Self {
            id,
            name,
            description,
            version,
        }
    }
}

#[async_trait]
impl Plugin for BasePlugin {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn version(&self) -> &str {
        &self.version
    }

    async fn init(&self, _config: Value) -> WorkflowResult<()> {
        Ok(())
    }

    async fn execute(&self, _node: &NodeConfig, _context: &Context) -> WorkflowResult<Value> {
        Err(WorkflowError::Plugin(
            t!("workflow.plugin.not_implemented").to_string(),
        ))
    }

    async fn cleanup(&self) -> WorkflowResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct TestPlugin {
        base: BasePlugin,
    }

    impl TestPlugin {
        fn new() -> Self {
            Self {
                base: BasePlugin::new(
                    "test".to_string(),
                    "Test Plugin".to_string(),
                    "A test plugin".to_string(),
                    "1.0.0".to_string(),
                ),
            }
        }
    }

    #[async_trait]
    impl Plugin for TestPlugin {
        fn id(&self) -> &str {
            self.base.id()
        }

        fn name(&self) -> &str {
            self.base.name()
        }

        fn description(&self) -> &str {
            self.base.description()
        }

        fn version(&self) -> &str {
            self.base.version()
        }

        async fn init(&self, _config: Value) -> WorkflowResult<()> {
            Ok(())
        }

        async fn execute(&self, _node: &NodeConfig, _context: &Context) -> WorkflowResult<Value> {
            Ok(json!({
                "status": "success",
                "message": "Test plugin executed"
            }))
        }

        async fn cleanup(&self) -> WorkflowResult<()> {
            Ok(())
        }

        fn is_parallel(&self) -> bool {
            true
        }

        fn timeout(&self) -> u64 {
            1000
        }
    }

    #[tokio::test]
    async fn test_plugin_execution() {
        let plugin = TestPlugin::new();
        let context = Context::new();
        let node = NodeConfig::default();

        assert_eq!(plugin.id(), "test");
        assert_eq!(plugin.name(), "Test Plugin");
        assert!(plugin.is_parallel());
        assert_eq!(plugin.timeout(), 1000);

        // Test initialization
        plugin.init(json!({})).await.unwrap();

        // Test execution
        let result = plugin.execute(&node, &context).await.unwrap();
        assert_eq!(result["status"], "success");

        // Test cleanup
        plugin.cleanup().await.unwrap();
    }
}
