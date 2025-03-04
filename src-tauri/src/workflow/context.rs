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
    /// 节点状态追踪 (节点ID -> 状态)
    node_states: HashMap<String, NodeState>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum NodeState {
    Pending,
    Running,
    Completed,
    Failed(u32),   // 记录失败次数
    Retrying(u32), // 记录重试次数
    Paused,
}

/// 节点状态枚举
impl Context {
    /// Create a new context instance
    pub fn new() -> Self {
        Self {
            data: Arc::new(RwLock::new(ContextData::default())),
            cache: Arc::new(RwLock::new(LruCache::new(NonZeroUsize::new(100).unwrap()))), // 缓存最近的100个值
            cache_version: Arc::new(AtomicU64::new(0)),
            cache_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// Get a value from the context with caching
    pub async fn get(&self, key: &str) -> Option<Value> {
        // 先检查缓存
        {
            let mut cache = self.cache.write().await;
            if let Some(value) = cache.get(key) {
                return Some(value.clone());
            }
        }

        // 缓存未命中，从存储中获取
        let data = self.data.read().await;
        let result = data.values.get(key).cloned();

        // 如果找到了值，更新缓存
        if let Some(ref value) = result {
            let mut cache = self.cache.write().await;
            cache.put(key.to_string(), value.clone());
        }

        result
    }

    /// Set a value in the context and update cache
    pub async fn set(&self, key: String, value: Value) -> WorkflowResult<()> {
        // 更新存储
        {
            let mut data = self.data.write().await;
            data.values.insert(key.clone(), value.clone());
        }

        // 更新缓存
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
    /// # Paraments
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

    /// 解析参数中的引用并替换为实际值
    ///
    /// 支持的引用格式：
    /// - ${node_id} - 引用节点的整个输出
    /// - ${node_id.path.to.data} - 引用节点输出中的特定字段
    pub async fn resolve_params(&self, params: Value) -> WorkflowResult<Value> {
        self.resolve_params_inner(params).await
    }

    /// 内部实现，处理递归异步调用
    async fn resolve_params_inner(&self, params: Value) -> WorkflowResult<Value> {
        match params {
            Value::Object(map) => {
                let mut result = serde_json::Map::new();
                for (key, value) in map {
                    // 使用 Box::pin 避免无限大小的 future
                    println!("解析对象字段: {}", key);
                    let resolved = Box::pin(self.resolve_params_inner(value)).await?;
                    println!("字段 {} 解析结果: {}", key, resolved);
                    result.insert(key, resolved);
                }
                Ok(Value::Object(result))
            }
            Value::Array(arr) => {
                let mut result = Vec::new();
                for (index, item) in arr.iter().enumerate() {
                    // 使用 Box::pin 避免无限大小的 future
                    println!("解析数组元素 [{}]", index);
                    let resolved = Box::pin(self.resolve_params_inner(item.clone())).await?;
                    println!("数组元素 [{}] 解析结果: {}", index, resolved);
                    result.push(resolved);
                }
                Ok(Value::Array(result))
            }
            Value::String(s) => {
                // 检查字符串是否包含引用
                if s.starts_with("${") && s.ends_with("}") {
                    // 如果整个字符串就是一个引用，直接返回引用的值
                    // 去掉 ${ 和 } 字符
                    let reference = &s[2..s.len() - 1];
                    println!("解析引用: {}", reference);

                    // 分割节点ID和路径
                    let parts: Vec<&str> = reference.splitn(2, '.').collect();
                    let node_id = parts[0];
                    let path = parts.get(1).copied();

                    println!("引用节点ID: {}, 路径: {:?}", node_id, path);

                    // 获取节点输出
                    let result = self.get_node_output_with_path(node_id, path).await;
                    match &result {
                        Ok(value) => println!("引用解析结果: {}", value),
                        Err(e) => println!("引用解析错误: {}", e),
                    }

                    return result;
                } else if s.contains("${") && s.contains("}") {
                    // 对于包含多个引用的字符串，使用字符串操作处理
                    let mut result = s.clone();
                    let mut start_pos = 0;

                    while let Some(start_idx) = result[start_pos..].find("${") {
                        let real_start = start_pos + start_idx;
                        if let Some(end_idx) = result[real_start..].find("}") {
                            let real_end = real_start + end_idx + 1;

                            // 提取引用内容 (不包括 ${ 和 })
                            let reference = &result[real_start + 2..real_end - 1];

                            // 分割节点ID和路径
                            let parts: Vec<&str> = reference.splitn(2, '.').collect();
                            if !parts.is_empty() {
                                let node_id = parts[0];
                                let path = parts.get(1).copied();

                                // 获取引用的值
                                match self.get_node_output_with_path(node_id, path).await {
                                    Ok(node_output) => {
                                        // 将值转换为字符串并替换
                                        let replacement = match node_output {
                                            Value::String(s) => s,
                                            Value::Null => "null".to_string(),
                                            Value::Bool(b) => b.to_string(),
                                            Value::Number(n) => n.to_string(),
                                            _ => node_output.to_string(),
                                        };

                                        // 替换引用
                                        result.replace_range(real_start..real_end, &replacement);

                                        // 更新起始位置
                                        start_pos = real_start + replacement.len();
                                    }
                                    Err(e) => return Err(e),
                                }
                            } else {
                                // 引用格式不正确，跳过
                                start_pos = real_end;
                            }
                        } else {
                            // 没有找到匹配的结束括号，跳过
                            break;
                        }
                    }

                    Ok(Value::String(result))
                } else {
                    Ok(Value::String(s))
                }
            }
            // 其他类型直接返回
            _ => Ok(params),
        }
    }

    /// 获取节点输出，并可选择性地访问其中的特定路径
    async fn get_node_output_with_path(
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

    /// 访问 JSON 中的特定路径
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

    /// 根据节点ID列表获取对应的节点配置
    pub async fn get_node_configs(&self, node_ids: &[String]) -> WorkflowResult<Vec<NodeConfig>> {
        // 如果节点ID列表为空，直接返回空结果
        if node_ids.is_empty() {
            return Ok(Vec::new());
        }

        // 双重校验
        let mut cache_key;
        loop {
            let _lock = self.cache_lock.lock().await;
            // 获取当前缓存版本
            let current_version = self.cache_version.load(Ordering::Acquire);

            // 构建缓存键，包含版本信息以确保版本变更时缓存失效
            cache_key = format!("node_configs:{}:v{}", node_ids.join(","), current_version);

            // 先检查缓存
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

        // 缓存未命中，从元数据中获取节点映射
        let nodes_json = self.get_metadata("workflow_nodes").await.ok_or_else(|| {
            WorkflowError::Config("Workflow nodes not found in context".to_string())
        })?;

        let nodes: HashMap<String, NodeConfig> = serde_json::from_str(&nodes_json)
            .map_err(|e| WorkflowError::Config(format!("Failed to parse workflow nodes: {}", e)))?;

        // 检查是否有不存在的节点ID，防止缓存穿透
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

        // 过滤出请求的节点
        let result = node_ids
            .iter()
            .filter_map(|id| nodes.get(id).cloned())
            .collect::<Vec<NodeConfig>>();

        // 更新缓存，只有当结果非空时才缓存
        if !result.is_empty() {
            if let Ok(cache_value) = serde_json::to_value(&result) {
                let mut cache = self.cache.write().await;
                cache.put(cache_key, cache_value);
            }
        }

        Ok(result)
    }

    /// 清除所有缓存
    pub async fn clear_cache(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
        self.cache_version.fetch_add(1, Ordering::Release);
    }

    /// 更新缓存版本，使所有缓存失效
    pub async fn invalidate_cache(&self) {
        self.cache_version.fetch_add(1, Ordering::Release);
    }

    /// 批量更新缓存
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

    /// 更新节点状态
    pub async fn update_node_state(&self, node_id: &str, state: NodeState) {
        let mut data = self.data.write().await;
        data.node_states.insert(node_id.to_string(), state);
    }

    /// 更新节点状态，如果节点状态已经是失败或重试中，则计数加一
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

    /// 获取节点状态
    pub async fn get_node_state(&self, node_id: &str) -> NodeState {
        let data = self.data.read().await;
        data.node_states
            .get(node_id)
            .cloned()
            .unwrap_or(NodeState::Pending)
    }

    /// 获取已完成节点集合
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

        // 设置节点输出
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

        // 测试简单引用
        let params = json!({
            "param1": "${node1}",
            "param2": "${node2}",
            "param3": "prefix ${node2} suffix"
        });

        let resolved = ctx.resolve_params(params).await.unwrap();
        assert_eq!(resolved["param1"], ctx.get_output("node1").await.unwrap());
        assert_eq!(resolved["param2"], ctx.get_output("node2").await.unwrap());
        assert_eq!(resolved["param3"], json!("prefix simple string suffix"));

        // 测试路径引用
        let params = json!({
            "param1": "${node1.data.value}",
            "param2": "${node1.data.nested.array.1}"
        });

        let resolved = ctx.resolve_params(params).await.unwrap();
        assert_eq!(resolved["param1"], json!(42));
        assert_eq!(resolved["param2"], json!(2));

        // 测试数组中的引用
        let params = json!({
            "array": [1, "${node1.data.value}", 3]
        });

        let resolved = ctx.resolve_params(params).await.unwrap();
        assert_eq!(resolved["array"][1], json!(42));
    }
}
