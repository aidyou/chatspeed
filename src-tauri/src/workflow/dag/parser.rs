//! # DAG Workflow Configuration Guide (JSON)
//!
//! ## Configuration Structure
//! ### Root Elements
//! A workflow consists of **nodes** (individual tasks), **loops** (iterative tasks), and **groups** (task containers):
//! - Nodes/groups are defined in a flat JSON array (`[...]`)
//! - Loops **do execute** the tools defined in the `functions` array
//! - Groups **do not execute tools directly** – they orchestrate node execution
//!
//! ### Node Definition (Executable Task)
//! ```json
//! {
//!   "id": "<unique_id>",          // Required
//!   "desc": "<description>",        // Optional
//!   "dependencies": ["id1", "id2"], // Optional (node/group IDs)
//!   "tool": {                       // Required (execution logic)
//!     "function": "<function_name>",
//!     "param": {                    // Supports value templates
//!       "key": "${source_node.output_field}"
//!     },
//!     "output": "<output_field>"     // Optional (output field). If multiple nodes have the same output field, their data will be merged together. If output is set, the data can be accessed directly via the output field name, e.g., `${search_result}`, without including the node name in the path.
//!   }
//! }
//! ```
//!
//!
//! ### Loop Node Definition (Iterative Task)
//! ```json
//! {
//!   "id": "<unique_id>",          // Required
//!   "desc": "<description>",        // Optional
//!   "dependencies": ["id1", "id2"], // Optional (node/group IDs)
//!   "loop": {
//!     "input": "${source_node}",      // Required (input data to iterate over)
//!     "functions": [                  // Required (list of functions to execute for each item)
//!       {
//!         "function": "<function_name>",
//!         "param": {                  // Supports value templates
//!           "key": "${item.field}"    // Use "item" to reference the current iteration item
//!         },
//!         "output": "<output_field>"     // Optional (output field).
//!       }
//!     ],
//!     "limit": 10,                   // Optional (maximum number of items to process)
//!     "filter": "${item.score > 0.5}" // Optional (condition to filter items)
//!   }
//! }
//! ```
//! filter Condition judgment supports compound comparison operators, including:
//! - Logical operators: `&&` (AND), `||` (OR)
//! - Comparison operators: `>=` (greater than or equal), `<=` (less than or equal), `!=` (not equal), `==` (equal), `>` (greater than), `<` (less than)
//! - Parentheses: for priority grouping
//!
//!
//! ### Group Definition (Task Container)
//! ```json
//! {
//!   "id": "<unique_id>",         // Required
//!   "parallel": true/false,         // Required
//!   "desc": "<description>",        // Optional
//!   "dependencies": ["id1", "id2"], // Optional (node/group IDs)
//!   "nodes": [/* Node Definitions */]  // Required (contains tools)
//! }
//! ```
//!
//! ## Critical Rules
//! 1. **Tool Ownership**:
//!    - Only **nodes** require a `tool` configuration
//!    - Groups **cannot** directly execute tools (they manage node execution)
//!
//! 2. **Dependency Scope**:
//!    - Groups can depend on other groups/nodes
//!    - Nodes can depend on other nodes/groups
//!    - Circular dependencies are prohibited
//!
//! 3. **Parameter Templating**:
//!    - References must point to **node outputs** (groups have no outputs)
//!    - Syntax: `${node_id.output_key}`
//!
//! ## Complete Example
//! ```json
//! [
//!   {
//!     "id": "data_fetching",
//!     "desc": "Fetch weather and transport data",
//!     "parallel": true,
//!     "nodes": [
//!       {
//!         "id": "fetch_weather",
//!         "tool": {
//!           "function": "weather_api",
//!           "param": {"location": "Tokyo"}
//!         }
//!       },
//!       {
//!         "id": "fetch_transport",
//!         "tool": {
//!           "function": "transport_api",
//!           "param": {"from": "Osaka", "to": "Tokyo"}
//!         }
//!       }
//!     ]
//!   },
//!   {
//!     "id": "plan_trip",
//!     "dependencies": ["data_fetching"],
//!     "tool": {
//!       "function": "trip_planner",
//!       "param": {
//!         "weather": "${fetch_weather.forecast}",
//!         "train_schedule": "${fetch_transport.timetable}"
//!       }
//!     }
//!   },
//!   {
//!     "id": "loop1",
//!     "dependencies": ["parallel_group_1"],
//!     "loop":{
//!         "input": "${parallel_group_1}",
//!         "functions": [
//!             {
//!                 "function": "function2",
//!                 "param": { "key": "${item.key}" }
//!             }
//!         ],
//!         "limit": 10,
//!         "filter": "${item.score > 0.5}"
//!     }
//!   }
//! ]
//!
//! For more details, refer to the workflow parser implementation in this module.

