use serde::{Deserialize, Serialize};
use serde_json::Value;

// /// Flow configuration containing nodes and edges
// #[derive(Debug, Clone, Serialize, Deserialize)]
// pub struct FlowConfig {
//     /// Node configurations
//     pub nodes: Vec<NodeConfig>,
//     /// Edge configurations defining the workflow graph
//     pub edges: Vec<EdgeConfig>,
// }

// /// State configuration
// #[derive(Debug, Clone, Serialize, Deserialize)]
// pub struct StateConfig {
//     /// Context configuration
//     pub context: ContextConfig,
//     /// Memory configuration
//     pub memory: MemoryConfig,
// }

// /// Context configuration
// #[derive(Debug, Clone, Serialize, Deserialize)]
// pub struct ContextConfig {
//     /// Type of context (e.g., "memory")
//     pub r#type: String,
//     /// Format of context data (e.g., "json")
//     pub format: String,
//     /// Schema definition
//     pub schema: HashMap<String, String>,
// }

// /// Memory configuration
// #[derive(Debug, Clone, Serialize, Deserialize)]
// pub struct MemoryConfig {
//     /// Type of memory storage (e.g., "redis")
//     pub r#type: String,
//     /// Time to live in seconds
//     pub ttl: u64,
// }

// /// Error handling configuration
// #[derive(Debug, Clone, Serialize, Deserialize)]
// pub struct ErrorHandlingConfig {
//     /// Retry configuration
//     pub retry: RetryConfig,
//     /// Fallback configuration
//     pub fallback: FallbackConfig,
// }

// /// Retry configuration
// #[derive(Debug, Clone, Serialize, Deserialize)]
// pub struct RetryConfig {
//     /// Maximum number of retry attempts
//     pub max_attempts: u32,
//     /// Delay type (e.g., "fixed")
//     pub delay: String,
//     /// Delay in milliseconds
//     pub delay_ms: u64,
// }

// /// Fallback configuration
// #[derive(Debug, Clone, Serialize, Deserialize)]
// pub struct FallbackConfig {
//     /// Whether fallback is enabled
//     pub enabled: bool,
//     /// Default response message
//     pub default_response: String,
// }

// /// Monitoring configuration
// #[derive(Debug, Clone, Serialize, Deserialize)]
// pub struct MonitoringConfig {
//     /// Metrics to collect
//     pub metrics: Vec<String>,
//     /// Logging configuration
//     pub logging: LoggingConfig,
//     /// Performance configuration
//     pub performance: PerformanceConfig,
// }

// /// Logging configuration
// #[derive(Debug, Clone, Serialize, Deserialize)]
// pub struct LoggingConfig {
//     /// Log level
//     pub level: String,
//     /// Whether to include context
//     pub include_context: bool,
//     /// Sensitive fields to mask
//     pub sensitive_fields: Vec<String>,
// }

// /// Performance configuration
// #[derive(Debug, Clone, Serialize, Deserialize)]
// pub struct PerformanceConfig {
//     /// Latency threshold in milliseconds
//     pub latency_threshold_ms: u64,
//     /// Whether caching is enabled
//     pub cache_enabled: bool,
//     /// Cache time to live in seconds
//     pub cache_ttl: u64,
// }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowNode {
    pub id: String,
    #[serde(default)]
    pub desc: Option<String>,
    #[serde(default)]
    pub dependencies: Option<Vec<String>>,
    pub tool: ToolConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowGroup {
    pub id: String,
    #[serde(default)]
    pub desc: Option<String>,
    #[serde(default)]
    pub dependencies: Option<Vec<String>>,
    #[serde(default = "default_parallel")]
    pub parallel: bool,
    pub nodes: Vec<WorkflowItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolConfig {
    pub function: String,
    pub param: Value,
    // 如果 工作流的 json 配置文件指定了 output, 则数据会自动写入上下文的该字段值，否则写入上下文的 ${node_id}
    // 当多个节点的 output 相同时，他们的数据将被自动合并，对于数据格式不同的不建议使用相同的 output 字段
    //
    // If the workflow JSON configuration file specifies an output,
    // the data will be automatically written to this field in the context;
    // otherwise, it will be written to ${node_id} in the context.
    // When multiple nodes have the same output, their data will be automatically merged.
    // It is not recommended to use the same output field for data with different formats.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

impl Default for ToolConfig {
    fn default() -> Self {
        Self {
            function: String::new(),
            param: Value::Null,
            output: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowLoop {
    pub id: String, // node id
    #[serde(default)]
    pub desc: Option<String>,
    #[serde(default)]
    pub dependencies: Option<Vec<String>>,
    #[serde(rename = "loop")]
    pub r#loop: LoopConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoopConfig {
    pub input: String, // input field name, e.g., "${search_result_dedup}"
    pub functions: Vec<ToolConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>, // loop limit
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Condition judgment supports compound comparison operators, including:
    /// - Logical operators: `&&` (AND), `||` (OR)
    /// - Comparison operators: `>=` (greater than or equal), `<=` (less than or equal), `!=` (not equal), `==` (equal), `>` (greater than), `<` (less than)
    /// - Parentheses: for priority grouping
    pub filter: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WorkflowItem {
    Node(WorkflowNode),
    Loop(WorkflowLoop),
    Group(WorkflowGroup),
}

fn default_parallel() -> bool {
    false
}

/// Node configuration in the workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    /// Unique identifier for the node
    pub id: String,
    /// Type of the node (function, ai, etc.)
    pub r#type: NodeType,
    /// Timeout in seconds
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Description of the node
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Node type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum NodeType {
    /// Node will be executed as a task
    Task(ToolConfig),
    // Loop node
    Loop(LoopConfig),
    /// Node is a group of nodes
    Group(GroupConfig),
}

impl NodeType {
    pub fn matches(&self, node_type: &NodeType) -> bool {
        match (self, node_type) {
            (Self::Group(_), NodeType::Group(_)) => true,
            (Self::Task(_), NodeType::Task(_)) => true,
            (Self::Loop(_), NodeType::Loop(_)) => true,
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
            r#type: NodeType::Task(ToolConfig::default()),
            timeout_secs: default_timeout(),
            description: None,
        }
    }
}

impl NodeConfig {
    /// Create a new node configuration
    pub fn new(id: String, r#type: NodeType) -> Self {
        Self {
            id,
            r#type,
            timeout_secs: default_timeout(),
            description: None,
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
