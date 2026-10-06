use json_value_merge::Merge;
use lru::LruCache;
use rust_i18n::t;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::types::WorkflowResult;
use crate::workflow::error::WorkflowError;

/// Workflow context for sharing data between nodes
#[derive(Debug, Clone)]
pub struct Context {
    /// Global data storage
    data: Arc<RwLock<ContextData>>,
    /// Cache for frequently accessed data
    cache: Arc<RwLock<LruCache<String, Value>>>,
    // Last result key
    last_result_key: Arc<RwLock<String>>,
    /// Cache version control for invalidation
    cache_version: Arc<AtomicU64>,
    cache_lock: Arc<tokio::sync::Mutex<()>>,
}

#[derive(Debug, Default)]
struct ContextData {
    /// General data storage
    values: HashMap<String, Value>,
    /// Node output cache
    outputs: HashMap<String, Value>,
    /// Workflow metadata
    metadata: HashMap<String, String>,
    /// Node state tracking (node ID -> state)
    node_states: HashMap<String, NodeState>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum NodeState {
    Pending,
    Running,
    Completed,
    Failed(u32),   // Record failure count
    Retrying(u32), // Record retry count
    Paused,
}

/// Node state enumeration
impl Context {
    /// Create a new context instance
    pub fn new() -> Self {
        Self {
            data: Arc::new(RwLock::new(ContextData::default())),
            cache: Arc::new(RwLock::new(LruCache::new(NonZeroUsize::new(100).unwrap()))), // Cache the latest 100 values
            last_result_key: Arc::new(RwLock::new(String::new())),
            cache_version: Arc::new(AtomicU64::new(0)),
            cache_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// Get a node's output
    pub async fn get_output(&self, node_id: &str) -> Option<Value> {
        let data = self.data.read().await;
        data.outputs.get(node_id).cloned()
    }

    /// Get last output
    ///
    /// # Returns
    /// * `Option<Value>` - Returns the last output value if it exists, otherwise returns `None`
    pub async fn get_last_output(&self) -> Option<Value> {
        let key = self.last_result_key.read().await.clone();
        self.get_output(&key).await
    }

    /// Set a node's execution output into context.
    /// When the data already exists, merge the new output with the existing data.
    ///
    /// # Arguments
    /// * `output_key` - The key of the output
    /// * `output` - The output value
    ///
    /// # Returns
    /// * `WorkflowResult<()>` - Returns `Ok(())` if the output is set successfully,
    ///   otherwise returns an error with details
    ///
    /// # Errors
    /// * Returns `WorkflowError` if:
    ///   - The output key is not valid
    ///   - The output data is invalid
    pub async fn set_output(&self, output_key: String, output: Value) -> WorkflowResult<()> {
        let mut data = self.data.write().await;

        if let Some(existing_output) = data.outputs.get_mut(&output_key) {
            // If the data already exists, merge the new output with the existing data
            existing_output.merge(&output);
        } else {
            // If the data does not exist, insert the new output
            data.outputs.insert(output_key.clone(), output);
        }

        *self.last_result_key.write().await = output_key;
        Ok(())
    }

    /// Resolve parameters with references and replace with actual values
    ///
    /// Supported reference formats:
    /// - ${node_id} - Reference to a node's entire output
    /// - ${node_id.path.to.data} - Reference to a specific field in a node's output
    pub async fn resolve_params(&self, params: Value) -> WorkflowResult<Value> {
        self.resolve_params_inner(params).await
    }

    /// Get node output, and optionally access a specific path within it
    pub(crate) async fn get_node_output_with_path(
        &self,
        node_id: &str,
        path: Option<&str>,
    ) -> WorkflowResult<Value> {
        let output = self.get_output(node_id).await.ok_or_else(|| {
            WorkflowError::Context(
                t!(
                    "workflow.node_output_not_found",
                    node_id = node_id,
                    path = path.unwrap_or_default()
                )
                .to_string(),
            )
        })?;

        if let Some(path) = path {
            self.access_json_path(&output, path)
        } else {
            Ok(output)
        }
    }

    /// Access a specific path within a JSON value
    fn access_json_path(&self, value: &Value, path: &str) -> WorkflowResult<Value> {
        let parts: Vec<&str> = path.split('.').collect();
        let mut current = value;

        for part in parts {
            match current {
                Value::Object(map) => {
                    current = map.get(part).ok_or_else(|| {
                        WorkflowError::Context(
                            t!("workflow.json_path_not_found", path = part).to_string(),
                        )
                    })?;
                }
                Value::Array(arr) => {
                    if let Ok(index) = part.parse::<usize>() {
                        current = arr.get(index).ok_or_else(|| {
                            WorkflowError::Context(
                                t!("workflow.array_index_out_of_bounds", index = index).to_string(),
                            )
                        })?;
                    } else {
                        return Err(WorkflowError::Context(
                            t!("workflow.invalid_array_index", index = part).to_string(),
                        ));
                    }
                }
                _ => {
                    return Err(WorkflowError::Context(
                        t!("workflow.cannot_access_path", path = part).to_string(),
                    ));
                }
            }
        }

        Ok(current.clone())
    }

    // Get node state
    pub async fn get_node_state(&self, node_id: &str) -> NodeState {
        let data = self.data.read().await;
        data.node_states
            .get(node_id)
            .cloned()
            .unwrap_or(NodeState::Pending)
    }

    /// Get completed node set
    pub async fn get_completed_nodes(&self) -> HashSet<String> {
        let data = self.data.read().await;
        data.node_states
            .iter()
            .filter(|(_, s)| matches!(s, NodeState::Completed))
            .map(|(k, _)| k.clone())
            .collect()
    }

    /// Update node state
    pub async fn update_node_state(&self, node_id: &str, state: NodeState) {
        let mut data = self.data.write().await;
        data.node_states.insert(node_id.to_string(), state);
    }

    /// Update node state, incrementing failure or retry count if already failed or retrying
    pub async fn update_node_with_count(&self, node_id: &str, state: NodeState) {
        let mut data = self.data.write().await;
        data.node_states
            .entry(node_id.to_string())
            .and_modify(|existing_state| match existing_state {
                NodeState::Failed(count) => *count += 1,
                NodeState::Retrying(count) => *count += 1,
                _ => *existing_state = state,
            })
            .or_insert(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_context_operations() {
        let ctx = Context::new();

        // Test basic operations
        ctx.set_output("key1".to_string(), json!("value1"))
            .await
            .unwrap();
        assert_eq!(ctx.get_output("key1").await, Some(json!("value1")));

        ctx.set_output("node1".to_string(), json!({"result": "data1"}))
            .await
            .unwrap();
        assert_eq!(
            ctx.get_output("node1").await,
            Some(json!({"result": "data1"}))
        );
    }

    #[tokio::test]
    async fn test_param_resolution() {
        let ctx = Context::new();

        // Set node output
        ctx.set_output(
            "node1".to_string(),
            json!({
                "data": {
                    "value": 42,
                    "nested": {
                        "array": [1, 2, 3]
                    }
                }
            }),
        )
        .await
        .unwrap();

        ctx.set_output("node2".to_string(), json!("simple string"))
            .await
            .unwrap();

        // Test simple reference
        let params = json!({
            "param1": "${node1}",
            "param2": "${node2}",
            "param3": "prefix ${node2} suffix"
        });

        let resolved = ctx.resolve_params(params).await.unwrap();
        assert_eq!(resolved["param1"], ctx.get_output("node1").await.unwrap());
        assert_eq!(resolved["param2"], ctx.get_output("node2").await.unwrap());
        assert_eq!(resolved["param3"], json!("prefix simple string suffix"));

        // Test path reference
        let params = json!({
            "param1": "${node1.data.value}",
            "param2": "${node1.data.nested.array.1}"
        });

        let resolved = ctx.resolve_params(params).await.unwrap();
        assert_eq!(resolved["param1"], json!(42));
        assert_eq!(resolved["param2"], json!(2));

        // Test array reference
        let params = json!({
            "array": [1, "${node1.data.value}", 3]
        });

        let resolved = ctx.resolve_params(params).await.unwrap();
        assert_eq!(resolved["array"][1], json!(42));
    }
}
