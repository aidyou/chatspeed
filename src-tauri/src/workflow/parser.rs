use lazy_static::lazy_static;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

use crate::workflow::{
    config::{EdgeConfig, GroupConfig, NodeConfig, NodeType},
    error::WorkflowError,
    types::WorkflowResult,
};

lazy_static! {
    static ref ID_REGEX: Regex = Regex::new(r"^[a-zA-Z][a-zA-Z0-9\-_]*$").unwrap();
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowNode {
    pub node: String,
    #[serde(default)]
    pub desc: Option<String>,
    #[serde(default)]
    pub dependencies: Option<Vec<String>>,
    pub tool: ToolDefinition,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowGroup {
    pub group: String,
    #[serde(default = "default_parallel")]
    pub parallel: bool,
    #[serde(default)]
    pub dependencies: Option<Vec<String>>,
    #[serde(default)]
    pub nodes: Vec<WorkflowNode>,
    #[serde(default)]
    pub desc: Option<String>,
}

fn default_parallel() -> bool {
    false
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub function: String,
    pub param: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WorkflowItem {
    Node(WorkflowNode),
    Group(WorkflowGroup),
}

pub struct WorkflowParser;

impl WorkflowParser {
    /// Parse workflow configuration and generate nodes and edges
    ///
    /// This method parses the workflow configuration from a JSON string and validates
    /// the nodes and edges. It also generates implicit edges for group nodes.
    ///
    /// # Parameters
    /// * `workflow_json` - The JSON string containing the workflow configuration
    ///
    /// # Returns
    /// * `WorkflowResult<(Vec<NodeConfig>, Vec<EdgeConfig>)>` - Returns a tuple containing:
    ///   - A vector of validated node configurations
    ///   - A vector of validated edge configurations, including implicit edges
    ///
    /// # Errors
    /// * Returns `WorkflowError` if:
    ///   - The JSON string is invalid
    ///   - Node validation fails
    ///   - Edge validation fails
    pub fn parse(workflow_json: &str) -> WorkflowResult<(Vec<NodeConfig>, Vec<EdgeConfig>)> {
        let workflow_items: Vec<WorkflowItem> = serde_json::from_str(workflow_json)
            .map_err(|e| WorkflowError::Config(format!("Invalid workflow JSON: {}", e)))?;

        Self::convert_to_dag(workflow_items)
    }

    /// Validate the format of a node or group ID
    ///
    /// This method ensures that the ID follows the required format:
    /// - Starts with a letter
    /// - Contains only alphanumeric characters, underscores (`_`), or hyphens (`-`)
    ///
    /// # Parameters
    /// * `id` - The ID to validate
    ///
    /// # Returns
    /// * `WorkflowResult<()>` - Returns `Ok(())` if the ID is valid
    ///
    /// # Errors
    /// * Returns `WorkflowError` if the ID format is invalid
    fn validate_id_format(id: &str) -> WorkflowResult<()> {
        if !ID_REGEX.is_match(id) {
            return Err(WorkflowError::Config(format!(
                "Invalid ID format: '{}'. Only alphanumeric, _, - are allowed and must start with a letter",
                id
            )));
        }
        Ok(())
    }

    /// Collect and validate all node and group IDs in the workflow
    ///
    /// This method ensures that all IDs are unique and follow the required format.
    ///
    /// # Parameters
    /// * `items` - A slice of `WorkflowItem` containing nodes and groups
    ///
    /// # Returns
    /// * `WorkflowResult<HashSet<String>>` - Returns a set of all valid IDs
    ///
    /// # Errors
    /// * Returns `WorkflowError` if:
    ///   - An ID format is invalid
    ///   - Duplicate IDs are found
    fn collect_and_validate_node_ids(items: &[WorkflowItem]) -> WorkflowResult<HashSet<String>> {
        let mut node_ids = HashSet::new();

        for item in items {
            match item {
                WorkflowItem::Node(node) => {
                    Self::validate_id_format(&node.node)?;
                    if !node_ids.insert(node.node.clone()) {
                        return Err(WorkflowError::Config(format!(
                            "Duplicate node ID: {}",
                            node.node
                        )));
                    }
                }
                WorkflowItem::Group(group) => {
                    Self::validate_id_format(&group.group)?;
                    if !node_ids.insert(group.group.clone()) {
                        return Err(WorkflowError::Config(format!(
                            "Duplicate group ID: {}",
                            group.group
                        )));
                    }

                    for node in &group.nodes {
                        Self::validate_id_format(&node.node)?;
                        if !node_ids.insert(node.node.clone()) {
                            return Err(WorkflowError::Config(format!(
                                "Duplicate node ID: {}",
                                node.node
                            )));
                        }
                    }
                }
            }
        }

        Ok(node_ids)
    }

    /// Process dependencies for a node or group
    ///
    /// This method validates dependencies and adds corresponding edges to the graph.
    ///
    /// # Parameters
    /// * `node_id` - The ID of the node or group
    /// * `dependencies` - A slice of dependency IDs
    /// * `node_ids` - A set of all valid node and group IDs
    /// * `edges` - A mutable reference to the vector of edges
    ///
    /// # Returns
    /// * `WorkflowResult<()>` - Returns `Ok(())` if all dependencies are valid
    ///
    /// # Errors
    /// * Returns `WorkflowError` if:
    ///   - A dependency does not exist
    ///   - A self-dependency is detected
    fn process_dependencies(
        node_id: &str,
        dependencies: &[String],
        node_ids: &HashSet<String>,
        edges: &mut Vec<EdgeConfig>,
    ) -> WorkflowResult<()> {
        for dep in dependencies {
            if !node_ids.contains(dep) {
                return Err(WorkflowError::Config(format!(
                    "Dependency '{}' of '{}' does not exist",
                    dep, node_id
                )));
            }

            if dep == node_id {
                return Err(WorkflowError::Config(format!(
                    "Self-dependency detected for '{}'",
                    node_id
                )));
            }

            edges.push(EdgeConfig {
                from: dep.clone(),
                to: node_id.to_string(),
            });
        }
        Ok(())
    }

    /// Convert workflow items into a directed acyclic graph (DAG)
    ///
    /// This method processes nodes and groups, validates their dependencies,
    /// and generates the final DAG structure.
    ///
    /// # Parameters
    /// * `items` - A vector of `WorkflowItem` containing nodes and groups
    ///
    /// # Returns
    /// * `WorkflowResult<(Vec<NodeConfig>, Vec<EdgeConfig>)>` - Returns a tuple containing:
    ///   - A vector of validated node configurations
    ///   - A vector of validated edge configurations
    ///
    /// # Errors
    /// * Returns `WorkflowError` if any validation fails
    fn convert_to_dag(
        items: Vec<WorkflowItem>,
    ) -> WorkflowResult<(Vec<NodeConfig>, Vec<EdgeConfig>)> {
        let node_ids = Self::collect_and_validate_node_ids(&items)?;
        let mut edges = Vec::new();
        let mut nodes = Vec::new();

        for item in items {
            match item {
                WorkflowItem::Node(node) => {
                    Self::process_node(node, &node_ids, &mut edges, &mut nodes)?;
                }
                WorkflowItem::Group(group) => {
                    Self::process_group(group, &node_ids, &mut edges, &mut nodes)?;
                }
            }
        }

        Ok((nodes, edges))
    }

    /// Process a group and its child nodes
    ///
    /// This method handles group-specific logic, including dependency validation,
    /// edge generation, and node configuration creation.
    ///
    /// # Parameters
    /// * `group` - The `WorkflowGroup` to process
    /// * `node_ids` - A set of all valid node and group IDs
    /// * `edges` - A mutable reference to the vector of edges
    /// * `nodes` - A mutable reference to the vector of node configurations
    ///
    /// # Returns
    /// * `WorkflowResult<()>` - Returns `Ok(())` if the group is processed successfully
    ///
    /// # Errors
    /// * Returns `WorkflowError` if any validation fails
    fn process_group(
        group: WorkflowGroup,
        node_ids: &HashSet<String>,
        edges: &mut Vec<EdgeConfig>,
        nodes: &mut Vec<NodeConfig>,
    ) -> WorkflowResult<()> {
        // 提前捕获所有必要字段（避免后续部分移动）
        let group_id = group.group.clone();
        let is_parallel = group.parallel;
        let group_desc = group.desc.clone();
        let dependencies = group.dependencies.clone();

        // 收集子节点ID（必须在消费group.nodes前完成）
        let child_nodes: Vec<String> = group.nodes.iter().map(|n| n.node.clone()).collect();

        // Generate bidirectional edges for group → child nodes
        for node in &group.nodes {
            // 组 → 子节点（子节点依赖组）
            edges.push(EdgeConfig {
                from: node.node.clone(),
                to: group_id.clone(),
            });
        }

        // 处理顺序组链式依赖（消费group.nodes所有权）
        // Handle sequential dependencies for non-parallel groups
        let mut prev_node_id = None;
        for mut node in group.nodes {
            // 处理顺序依赖
            if !is_parallel {
                if let Some(prev_id) = prev_node_id {
                    node.dependencies.get_or_insert_with(Vec::new).push(prev_id);
                }
                prev_node_id = Some(node.node.clone());
            }

            Self::process_node(node, node_ids, edges, nodes)?;
        }

        // 处理组的外部依赖
        // Process external dependencies for the group
        if let Some(deps) = &dependencies {
            Self::process_dependencies(&group_id, deps, node_ids, edges)?;
        }

        // Create and add the group configuration
        nodes.push(Self::create_group_config(
            group_id,
            group_desc,
            is_parallel,
            child_nodes,
        ));

        Ok(())
    }

    /// Process a node and add it to the graph
    ///
    /// This method handles node-specific logic, including dependency validation
    /// and node configuration creation.
    ///
    /// # Parameters
    /// * `node` - The `WorkflowNode` to process
    /// * `node_ids` - A set of all valid node and group IDs
    /// * `edges` - A mutable reference to the vector of edges
    /// * `nodes` - A mutable reference to the vector of node configurations
    ///
    /// # Returns
    /// * `WorkflowResult<()>` - Returns `Ok(())` if the node is processed successfully
    ///
    /// # Errors
    /// * Returns `WorkflowError` if any validation fails
    fn process_node(
        node: WorkflowNode,
        node_ids: &HashSet<String>,
        edges: &mut Vec<EdgeConfig>,
        nodes: &mut Vec<NodeConfig>,
    ) -> WorkflowResult<()> {
        if let Some(deps) = &node.dependencies {
            Self::process_dependencies(&node.node, deps, node_ids, edges)?;
        }

        nodes.push(Self::create_node_config(node));
        Ok(())
    }

    /// Create a node configuration from a `WorkflowNode`
    ///
    /// This method converts a `WorkflowNode` into a `NodeConfig`.
    ///
    /// # Parameters
    /// * `node` - The `WorkflowNode` to convert
    ///
    /// # Returns
    /// * `NodeConfig` - The resulting node configuration
    fn create_node_config(node: WorkflowNode) -> NodeConfig {
        NodeConfig {
            id: node.node,
            r#type: NodeType::Task,
            function: Some(node.tool.function),
            params: Some(node.tool.param),
            timeout_secs: 0,
            description: node.desc,
            parallel: false,
            output: node.output,
        }
    }

    /// Create a group configuration from group properties
    ///
    /// This method converts group properties into a `NodeConfig` with a `GroupConfig`.
    ///
    /// # Parameters
    /// * `id` - The ID of the group
    /// * `description` - An optional description of the group
    /// * `parallel` - Whether the group is parallel
    /// * `child_nodes` - A vector of child node IDs
    ///
    /// # Returns
    /// * `NodeConfig` - The resulting group configuration
    fn create_group_config(
        id: String,
        description: Option<String>,
        parallel: bool,
        child_nodes: Vec<String>,
    ) -> NodeConfig {
        NodeConfig {
            id,
            r#type: NodeType::Group(GroupConfig {
                parallel,
                nodes: child_nodes,
            }),
            function: None,
            params: None,
            timeout_secs: 0,
            description,
            parallel,
            output: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_node(id: &str) -> WorkflowItem {
        WorkflowItem::Node(WorkflowNode {
            node: id.to_string(),
            desc: None,
            dependencies: None,
            tool: ToolDefinition {
                function: "test".to_string(),
                param: json!({}),
            },
            output: None,
        })
    }

    #[test]
    fn test_basic_parsing() {
        let json = r#"[{
            "node": "step1",
            "tool": {"function": "func1", "param": {}}
        }]"#;

        let (nodes, edges) = WorkflowParser::parse(json).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(edges.len(), 0);
        assert_eq!(nodes[0].id, "step1");
    }

    #[test]
    fn test_parse_simple_workflow() {
        let workflow_json = r#"[
            {
                "node": "step1",
                "desc": "First node",
                "tool": {
                    "function": "function1",
                    "param": { "key": "value" }
                }
            },
            {
                "node": "step2",
                "desc": "Second node",
                "dependencies": ["step1"],
                "tool": {
                    "function": "function2",
                    "param": { "key": "value" }
                }
            }
        ]"#;

        let (nodes, edges) = WorkflowParser::parse(workflow_json).unwrap();

        assert_eq!(nodes.len(), 2);
        assert_eq!(edges.len(), 1);

        // 验证节点
        let step1 = nodes.iter().find(|n| n.id == "step1").unwrap();
        assert_eq!(step1.function.as_ref().unwrap(), "function1");

        let step2 = nodes.iter().find(|n| n.id == "step2").unwrap();
        assert_eq!(step2.function.as_ref().unwrap(), "function2");

        // 验证边
        let edge = &edges[0];
        assert_eq!(edge.from, "step1");
        assert_eq!(edge.to, "step2");
    }

    #[test]
    fn test_parse_group_workflow() {
        let workflow_json = r#"[
            {
                "group": "parallel_group_1",
                "parallel": true,
                "desc": "基础数据查询",
                "nodes": [
                {
                    "node": "query_finance",
                    "desc": "获取近10年核心财务指标",
                    "tool": {
                    "function": "get_financial_data",
                    "param": {
                        "metrics": ["PE", "PB", "ROE", "净利润增长率", "毛利率", "资产负债率", "自由现金流"],
                        "period": "10y"
                    }
                    }
                },
                {
                    "node": "search_news",
                    "desc": "聚合近期新闻与舆情",
                    "tool": {
                    "function": "crawl_news",
                    "param": {
                        "keywords": ["股票名称", "所属行业"],
                        "time_range": "1y",
                        "sources": ["证监会", "财报", "并购", "政策"]
                    }
                    }
                }
                ]
            },
            {
                "group": "parallel_group_2",
                "parallel": true,
                "dependencies": ["parallel_group_1"],
                "nodes": [
                {
                    "node": "industry_policy",
                    "desc": "解析行业政策影响",
                    "dependencies": ["search_news"],
                    "tool": {
                        "function": "analyze_policy",
                        "param": {
                            "industry": "所属行业",
                            "scope": ["国家级", "地方级", "行业规范"]
                        }
                    }
                },
                {
                    "node": "technical_analysis",
                    "desc": "生成K线与技术指标",
                    "tool": {
                    "function": "plot_technical_chart",
                    "param": {
                        "indicators": ["K线", "MACD", "KDJ", "RSI", "布林带"],
                        "period": ["1D", "1W", "1M"]
                    }
                    }
                },
                {
                    "node": "capital_flow",
                    "desc": "监控资金流向与筹码",
                    "tool": {
                    "function": "track_capital",
                    "param": {
                        "data": ["北向资金", "主力净流入", "股东户数变化"]
                    }
                    }
                }
                ]
            },
            {
                "node": "valuation_compare",
                "desc": "横向与历史估值对比",
                "dependencies": ["query_finance"],
                "tool": {
                "function": "compare_valuation",
                "param": {
                    "benchmarks": ["行业平均PE", "历史分位数", "PEG"],
                    "current_pe": "${query_finance.metrics.PE}",
                    "growth_rate": "${query_finance.metrics.净利润增长率}"
                }
                }
            },
            {
                "node": "risk_check",
                "desc": "排查潜在黑天鹅风险",
                "dependencies": ["search_news"],
                "tool": {
                "function": "detect_risks",
                "param": {
                    "filters": ["股权质押", "商誉减值", "诉讼纠纷"],
                    "news_data": "${search_news.news}",
                    "sentiment": "当前舆情: ${search_news.sentiment}"
                }
                }
            },
            {
                "node": "synthesize",
                "desc": "生成综合买卖建议",
                "dependencies": ["parallel_group_2", "valuation_compare", "risk_check"],
                "tool": {
                "function": "generate_conclusion",
                "param": {
                    "weight": {
                    "基本面": 0.6,
                    "技术面": 0.3,
                    "风险": 0.1
                    }
                }
                }
            }
        ]"#;

        // 解析工作流JSON
        let (nodes, edges) = WorkflowParser::parse(workflow_json).unwrap();

        let _ = crate::workflow::WorkflowGraph::new(nodes.clone(), edges.clone()).unwrap();

        // 验证依赖关系
        println!("Edges:");
        for edge in &edges {
            println!("  {} -> {:?}", edge.from, edge.to);
        }

        println!("Nodes:");
        for node in &nodes {
            println!(
                "  {}: type={:?}, function={:?}",
                node.id, node.r#type, node.function
            );
        }

        // 验证节点数量
        // 8个普通节点:
        // - query_finance, search_news
        // - industry_policy, technical_analysis, capital_flow
        // - valuation_compare, risk_check, synthesize
        // 2个组节点: parallel_group_1, parallel_group_2
        assert_eq!(nodes.len(), 10);

        // 验证边的数量
        // 依赖关系:
        // 1. parallel_group_2 依赖 query_finance
        // 2. valuation_compare 依赖 query_finance
        // 3. risk_check 依赖 search_news
        // 4. synthesize 依赖 parallel_group_2, valuation_compare, risk_check
        // 总共至少7条边
        assert!(edges.len() >= 7);

        assert!(edges
            .iter()
            .any(|e| e.from == "parallel_group_1" && e.to == "parallel_group_2"));

        // 验证特定节点的存在
        let node_ids: Vec<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
        // 验证普通节点
        assert!(node_ids.contains(&"query_finance"));
        assert!(node_ids.contains(&"search_news"));
        assert!(node_ids.contains(&"industry_policy"));
        assert!(node_ids.contains(&"technical_analysis"));
        assert!(node_ids.contains(&"capital_flow"));
        assert!(node_ids.contains(&"valuation_compare"));
        assert!(node_ids.contains(&"risk_check"));
        assert!(node_ids.contains(&"synthesize"));
        // 验证组节点
        assert!(node_ids.contains(&"parallel_group_1"));
        assert!(node_ids.contains(&"parallel_group_2"));

        // 验证函数映射
        let function_map: std::collections::HashMap<&str, &str> = nodes
            .iter()
            .filter_map(|n| n.function.as_ref().map(|f| (n.id.as_str(), f.as_str())))
            .collect();

        assert_eq!(
            function_map.get("query_finance"),
            Some(&"get_financial_data")
        );
        assert_eq!(function_map.get("search_news"), Some(&"crawl_news"));
        assert_eq!(function_map.get("industry_policy"), Some(&"analyze_policy"));
        assert_eq!(
            function_map.get("technical_analysis"),
            Some(&"plot_technical_chart")
        );
        assert_eq!(function_map.get("capital_flow"), Some(&"track_capital"));
        assert_eq!(
            function_map.get("valuation_compare"),
            Some(&"compare_valuation")
        );
        assert_eq!(function_map.get("risk_check"), Some(&"detect_risks"));
        assert_eq!(function_map.get("synthesize"), Some(&"generate_conclusion"));

        let mut source_to_target: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for edge in &edges {
            source_to_target
                .entry(edge.from.clone())
                .or_insert_with(Vec::new)
                .push(edge.to.clone());
        }

        let empty_vec: Vec<String> = Vec::new();
        let parallel_group_2_deps = source_to_target.get("query_finance").unwrap_or(&empty_vec);
        assert!(parallel_group_2_deps.contains(&"valuation_compare".to_string()));

        let risk_check_deps = source_to_target.get("search_news").unwrap_or(&empty_vec);
        assert!(risk_check_deps.contains(&"industry_policy".to_string()));

        let synthesize_deps = edges
            .iter()
            .filter(|e| e.to.contains(&"synthesize".to_string()))
            .map(|e| e.from.clone())
            .collect::<Vec<String>>();

        assert!(synthesize_deps.contains(&"parallel_group_2".to_string()));
        assert!(synthesize_deps.contains(&"valuation_compare".to_string()));
        assert!(synthesize_deps.contains(&"risk_check".to_string()));
    }
}
