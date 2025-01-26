use rust_i18n::t;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::error::WorkflowError;
use super::types::WorkflowResult;
use crate::plugins::traits::PluginType;

/// Flow configuration containing nodes and edges
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowConfig {
    /// Node configurations
    pub nodes: Vec<NodeConfig>,
    /// Edge configurations defining the workflow graph
    pub edges: Vec<EdgeConfig>,
}

/// State configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateConfig {
    /// Context configuration
    pub context: ContextConfig,
    /// Memory configuration
    pub memory: MemoryConfig,
}

/// Context configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextConfig {
    /// Type of context (e.g., "memory")
    pub r#type: String,
    /// Format of context data (e.g., "json")
    pub format: String,
    /// Schema definition
    pub schema: HashMap<String, String>,
}

/// Memory configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    /// Type of memory storage (e.g., "redis")
    pub r#type: String,
    /// Time to live in seconds
    pub ttl: u64,
}

/// Error handling configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorHandlingConfig {
    /// Retry configuration
    pub retry: RetryConfig,
    /// Fallback configuration
    pub fallback: FallbackConfig,
}

/// Retry configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    pub max_attempts: u32,
    /// Delay type (e.g., "fixed")
    pub delay: String,
    /// Delay in milliseconds
    pub delay_ms: u64,
}

/// Fallback configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackConfig {
    /// Whether fallback is enabled
    pub enabled: bool,
    /// Default response message
    pub default_response: String,
}

/// Monitoring configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringConfig {
    /// Metrics to collect
    pub metrics: Vec<String>,
    /// Logging configuration
    pub logging: LoggingConfig,
    /// Performance configuration
    pub performance: PerformanceConfig,
}

/// Logging configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Log level
    pub level: String,
    /// Whether to include context
    pub include_context: bool,
    /// Sensitive fields to mask
    pub sensitive_fields: Vec<String>,
}

/// Performance configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceConfig {
    /// Latency threshold in milliseconds
    pub latency_threshold_ms: u64,
    /// Whether caching is enabled
    pub cache_enabled: bool,
    /// Cache time to live in seconds
    pub cache_ttl: u64,
}

/// Workflow configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowConfig {
    /// Display name of the workflow
    pub name: String,
    /// Description of the workflow
    pub description: String,
    /// Flow configuration
    pub flow: FlowConfig,
    /// State configuration
    pub state: StateConfig,
    /// Error handling configuration
    pub error_handling: ErrorHandlingConfig,
    /// Monitoring configuration
    pub monitoring: MonitoringConfig,
}

/// Node configuration in the workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    /// Unique identifier for the node
    pub id: String,
    /// Type of the node (plugin, ai, etc.)
    pub r#type: String,
    /// Plugin identifier (for plugin type nodes)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin: Option<String>,
    /// Plugin type, defaults to Native if not specified
    pub plugin_type: Option<PluginType>,
    /// Model identifier (for ai type nodes)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Prompt template (for ai type nodes)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// Node-specific configuration
    pub config: Option<serde_json::Value>,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            r#type: String::new(),
            plugin: None,
            plugin_type: None,
            model: None,
            prompt: None,
            config: None,
        }
    }
}

impl NodeConfig {
    pub fn new(id: String, r#type: String) -> Self {
        Self {
            id,
            r#type,
            plugin: None,
            plugin_type: None,
            model: None,
            prompt: None,
            config: None,
        }
    }
}

/// Edge configuration defining connections between nodes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeConfig {
    /// Source node identifier
    pub from: String,
    /// Target node identifier(s)
    #[serde(with = "string_or_vec_string")]
    pub to: Vec<String>,
}

/// Custom serializer/deserializer for handling both string and array of strings
mod string_or_vec_string {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &Vec<String>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if value.len() == 1 {
            serializer.serialize_str(&value[0])
        } else {
            serializer.collect_seq(value)
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum StringOrVec {
            String(String),
            Vec(Vec<String>),
        }

        match StringOrVec::deserialize(deserializer)? {
            StringOrVec::String(s) => Ok(vec![s]),
            StringOrVec::Vec(v) => Ok(v),
        }
    }
}

impl WorkflowConfig {
    /// Validate the workflow configuration
    pub fn validate(&self) -> WorkflowResult<()> {
        // Check for empty or invalid fields
        if self.name.is_empty() {
            return Err(WorkflowError::Config(
                t!("workflow.config.empty_workflow_name").to_string(),
            ));
        }

        // Validate nodes
        let mut node_ids = std::collections::HashSet::new();
        for node in &self.flow.nodes {
            if node.id.is_empty() {
                return Err(WorkflowError::Config(
                    t!("workflow.config.empty_node_id").to_string(),
                ));
            }
            if node.r#type.is_empty() {
                return Err(WorkflowError::Config(
                    t!("workflow.config.empty_node_type").to_string(),
                ));
            }
            if !node_ids.insert(&node.id) {
                return Err(WorkflowError::Config(
                    t!("workflow.config.duplicate_node_id", id = node.id).to_string(),
                ));
            }
        }

