use json_value_merge::Merge;
use lru::LruCache;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

use super::config::NodeConfig;
use super::error::WorkflowError;
use super::types::WorkflowResult;

/// Workflow context for sharing data between nodes
#[derive(Debug, Clone)]
pub struct Context {
    /// Global data storage
    data: Arc<RwLock<ContextData>>,
    /// Cache for frequently accessed data
    cache: Arc<RwLock<LruCache<String, Value>>>,
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
            cache_version: Arc::new(AtomicU64::new(0)),
            cache_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// Get a value from the context with caching
    pub async fn get(&self, key: &str) -> Option<Value> {
        // Check cache first
        {
            let mut cache = self.cache.write().await;
            if let Some(value) = cache.get(key) {
                return Some(value.clone());
            }
        }

        // Cache miss, get from storage
        let data = self.data.read().await;
        let result = data.values.get(key).cloned();

        // If value is found, update cache
        if let Some(ref value) = result {
            let mut cache = self.cache.write().await;
            cache.put(key.to_string(), value.clone());
        }

        result
    }

    /// Set a value in the context and update cache
    pub async fn set(&self, key: String, value: Value) -> WorkflowResult<()> {
        // Update storage
        {
            let mut data = self.data.write().await;
            data.values.insert(key.clone(), value.clone());
        }

        // Update cache
        {
            let mut cache = self.cache.write().await;
            cache.put(key, value);
        }

        Ok(())
    }

    /// Get a node's output
    pub async fn get_output(&self, node_id: &str) -> Option<Value> {
        let data = self.data.read().await;
        data.outputs.get(node_id).cloned()
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
            data.outputs.insert(output_key, output);
        }
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
        let output = self
            .get_output(node_id)
            .await
            .ok_or_else(|| WorkflowError::Context(format!("Node output not found: {}", node_id)))?;

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
                        WorkflowError::Context(format!("Path not found: {}", part))
                    })?;
                }
                Value::Array(arr) => {
                    if let Ok(index) = part.parse::<usize>() {
                        current = arr.get(index).ok_or_else(|| {
                            WorkflowError::Context(format!("Array index out of bounds: {}", index))
                        })?;
                    } else {
                        return Err(WorkflowError::Context(format!(
                            "Invalid array index: {}",
                            part
                        )));
                    }
                }
                _ => {
                    return Err(WorkflowError::Context(format!(
                        "Cannot access path {} on non-object/non-array value",
                        part
                    )))
                }
            }
        }

        Ok(current.clone())
    }

    /// Get node configurations by node ID list
    pub async fn get_node_configs(&self, node_ids: &[String]) -> WorkflowResult<Vec<NodeConfig>> {
        // If node ID list is empty, return empty result
        if node_ids.is_empty() {
            return Ok(Vec::new());
        }

        // Double-check
        let mut cache_key;
        loop {
            let _lock = self.cache_lock.lock().await;
            // Get current cache version
            let current_version = self.cache_version.load(Ordering::Acquire);

            // Construct cache key, including version information to ensure cache invalidation on version change
            cache_key = format!("node_configs:{}:v{}", node_ids.join(","), current_version);

            // Check cache first
            let mut cache = self.cache.write().await;
            if let Some(cached_value) = cache.get(&cache_key) {
                if let Ok(configs) = serde_json::from_value::<Vec<NodeConfig>>(cached_value.clone())
                {
                    return Ok(configs);
                }
            }

            let version_after = self.cache_version.load(Ordering::Acquire);
            if current_version == version_after {
                break;
            }
        }

        // Cache miss, get from metadata
        let nodes_json = self.get_metadata("workflow_nodes").await.ok_or_else(|| {
            WorkflowError::Config("Workflow nodes not found in context".to_string())
        })?;

        let nodes: HashMap<String, NodeConfig> = serde_json::from_str(&nodes_json)
            .map_err(|e| WorkflowError::Config(format!("Failed to parse workflow nodes: {}", e)))?;

        // Check if there are non-existent node IDs to prevent cache penetration
        let missing_ids: Vec<String> = node_ids
            .iter()
            .filter(|id| !nodes.contains_key(*id))
            .cloned()
            .collect();

        if !missing_ids.is_empty() {
            return Err(WorkflowError::Config(format!(
                "Node IDs not found: {}",
                missing_ids.join(", ")
            )));
        }

        // Filter out requested nodes
        let result = node_ids
            .iter()
            .filter_map(|id| nodes.get(id).cloned())
            .collect::<Vec<NodeConfig>>();

        // Update cache, only cache non-empty results
        if !result.is_empty() {
            if let Ok(cache_value) = serde_json::to_value(&result) {
                let mut cache = self.cache.write().await;
                cache.put(cache_key, cache_value);
            }
        }

        Ok(result)
    }

    /// Clear all cache
    pub async fn clear_cache(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
        self.cache_version.fetch_add(1, Ordering::Release);
    }

    /// Invalidate cache version, making all cache invalid
    pub async fn invalidate_cache(&self) {
        self.cache_version.fetch_add(1, Ordering::Release);
    }

    /// Batch update cache
    pub async fn batch_update_cache<T: Serialize>(
        &self,
        key_prefix: &str,
        items: &HashMap<String, T>,
    ) -> WorkflowResult<()> {
        let current_version = self.cache_version.load(Ordering::Acquire);
        let mut cache = self.cache.write().await;

        for (key, value) in items {
            let cache_key = format!("{}:{}:v{}", key_prefix, key, current_version);
            if let Ok(cache_value) = serde_json::to_value(value) {
                cache.put(cache_key, cache_value);
            }
        }

        Ok(())
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

    /// Get node state
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_context_operations() {
        let ctx = Context::new();

        // Test basic operations
        ctx.set("key1".to_string(), json!("value1")).await.unwrap();
        assert_eq!(ctx.get("key1").await, Some(json!("value1")));

        ctx.set_output("node1".to_string(), json!({"result": "data1"}))
            .await
            .unwrap();
        assert_eq!(
            ctx.get_output("node1").await,
            Some(json!({"result": "data1"}))
        );

        ctx.set_metadata("meta1".to_string(), "value1".to_string())
            .await
            .unwrap();
        assert_eq!(ctx.get_metadata("meta1").await, Some("value1".to_string()));
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
