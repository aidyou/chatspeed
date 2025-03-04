use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

/// Node configuration in the workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    /// Unique identifier for the node
    pub id: String,
    /// Type of the node (function, ai, etc.)
    pub r#type: NodeType,
    /// Function identifier (for function type nodes)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
    /// Parameters for the node function
    pub params: Option<serde_json::Value>,
    /// Timeout in seconds
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Description of the node
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Whether the node is parallel
    #[serde(skip)]
    pub parallel: bool,
    /// Output key
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

/// Node type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum NodeType {
    /// Node will be executed as a task
    Task,
    /// Node is a group of nodes
    Group(GroupConfig),
}

impl NodeType {
    pub fn matches(&self, node_type: &NodeType) -> bool {
        match (self, node_type) {
            (Self::Group(_), NodeType::Group(_)) => true,
            (Self::Task, NodeType::Task) => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GroupConfig {
    pub parallel: bool,
    pub nodes: Vec<String>,
}

fn default_timeout() -> u64 {
    30
}

fn default_retryable() -> Option<bool> {
    Some(false)
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            r#type: NodeType::Task,
            function: None,
            params: None,
            timeout_secs: default_timeout(),
            description: None,
            parallel: false,
            output: None,
        }
    }
}

impl NodeConfig {
    /// Create a new node configuration
    pub fn new(id: String, r#type: NodeType) -> Self {
        Self {
            id,
            r#type,
            function: None,
            params: None,
            timeout_secs: default_timeout(),
            description: None,
            parallel: false,
            output: None,
        }
    }
}

/// Edge configuration defining connections between nodes
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EdgeConfig {
    /// Source node identifier
    pub from: String,
    /// Target node identifier(s)
    pub to: String,
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