use lazy_static::lazy_static;
use regex::Regex;
use rust_i18n::t;
use std::collections::HashSet;

use super::config::{WorkflowGroup, WorkflowItem, WorkflowLoop, WorkflowNode};
use crate::workflow::dag::{
    config::{EdgeConfig, GroupConfig, NodeConfig, NodeType},
    types::WorkflowResult,
};
use crate::workflow::error::WorkflowError;

lazy_static! {
    static ref ID_REGEX: Regex = Regex::new(r"^[a-zA-Z][a-zA-Z0-9\-_]*$").unwrap();
}

pub struct WorkflowParser;

impl WorkflowParser {
    /// Parse workflow configuration and generate nodes and edges
    ///
    /// This method parses the workflow configuration from a JSON string and validates
    /// the nodes and edges. It also generates implicit edges for group nodes.
    ///
    /// # Arguments
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
        let workflow_items: Vec<WorkflowItem> =
            serde_json::from_str(workflow_json).map_err(|e| {
                WorkflowError::Config(t!("workflow.invalid_workflow_json", error = e).to_string())
            })?;

        Self::convert_to_dag(workflow_items)
    }

    /// Validate the format of a node or group ID and output
    ///
    /// This method ensures that the ID follows the required format:
    /// - Starts with a letter
    /// - Contains only alphanumeric characters, underscores (`_`), or hyphens (`-`)
    /// - Output must be a valid ID and not "item"
    ///
    /// # Arguments
    /// * `id` - The ID to validate
    /// * `output` - The output to validate
    ///
    /// # Returns
    /// * `WorkflowResult<()>` - Returns `Ok(())` if the ID is valid
    ///
    /// # Errors
    /// * Returns `WorkflowError` if the ID format is invalid
    fn validate_node(id: Option<&str>, output: Option<&String>) -> WorkflowResult<()> {
        if let Some(id) = id {
            if !ID_REGEX.is_match(id) {
                return Err(WorkflowError::Config(
                    t!("workflow.invalid_node_id", id = id).to_string(),
                ));
            }
        }
        if let Some(output) = output {
            if !ID_REGEX.is_match(&output) || output == "item" {
                return Err(WorkflowError::Config(
                    t!(
                        "workflow.invalid_node_output",
                        id = id.unwrap_or(""),
                        output = output
                    )
                    .to_string(),
                ));
            }
        }
        Ok(())
    }

    /// Collect and validate all node and group IDs in the workflow
    ///
    /// This method ensures that all IDs are unique and follow the required format.
    ///
    /// # Arguments
    /// * `items` - A slice of `WorkflowItem` containing nodes and groups
    ///
    /// # Returns
    /// * `WorkflowResult<HashSet<String>>` - Returns a set of all valid IDs
    ///
    /// # Errors
    /// * Returns `WorkflowError` if:
    ///   - An ID format is invalid
    ///   - Duplicate IDs are found
    fn collect_and_validate_node(items: &[WorkflowItem]) -> WorkflowResult<HashSet<String>> {
        let mut node_ids = HashSet::new();

        for item in items {
            match item {
                WorkflowItem::Node(node) => {
                    Self::validate_node(Some(&node.id), node.tool.output.as_ref())?;
                    if !node_ids.insert(node.id.clone()) {
                        return Err(WorkflowError::Config(
                            t!("workflow.duplicate_node_id", id = node.id).to_string(),
                        ));
                    }
                }
                WorkflowItem::Loop(ref loop_item) => {
                    Self::validate_node(Some(&loop_item.id), None)?;
                    if !node_ids.insert(loop_item.id.clone()) {
                        return Err(WorkflowError::Config(
                            t!("workflow.duplicate_node_id", id = loop_item.id).to_string(),
                        ));
                    }
                    // check every loop function output
                    for func in &loop_item.r#loop.functions {
                        Self::validate_node(None, func.output.as_ref())?;
                    }
                }
                WorkflowItem::Group(group) => {
                    Self::validate_node(Some(&group.id), None)?;
                    if !node_ids.insert(group.id.clone()) {
                        return Err(WorkflowError::Config(
                            t!("workflow.duplicate_node_id", id = group.id).to_string(),
                        ));
                    }

                    for item in &group.nodes {
                        match item {
                            WorkflowItem::Node(node) => {
                                Self::validate_node(Some(&node.id), node.tool.output.as_ref())?;
                                if !node_ids.insert(node.id.clone()) {
                                    return Err(WorkflowError::Config(
                                        t!("workflow.duplicate_node_id", id = node.id).to_string(),
                                    ));
                                }
                            }
                            WorkflowItem::Group(_) => {
                                // Group nodes cannot be nested.
                                return Err(WorkflowError::Config(
                                    t!("workflow.nested_group", id = group.id).to_string(),
                                ));
                            }
                            WorkflowItem::Loop(loop_item) => {
                                Self::validate_node(Some(&loop_item.id), None)?;
                                if !node_ids.insert(loop_item.id.clone()) {
                                    return Err(WorkflowError::Config(
                                        t!("workflow.duplicate_node_id", id = loop_item.id)
                                            .to_string(),
                                    ));
                                }
                                // check every loop function output
                                for func in &loop_item.r#loop.functions {
                                    Self::validate_node(None, func.output.as_ref())?;
                                }
                            }
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
    /// # Arguments
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
                return Err(WorkflowError::Config(
                    t!("workflow.dependency_not_found", id = dep, node_id = node_id).to_string(),
                ));
            }

            if dep == node_id {
                return Err(WorkflowError::Config(
                    t!("workflow.self_dependency", node_id = node_id).to_string(),
                ));
            }

            // 检查边是否已经存在，避免重复添加
            let edge = EdgeConfig {
                from: dep.clone(),
                to: node_id.to_string(),
            };

            if !edges.iter().any(|e| e.from == edge.from && e.to == edge.to) {
                edges.push(edge);
            }
        }
        Ok(())
    }

    /// Convert workflow items into a directed acyclic graph (DAG)
    ///
    /// This method processes nodes and groups, validates their dependencies,
    /// and generates the final DAG structure.
    ///
    /// # Arguments
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
        let node_ids = Self::collect_and_validate_node(&items)?;
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
                WorkflowItem::Loop(loop_item) => {
                    Self::create_loop_config(loop_item, &node_ids, &mut edges, &mut nodes)?;
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
    /// # Arguments
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
        let group_id = group.id.clone();
        let is_parallel = group.parallel;
        let group_desc = group.desc.clone();
        let dependencies = group.dependencies.clone();

        // 收集子节点ID（必须在消费group.nodes前完成）
        let mut child_nodes = Vec::new();
        for item in &group.nodes {
            match item {
                WorkflowItem::Node(node) => child_nodes.push(node.id.clone()),
                WorkflowItem::Group(_) => {
                    // 不允许组嵌套
                    return Err(WorkflowError::Config(
                        t!("workflow.nested_group", id = group_id).to_string(),
                    ));
                }
                WorkflowItem::Loop(loop_item) => child_nodes.push(loop_item.id.clone()),
            }
        }

        // Generate bidirectional edges for group → child nodes
        for item in &group.nodes {
            // 组 → 子节点（子节点依赖组）
            let node_id = match item {
                WorkflowItem::Node(node) => node.id.clone(),
                WorkflowItem::Group(_) => {
                    // 不允许组嵌套
                    return Err(WorkflowError::Config(
                        t!("workflow.nested_group", id = group_id).to_string(),
                    ));
                }
                WorkflowItem::Loop(loop_item) => loop_item.id.clone(),
            };

            // 检查边是否已经存在，避免重复添加
            let edge = EdgeConfig {
                from: node_id,
                to: group_id.clone(),
            };

            if !edges.iter().any(|e| e.from == edge.from && e.to == edge.to) {
                edges.push(edge);
            }
        }

        // 处理顺序组链式依赖（消费group.nodes所有权）
        // Handle sequential dependencies for non-parallel groups
        let mut prev_node_id = None;
        for item in group.nodes {
            // 获取当前节点ID
            let node_id = match &item {
                WorkflowItem::Node(node) => node.id.clone(),
                WorkflowItem::Group(_) => {
                    // 不允许组嵌套
                    return Err(WorkflowError::Config(
                        t!("workflow.nested_group", id = group_id).to_string(),
                    ));
                }
                WorkflowItem::Loop(loop_item) => loop_item.id.clone(),
            };

            // 处理顺序依赖
            if !is_parallel {
                if let Some(prev_id) = prev_node_id {
                    // 根据节点类型添加依赖
                    match item {
                        WorkflowItem::Node(mut node) => {
                            node.dependencies.get_or_insert_with(Vec::new).push(prev_id);
                            Self::process_node(node, node_ids, edges, nodes)?;
                        }
                        WorkflowItem::Group(_) => {
                            // 不允许组嵌套
                            return Err(WorkflowError::Config(
                                t!("workflow.nested_group", id = group_id).to_string(),
                            ));
                        }
                        WorkflowItem::Loop(mut loop_item) => {
                            loop_item
                                .dependencies
                                .get_or_insert_with(Vec::new)
                                .push(prev_id);
                            Self::create_loop_config(loop_item, node_ids, edges, nodes)?;
                        }
                    }
                } else {
                    // 处理第一个节点（没有前置依赖）
                    match item {
                        WorkflowItem::Node(node) => {
                            Self::process_node(node, node_ids, edges, nodes)?
                        }
                        WorkflowItem::Group(_) => {
                            // 不允许组嵌套
                            return Err(WorkflowError::Config(
                                t!("workflow.nested_group", id = group_id).to_string(),
                            ));
                        }
                        WorkflowItem::Loop(loop_item) => {
                            Self::create_loop_config(loop_item, node_ids, edges, nodes)?
                        }
                    }
                }
                prev_node_id = Some(node_id);
            } else {
                // 并行组，直接处理节点
                match item {
                    WorkflowItem::Node(node) => Self::process_node(node, node_ids, edges, nodes)?,
                    WorkflowItem::Group(_) => {
                        // 不允许组嵌套
                        return Err(WorkflowError::Config(
                            t!("workflow.nested_group", id = group_id).to_string(),
                        ));
                    }
                    WorkflowItem::Loop(loop_item) => {
                        Self::create_loop_config(loop_item, node_ids, edges, nodes)?
                    }
                }
            }
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
    /// # Arguments
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
            Self::process_dependencies(&node.id, deps, node_ids, edges)?;
        }

        nodes.push(NodeConfig {
            id: node.id,
            r#type: NodeType::Task(node.tool),
            timeout_secs: 0,
            description: node.desc,
        });
        Ok(())
    }

    /// Process a loop node and add it to the graph
    ///
    /// This method handles loop-specific logic, including dependency validation
    /// and node configuration creation for loop operations.
    ///
    /// # Arguments
    /// * `loop_item` - The `WorkflowLoop` to process
    /// * `node_ids` - A set of all valid node and group IDs
    /// * `edges` - A mutable reference to the vector of edges
    /// * `nodes` - A mutable reference to the vector of node configurations
    ///
    /// # Returns
    /// * `WorkflowResult<()>` - Returns `Ok(())` if the loop is processed successfully
    ///
    /// # Errors
    /// * Returns `WorkflowError` if any validation fails
    fn create_loop_config(
        loop_item: WorkflowLoop,
        node_ids: &HashSet<String>,
        edges: &mut Vec<EdgeConfig>,
        nodes: &mut Vec<NodeConfig>,
    ) -> WorkflowResult<()> {
        // Process dependencies for the loop node
        if let Some(deps) = &loop_item.dependencies {
            Self::process_dependencies(&loop_item.id, deps, node_ids, edges)?;
        }

        // Create loop node configuration
        nodes.push(NodeConfig {
            id: loop_item.id.clone(),
            r#type: NodeType::Loop(loop_item.r#loop),
            timeout_secs: 0,
            description: loop_item.desc,
        });

        Ok(())
    }

    /// Create a group configuration from group properties
    ///
    /// This method converts group properties into a `NodeConfig` with a `GroupConfig`.
    ///
    /// # Arguments
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
            timeout_secs: 0,
            description,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{logger, workflow::dag::config::ToolConfig};

    use super::*;
    use serde_json::json;

    #[test]
    fn test_basic_parsing() {
        let json = r#"[
            {
                "id": "step1",
                "desc": "First node",
                "tool": {
                    "function": "function1",
                    "param": { "key": "value" }
                }
            }
        ]"#;

        let (nodes, edges) = WorkflowParser::parse(json).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(edges.len(), 0);
        assert_eq!(nodes[0].id, "step1");

        let json = r#"[
            {
                "id": "step2",
                "desc": "Second node",
                "dependencies": ["step1"],
                "tool": {
                    "function": "function2",
                    "param": { "key": "value" },
                    "output": "item"
                }
            }
        ]"#;

        // output field can't be "item"
        let r = WorkflowParser::parse(json);
        assert!(r.is_err());

        let json = r#"[
            {
                "id": "step2",
                "desc": "Second node",
                "dependencies": ["step1"],
                "tool": {
                    "function": "function2",
                    "param": { "key": "value" },
                    "output": "_item"
                }
            }
        ]"#;

        // output field can't be start with "_"
        let r = WorkflowParser::parse(json);
        assert!(r.is_err());
    }

    #[test]
    fn test_parse_simple_workflow() {
        let workflow_json = r#"[
            {
                "id": "step1",
                "desc": "First node",
                "tool": {
                    "function": "function1",
                    "param": { "key": "value" }
                }
            },
            {
                "id": "step2",
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
        assert_eq!(
            step1.r#type,
            NodeType::Task(ToolConfig {
                function: "function1".to_string(),
                param: json!({"key": "value"}),
                output: None,
            })
        );

        let step2 = nodes.iter().find(|n| n.id == "step2").unwrap();
        assert_eq!(
            step2.r#type,
            NodeType::Task(ToolConfig {
                function: "function2".to_string(),
                param: json!({"key": "value"}),
                output: None,
            })
        );

        // 验证边
        let edge = &edges[0];
        assert_eq!(edge.from, "step1");
        assert_eq!(edge.to, "step2");
    }

    #[test]
    fn test_parse_group_workflow() {
        let _ = logger::setup_test_logger();

        let workflow_json = r#"[
            {
                "id": "parallel_group_1",
                "parallel": true,
                "desc": "旅行基础信息查询",
                "nodes": [
                    {
                        "id": "query_weather",
                        "desc": "获取目的地未来7天天气预报",
                        "tool": {
                            "function": "get_weather_forecast",
                            "param": {
                                "location": "目的地名称",
                                "period": "7d"
                            }
                        }
                    },
                    {
                        "id": "search_attractions",
                        "desc": "查询目的地热门景点",
                        "tool": {
                            "function": "crawl_attractions",
                            "param": {
                                "location": "目的地名称",
                                "categories": ["自然景观", "历史遗迹", "主题公园"]
                            }
                        }
                    }
                ]
            },
            {
                "id": "parallel_group_2",
                "parallel": true,
                "dependencies": ["parallel_group_1"],
                "nodes": [
                    {
                        "id": "plan_itinerary",
                        "desc": "生成每日行程安排",
                        "dependencies": ["search_attractions"],
                        "tool": {
                            "function": "generate_itinerary",
                            "param": {
                                "attractions": "${search_attractions.results}",
                                "days": "旅行天数"
                            }
                        }
                    },
                    {
                        "id": "book_accommodation",
                        "desc": "预订住宿",
                        "tool": {
                            "function": "book_hotel",
                            "param": {
                                "location": "目的地名称",
                                "check_in": "入住日期",
                                "check_out": "离店日期",
                                "preferences": ["价格范围", "星级", "设施"]
                            }
                        }
                    },
                    {
                        "id": "arrange_transport",
                        "desc": "安排交通",
                        "tool": {
                            "function": "book_transport",
                            "param": {
                                "origin": "出发地",
                                "destination": "目的地",
                                "date": "出发日期",
                                "preferences": ["航班", "火车", "租车"]
                            }
                        }
                    }
                ]
            },
            {
                "id": "check_budget",
                "desc": "核对旅行预算",
                "dependencies": ["book_accommodation", "arrange_transport"],
                "tool": {
                    "function": "calculate_budget",
                    "param": {
                        "accommodation_cost": "${book_accommodation.total_cost}",
                        "transport_cost": "${arrange_transport.total_cost}",
                        "daily_budget": "每日预算"
                    }
                }
            },
            {
                "id": "packing_list",
                "desc": "生成行李清单",
                "dependencies": ["query_weather"],
                "tool": {
                    "function": "generate_packing_list",
                    "param": {
                        "weather_data": "${query_weather.forecast}",
                        "duration": "旅行天数",
                        "activities": ["徒步", "游泳", "观光"]
                    }
                }
            },
            {
                "id": "finalize_plan",
                "desc": "生成最终旅行计划",
                "dependencies": ["plan_itinerary", "check_budget", "packing_list"],
                "tool": {
                    "function": "generate_travel_plan",
                    "param": {
                        "sections": {
                            "行程安排": "${plan_itinerary.itinerary}",
                            "预算": "${check_budget.total_cost}",
                            "行李清单": "${packing_list.items}"
                        }
                    }
                }
            }
        ]"#;

        // 解析工作流JSON
        let (nodes, edges) = WorkflowParser::parse(workflow_json).unwrap();

        let _ =
            crate::workflow::dag::graph::WorkflowGraph::new(nodes.clone(), edges.clone()).unwrap();

        // 验证依赖关系
        println!("Edges:");
        for edge in &edges {
            println!("  {} -> {:?}", edge.from, edge.to);
        }

        println!("Nodes:");
        for node in &nodes {
            match &node.r#type {
                NodeType::Task(tool) => {
                    println!("  {}: type=Task, function={}", node.id, tool.function)
                }
                NodeType::Loop(loop_config) => {
                    println!("  {}: type=Loop, limit={:?}", node.id, loop_config.limit)
                }
                NodeType::Group(group_config) => println!(
                    "  {}: type=Group, parallel={}",
                    node.id, group_config.parallel
                ),
            }
        }

        // 验证节点数量
        // 8个普通节点:
        // - query_weather, search_attractions
        // - plan_itinerary, book_accommodation, arrange_transport
        // - check_budget, packing_list, finalize_plan
        // 2个组节点: parallel_group_1, parallel_group_2
        assert_eq!(nodes.len(), 10);

        // 验证边的数量
        // 依赖关系:
        // 1. parallel_group_2 依赖 parallel_group_1
        // 2. plan_itinerary 依赖 search_attractions
        // 3. check_budget 依赖 book_accommodation, arrange_transport
        // 4. packing_list 依赖 query_weather
        // 5. finalize_plan 依赖 plan_itinerary, check_budget, packing_list
        // 总共至少7条边
        assert!(edges.len() >= 7);

        assert!(edges
            .iter()
            .any(|e| e.from == "parallel_group_1" && e.to == "parallel_group_2"));

        // 验证特定节点的存在
        let node_ids: Vec<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
        // 验证普通节点
        assert!(node_ids.contains(&"query_weather"));
        assert!(node_ids.contains(&"search_attractions"));
        assert!(node_ids.contains(&"plan_itinerary"));
        assert!(node_ids.contains(&"book_accommodation"));
        assert!(node_ids.contains(&"arrange_transport"));
        assert!(node_ids.contains(&"check_budget"));
        assert!(node_ids.contains(&"packing_list"));
        assert!(node_ids.contains(&"finalize_plan"));
        // 验证组节点
        assert!(node_ids.contains(&"parallel_group_1"));
        assert!(node_ids.contains(&"parallel_group_2"));

        // 验证函数映射
        let function_map: std::collections::HashMap<&str, String> = nodes
            .iter()
            .filter_map(|n| match &n.r#type {
                NodeType::Task(tool) => Some((n.id.as_str(), tool.function.clone())),
                NodeType::Loop(loop_config) => Some((
                    n.id.as_str(),
                    loop_config
                        .functions
                        .iter()
                        .map(|f| f.function.as_str())
                        .collect::<Vec<_>>()
                        .join(","),
                )),
                _ => None,
            })
            .collect();

        assert_eq!(
            function_map.get("query_weather"),
            Some("get_weather_forecast".to_string()).as_ref()
        );
        assert_eq!(
            function_map.get("search_attractions"),
            Some("crawl_attractions".to_string()).as_ref()
        );
        assert_eq!(
            function_map.get("plan_itinerary"),
            Some("generate_itinerary".to_string()).as_ref()
        );
        assert_eq!(
            function_map.get("book_accommodation"),
            Some("book_hotel".to_string()).as_ref()
        );
        assert_eq!(
            function_map.get("arrange_transport"),
            Some("book_transport".to_string()).as_ref()
        );
        assert_eq!(
            function_map.get("check_budget"),
            Some("calculate_budget".to_string()).as_ref()
        );
        assert_eq!(
            function_map.get("packing_list"),
            Some("generate_packing_list".to_string()).as_ref()
        );
        assert_eq!(
            function_map.get("finalize_plan"),
            Some("generate_travel_plan".to_string()).as_ref()
        );

        let mut source_to_target: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for edge in &edges {
            source_to_target
                .entry(edge.from.clone())
                .or_insert_with(Vec::new)
                .push(edge.to.clone());
        }

        let empty_vec: Vec<String> = Vec::new();
        let plan_itinerary_deps = source_to_target
            .get("search_attractions")
            .unwrap_or(&empty_vec);
        assert!(plan_itinerary_deps.contains(&"plan_itinerary".to_string()));

        let check_budget_deps = source_to_target
            .get("book_accommodation")
            .unwrap_or(&empty_vec);
        assert!(check_budget_deps.contains(&"check_budget".to_string()));

        let finalize_plan_deps = edges
            .iter()
            .filter(|e| e.to.contains(&"finalize_plan".to_string()))
            .map(|e| e.from.clone())
            .collect::<Vec<String>>();

        assert!(finalize_plan_deps.contains(&"plan_itinerary".to_string()));
        assert!(finalize_plan_deps.contains(&"check_budget".to_string()));
        assert!(finalize_plan_deps.contains(&"packing_list".to_string()));
    }

    #[test]
    fn test_parse_workflow_with_loop() {
        let workflow_json = r#"[
            {
                "id": "parallel_group_1",
                "parallel": true,
                "desc": "旅行基础信息查询",
                "nodes": [
                    {
                        "id": "step1",
                        "desc": "First node",
                        "tool": {
                            "function": "function1",
                            "param": { "key": "value" }
                        }
                    },
                    {
                        "id": "step2",
                        "desc": "First node",
                        "tool": {
                            "function": "function1",
                            "param": { "key": "value" }
                        }
                    }
                ]
            },
            {
                "id": "loop1",
                "dependencies": ["parallel_group_1"],
                "desc": "Loop node",
                "loop": {
                    "input": "${parallel_group_1}",
                    "functions": [
                        {
                            "function": "function2",
                            "param": { "key": "${item.key}" }
                        }
                    ],
                    "limit": 10,
                    "filter": "${item.score > 0.5}"
                }
            }
        ]"#;

        let (nodes, edges) = WorkflowParser::parse(workflow_json).unwrap();
        assert_eq!(nodes.len(), 4);
        assert_eq!(edges.len(), 3);

        for node in &nodes {
            log::debug!("Node: {:?}", node.id);
        }

        for edge in &edges {
            log::debug!("Edge: {:?} -> {:?}", edge.from, edge.to);
        }

        // 验证循环节点
        match nodes[3].r#type.clone() {
            NodeType::Loop(loop_config) => {
                dbg!(&loop_config);
                assert_eq!(nodes[3].id, "loop1");
                assert_eq!(loop_config.functions.len(), 1);
                assert_eq!(loop_config.limit, Some(10));
                assert_eq!(loop_config.filter.as_deref(), Some("${item.score > 0.5}"));
            }
            _ => panic!("Expected loop node, find: {:?}", nodes[3].r#type),
        }
    }
}
