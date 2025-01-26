use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::types::WorkflowResult;

/// Workflow context for sharing data between nodes
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// Global data storage
    data: Arc<RwLock<ContextData>>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ContextData {
    /// General data storage
    values: HashMap<String, Value>,
    /// Node output cache
    outputs: HashMap<String, Value>,
    /// Workflow metadata
    metadata: HashMap<String, String>,
}

impl Context {
    /// Create a new context instance
    pub fn new() -> Self {
        Self {
            data: Arc::new(RwLock::new(ContextData::default())),
        }
    }

    /// Get a value from the context
    pub async fn get(&self, key: &str) -> Option<Value> {
        let data = self.data.read().await;
        data.values.get(key).cloned()
    }

    /// Set a value in the context
    pub async fn set(&self, key: String, value: Value) -> WorkflowResult<()> {
        let mut data = self.data.write().await;
        data.values.insert(key, value);
        Ok(())
    }

    /// Get a node's output
    pub async fn get_output(&self, node_id: &str) -> Option<Value> {
        let data = self.data.read().await;
        data.outputs.get(node_id).cloned()
    }

    /// Set a node's output
    pub async fn set_output(&self, node_id: String, output: Value) -> WorkflowResult<()> {
        let mut data = self.data.write().await;
        data.outputs.insert(node_id, output);
        Ok(())
    }

    /// Get metadata value
    pub async fn get_metadata(&self, key: &str) -> Option<String> {
        let data = self.data.read().await;
        data.metadata.get(key).cloned()
    }

    /// Set metadata value
    pub async fn set_metadata(&self, key: String, value: String) -> WorkflowResult<()> {
        let mut data = self.data.write().await;
        data.metadata.insert(key, value);
        Ok(())
    }

    /// Clear all context data
    pub async fn clear(&self) -> WorkflowResult<()> {
        let mut data = self.data.write().await;
        data.values.clear();
        data.outputs.clear();
        data.metadata.clear();
        Ok(())
    }

    /// Get a snapshot of all context data
    pub async fn snapshot(&self) -> WorkflowResult<Value> {
        let data = self.data.read().await;
        serde_json::to_value(&*data).map_err(From::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_context_operations() {
        let ctx = Context::new();

        // Test basic value operations
        ctx.set("test_key".to_string(), json!("test_value")).await.unwrap();
        assert_eq!(ctx.get("test_key").await.unwrap(), json!("test_value"));

        // Test node output operations
        ctx.set_output("node1".to_string(), json!({"result": "success"}))
            .await
            .unwrap();
        assert_eq!(
            ctx.get_output("node1").await.unwrap(),
            json!({"result": "success"})
        );

        // Test metadata operations
        ctx.set_metadata("version".to_string(), "1.0".to_string())
            .await
            .unwrap();
        assert_eq!(ctx.get_metadata("version").await.unwrap(), "1.0");

        // Test snapshot
        let snapshot = ctx.snapshot().await.unwrap();
        assert!(snapshot.is_object());

        // Test clear
        ctx.clear().await.unwrap();
        assert_eq!(ctx.get("test_key").await, None);
    }
}
