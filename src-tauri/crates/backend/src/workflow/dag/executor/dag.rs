//! Workflow Executor Core
//!
//! This module implements the core functionality of the workflow executor.
//! It manages the execution of tasks in a directed acyclic graph (DAG) structure.
//!
//! # Execution Flow
//!
//! The workflow executor follows these steps:
//!
//! 1. Initialization Phase
//!    +-------------------+
//!    | Parse Workflow    |
//!    | Configuration     |
//!    +---------+---------+
//!              |
//!              v
//!    +-------------------+
//!    | Build Task Graph  |
//!    +---------+---------+
//!              |
//!              v
//!    +-------------------+
//!    | Initialize State  |
//!    +---------+---------+
//!              |
//!              v
//! 2. Execution Phase
//!    +-------------------+
//!    | Schedule Tasks    |
//!    +---------+---------+
//!              |
//!              v
//!    +-------------------+
//!    | Monitor Progress  |
//!    +---------+---------+
//!              |
//!              v
//!    +-------------------+
//!    | Handle Completion |
//!    +---------+---------+
//!              |
//!              v
//! 3. Completion Phase
//!    +-------------------+
//!    | Collect Results   |
//!    +---------+---------+
//!              |
//!              v
//!    +-------------------+
//!    | Cleanup Resources |
//!    +---------+---------+
//!              |
//!              v
//!    +-------------------+
//!    | Generate Report   |
//!    +-------------------+
//!
//! # Key Components
//! - WorkflowExecutor: Main executor struct
//! - TaskDispatcher: Handles task scheduling and execution
//! - StateManager: Manages workflow state
//! - ErrorHandler: Processes errors and retries
//!
//! # Error Handling
//! The executor implements robust error handling with:
//! - Automatic retries for transient failures
//! - Circuit breakers for persistent failures
//! - Detailed error reporting
//!
//! # Performance Considerations
//! - Asynchronous task execution
//! - Parallel processing of independent tasks
//! - Efficient resource utilization
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
    time::Duration,
};

use chrono::Utc;
use log::{debug, error, info, warn};
use rust_i18n::t;
use serde_json::Value;
use tokio::{
    sync::{Mutex, RwLock, Semaphore},
    task::JoinSet,
    time,
};

use crate::{tools::ToolDefinition, workflow::error::WorkflowError};
use crate::{
    tools::ToolManager,
    workflow::dag::{
        config::{LoopConfig, NodeConfig, NodeType, ToolConfig},
        context::{Context, NodeState},
        executor::channel::{SharedChannel, WatchChannel},
        graph::WorkflowGraph,
        types::{WorkflowResult, WorkflowState},
    },
};

/// 工作流执行器，负责基于 DAG 的任务调度和执行
#[derive(Clone)]
pub struct WorkflowExecutor {
    context: Arc<Context>,
    /// DAG 图构建器
    graph: Arc<WorkflowGraph>,
    /// 函数管理器
    tool_manager: Arc<ToolManager>,
    /// 最大并行任务数
    max_parallel: usize,
    /// 工作流状态
    state: Arc<RwLock<WorkflowState>>,
    /// 状态通道
    state_channel: WatchChannel<WorkflowState>,
    /// 暂停通道
    pause_channel: SharedChannel<()>,
    /// 恢复通道
    resume_channel: SharedChannel<()>,
    /// 取消通道
    cancel_channel: SharedChannel<()>,
    /// 可运行的节点队列
    ready_queue: Arc<Mutex<VecDeque<NodeConfig>>>,
    /// 记录已完成的节点
    completed_nodes: Arc<Mutex<HashSet<String>>>,
}

impl WorkflowExecutor {
    /// Create a new workflow executor
    ///
    /// This method initializes a new workflow executor with the given context, function manager,
    /// maximum parallel tasks, and workflow graph. It also sets up the necessary channels for
    /// state management and control signals.
    ///
    /// # Arguments
    /// * `context` - The context of the workflow
    /// * `function_manager` - The function manager
    /// * `max_parallel` - The maximum number of parallel tasks
    /// * `graph` - The workflow graph
    ///
    /// # Returns
    /// * `WorkflowResult<Self>` - The created workflow executor
    ///
    /// # Errors
    /// * Returns `WorkflowError` if:
    ///   - The graph is invalid
    ///   - The context is not properly initialized
    pub fn create(
        context: Arc<Context>,
        tool_manager: Arc<ToolManager>,
        max_parallel: usize,
        graph: Arc<WorkflowGraph>,
    ) -> WorkflowResult<Self> {
        let state_channel = WatchChannel::new(WorkflowState::Init);
        let pause_channel = SharedChannel::new();
        let resume_channel = SharedChannel::new();
        let cancel_channel = SharedChannel::new();

        Ok(Self {
            context,
            graph,
            tool_manager,
            state: Arc::new(RwLock::new(WorkflowState::Init)),
            state_channel,
            max_parallel,
            pause_channel,
            resume_channel,
            cancel_channel,
            ready_queue: Arc::new(Mutex::new(VecDeque::new())),
            completed_nodes: Arc::new(Mutex::new(HashSet::new())),
        })
    }

