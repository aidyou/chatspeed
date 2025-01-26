use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rust_i18n::t;
use tokio::sync::{watch, RwLock};
use tokio::time::timeout;

use crate::plugins::manager::PluginManager;
use crate::plugins::traits::PluginType;
use crate::workflow::config::{EdgeConfig, NodeConfig};
use crate::workflow::context::Context;
use crate::workflow::error::WorkflowError;
use crate::workflow::types::{
    WorkflowResult,
    WorkflowState::{self, *},
};

/// Executor for workflow nodes
#[async_trait]
pub trait NodeExecutor: Send + Sync {
    /// Execute a single node
    async fn execute_node(&self, node: &NodeConfig, context: &Context) -> WorkflowResult<()>;

    /// Execute multiple nodes in parallel
    async fn execute_nodes(&self, nodes: &[NodeConfig], context: &Context) -> WorkflowResult<()>;
}

/// Default implementation of workflow executor
pub struct WorkflowExecutor {
    /// Plugin manager for handling plugin operations
    plugin_manager: Arc<PluginManager>,
    /// Maximum number of parallel executions
    max_parallel: usize,
    /// Default timeout in milliseconds
    default_timeout: u64,
    /// State management
    state: Arc<RwLock<WorkflowState>>,
    state_tx: watch::Sender<WorkflowState>,
    state_rx: watch::Receiver<WorkflowState>,
}

impl WorkflowExecutor {
    /// Create a new workflow executor
    pub fn new(
        plugin_manager: Arc<PluginManager>,
        max_parallel: usize,
        default_timeout: u64,
    ) -> Self {
        let (state_tx, state_rx) = watch::channel(WorkflowState::Ready);
        Self {
            plugin_manager,
            max_parallel,
            default_timeout,
            state: Arc::new(RwLock::new(WorkflowState::Ready)),
            state_tx,
            state_rx,
        }
    }

    /// Get the current state of the workflow
    pub(crate) async fn get_state(&self) -> WorkflowState {
        self.state.read().await.clone()
    }

    /// Subscribe to workflow state changes
    pub(crate) fn subscribe_state(&self) -> watch::Receiver<WorkflowState> {
        self.state_rx.clone()
    }

    /// Pause the workflow execution
    pub async fn pause(&self) -> WorkflowResult<()> {
        let current_state = {
            let state = self.state.read().await;
            state.clone()
        };

        match current_state {
            WorkflowState::Running | WorkflowState::Ready => {
                // Send state change notification
                self.set_state(WorkflowState::Paused).await?;
                Ok(())
            }
            _ => Err(WorkflowError::Execution(
                t!("workflow.executor.invalid_state_transition").to_string(),
            )),
        }
    }

    /// Resume the workflow execution
    pub async fn resume(&self) -> WorkflowResult<()> {
        let current_state = {
            let state = self.state.read().await;
            state.clone()
        };

        match current_state {
            WorkflowState::Paused => {
                // Send state change notification
                self.set_state(WorkflowState::Running).await?;
                Ok(())
            }
            _ => Err(WorkflowError::Execution(
                t!("workflow.executor.invalid_state_transition").to_string(),
            )),
        }
    }

    /// Execute a single node with plugin manager
    async fn execute_node_with_plugin(
        &self,
        node: &NodeConfig,
        context: &Context,
    ) -> WorkflowResult<()> {
        if let Some(plugin_id) = &node.plugin {
            let plugin_type = node.plugin_type.clone().unwrap_or(PluginType::Native);
            match self
                .plugin_manager
                .execute_plugin(
                    plugin_id,
                    plugin_type,
                    node.config.as_ref(),
                    None, // TODO: Add input handling
                )
                .await
            {
                Ok(output) => {
                    // Store output in context
                    context
                        .set_metadata(format!("{}:output", node.id), output.to_string())
                        .await?;
                    Ok(())
                }
                Err(e) => Err(WorkflowError::Plugin(format!(
                    "Plugin execution failed: {}",
                    e
                ))),
            }
        } else {
            Ok(())
        }
    }