        // Validate edges
        for edge in &self.flow.edges {
            if !node_ids.contains(&edge.from) {
                return Err(WorkflowError::Config(
                    t!("workflow.config.edge_source_not_found", id = edge.from).to_string(),
                ));
            }
            for target in &edge.to {
                if !node_ids.contains(target) {
                    return Err(WorkflowError::Config(
                        t!("workflow.config.edge_target_not_found", id = target).to_string(),
                    ));
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_workflow_config_validation() {
        // Create a valid workflow configuration
        let config = WorkflowConfig {
            name: "Test Workflow".to_string(),
            description: "Test workflow description".to_string(),
            flow: FlowConfig {
                nodes: vec![
                    NodeConfig {
                        id: "node1".to_string(),
                        r#type: "plugin".to_string(),
                        plugin: Some("test-plugin".to_string()),
                        plugin_type: None,
                        model: None,
                        prompt: None,
                        config: None,
                    },
                    NodeConfig {
                        id: "node2".to_string(),
                        r#type: "ai".to_string(),
                        plugin: None,
                        plugin_type: None,
                        model: Some("gpt-4".to_string()),
                        prompt: Some("test prompt".to_string()),
                        config: None,
                    },
                ],
                edges: vec![EdgeConfig {
                    from: "node1".to_string(),
                    to: vec!["node2".to_string()],
                }],
            },
            state: StateConfig {
                context: ContextConfig {
                    r#type: "memory".to_string(),
                    format: "json".to_string(),
                    schema: HashMap::new(),
                },
                memory: MemoryConfig {
                    r#type: "redis".to_string(),
                    ttl: 3600,
                },
            },
            error_handling: ErrorHandlingConfig {
                retry: RetryConfig {
                    max_attempts: 3,
                    delay: "fixed".to_string(),
                    delay_ms: 1000,
                },
                fallback: FallbackConfig {
                    enabled: true,
                    default_response: "Error occurred".to_string(),
                },
            },
            monitoring: MonitoringConfig {
                metrics: vec!["response_time".to_string()],
                logging: LoggingConfig {
                    level: "info".to_string(),
                    include_context: true,
                    sensitive_fields: vec![],
                },
                performance: PerformanceConfig {
                    latency_threshold_ms: 2000,
                    cache_enabled: true,
                    cache_ttl: 300,
                },
            },
        };

        // Test validation
        assert!(config.validate().is_ok());

        // Test invalid configurations
        let mut invalid_config = config.clone();
        invalid_config.name = "".to_string();
        assert!(invalid_config.validate().is_err());

        let mut invalid_config = config.clone();
        invalid_config.flow.nodes[0].id = "".to_string();
        assert!(invalid_config.validate().is_err());

        let mut invalid_config = config.clone();
        invalid_config.flow.nodes[0].r#type = "".to_string();
        assert!(invalid_config.validate().is_err());

        let mut invalid_config = config;
        invalid_config.flow.edges[0].from = "non_existent".to_string();
        assert!(invalid_config.validate().is_err());
    }

    #[test]
    fn test_parse_example_workflow() {
        let yaml_content = fs::read_to_string("../plugin/examples/workflow/chat_assistant.yml")
            .expect("Failed to read example workflow file");

        let config: WorkflowConfig =
            serde_yaml::from_str(&yaml_content).expect("Failed to parse workflow YAML");

        assert_eq!(config.name, "聊天助手工作流");
        assert_eq!(
            config.description,
            "处理用户输入并生成智能回复的聊天助手流程"
        );

        // Validate the parsed configuration
        assert!(config.validate().is_ok());

        // Check some specific fields
        assert_eq!(config.flow.nodes.len(), 5);
        assert_eq!(config.flow.edges.len(), 4);

        // Check the first node
        let first_node = &config.flow.nodes[0];
        assert_eq!(first_node.id, "input_processor");
        assert_eq!(first_node.r#type, "plugin");
        assert_eq!(first_node.plugin.as_ref().unwrap(), "input-processor");

        // Check state configuration
        assert_eq!(config.state.context.r#type, "memory");
        assert_eq!(config.state.context.format, "json");
        assert_eq!(config.state.memory.r#type, "redis");
        assert_eq!(config.state.memory.ttl, 3600);

        // Check error handling configuration
        assert_eq!(config.error_handling.retry.max_attempts, 2);
        assert_eq!(config.error_handling.retry.delay, "fixed");
        assert_eq!(config.error_handling.retry.delay_ms, 500);
        assert!(config.error_handling.fallback.enabled);

        // Check monitoring configuration
        assert!(config
            .monitoring
            .metrics
            .contains(&"response_time".to_string()));
        assert_eq!(config.monitoring.logging.level, "info");
        assert!(config.monitoring.logging.include_context);
        assert_eq!(config.monitoring.performance.latency_threshold_ms, 2000);
    }
}