    /// Execute the workflow based on the DAG algorithm
    ///
    /// This method implements a parallel task scheduler using a directed acyclic graph (DAG).
    /// It performs the following steps:
    /// 1. Validates the execution state
    /// 2. Initializes the execution context
    /// 3. Processes tasks in topological order
    /// 4. Manages task dependencies and parallel execution
    /// 5. Ensures all tasks are completed
    ///
    /// # Algorithm Details
    /// - Uses topological sorting to determine task execution order
    /// - Limits parallel execution using a semaphore
    /// - Tracks task completion using a JoinSet
    ///
    /// # Returns
    /// * `WorkflowResult<()>` - Returns `Ok(())` if all tasks are executed successfully,
    ///   otherwise returns an error with details
    ///
    /// # Errors
    /// * Returns `WorkflowError` if:
    ///   - The workflow is in an invalid state
    ///   - Any task fails to execute
    ///   - Task dependencies cannot be resolved
    pub async fn execute(&mut self) -> WorkflowResult<()> {
        // state check
        self.validate_execution_state().await?;

        let graph = self.graph.clone();
        info!(
            "Starting workflow execution with {} nodes",
            graph.node_count()
        );

        #[cfg(debug_assertions)]
        debug!(
            "Workflow nodes: {:?}",
            graph.nodes().map(|n| &n.id).collect::<Vec<_>>()
        );

        if graph.node_count() == 0 {
            warn!("Workflow has no nodes");
            self.update_state(WorkflowState::Completed).await?;
            return Ok(());
        }

        #[cfg(debug_assertions)]
        {
            let edges = self.graph.edges();
            for edge in edges {
                debug!("Edge: {} -> {}", edge.from, edge.to);
            }
        }

        // transition to running state
        self.update_state(WorkflowState::Running).await?;

        // prepare execution context
        let nodes = graph.nodes().cloned().collect::<Vec<_>>();

        // update context node state to Pending (waiting for execution)
        for node in &nodes {
            self.context
                .update_node_state(&node.id, NodeState::Pending)
                .await;
        }

        // initialize execution queue
        {
            let mut ready_queue = self.ready_queue.lock().await;
            *ready_queue = self.initialize_ready_queue(&nodes).await;
        }

        #[cfg(debug_assertions)]
        {
            let ready_queue = self.ready_queue.lock().await;
            debug!("Initial ready queue size: {}", ready_queue.len());
            if !ready_queue.is_empty() {
                debug!(
                    "Ready queue nodes: {:?}",
                    ready_queue.iter().map(|n| &n.id).collect::<Vec<_>>()
                );
            }
        }

        // parallel control
        let semaphore = Arc::new(Semaphore::new(self.max_parallel));

        // task communication channel
        let mut task_set = JoinSet::new();

        // main execution loop
        while {
            let queue_empty = self.ready_queue.lock().await.is_empty();
            !queue_empty || task_set.len() > 0
        } {
            // process control signals
            self.process_control_signals().await?;

            // check task results
            let tasks_to_dispatch = {
                let mut queue = self.ready_queue.lock().await;
                queue.drain(..).collect::<Vec<_>>()
            };

            // dispatch new tasks
            self.dispatch_tasks(tasks_to_dispatch, &semaphore, &mut task_set)
                .await?;

            // wait for task completion
            self.wait_for_task_completion(&mut task_set).await?;
        }

        // final check
        self.finalize_execution(&nodes).await
    }

    /// Validate the execution state
    ///
    /// This method checks if the workflow is in a valid state to start execution.
    /// It ensures that the workflow is not already running or in an invalid state.
    ///
    /// # Returns
    /// * `WorkflowResult<()>` - Returns `Ok(())` if the state is valid,
    ///   otherwise returns an error with details
    ///
    /// # Errors
    /// * Returns `WorkflowError` if:
    ///   - The workflow is already running
    ///   - The workflow is in an invalid state
    async fn validate_execution_state(&self) -> WorkflowResult<()> {
        let state = self.state.read().await;
        if !matches!(*state, WorkflowState::Init | WorkflowState::Ready) {
            return Err(WorkflowError::InvalidState(
                t!("workflow.invalid_state").to_string(),
            ));
        }
        Ok(())
    }

    async fn initialize_ready_queue(&self, nodes: &[NodeConfig]) -> VecDeque<NodeConfig> {
        let in_degree_map = self.graph.in_degree_map().lock().await.clone();

        #[cfg(debug_assertions)]
        debug!(
            "Initializing ready queue with in-degree map: {:?}",
            in_degree_map
        );

        let ready_nodes = nodes
            .iter()
            .filter(|n| in_degree_map.get(&n.id).copied().unwrap_or(1) == 0)
            .cloned()
            .collect::<Vec<_>>();

        #[cfg(debug_assertions)]
        debug!(
            "Initial ready nodes: {:?}",
            ready_nodes.iter().map(|n| &n.id).collect::<Vec<_>>()
        );

        VecDeque::from(ready_nodes)
    }