    /// Execute the entire workflow
    pub async fn execute_workflow(
        &self,
        nodes: &[NodeConfig],
        edges: &[EdgeConfig],
        context: &Context,
    ) -> WorkflowResult<()> {
        // Set initial state to Running
        self.transition_state(WorkflowState::Running).await?;

        // Track completed nodes
        let mut completed = HashSet::new();

        // Execute nodes until all are completed
        while completed.len() < nodes.len() {
            // Get nodes that can be executed in parallel
            let parallel_nodes = self.get_parallel_nodes(nodes, edges, &completed).await?;
            if parallel_nodes.is_empty() {
                break;
            }

            // Wait for running state
            while !matches!(*self.state.read().await, WorkflowState::Running) {
                if matches!(*self.state.read().await, WorkflowState::Paused) {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
                return Ok(());
            }

            // Execute parallel nodes
            self.execute_nodes(&parallel_nodes, context).await?;

            // Update completed nodes
            for node in parallel_nodes {
                completed.insert(node.id.clone());
            }
        }

        // Set final state
        if completed.len() == nodes.len() {
            self.transition_state(WorkflowState::Completed).await?;
        }

        Ok(())
    }

    /// Get nodes that can be executed in parallel based on their dependencies
    ///
    /// # Arguments
    /// * `nodes` - List of all nodes in the workflow
    /// * `edges` - List of edges defining dependencies between nodes
    /// * `completed` - Set of node IDs that have already been completed
    ///
    /// # Returns
    /// * `WorkflowResult<Vec<NodeConfig>>` - List of nodes that can be executed in parallel
    ///
    /// # Algorithm
    /// 1. Builds a dependency map for each node
    /// 2. Checks for circular dependencies using DFS
    /// 3. Identifies nodes whose dependencies are all satisfied
    ///
    /// # Error Cases
    /// * Returns error if circular dependencies are detected
    /// * Returns error if dependency validation fails
    pub(crate) async fn get_parallel_nodes(
        &self,
        nodes: &[NodeConfig],
        edges: &[EdgeConfig],
        completed: &HashSet<String>,
    ) -> WorkflowResult<Vec<NodeConfig>> {
        let mut ready_nodes = Vec::new();
        let mut dependency_map: HashMap<String, HashSet<String>> = HashMap::new();

        // Build dependency map: for each node, store its dependencies (nodes it depends on)
        for edge in edges {
            for target in &edge.to {
                dependency_map
                    .entry(target.clone())
                    .or_insert_with(HashSet::new)
                    .insert(edge.from.clone());
            }
        }

        // Check for circular dependencies using DFS
        let mut visited = HashSet::new();
        let mut stack = HashSet::new();

        fn has_cycle(
            node: &str,
            dependency_map: &HashMap<String, HashSet<String>>,
            visited: &mut HashSet<String>,
            stack: &mut HashSet<String>,
        ) -> bool {
            if !visited.contains(node) {
                visited.insert(node.to_string());
                stack.insert(node.to_string());

                if let Some(deps) = dependency_map.get(node) {
                    for dep in deps {
                        if !visited.contains(dep) && has_cycle(dep, dependency_map, visited, stack)
                            || stack.contains(dep)
                        {
                            return true;
                        }
                    }
                }
            }
            stack.remove(node);
            false
        }

        // Check each node for cycles
        for node in nodes {
            if !visited.contains(&node.id) {
                if has_cycle(&node.id, &dependency_map, &mut visited, &mut stack) {
                    return Err(WorkflowError::Execution(
                        t!("workflow.executor.circular_dependency").to_string(),
                    ));
                }
            }
        }

        // Find nodes with all dependencies satisfied
        for node in nodes {
            if completed.contains(&node.id) {
                continue;
            }

            let dependencies = dependency_map.get(&node.id);
            let all_dependencies_completed = dependencies
                .map(|deps| deps.iter().all(|dep| completed.contains(dep)))
                .unwrap_or(true);

            if all_dependencies_completed {
                ready_nodes.push(node.clone());
            }
        }

        // Limit parallel executions if max_parallel is set
        if self.max_parallel > 0 && ready_nodes.len() > self.max_parallel {
            ready_nodes.truncate(self.max_parallel);
        }

        Ok(ready_nodes)
    }

    /// Atomic state transition with validation and notification
    pub(crate) async fn transition_state(&self, new_state: WorkflowState) -> WorkflowResult<()> {
        let mut state = self.state.write().await;
        let current_state = (*state).clone();

        // Validate state transition
        if !self.is_valid_transition(&current_state, &new_state) {
            return Err(WorkflowError::Execution(format!(
                "Invalid state transition from {:?} to {:?}",
                current_state, new_state
            )));
        }

        // Update state and notify subscribers
        *state = new_state.clone();
        if let Err(e) = self.state_tx.send(new_state.clone()) {
            // Rollback on notification failure
            *state = current_state;
            return Err(WorkflowError::Execution(format!(
                "Failed to notify state change: {}",
                e
            )));
        }

        // Log state transition
        tracing::info!(
            "Workflow state transitioned from {:?} to {:?}",
            current_state,
            new_state
        );

        Ok(())
    }

    /// Set workflow state with atomic transition
    pub(crate) async fn set_state(&self, new_state: WorkflowState) -> WorkflowResult<()> {
        self.transition_state(new_state).await
    }

    /// Check if a state transition is valid
    fn is_valid_transition(&self, from: &WorkflowState, to: &WorkflowState) -> bool {
        match (from, to) {
            // Initial transitions
            (Init, Ready) => true,
            (Ready, Running) => true,

            // Running state transitions
            (Running, Paused) => true,
            (Running, Completed) => true,
            (Running, Error(_)) => true,
            (Running, FailedWithRetry { .. }) => true,
            (Running, Failed { .. }) => true,
            (Running, Cancelled) => true,
            (Running, Stopped) => true,
            (Running, ReviewFailed { .. }) => true,

            // Paused state transitions
            (Paused, Running) => true,
            (Paused, Cancelled) => true,

            // Error state transitions
            (Error(_), Running) => true,
            (Error(_), Failed { .. }) => true,

            // Retry state transitions
            (FailedWithRetry { .. }, Running) => true,
            (FailedWithRetry { .. }, Failed { .. }) => true,

            // Failed state transitions
            (Failed { .. }, Running) => true,

            // Review Failed transitions
            (ReviewFailed { .. }, Running) => true,
            (ReviewFailed { .. }, Failed { .. }) => true,

            // Stopped state transitions
            (Stopped, Running) => true,
            (Stopped, Failed { .. }) => true,

            // No transitions from final states
            (Completed, _) => false,
            (Cancelled, _) => false,

            // All other transitions are invalid
            _ => false,
        }
    }

    /// Wait for workflow state to match expected state
    async fn wait_for_state(&mut self, expected_state: WorkflowState) -> WorkflowResult<()> {
        // First check if we're already in a final state
        let current_state = self.state.read().await.clone();
        if current_state.is_final() && current_state != expected_state {
            return Err(WorkflowError::Execution(
                t!("workflow.executor.invalid_state_transition").to_string(),
            ));
        }

        // Use a reasonable timeout duration for workflow state changes
        let timeout_duration = Duration::from_secs(30);

        match timeout(timeout_duration, async {
            loop {
                // Check current state
                let state = self.state.read().await.clone();
                if state == expected_state {
                    return Ok(());
                }

                // Wait for next state change
                if self.state_rx.changed().await.is_err() {
                    return Err(WorkflowError::Execution(
                        t!("workflow.executor.state_channel_closed").to_string(),
                    ));
                }
            }
        })
        .await
        {
            Ok(result) => result,
            Err(_) => Err(WorkflowError::Execution(format!(
                "Timeout waiting for workflow state: expected {:?}, current {:?}",
                expected_state, current_state
            ))),
        }
    }

    /// Handle workflow failure with retry logic
    pub async fn handle_failure(&self, error: String, max_retries: u32) -> WorkflowResult<()> {
        let current_state = {
            let state = self.state.read().await;
            state.clone()
        };

        let new_state = match current_state {
            WorkflowState::FailedWithRetry {
                retry_count,
                max_retries: current_max,
                ..
            } => {
                if retry_count >= max_retries {
                    WorkflowState::Failed {
                        error,
                        timestamp: chrono::Utc::now().timestamp(),
                    }
                } else {
                    WorkflowState::FailedWithRetry {
                        retry_count: retry_count + 1,
                        max_retries: current_max,
                        error,
                    }
                }
            }
            _ => WorkflowState::FailedWithRetry {
                retry_count: 1,
                max_retries,
                error,
            },
        };

        // Send state change notification
        self.set_state(new_state.clone()).await?;
        Ok(())
    }

    /// Cancel the workflow execution
    pub async fn cancel(&self) -> WorkflowResult<()> {
        let current_state = {
            let state = self.state.read().await;
            state.clone()
        };

        if current_state.is_final() {
            return Err(WorkflowError::Execution(
                t!("workflow.executor.workflow_already_finished").to_string(),
            ));
        }

        // Send state change notification
        self.set_state(WorkflowState::Cancelled).await?;
        Ok(())
    }

    /// Execute the workflow from a specific node
    pub async fn execute_from_node(
        &self,
        start_node_id: &str,
        nodes: &[NodeConfig],
        edges: &[EdgeConfig],
        context: &Context,
    ) -> WorkflowResult<()> {
        // Find the start node and its descendants
        let mut execution_nodes = Vec::new();
        let mut visited = HashSet::new();

        // Helper function to collect nodes
        fn collect_nodes(
            node_id: &str,
            nodes: &[NodeConfig],
            edges: &[EdgeConfig],
            visited: &mut HashSet<String>,
            execution_nodes: &mut Vec<NodeConfig>,
        ) {
            if visited.contains(node_id) {
                return;
            }

            visited.insert(node_id.to_string());
            if let Some(node) = nodes.iter().find(|n| n.id == node_id) {
                execution_nodes.push(node.clone());

                // Find child nodes
                if let Some(edge) = edges.iter().find(|e| e.from == node_id) {
                    for child_id in &edge.to {
                        collect_nodes(child_id, nodes, edges, visited, execution_nodes);
                    }
                }
            }
        }

        collect_nodes(
            start_node_id,
            nodes,
            edges,
            &mut visited,
            &mut execution_nodes,
        );

        // Execute the collected nodes
        self.execute_nodes(&execution_nodes, context).await
    }

    /// Handle review failure
    pub async fn handle_review_failure(
        &self,
        node_id: String,
        reason: String,
        original_output: Option<String>,
    ) -> WorkflowResult<()> {
        let timestamp = chrono::Utc::now().timestamp();
        self.transition_state(WorkflowState::ReviewFailed {
            node_id,
            reason,
            original_output,
            timestamp,
        })
        .await
    }

    /// Retry execution from a failed review node
    pub async fn retry_from_review_failed(
        &self,
        nodes: &[NodeConfig],
        edges: &[EdgeConfig],
        context: &Context,
    ) -> WorkflowResult<()> {
        // Get current state
        let current_state = self.get_state().await;

        // Verify we're in ReviewFailed state
        let failed_node_id = match current_state {
            WorkflowState::ReviewFailed { node_id, .. } => node_id,
            _ => {
                return Err(WorkflowError::InvalidState(
                    "Can only retry from ReviewFailed state".into(),
                ))
            }
        };

        // Reset state to Running
        self.transition_state(WorkflowState::Running).await?;

        // Execute from the failed node
        self.execute_from_node(&failed_node_id, nodes, edges, context)
            .await
    }

    async fn execute_nodes(&self, nodes: &[NodeConfig], context: &Context) -> WorkflowResult<()> {
        use futures::future::join_all;

        let mut completed = HashSet::new();
        let mut remaining_nodes = nodes.to_vec();

        while !remaining_nodes.is_empty() {
            // Check if the workflow should continue execution
            if !matches!(*self.state.read().await, WorkflowState::Running) {
                return Ok(());
            }

            // Get nodes that can be executed in parallel
            let parallel_nodes = self
                .get_parallel_nodes(&remaining_nodes, &[], &completed)
                .await?;

            if parallel_nodes.is_empty() {
                break;
            }

            // Execute the current batch of nodes
            let results = join_all(
                parallel_nodes
                    .iter()
                    .map(|node| self.execute_node(node, context)),
            )
            .await;

            // Handle execution results
            for (node, result) in parallel_nodes.iter().zip(results) {
                match result {
                    Ok(_) => {
                        completed.insert(node.id.clone());
                    }
                    Err(e) => {
                        // Handle node execution error
                        return Err(e);
                    }
                }
            }

            // Remove completed nodes from remaining
            remaining_nodes.retain(|node| !completed.contains(&node.id));
        }

        Ok(())
    }
}

#[async_trait]
impl NodeExecutor for WorkflowExecutor {
    async fn execute_node(&self, node: &NodeConfig, context: &Context) -> WorkflowResult<()> {
        self.execute_node_with_plugin(node, context).await
    }

    async fn execute_nodes(&self, nodes: &[NodeConfig], context: &Context) -> WorkflowResult<()> {
        self.execute_nodes(nodes, context).await
    }
}