    /// Dispatch new tasks
    ///
    /// This method dispatches new tasks from the ready queue, respecting the maximum parallel
    /// task limit. It also sets up the necessary context for each task.
    ///
    /// # Arguments
    /// * `ready_queue` - The queue of ready tasks
    /// * `semaphore` - The semaphore for parallel control
    /// * `task_result_tx` - The sender channel for task results
    /// * `task_set` - The set of running tasks
    ///
    /// # Returns
    /// * `WorkflowResult<()>` - Returns `Ok(())` if all tasks are dispatched successfully,
    ///   otherwise returns an error with details
    ///
    /// # Errors
    /// * Returns `WorkflowError` if:
    ///   - The ready queue is empty
    ///   - The semaphore cannot be acquired
    async fn dispatch_tasks(
        &self,
        tasks: Vec<NodeConfig>,
        semaphore: &Arc<Semaphore>,
        task_set: &mut JoinSet<WorkflowResult<()>>,
    ) -> WorkflowResult<()> {
        for node in tasks {
            let semaphore_clone = semaphore.clone();

            // 准备任务上下文
            let node_id = node.id.clone();
            let self_ref = self.clone();

            // 生成异步任务
            task_set.spawn(async move {
                let start_time = Utc::now();
                debug!("Starting node execution: {}", node_id);

                // 执行节点逻辑
                let result = match node.r#type {
                    NodeType::Group(_) => {
                        // 立即释放信号量，仅用于触发子节点入队
                        let _ = semaphore_clone.acquire().await;
                        self_ref.execute_group_node(node).await
                    }
                    _ => {
                        // 持有信号量直到异步任务完成（通过变量绑定）
                        let _hold_permit = semaphore_clone.acquire_owned().await;
                        self_ref.execute_task_node_with_retry(node).await
                    }
                };

                // 处理任务结果
                self_ref
                    .handle_task_result(result, start_time, &node_id)
                    .await
            });
        }
        Ok(())
    }

    /// Handle task result
    ///
    /// This method processes the result of a completed task, updates the node state,
    /// and handles any errors that occurred during task execution.
    ///
    /// # Arguments
    /// * `result` - The result of the task
    /// * `start_time` - The start time of the task
    /// * `node_id` - The ID of the node
    ///
    /// # Returns
    /// * `WorkflowResult<()>` - Returns `Ok(())` if the task result is processed successfully,
    ///   otherwise returns an error with details
    ///
    /// # Errors
    /// * Returns `WorkflowError` if:
    ///   - The task result is invalid
    ///   - The node state cannot be updated
    async fn handle_task_result(
        &self,
        result: WorkflowResult<()>,
        start_time: chrono::DateTime<Utc>,
        node_id: &str,
    ) -> WorkflowResult<()> {
        match result {
            Ok(()) => {
                // 计算执行耗时
                let duration = Utc::now() - start_time;
                info!(
                    "Node {} completed in {} ms",
                    node_id,
                    duration.num_milliseconds()
                );

                self.context
                    .update_node_state(&node_id, NodeState::Completed)
                    .await;

                // 获取所有后继节点（显式 + 隐式）
                let mut successors = self.graph.successors(node_id).to_vec();
                // 确保没有重复项
                successors.sort_unstable();
                successors.dedup();

                self.process_completed_tasks(node_id.to_string(), successors)
                    .await?;
                Ok(())
            }
            Err(e) => {
                error!("Node {} failed: {}", node_id, e);

                self.context
                    .update_node_with_count(&node_id, NodeState::Failed(1))
                    .await;
                Err(e)
            }
        }
    }

    /// Process completed tasks
    ///
    /// This method handles the results of completed tasks, updates the task dependencies,
    /// and prepares the next set of tasks for execution.
    ///
    /// # Arguments
    /// * `task_result_rx` - The receiver channel for task results
    /// * `in_degree_map` - The map of node in-degrees
    /// * `ready_queue` - The queue of ready tasks
    ///
    /// # Returns
    /// * `WorkflowResult<()>` - Returns `Ok(())` if all completed tasks are processed successfully,
    ///   otherwise returns an error with details
    ///
    /// # Errors
    /// * Returns `WorkflowError` if:
    ///   - Any task result is invalid
    ///   - Task dependencies cannot be updated
    async fn process_completed_tasks(
        &self,
        node_id: String,
        successors: Vec<String>,
    ) -> WorkflowResult<()> {
        debug!(
            "Processing completed task: {} with successors: {:?}",
            node_id, successors
        );

        #[cfg(debug_assertions)]
        debug!("Adding node {} to completed nodes", node_id);

        for node_id in successors {
            self.update_node_degree(&node_id).await?;
        }

        // 添加节点到已完成节点集合
        self.completed_nodes.lock().await.insert(node_id);

        Ok(())
    }

    /// Update node in-degree
    ///
    /// This method updates the in-degree of a node in the workflow graph.
    ///
    /// # Arguments
    /// * `node_id` - The ID of the node
    ///
    /// # Returns
    /// * `WorkflowResult<()>` - Returns `Ok(())` if the in-degree is updated successfully,
    ///   otherwise returns an error with details
    ///
    /// # Errors
    /// * Returns `WorkflowError` if:
    ///   - The in-degree cannot be updated
    async fn update_node_degree(&self, node_id: &str) -> WorkflowResult<()> {
        let in_degree_map_ref = self.graph.in_degree_map();
        let mut in_degree_map = in_degree_map_ref.lock().await;

        if let Some(degree) = in_degree_map.get_mut(node_id) {
            let _old_degree = *degree;
            *degree = degree.saturating_sub(1);
            #[cfg(debug_assertions)]
            debug!(
                "Updating node degree: {} from {} to {}",
                node_id, _old_degree, *degree
            );

            let mut ready_queue = self.ready_queue.lock().await;
            if *degree == 0 {
                #[cfg(debug_assertions)]
                {
                    debug!("Node {} has zero in-degree, adding to ready queue", node_id);
                }

                if let Some(node) = self.graph.get_node(node_id) {
                    ready_queue.push_back(node.clone());

                    #[cfg(debug_assertions)]
                    {
                        debug!(
                            "Added node {} to ready queue, queue size: {}",
                            node_id,
                            ready_queue.len()
                        );
                    }
                } else {
                    warn!("Node {} not found in graph nodes", node_id);
                }
            }
        } else {
            warn!("Node {} not found in in_degree_map", node_id);
        }
        Ok(())
    }

    /// Execute a task node with retry
    ///
    /// This method executes a task node with retry logic in case of failure.
    ///
    /// # Arguments
    /// * `node` - The configuration of the task node
    ///
    /// # Returns
    /// * `WorkflowResult<()>` - Returns `Ok(())` if the task is executed successfully,
    ///   otherwise returns an error with details
    ///
    /// # Errors
    /// * Returns `WorkflowError` if:
    ///   - The task fails after all retries
    async fn execute_task_node_with_retry(&self, node: NodeConfig) -> WorkflowResult<()> {
        // 重试逻辑
        const MAX_RETRIES: usize = 3;
        const BASE_DELAY_MS: u64 = 100;

        let mut retries = 0;
        let mut current_delay = BASE_DELAY_MS;
        let tool_manager = self.tool_manager.clone();

        loop {
            match self
                .execute_task_node(node.clone(), tool_manager.clone())
                .await
            {
                Ok(()) => return Ok(()),
                Err(e) if e.is_retriable() && retries < MAX_RETRIES => {
                    log::warn!(
                        "Retrying node {} (attempt {}/{}), error: {}",
                        node.id,
                        retries + 1,
                        MAX_RETRIES,
                        e
                    );
                    time::sleep(Duration::from_millis(current_delay)).await;
                    current_delay *= 2;
                    retries += 1;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Execute a task node
    ///
    /// This method executes a task node and set the result to context output.
    ///
    /// # Arguments
    /// * `node` - The configuration of the task node
    /// * `function_manager` - The function manager
    ///
    /// # Returns
    /// * `WorkflowResult<()>` - Returns `Ok(())` if the task is executed successfully,
    ///   otherwise returns an error with details
    ///
    /// # Errors
    /// * Returns `WorkflowError` if:
    ///   - The task fails to execute
    async fn execute_task_node(
        &self,
        node: NodeConfig,
        tool_manager: Arc<ToolManager>,
    ) -> WorkflowResult<()> {
        match node.r#type {
            NodeType::Task(ToolConfig {
                function,
                param,
                output,
            }) => {
                let function = tool_manager.get_tool(&function).await.map_err(|_| {
                    WorkflowError::Execution(
                        t!("tools.function_not_found", name = function).to_string(),
                    )
                })?;

                let params = self.context.resolve_params(param).await?;
                let result = function.call(params).await?;

                #[cfg(debug_assertions)]
                {
                    debug!(
                        "Task {} executed successfully, result: {:?}",
                        node.id, result
                    );
                }

                self.context
                    .set_output(output.unwrap_or(node.id), result.into())
                    .await?;
            }
            NodeType::Loop(_) => {
                return self.execute_loop_node(node, tool_manager).await;
            }
            _ => {
                return Err(WorkflowError::Validation(
                    t!(
                        "workflow.invalid_node_type",
                        node_type = node.r#type.to_string()
                    )
                    .to_string(),
                ));
            }
        }

        Ok(())
    }

    /// Execute a loop node
    ///
    /// This method executes a loop node and set the result to context output.
    ///
    /// # Arguments
    /// * `node` - The configuration of the loop node
    /// * `function_manager` - The function manager
    ///
    /// # Returns
    /// * `WorkflowResult<()>` - Returns `Ok(())` if the loop is executed successfully,
    ///   otherwise returns an error with details
    ///
    /// # Errors
    /// * Returns `WorkflowError` if:
    ///   - The loop fails to execute
    async fn execute_loop_node(
        &self,
        node: NodeConfig,
        function_manager: Arc<ToolManager>,
    ) -> WorkflowResult<()> {
        // Directly parse the LoopConfig from the node configuration
        match node.r#type {
            NodeType::Loop(LoopConfig {
                input,
                functions,
                limit,
                filter,
            }) => {
                // parse input field reference
                let input_reference = input.trim();
                let input_data = self
                    .context
                    .resolve_params(Value::String(input_reference.to_string()))
                    .await?;

                // convert input data to iterable items
                let items: Vec<Value> = match input_data {
                    Value::Array(arr) => arr,
                    Value::Object(map) => {
                        // convert object to array with original key-value pairs, e.g. {a:b,c:d} -> [{a:b},{c:d}]
                        map.into_iter()
                            .map(|(k, v)| {
                                let mut obj = serde_json::Map::new();
                                obj.insert(k, v);
                                Value::Object(obj)
                            })
                            .collect()
                    }
                    Value::String(s) => {
                        let s = crate::libs::util::format_json_str(&s);
                        // try to parse as json array
                        match serde_json::from_str::<Vec<Value>>(&s) {
                            Ok(values) => values,
                            Err(_) => {
                                // if cannot parse as array, try to parse as single object
                                match serde_json::from_str::<Value>(&s) {
                                    Ok(value) => vec![value],
                                    Err(_) => vec![Value::String(s.to_string())],
                                }
                            }
                        }
                    }
                    _ => {
                        return Err(WorkflowError::Config(
                            t!(
                                "workflow.invalid_loop_input",
                                input = input_data.to_string()
                            )
                            .to_string(),
                        ))
                    }
                };

                // Apply limit
                let limit = limit.unwrap_or(items.len());
                let actual_limit = std::cmp::min(limit, items.len());

                // Prepare result array
                let mut results = HashMap::with_capacity(actual_limit);

                // Iterate over each item
                for item in items.into_iter().take(actual_limit) {
                    // Set current item to context, so that filters and functions can reference it
                    self.context
                        .set_output("item".to_string(), item.clone())
                        .await?;

                    // Check filter condition
                    if let Some(filter) = &filter {
                        // Use specialized condition parsing method to handle filter condition
                        let pass = self.context.resolve_condition(filter).await?;
                        if !pass {
                            // If filter condition is not satisfied, skip this item
                            continue;
                        }
                    }

                    // Execute all functions
                    for tool_def in &functions {
                        // Create function call parameters
                        let function_name = &tool_def.function;
                        let function =
                            function_manager
                                .get_tool(function_name)
                                .await
                                .map_err(|_| {
                                    WorkflowError::Execution(
                                        t!("tools.function_not_found", name = function_name)
                                            .to_string(),
                                    )
                                })?;

                        // Parse function parameters
                        let mut function_params = tool_def.param.clone();
                        function_params = self.context.resolve_params(function_params).await?;

                        // Execute function
                        // let result = function.execute(function_params).await?;
                        let result = self
                            .execute_function_with_retry(&node.id, function, function_params)
                            .await?;
                        // Add processed item to results array
                        let output = tool_def.output.as_ref().unwrap_or(&function_name);
                        // results.insert(output, result);
                        results
                            .entry(output.to_string())
                            .and_modify(|v: &mut Vec<Arc<Value>>| v.push(Arc::new(result.clone())))
                            .or_insert_with(|| vec![Arc::new(result.clone())]);
                    }
                }

                // Set results to context output
                for tool_def in &functions {
                    let result_key = tool_def.output.as_ref().unwrap_or(&tool_def.function);
                    if results.contains_key(result_key) {
                        let default_val = vec![];
                        let output_key = tool_def.output.as_ref().unwrap_or(&node.id);
                        let result_value = results.get(result_key).unwrap_or(&default_val);
                        self.context
                            .set_output(
                                output_key.clone(),
                                serde_json::Value::Array(
                                    result_value
                                        .into_iter()
                                        .map(|rc_value| rc_value.as_ref().clone())
                                        .collect(),
                                ),
                            )
                            .await?;
                    }
                }
            }
            _ => {
                return Err(WorkflowError::Validation(
                    t!(
                        "workflow.invalid_node_type",
                        node_type = node.r#type.to_string()
                    )
                    .to_string(),
                ));
            }
        }

        Ok(())
    }

    async fn execute_function_with_retry(
        &self,
        node_id: &str,
        function: Arc<dyn ToolDefinition>,
        param: Value,
    ) -> WorkflowResult<Value> {
        let params = self.context.resolve_params(param).await?;
        for i in 0..3 {
            match function.call(params.clone()).await {
                Ok(result) => {
                    return Ok(result.into());
                }
                Err(e) => {
                    log::error!(
                        "Function {} execution failed: {:?}, retry: {}/3",
                        function.name(),
                        e,
                        i + 1
                    );
                    time::sleep(Duration::from_millis(1000)).await;
                    if i == 3 {
                        return Err(e.into());
                    }
                }
            }
        }

        Err(WorkflowError::MaxRetriesExceeded(node_id.to_string()))
    }

    /// Execute a group node
    ///
    /// This method handles the completion of a group node in the workflow.
    /// Group nodes do not perform any tasks themselves; they are used to group
    /// related tasks and manage their dependencies. When all child nodes and
    /// dependencies of a group node are completed, this method is called to
    /// mark the group node as completed.
    ///
    /// # Arguments
    /// * node - The configuration of the group node
    ///
    /// # Returns
    /// * `WorkflowResult<()>` - Returns `Ok(())` if the group node is marked as completed successfully,
    ///   otherwise returns an error with details
    ///
    /// # Errors
    /// * Returns `WorkflowError` if:
    ///   - The group node cannot be marked as completed
    async fn execute_group_node(&self, node: NodeConfig) -> WorkflowResult<()> {
        // A group node is completed when its dependencies and all its child nodes are completed.
        debug!("Group node {} coordinator completed", node.id);

        Ok(())
    }

    /// Wait for task completion
    ///
    /// This method waits for the next task in the task set to complete and handles the result.
    ///
    /// # Arguments
    /// * `task_set` - The set of running tasks
    ///
    /// # Returns
    /// * `WorkflowResult<()>` - Returns `Ok(())` if the task completes successfully,
    ///   otherwise returns an error with details
    ///
    /// # Errors
    /// * Returns `WorkflowError` if:
    ///   - The task fails to execute
    ///   - The task set is empty
    async fn wait_for_task_completion(
        &self,
        task_set: &mut JoinSet<WorkflowResult<()>>,
    ) -> WorkflowResult<()> {
        match task_set.join_next().await {
            Some(Ok(Ok(_))) => Ok(()),
            Some(Ok(Err(e))) => Err(e),
            Some(Err(join_error)) => Err(WorkflowError::Execution(
                t!("workflow.task_join_error", error = join_error.to_string()).to_string(),
            )),
            None => {
                time::sleep(Duration::from_millis(10)).await;
                Ok(())
            }
        }
    }

    /// Finalize the execution
    ///
    /// This method performs the final checks after all tasks have been executed, ensuring
    /// that all nodes have been processed and updating the workflow state accordingly.
    ///
    /// # Arguments
    /// * `nodes` - The list of nodes in the workflow
    ///
    /// # Returns
    /// * `WorkflowResult<()>` - Returns `Ok(())` if the execution is finalized successfully,
    ///   otherwise returns an error with details
    ///
    /// # Errors
    /// * Returns `WorkflowError` if:
    ///   - Not all nodes have been processed
    ///   - The workflow state cannot be update
    async fn finalize_execution(&self, nodes: &[NodeConfig]) -> WorkflowResult<()> {
        let completed = self.completed_nodes.lock().await;
        info!("Finalizing execution. Completed nodes: {:?}", completed);
        info!(
            "Total nodes: {}, Completed nodes: {}",
            nodes.len(),
            completed.len()
        );

        if completed.len() == nodes.len() {
            self.update_state(WorkflowState::Completed).await?;
            info!("Workflow execution completed");
            Ok(())
        } else {
            let missing = nodes
                .iter()
                .filter(|n| !completed.contains(&n.id))
                .map(|n| n.id.clone())
                .collect::<Vec<_>>();

            // Check the type of each missing node
            for node_id in &missing {
                if let Some(node_config) = self.graph.get_node(node_id) {
                    debug!(
                        "Uncompleted node {} is of type {:?}",
                        node_id, node_config.r#type
                    );

                    // If it's a group node, check the completion status of its child nodes
                    match node_config.r#type.clone() {
                        NodeType::Group(group_config) => {
                            let children = &group_config.nodes;
                            let children_status = children
                                .iter()
                                .map(|child| {
                                    if completed.contains(child) {
                                        format!("{}: completed", child)
                                    } else {
                                        format!("{}: not completed", child)
                                    }
                                })
                                .collect::<Vec<_>>();

                            debug!(
                                "Children of uncompleted group node {}: {:?}",
                                node_id, children_status
                            );
                        }
                        _ => {}
                    }
                }
            }

            error!("Incomplete execution. Missing nodes: {:?}", missing);
            Err(WorkflowError::Execution(
                t!(
                    "workflow.incomplete_execution",
                    missing = missing.join(", ")
                )
                .to_string(),
            ))
        }
    }

    /// Process control signals
    ///
    /// This method processes control signals such as pause, resume, and cancel.
    ///
    /// # Returns
    /// * `WorkflowResult<()>` - Returns `Ok(())` if the control signals are processed successfully,
    ///   otherwise returns an error with details
    ///
    /// # Errors
    /// * Returns `WorkflowError` if:
    ///   - The control signals cannot be processed
    async fn process_control_signals(&mut self) -> WorkflowResult<()> {
        match *self.state.read().await {
            WorkflowState::Paused => {
                // Wait for resume or cancel signal
                tokio::select! {
                    resume = self.resume_channel.recv() => {
                        if resume.is_some() {
                            // Received resume signal, continue execution
                            self.update_state(WorkflowState::Running).await?;
                            Ok(())
                        } else {
                            // keep waiting
                            time::sleep(time::Duration::from_millis(100)).await;
                            Ok(())
                        }
                    }
                    cancel = self.cancel_channel.recv() => {
                        if cancel.is_some() {
                            // Received cancel signal, stop execution
                            self.update_state(WorkflowState::Cancelled).await?;
                            Err(WorkflowError::Cancelled(
                                t!("workflow.cancelled").to_string(),
                            ))
                        } else {
                            // keep waiting
                            time::sleep(time::Duration::from_millis(100)).await;
                            Ok(())
                        }
                    }
                    else => {
                        // keep waiting
                        time::sleep(time::Duration::from_millis(100)).await;
                        Ok(())
                    }
                }
            }
            WorkflowState::Running => {
                // Check for pause or cancel signals
                tokio::select! {
                    biased; // Prioritize handling control signals

                    pause = self.pause_channel.recv() => {
                        if pause.is_some() {
                            self.update_state(WorkflowState::Paused).await?;
                            Ok(())
                        } else {
                            // keep waiting
                            Ok(())
                        }
                    }
                    cancel = self.cancel_channel.recv() => {
                        if cancel.is_some() {
                            self.update_state(WorkflowState::Cancelled).await?;
                            Err(WorkflowError::Cancelled(
                                t!("workflow.cancelled").to_string(),
                            ))
                        } else {
                            // keep waiting
                            Ok(())
                        }
                    }
                    else => {
                        // No control signals, continue execution
                        Ok(())
                    }
                }
            }
            _ => Ok(()),
        }
    }

    /// Update workflow state
    ///
    /// This method updates the state of the workflow.
    ///
    /// # Arguments
    /// * `new_state` - The new state of the workflow
    ///
    /// # Returns
    /// * `WorkflowResult<()>` - Returns `Ok(())` if the state is updated successfully,
    ///   otherwise returns an error with details
    ///
    /// # Errors
    /// * Returns `WorkflowError` if:
    ///   - The state cannot be updated
    async fn update_state(&self, new_state: WorkflowState) -> WorkflowResult<()> {
        let mut state = self.state.write().await;

        // Validate state transition
        if !Self::valid_transition(&state, &new_state) {
            // Use t! for localization
            return Err(WorkflowError::InvalidState(
                t!(
                    "workflow.invalid_state_transition",
                    current = format!("{:?}", *state),
                    new = format!("{:?}", new_state)
                )
                .to_string(),
            ));
        }

        // Update state
        *state = new_state.clone();

        // Notify state change
        if let Err(_) = self.state_channel.tx.send(new_state) {
            // This error is less likely to be user-facing, but still good to localize
            return Err(WorkflowError::Execution(
                // Consider a more specific key like workflow.state_notification_send_failed
                t!("workflow.state_notification_error").to_string(),
            ));
        }

        Ok(())
    }

    /// Validate state transition
    ///
    /// This method validates if a state transition is valid.
    ///
    /// # Arguments
    /// * `current` - The current state of the workflow
    /// * `new` - The new state of the workflow
    ///
    /// # Returns
    /// * `bool` - Returns `true` if the transition is valid, otherwise `false`
    fn valid_transition(current: &WorkflowState, new: &WorkflowState) -> bool {
        match (current, new) {
            // Initial state can transition to ready or running
            (WorkflowState::Init, WorkflowState::Ready) => true,
            (WorkflowState::Init, WorkflowState::Running) => true,

            // Ready state can transition to running or cancelled
            (WorkflowState::Ready, WorkflowState::Running) => true,
            (WorkflowState::Ready, WorkflowState::Cancelled) => true,

            // Running state can transition to paused, completed, failed, error, failed with retry, or review failed
            (WorkflowState::Running, WorkflowState::Paused) => true,
            (WorkflowState::Running, WorkflowState::Completed) => true,
            (WorkflowState::Running, WorkflowState::Failed { .. }) => true,
            (WorkflowState::Running, WorkflowState::Error(_)) => true,
            (WorkflowState::Running, WorkflowState::FailedWithRetry { .. }) => true,
            (WorkflowState::Running, WorkflowState::ReviewFailed { .. }) => true,
            (WorkflowState::Running, WorkflowState::Cancelled) => true,

            // Paused state can transition to running or cancelled
            (WorkflowState::Paused, WorkflowState::Running) => true,
            (WorkflowState::Paused, WorkflowState::Cancelled) => true,
            (WorkflowState::Paused, WorkflowState::Failed { .. }) => true,

            // Error state can retry
            (WorkflowState::Error(_), WorkflowState::Running) => true,
            (WorkflowState::Error(_), WorkflowState::Failed { .. }) => true,

            // Failed with retry state can transition to running or failed
            (WorkflowState::FailedWithRetry { .. }, WorkflowState::Running) => true,
            (WorkflowState::FailedWithRetry { .. }, WorkflowState::Failed { .. }) => true,

            // Review failed state can retry
            (WorkflowState::ReviewFailed { .. }, WorkflowState::Running) => true,

            // Other transitions are not allowed
            _ => false,
        }
    }

    /// Pause workflow execution
    ///
    /// This method sends a signal to pause the workflow execution.
    ///
    /// # Returns
    /// * `WorkflowResult<()>` - Returns `Ok(())` if the pause signal is sent,
    ///   otherwise returns an error with details
    ///
    /// # Errors
    /// * Returns `WorkflowError` if:
    ///   - The pause signal cannot be sent
    pub async fn pause(&mut self) -> WorkflowResult<()> {
        if let Err(_) = self.pause_channel.send(()).await {
            // Consider a more specific key like workflow.pause_signal_send_failed
            return Err(WorkflowError::Execution(
                // This message is already specific, but could be localized
                t!("workflow.pause_notification_error").to_string(),
            ));
        }
        Ok(())
    }

    /// Resume workflow execution
    ///
    /// This method sends a signal to resume the workflow execution.
    ///
    /// # Returns
    /// * `WorkflowResult<()>` - Returns `Ok(())` if the resume signal is sent,
    ///   otherwise returns an error with details
    ///
    /// # Errors
    /// * Returns `WorkflowError` if:
    ///   - The resume signal cannot be sent
    pub async fn resume(&mut self) -> WorkflowResult<()> {
        if let Err(_) = self.resume_channel.send(()).await {
            // Consider a more specific key like workflow.resume_signal_send_failed
            return Err(WorkflowError::Execution(
                // This message is already specific, but could be localized
                t!("workflow.resume_notification_error").to_string(),
            ));
        }
        Ok(())
    }

    /// Cancel workflow execution
    ///
    /// This method sends a signal to cancel the workflow execution.
    ///
    /// # Returns
    /// * `WorkflowResult<()>` - Returns `Ok(())` if the cancel signal is sent,
    ///   otherwise returns an error with details
    ///
    /// # Errors
    /// * Returns `WorkflowError` if:
    ///   - The cancel signal cannot be sent
    pub async fn cancel(&mut self) -> WorkflowResult<()> {
        if let Err(_) = self.cancel_channel.send(()).await {
            // Consider a more specific key like workflow.cancel_signal_send_failed
            return Err(WorkflowError::Execution(
                // This message is already specific, but could be localized
                t!("workflow.cancel_notification_error").to_string(),
            ));
        }
        Ok(())
    }

    /// Generate status map
    ///
    /// This method generates a map of status icons based on the current state of the workflow.
    ///
    /// # Returns
    /// * `HashMap<String, &'static str>` - Returns a map of status icons where the key is the node ID and the value is the status icon
    pub async fn generate_status_map(&self) -> HashMap<String, &'static str> {
        let mut status_map = HashMap::new();
        let mut tasks = Vec::new();

        // 从缓存获取节点配置
        for node in self.graph.nodes() {
            let context = self.context.clone();
            let id = node.id.clone();
            let task = async move {
                let state = context.get_node_state(&id).await;
                let icon = match state {
                    NodeState::Completed => "✅",
                    NodeState::Failed { .. } => "❌",
                    NodeState::Retrying(_) => "🔄",
                    NodeState::Running => "▶️",
                    NodeState::Paused => "⏸️",
                    NodeState::Pending => "⏳",
                };
                (id, icon)
            };
            tasks.push(task);
        }
        let results = futures::future::join_all(tasks).await;
        for (id, icon) in results {
            status_map.insert(id, icon);
        }

        status_map
    }

    /// Generate Markmap content
    ///
    /// This method generates a Markmap content based on the current state of the workflow.
    ///
    /// # Returns
    /// * `String` - Returns the Markmap content
    pub async fn generate_markmap(&self) -> String {
        let status_map = self.generate_status_map().await;
        let mut output = String::from("# Workflow Status\n\n");

        // 按照原始顺序遍历所有节点
        for node in self.graph.nodes() {
            match &node.r#type {
                NodeType::Group(group_config) => {
                    // 处理组节点
                    let icon = status_map.get(&node.id).copied().unwrap_or("");
                    output.push_str(&format!("## {}{}\n", icon, node.id));

                    if let Some(desc) = &node.description {
                        output.push_str(&format!("- Description: {}\n", desc));
                    }

                    output.push_str(&format!("- Parallel: {}\n", group_config.parallel));

                    // 添加子节点
                    if !group_config.nodes.is_empty() {
                        output.push_str("- Child Nodes:\n");
                        for child_id in &group_config.nodes {
                            let child_icon = status_map.get(child_id).copied().unwrap_or("");
                            output.push_str(&format!("  - {}{}\n", child_icon, child_id));
                        }
                    }

                    output.push_str("\n");
                }
                NodeType::Task(_) => {
                    // 处理任务节点
                    let icon = status_map.get(&node.id).copied().unwrap_or("");
                    output.push_str(&format!("## {}{}\n", icon, node.id));

                    if let Some(desc) = &node.description {
                        output.push_str(&format!("- Description: {}\n", desc));
                    }

                    output.push_str("\n");
                }
                NodeType::Loop(_) => {
                    // 处理循环节点
                    let icon = status_map.get(&node.id).copied().unwrap_or("");
                    output.push_str(&format!("## {}{} (Loop)\n", icon, node.id));

                    if let Some(desc) = &node.description {
                        output.push_str(&format!("- Description: {}\n", desc));
                    }

                    output.push_str("\n");
                }
            }
        }

        output
    }
}
