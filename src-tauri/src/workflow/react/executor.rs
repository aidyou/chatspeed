use log::{debug, error, info, warn};
use rust_i18n::t;
use serde_json::{json, Value};
use std::{collections::HashSet, sync::Arc};

use super::{types::Step, MessageRole};
use crate::{
    ai::interaction::chat_completion::ChatState,
    db::MainStore,
    libs::util::format_json_str,
    search::SearchResult,
    tools::{ModelName, ToolManager, WebSearch},
    workflow::{
        error::WorkflowError,
        react::{
            context::ReactContext,
            planning::PlanManager,
            prompts::{
                OBSERVATION_PROMPT, PLAN_GENERATION_PROMPT, REASONING_PROMPT, SUMMARY_PROMPT,
            },
            types::{FunctionCall, Plan, PlanStatus, StepError, StepResult, StepStatus},
        },
    },
};

/// ReAct executor
pub struct ReactExecutor {
    /// Execution context
    context: ReactContext,
    /// Plan manager
    plan_manager: PlanManager,
    /// Tool spec
    tool_spec: Arc<String>,
    /// Maximum retries
    max_retries: u32,
}

impl ReactExecutor {
    /// Create a new ReAct executor
    ///
    /// # Arguments
    /// * `main_store` - Main store
    /// * `chat_state` - Chat state
    /// * `max_retries` - Maximum retries
    ///
    /// # Returns
    /// A new ReAct executor
    pub async fn new(
        main_store: Arc<std::sync::RwLock<MainStore>>,
        chat_state: Arc<ChatState>,
        max_retries: u32,
    ) -> Result<Self, WorkflowError> {
        // Initialize function manager
        let function_manager = Arc::new(ToolManager::new());
        function_manager
            .clone()
            .register_available_tools(chat_state.clone(), main_store.clone())
            .await?;

        // Create context
        let context = ReactContext::new(function_manager.clone(), max_retries);

        // Create plan manager
        let plan_manager = PlanManager::new();

        // In ReAct mode, we don't need chat_completion,
        // and search_dedup tool may cause AI to overuse, so we exclude these two
        let exclude: HashSet<String> =
            vec!["chat_completion".to_string(), "search_dedup".to_string()]
                .into_iter()
                .collect();
        let tool_spec = context
            .function_manager
            .get_tool_calling_spec(Some(exclude))
            .await?
            .iter()
            .map(|td| td.to_standard())
            .collect::<Vec<Value>>();

        Ok(Self {
            context,
            plan_manager,
            tool_spec: Arc::new(serde_json::to_string(&tool_spec).unwrap_or_default()),
            max_retries,
        })
    }

    /// Execute a user request from start to finish
    ///
    /// # Arguments
    /// * `user_request` - User's request
    ///
    /// # Returns
    /// The result of the request execution or an error if execution fails
    pub async fn execute(&mut self, user_request: String) -> Result<Value, WorkflowError> {
        // Generate a plan based on the user request
        let plan = self.generate_plan(user_request).await?;
        info!("已生成计划: {}", plan.name);

        // Execute the plan
        let result = self.execute_react().await?;
        info!("计划执行完成");

        Ok(result)
    }

    /// Execute a complete ReAct loop
    ///
    /// This method executes the entire ReAct workflow, processing each step of the plan
    /// through the think-act-observe cycle until completion or failure.
    ///
    /// # Returns
    /// The result of the plan execution or an error if execution fails
    async fn execute_react(&mut self) -> Result<Value, WorkflowError> {
        // Get current plan
        let current_plan = self.context.get_current_plan().ok_or_else(|| {
            WorkflowError::Config(t!("workflow.react.no_active_plan").to_string())
        })?;

        info!("Starting plan execution: {}", &current_plan.name);

        // Get plan steps
        let steps = &current_plan.steps;

        if steps.is_empty() {
            return Err(WorkflowError::Config(
                t!("workflow.react.no_detailed_execution_plan").to_string(),
            ));
        }
        let step_len = steps.len();

        // Initialize result storage
        let mut step_results = Vec::new();

        // Record plan execution start
        self.context
            .add_plan_execution_record("start".to_string(), &current_plan, steps);

        // Execute each step
        for (step_index, step) in steps.iter().enumerate() {
            // Initialize step execution
            self.initialize_step_execution(step_index, step, step_len)
                .await?;

            // Execute the ReAct loop for this step
            let (step_result, tools_used, retry_count, react_cycle_count) = self
                .execute_step_react_loop(step_index, step_len, &step.name, &step.goal)
                .await?;

            // Finalize step execution and record results
            let step_result_with_metadata = self
                .finalize_step_execution(
                    step_index,
                    &step.name,
                    &step.goal,
                    step_result,
                    tools_used,
                    retry_count,
                    react_cycle_count,
                )
                .await?;

            step_results.push(step_result_with_metadata);
        }

        // Finalize plan execution and generate summary
        self.finalize_plan_execution(current_plan.clone(), steps, step_results)
            .await
    }

    /// Initialize the execution of a step
    ///
    /// Sets up the context and records for a step before execution begins
    ///
    /// # Arguments
    /// * `step_index` - Index of the step in the plan
    /// * `step` - Reference to the step being executed
    /// * `steps_len` - Total number of steps in the plan
    ///
    /// # Returns
    /// Result indicating success or error
    async fn initialize_step_execution(
        &mut self,
        step_index: usize,
        step: &Step,
        steps_len: usize,
    ) -> Result<(), WorkflowError> {
        let step_name = &step.name;
        let step_goal = &step.goal;

        info!(
            "Executing step {}/{}: {}",
            step_index + 1,
            steps_len,
            step_name
        );
        self.context.step_state.init(step_index);

        // Record step execution start
        self.context.add_step_execution_record(
            "start".to_string(),
            step_index,
            step_name.clone(),
            step_goal.clone(),
            None,
            None,
            None,
            None,
            None,
        );
        self.context.clear_messages();

        // Update plan status to running
        let current_plan = self.context.get_current_plan().ok_or_else(|| {
            WorkflowError::Config(t!("workflow.react.no_active_plan_init").to_string())
        })?;
        let mut updated_plan = current_plan.clone();
        updated_plan.status = PlanStatus::Running;
        self.plan_manager.update_plan(updated_plan.clone()).await?;
        self.context.set_current_plan(updated_plan);

        info!("⏳ Step {} initialized, start reasoning...", step_name);
        Ok(())
    }

    /// Execute the ReAct loop for a single step
    ///
    /// Runs the reasoning, acting, and observing phases in a loop until the step
    /// is completed or the maximum number of retries is reached
    ///
    /// # Arguments
    /// * `step_index` - Index of the step in the plan
    /// * `step_len` - Total number of steps in the plan
    /// * `step_name` - Name of the step
    /// * `step_goal` - Goal of the step
    ///
    /// # Returns
    /// Tuple containing the step result, tools used, retry count, and cycle count
    async fn execute_step_react_loop(
        &mut self,
        step_index: usize,
        step_len: usize,
        step_name: &str,
        step_goal: &str,
    ) -> Result<(StepResult, Vec<String>, u32, u32), WorkflowError> {
        let mut step_completed = false;
        let mut step_result = StepResult::default();
        let mut retry_count = 0;
        let mut tools_used = Vec::new();
        let mut react_cycle_count = 0;

        // ReAct loop: Think-Act-Observe
        while !step_completed && retry_count < self.context.max_retries {
            react_cycle_count += 1;
            debug!(
                "🔄 Step[{}/{}] {} - {}: Starting ReAct cycle {}",
                step_index, step_len, step_name, step_goal, react_cycle_count
            );

            // Start ReAct cycle
            self.context
                .start_react_cycle(step_index, step_name, react_cycle_count);

            // Reasoning phase - Decide which tools and methods to use
            debug!(
                "🔄 Step {}: Reasoning phase (cycle {})",
                step_name, react_cycle_count
            );
            let reasoning_result = match self.reasoning().await {
                Ok(result) => {
                    if result.status == StepStatus::Failed {
                        return Err(WorkflowError::Execution(
                            result.error_message.unwrap_or("".to_string()),
                        ));
                    }
                    // Record tool information that may be included in the reasoning result
                    if let Some(function_call) = result.function_call.clone() {
                        tools_used.push(function_call.name.clone());
                    }
                    result
                }
                Err(e) => {
                    // Handle reasoning phase error
                    if let Some(error) = self.handle_phase_error(
                        e,
                        "Reasoning",
                        step_index,
                        step_name,
                        step_goal,
                        &mut retry_count,
                        react_cycle_count,
                        &tools_used,
                    ) {
                        return Err(error);
                    }

                    tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                    continue;
                }
            };

            // Acting phase - Execute tool calls
            debug!(
                "🔄 Step {}: Acting phase (cycle {})",
                step_name, react_cycle_count
            );
            let acting_result = match self.acting(reasoning_result).await {
                Ok(result) => result,
                Err(e) => {
                    // Handle acting phase error
                    if let Some(error) = self.handle_phase_error(
                        e.clone(),
                        "Acting",
                        step_index,
                        step_name,
                        step_goal,
                        &mut retry_count,
                        react_cycle_count,
                        &tools_used,
                    ) {
                        return Err(error);
                    }
                    tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                    continue;
                }
            };

            // Observing phase - Process tool results
            debug!(
                "🔄 Step {}: Observing phase (cycle {})",
                step_name, react_cycle_count
            );
            match self.observing(acting_result).await {
                Ok(result) => {
                    step_completed = result.status == StepStatus::Completed;
                    step_result = result;
                }
                Err(e) => {
                    // Handle observing phase error
                    if let Some(error) = self.handle_phase_error(
                        e,
                        "Observing",
                        step_index,
                        step_name,
                        step_goal,
                        &mut retry_count,
                        react_cycle_count,
                        &tools_used,
                    ) {
                        return Err(error);
                    }
                    tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                }
            };
        }

        Ok((step_result, tools_used, retry_count, react_cycle_count))
    }

    /// Finalize the execution of a step
    ///
    /// Records step completion, updates the step result in the plan, and prepares the metadata
    ///
    /// # Arguments
    /// * `step_index` - Index of the step in the plan
    /// * `step_name` - Name of the step
    /// * `step_goal` - Goal of the step
    /// * `step_result` - Result of the step execution
    /// * `tools_used` - List of tools used during step execution
    /// * `retry_count` - Number of retries performed
    /// * `react_cycle_count` - Number of ReAct cycles executed
    ///
    /// # Returns
    /// JSON value containing the step result with metadata
    async fn finalize_step_execution(
        &mut self,
        step_index: usize,
        step_name: &str,
        step_goal: &str,
        step_result: StepResult,
        tools_used: Vec<String>,
        retry_count: u32,
        react_cycle_count: u32,
    ) -> Result<Value, WorkflowError> {
        // Record step completion
        self.context.add_step_execution_record(
            "completed".to_string(),
            step_index,
            step_name.to_string(),
            step_goal.to_string(),
            None,
            Some(retry_count),
            Some(self.context.max_retries),
            Some(react_cycle_count),
            Some(tools_used.clone()),
        );

        // Store step result with detailed metadata
        let step_result_with_metadata = json!({
            "step_index": step_index,
            "step_name": step_name,
            "step_goal": step_goal,
            "tools_used": tools_used,
            "retry_count": retry_count,
            "react_cycles": react_cycle_count,
            "result": step_result
        });

        // Update current plan with step result
        let current_plan = self.context.get_current_plan().ok_or_else(|| {
            WorkflowError::Config(t!("workflow.react.no_active_plan_finalization").to_string())
        })?;
        let mut updated_plan = current_plan.clone();

        if let Err(e) = updated_plan.update_step_result(
            step_index,
            json!(step_result.clone()),
            step_result.status.to_string(),
            tools_used.clone(),
            retry_count,
            react_cycle_count,
        ) {
            debug!("Failed to update step result: {}", e);
        }

        self.plan_manager.update_plan(updated_plan.clone()).await?;
        self.context.set_current_plan(updated_plan);

        info!("✅ Step {} completed", step_name);

        Ok(step_result_with_metadata)
    }

    /// Finalize the execution of the plan
    ///
    /// Generates a summary, marks the plan as completed, and updates the plan with the summary
    ///
    /// # Arguments
    /// * `current_plan` - The current plan being executed
    /// * `steps` - List of steps in the plan
    /// * `step_results` - Results of all step executions
    ///
    /// # Returns
    /// The summary of the plan execution
    async fn finalize_plan_execution(
        &mut self,
        _current_plan: Plan,
        steps: &[Step],
        step_results: Vec<Value>,
    ) -> Result<Value, WorkflowError> {
        info!("生成计划执行总结");
        // Clear previous context
        self.context.clear_messages();

        // Get current plan
        let current_plan = self.context.get_current_plan().ok_or_else(|| {
            WorkflowError::Config(t!("workflow.react.no_executing_plan").to_string())
        })?;

        // Add step results
        self.context.add_user_message(format!(
            "{}\n\n计划名称：{}\n\n计划目标：{}\n\n步骤执行结果：\n[Start of Steps Execution Results]\n{}\n[End of Steps Execution Results]",
            SUMMARY_PROMPT,
            current_plan.name,
            current_plan.goal,
            serde_json::to_string_pretty(&step_results).unwrap_or_default(),
        ));

        // Get messages for chat completion
        let messages = self.context.get_chat_messages(None);

        // Call chat completion API for summary generation
        let chat_response = self
            .chat_completion(ModelName::General, messages)
            .await
            .map_err(|e| {
                WorkflowError::Config(
                    t!(
                        "workflow.react.summary_chat_completion_failed",
                        error = e.to_string()
                    )
                    .to_string(),
                )
            })?;

        // Extract assistant response
        let content = chat_response["content"].as_str().unwrap_or("").to_string();

        // Add assistant message to context
        self.context.add_assistant_message(content.clone(), None);

        debug!("✅ 总结生成阶段调用聊天完成API成功: \n{}", content);

        // Create summary JSON
        let summary = json!({
            "plan_name": current_plan.name,
            "plan_goal": current_plan.goal,
            "step_results": step_results,
            "summary": content
        });

        // Mark plan as completed
        self.context
            .add_plan_execution_record("completed".to_string(), &current_plan, steps);

        // Complete the plan and add summary
        let updated_plan = self
            .plan_manager
            .complete_plan(current_plan.id, Some(summary.clone()))
            .await?;
        self.context.set_current_plan(updated_plan);

        Ok(summary)
    }

    /// Create reasoning messages
    ///
    /// To help the AI remember previously executed steps, we construct the following information:
    /// 1. {role: tool, content: "All historical tool call information at current step"}
    /// 2. If the last tool call failed, add: {role: tool, content: "Last tool call error"}
    /// 3. The reasoning prompt for the current step
    ///
    /// # Arguments
    /// * `step_name` - Name of the current step
    /// * `step_goal` - Goal of the current step
    ///
    /// # Returns
    /// A vector of reasoning messages
    async fn create_reasoning_messages(
        &mut self,
        step_name: &str,
        step_goal: &str,
    ) -> Result<(), WorkflowError> {
        // Store each tool call result in the context's tool_results,
        // these messages are passed to AI as role:tool memory
        let mut call_results = vec![];
        for result in self.context.step_state.goal_summaries.iter() {
            call_results.push(format!("- {}", result.clone()));
        }
        let summary = if !call_results.is_empty() {
            call_results.join("\n")
        } else {
            t!("workflow.react.no_information_collected").to_string() // Localized
        };

        let search_result = if let Some(sr) = self.context.step_state.last_search_result.clone() {
            if sr.is_empty() {
                "".to_string()
            } else {
                format!("[web_search_result start]\n{}\n[web_search_result end]", sr)
            }
        } else {
            "".to_string()
        };

        // keep last 5 tool call errors
        self.context.keep_last_tool_call_error();
        let mut errors = vec![];
        for msg in self.context.messages.iter() {
            if msg.role == MessageRole::Tool {
                errors.push(msg.content.clone());
            }
        }
        self.context.clear_messages();
        let error_logs = if !errors.is_empty() {
            format!(
                "[tool_error start]\n{}\n[tool_error end]",
                errors.join("\n\n")
            )
        } else {
            t!("workflow.react.no_error_information").to_string() // Localized
        };

        let step_count = self.context.get_step_len();
        // Add reasoning prompt with tools list
        let formatted_prompt = REASONING_PROMPT
            .replace("{tool_spec}", &self.tool_spec)
            .replace("{summary}", &summary)
            .replace("{search_result}", &search_result)
            .replace(
                "{tool_result}",
                self.context
                    .step_state
                    .last_tool_result
                    .as_ref()
                    .map(|r| r.clone())
                    .unwrap_or(t!("workflow.react.no_tool_call_information").to_string()) // Localized
                    .as_str(),
            )
            .replace("{tool_error}", &error_logs)
            .replace(
                "{step_index}",
                &self.context.step_state.step_index.to_string(),
            )
            .replace("{step_count}", &step_count.to_string())
            .replace("{step_name}", step_name)
            .replace("{step_goal}", step_goal)
            .replace("{current_time}", &chrono::Utc::now().to_rfc3339());

        self.context.add_user_message(formatted_prompt);

        Ok(())
    }

    /// Reasoning stage - decide next action
    ///
    /// This is the reasoning phase of the ReAct loop where the model analyzes the current situation
    /// and decides what tools to use based on the current step's goal and context.
    ///
    /// # Returns
    /// The assistant's response including any function calls to be executed
    async fn reasoning(&mut self) -> Result<StepResult, WorkflowError> {
        debug!("Reasoning stage: Analyze current step goal and decide action");

        // Add step information
        let step = self.context.get_current_step();
        let (step_name, step_goal) = if let Some(step) = step {
            (step.name.clone(), step.goal.clone())
        } else {
            (
                t!("workflow.react.unknown_step_name").to_string(),
                t!("workflow.react.unknown_step_goal").to_string(),
            ) // Localized
        };
        self.create_reasoning_messages(&step_name, &step_goal)
            .await?;

        let messages = self.context.get_chat_messages(None);

        debug!("===============================\n\n");
        for message in messages.iter() {
            debug!(
                "Role: {}\nMessage: \n{}",
                &message["role"].as_str().unwrap_or_default(),
                &message["content"].as_str().unwrap_or_default()
            );
        }
        debug!("===============================\n\n");

        // Call chat completion API for reasoning
        debug!("Calling reasoning chat completion API");
        let chat_response = self
            .chat_completion(ModelName::Reasoning, messages)
            .await
            .map_err(|e| {
                WorkflowError::Execution(
                    t!(
                        "workflow.react.reasoning_chat_completion_failed",
                        error = e.to_string()
                    )
                    .to_string(),
                )
            })?;

        // Extract assistant response
        let assistant_message = chat_response["content"].as_str().ok_or_else(|| {
            WorkflowError::Config(t!("workflow.react.assistant_message_empty").to_string())
        })?;

        let mut result_message = StepResult::default();

        // Try to parse JSON response
        match serde_json::from_str::<serde_json::Value>(&format_json_str(assistant_message)) {
            Ok(json_response) => {
                let status = json_response
                    .get("status")
                    .and_then(|s| s.as_str())
                    .unwrap_or("unknown");
                result_message.status = StepStatus::from(status);
                result_message.reasoning = json_response
                    .get("reasoning")
                    .and_then(|s| s.as_str().map(|s| s.to_string()));

                match result_message.status {
                    StepStatus::Running => {
                        // Handle tool call
                        if let Some(tool) = json_response.get("tool") {
                            if let (Some(function_name), Some(params)) = (
                                tool.get("name").and_then(|n| n.as_str()),
                                tool.get("arguments"),
                            ) {
                                debug!(
                                    "🔨 决定使用工具: {}, 推理：{}",
                                    function_name,
                                    result_message.reasoning.clone().unwrap_or_default()
                                );

                                // Create function call
                                result_message.function_call = Some(FunctionCall::new(
                                    function_name.to_string(),
                                    Some(params.clone()),
                                ));
                            }
                        }
                    }
                    StepStatus::Failed => {
                        result_message.status = StepStatus::Failed;
                        result_message.error_message = json_response
                            .get("error")
                            .and_then(|s| s.as_str().map(|s| s.to_string()));
                        error!(
                            "❌ Failed to execute step: {}",
                            result_message.error_message.clone().unwrap_or_default()
                        );
                    }
                    StepStatus::Completed => {
                        info!("✅ Step completed");
                        result_message.status = StepStatus::Completed;
                    }
                    _ => {
                        warn!(
                            "Unknown response status: {}, assistant message: {}",
                            status, assistant_message
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                }
            }
            Err(e) => {
                log::error!(
                    "Error in the JSON configuration format generated during the reasoning phase: {}, error: {}",
                    assistant_message, e
                );
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                return Err(WorkflowError::Config(
                    t!(
                        "workflow.react.reasoning_json_format_error",
                        error = e.to_string()
                    )
                    .to_string(),
                ));
            }
        }

        // Add assistant message to context
        self.context.add_assistant_message(
            assistant_message.to_string(),
            result_message
                .function_call
                .clone()
                .map(|fc| fc.to_string()),
        );

        Ok(result_message)
    }

    /// Acting stage - execute function
    ///
    /// This is the action phase of the ReAct loop where the model executes
    /// the chosen tool based on the reasoning phase's decision.
    ///
    /// # Arguments
    /// * `assistant_message` - The assistant's message from the reasoning stage
    ///
    /// # Returns
    /// The result of the action, which could be a function execution result or
    /// a completion status
    async fn acting(&mut self, assistant_message: StepResult) -> Result<StepResult, WorkflowError> {
        if assistant_message.status == StepStatus::Completed {
            return Ok(assistant_message);
        }

        // Get current plan
        let current_plan = self.context.get_current_plan().ok_or_else(|| {
            WorkflowError::Config(t!("workflow.react.no_executing_plan").to_string())
        })?;

        // Extract function call information
        if let Some(function_call) = assistant_message.function_call.clone() {
            debug!(
                "Executing function: {}, arguments: {:?}",
                function_call.name, function_call.arguments
            );

            // Record function call to step history
            self.context.add_toolcall_record(function_call.clone());

            // Execute function
            let arguments = function_call.arguments.clone().unwrap_or_default();
            let function_result = match self
                .context
                .function_manager
                .tool_call(function_call.name.as_str(), arguments.clone())
                .await
            {
                Ok(result) => {
                    debug!("Function execution successful: {}", function_call.name);

                    // Record function execution result to step history
                    self.context.add_toolresult_record(
                        function_call.clone(),
                        StepStatus::Success.to_string(),
                        Some(json!(result.clone())),
                        None,
                    );

                    // Create action result
                    let action_result = json!({
                        "status": StepStatus::Success.to_string(),
                        "function": function_call.name.clone(),
                        "parameters": arguments.clone(),
                        "result": result,
                        "timestamp": chrono::Utc::now().to_rfc3339()
                    });

                    // Add step result
                    StepResult::new(
                        StepStatus::Running,
                        Some(function_call.clone()),
                        None,
                        Some(action_result),
                        None,
                        None,
                        None,
                        None,
                    )
                }
                Err(e) => {
                    // Handle tool execution error, but don't update the entire plan status
                    let error_message = t!(
                        "workflow.react.tool_call_failed_details",
                        tool_name = function_call.name,
                        arguments = arguments.to_string(),
                        error = e.to_string()
                    )
                    .to_string();

                    // Record function execution error to step history
                    self.context.add_toolresult_record(
                        function_call.clone(),
                        StepStatus::Error.to_string(),
                        None,
                        Some(error_message.clone()),
                    );

                    // Get current step index
                    if self.context.step_state.step_index > 0 {
                        // Only update the current step's status, not the entire plan
                        let mut updated_plan = current_plan.clone();

                        // Record tool execution error to current step
                        let error_result = json!({
                            "status": StepStatus::Error.to_string(),
                            "message": error_message.clone(),
                            "timestamp": chrono::Utc::now().to_rfc3339()
                        });

                        // Update step result
                        if let Err(e) = updated_plan.update_step_result(
                            self.context.step_state.step_index,
                            error_result,
                            StepStatus::Error.to_string(),
                            vec![function_call.name.clone()],
                            0,
                            1,
                        ) {
                            debug!("Failed to update step result: {}", e);
                        }

                        // Update plan but don't mark as failed
                        self.plan_manager.update_plan(updated_plan.clone()).await?;
                        self.context.set_current_plan(updated_plan);
                    }

                    // Determine if this is a fatal error that requires updating the entire plan status
                    if self.is_fatal_error(&(e.clone().into())) {
                        // Update plan status
                        let updated_plan = self
                            .plan_manager
                            .record_plan_error(current_plan.id, error_message.clone())
                            .await?;
                        self.context.set_current_plan(updated_plan.clone());

                        // If retry count exceeds max retries, mark plan as failed
                        if updated_plan.retry_count > self.context.max_retries {
                            // Record final failure status to step history
                            self.context.add_completion_record(
                                StepStatus::Failed.to_string(), // Status
                                t!(
                                    "workflow.react.plan_execution_failed_max_retries",
                                    error = error_message
                                )
                                .to_string(), // Message
                            );

                            return Err(WorkflowError::MaxRetriesExceeded(
                                t!(
                                    "workflow.react.plan_max_retries_exceeded",
                                    error = error_message
                                )
                                .to_string(),
                            ));
                        }
                    }

                    // Create error result
                    let action_result = json!({
                        "status": StepStatus::Error.to_string(),
                        "function": function_call.name.clone(),
                        "parameters": arguments.clone(),
                        "error": t!(
                            "workflow.react.function_execution_failed_details",
                            function = function_call.name, param = arguments, error = e.to_string()
                        )
                        .to_string(),
                        "retry_count": current_plan.retry_count,
                        "max_retries": self.context.max_retries,
                        "timestamp": chrono::Utc::now().to_rfc3339()
                    });

                    // Create step result
                    StepResult::new(
                        StepStatus::Error,
                        Some(function_call.clone()),
                        None,
                        Some(action_result),
                        Some(StepStatus::Error),
                        Some(error_message.clone()),
                        Some(StepError::FunctionCallFailed),
                        None,
                    )
                }
            };
            Ok(function_result)
        } else {
            // No function call found in assistant message
            Ok(StepResult::new(
                StepStatus::Error,
                None,
                None,
                None,
                Some(StepStatus::Error),
                Some(t!("workflow.react.function_info_not_found").to_string()),
                Some(StepError::FunctionNotFound),
                None,
            ))
        }
    }

    /// Observing phase
    ///
    /// This function is part of the ReAct loop and is responsible for processing the results
    /// of the action phase. It analyzes the output of the executed tool and generates a summary
    /// or extracts relevant information based on the tool's purpose.
    ///
    /// For example:
    /// - For web search results, it extracts and summarizes the most relevant information.
    /// - For web crawling results, it generates a concise summary of the crawled content.
    /// - For other tools, it simply logs the raw result.
    ///
    /// # Arguments
    /// * `action_result` - The result of the action phase, containing the tool's output and status.
    ///
    /// # Returns
    /// * `Result<StepResult, WorkflowError>` - The result of the observation phase, which may include
    ///   the summarized information or an error if the observation fails.
    ///
    /// # Errors
    /// Returns an error if:
    /// - The tool's result is empty or invalid.
    /// - The current step cannot be retrieved from the context.
    /// - The function fails to process the tool's output.
    async fn observing(&mut self, action_result: StepResult) -> Result<StepResult, WorkflowError> {
        match action_result.status {
            // Tool call success
            StepStatus::Running => {
                if let Some(result) = &action_result.action_result {
                    if result["result"].is_null() {
                        return Err(WorkflowError::Execution(
                            t!("workflow.react.function_execution_no_result").to_string(),
                        ));
                    }
                    if let Some(function_call) = &action_result.function_call {
                        let current_step =
                            self.context.get_current_step().cloned().ok_or_else(|| {
                                WorkflowError::Execution(
                                    t!("workflow.react.cannot_get_current_step").to_string(),
                                )
                            })?;

                        match function_call.name.as_str() {
                            "web_search" => {
                                if let Some(arguments) = &function_call.arguments {
                                    let kw = WebSearch::extract_keywords(arguments)?;
                                    let dedup_tool = self
                                        .context
                                        .function_manager
                                        .get_tool("search_dedup")
                                        .await?;
                                    let result = dedup_tool
                                        .call(json!({"results": result["result"].clone(), "query": kw}))
                                        .await?;

                                    // Add function result message
                                    let result_str =
                                        serde_json::to_string(&result).unwrap_or_default();
                                    self.context.step_state.last_search_result = Some(result_str);
                                }
                            }
                            "web_crawler" => {
                                if let Some(obj) = &result["result"].as_object() {
                                    let last_search_result =
                                        self.context.step_state.last_search_result.clone();
                                    // remove from search result
                                    if let Some(sr) = last_search_result {
                                        let url = Self::get_string_from_map(&obj, "url");
                                        if let Ok(search_result) =
                                            serde_json::from_str::<Vec<SearchResult>>(&sr).map(
                                                |sr| {
                                                    sr.into_iter()
                                                        .filter(|s| s.url != url)
                                                        .collect::<Vec<_>>()
                                                },
                                            )
                                        {
                                            debug!(
                                                "Observing: remove search result by url: {}",
                                                &url
                                            );
                                            self.context.step_state.last_search_result = Some(
                                                serde_json::to_string(&search_result)
                                                    .unwrap_or_default(),
                                            );
                                        }
                                    }

                                    if let Some(content) =
                                        obj.get("content").and_then(|v| v.as_str())
                                    {
                                        debug!("Observing: web crawler result summary...");
                                        let title = Self::get_string_from_map(&obj, "title");
                                        let url = Self::get_string_from_map(&obj, "url");
                                        match self
                                            .summary_by_ai(&current_step, content, &title, &url)
                                            .await
                                        {
                                            Ok(()) => {
                                                self.context.step_state.snippets.last().map(|sn| {
                                                    let arg = action_result
                                                        .function_call
                                                        .as_ref()
                                                        .and_then(|fc| fc.arguments.as_ref())
                                                        .map(|arg| {
                                                            serde_json::to_string(&arg)
                                                                .unwrap_or_default()
                                                        })
                                                        .unwrap_or_default();
                                                    let tool_result = json!({
                                                        "tool": "web_crawler",
                                                        "arguments": arg,
                                                        "result": serde_json::to_string(&sn).unwrap_or_default()
                                                    });
                                                    self.context.step_state.last_tool_result =
                                                        Some(tool_result.to_string());
                                                });
                                            }
                                            Err(_) => {}
                                        }
                                    }
                                }
                            }
                            _ => {
                                let result_str = serde_json::to_string(&result).unwrap_or_default();
                                let param = action_result
                                    .function_call
                                    .as_ref()
                                    .and_then(|fc| fc.arguments.as_ref())
                                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                                    .unwrap_or_default();
                                let tool_result = json!({
                                    "tool": function_call.name.as_str(),
                                    "arguments": param,
                                    "result": result_str,
                                });
                                self.context.step_state.last_tool_result =
                                    Some(tool_result.to_string());
                                self.context.step_state.snippets.push(tool_result);
                            }
                        }
                    }
                }
            }
            StepStatus::Error => {
                match action_result.error_type {
                    Some(StepError::FunctionCallFailed) => {
                        let error_message = &action_result
                            .error_message
                            .as_ref()
                            .map_or("unknown error", |v| v);
                        let function_name = if let Some(fc) = &action_result.function_call {
                            fc.name.clone()
                        } else {
                            "unknown".to_string()
                        };
                        // Add function error message
                        self.context.add_tool_message(
                            function_name,
                            error_message.to_string(),
                            false,
                        );
                    }
                    Some(StepError::FunctionNotFound) => {
                        // Add function error message
                        self.context.add_tool_message(
                            "FunctionNotFoundError".to_string(),
                            t!("workflow.react.function_info_not_provided").to_string(), // Localized
                            false,
                        );
                    }
                    _ => {}
                }
            }
            StepStatus::Completed => {
                debug!("Observing stage: step completed");

                let summary =
                    serde_json::to_string(&self.context.step_state.snippets).unwrap_or_default();
                let result = json!({
                    "status": StepStatus::Completed.to_string(),
                    "timestamp": chrono::Utc::now().to_rfc3339()
                });

                // Record completion status to step history
                self.context.add_completion_record(
                    "success".to_string(),
                    t!("workflow.react.step_completed_message").to_string(),
                ); // Localized

                return Ok(StepResult::new(
                    StepStatus::Completed,
                    None,
                    Some(summary),
                    Some(result),
                    None,
                    None,
                    None,
                    None,
                ));
            }
            _ => {}
        }
        Ok(action_result.clone())
    }

    /// Extract relevant snippets from web content related to the current step and its goal,
    /// and generate a summary
    ///
    /// # Arguments
    /// * `current_step` - The current step
    /// * `content` - The content of the web page
    /// * `title` - The title of the web page
    /// * `url` - The URL of the web page
    ///
    /// # Returns
    /// * `Result<(), WorkflowError>` - The result of the operation
    ///
    /// # Errors
    /// * `WorkflowError` - If the operation fails
    async fn summary_by_ai(
        &mut self,
        current_step: &Step,
        content: &str,
        title: &str,
        url: &str,
    ) -> Result<(), WorkflowError> {
        let messages = vec![json!({
            "role": "user",
            "content": format!("{}\n\n当前时间:{}\n当前步骤:{}\n步骤目标：\n{}\n\n网页内容：\n[crawl data start]\n{}\n[crawl data end]\n", OBSERVATION_PROMPT, chrono::Utc::now().to_rfc3339(), &current_step.name, &current_step.goal, content)
        })];
        let chat_response = self.chat_completion(ModelName::General, messages).await?;

        chat_response["content"].as_str().map(|content| {
            serde_json::from_str::<serde_json::Value>(&format_json_str(content))
                .map_err(|e| {
                    WorkflowError::Execution(
                        t!(
                            "workflow.react.summary_json_parse_failed",
                            content = content,
                            error = e.to_string()
                        )
                        .to_string(),
                    )
                }) // Added error handling
                .map(|v| {
                    let summary = Self::get_string_from_value(&v, "summary");
                    debug!("Observing: web_crawler summary: {}", &summary);
                    if !summary.is_empty() {
                        self.context.step_state.goal_summaries.push(summary);
                    }

                    let snippet = Self::get_string_from_value(&v, "snippet");
                    debug!("Observing: web_crawler snippet: {}", &snippet);
                    if !snippet.is_empty() {
                        self.context.step_state.snippets.push(json!({
                            "url": url,
                            "title": title,
                            "content": snippet
                        }));
                    }

                    Ok::<(), WorkflowError>(())
                })
        }); // Propagate the error
        Ok(())
    }

    /// Chat completion
    ///
    /// # Arguments
    /// * `model_name` - Name of the model to use
    /// * `messages` - List of messages to send
    ///
    /// # Returns
    /// The chat completion response or an error if the call fails
    async fn chat_completion(
        &self,
        model_name: ModelName,
        messages: Vec<Value>,
    ) -> Result<Value, WorkflowError> {
        // Get chat completion function
        let function = self
            .context
            .function_manager
            .get_tool("chat_completion")
            .await?;

        let temperature = {
            match model_name {
                ModelName::Reasoning => 0.6,
                ModelName::General => 0.0,
            }
        };
        // Build parameters, avoid double JSON serialization
        let mut params = serde_json::Map::new();
        params.insert("model_name".to_string(), json!(model_name.as_ref()));
        params.insert("messages".to_string(), Value::Array(messages));
        params.insert("temperature".to_string(), json!(temperature));
        params.insert("top_p".to_string(), json!(0.9));

        // Execute function
        Ok(function
            .call(Value::Object(params))
            .await
            .map(|r| r.into())?)
    }

    /// Generate a plan based on user request, retry up to 3 times
    ///
    /// # Arguments
    /// * `user_request` - User's request
    ///
    /// # Returns
    /// The generated plan or an error if generation fails
    pub(crate) async fn generate_plan(
        &mut self,
        user_request: String,
    ) -> Result<Plan, WorkflowError> {
        // Generate plan
        for i in 0..3 {
            match self.generate_plan_inner(user_request.clone()).await {
                Ok(plan) => return Ok(plan),
                Err(e) => {
                    error!("Generate plan failed: {}, retrying ({}/3)...", e, i + 1);
                }
            }
        }
        Err(WorkflowError::MaxRetriesExceeded(
            t!("workflow.react.generate_plan_max_retries_exceeded").to_string(), // Localized
        ))
    }
    /// Generate a plan based on user request
    ///
    /// # Arguments
    /// * `user_request` - User's request
    ///
    /// # Returns
    /// The generated plan or an error if generation fails
    async fn generate_plan_inner(&mut self, user_request: String) -> Result<Plan, WorkflowError> {
        info!("根据用户请求生成计划: {}", user_request);

        // Clear previous context
        self.context.clear_messages();

        // Add user request with plan generation prompt
        self.context.add_user_message(format!(
            "{}\n\n用户请求: {}",
            PLAN_GENERATION_PROMPT, user_request
        ));

        // Get messages for chat completion
        let messages = self.context.get_chat_messages(None);

        // Call chat completion API for plan generation
        let chat_response = self
            .chat_completion(ModelName::Reasoning, messages)
            .await
            .map_err(|e| {
                WorkflowError::Config(
                    t!(
                        "workflow.react.plan_generation_chat_completion_failed",
                        error = e.to_string()
                    )
                    .to_string(),
                )
            })?;

        let content = chat_response["content"].as_str().ok_or_else(|| {
            WorkflowError::Config(t!("workflow.react.plan_generation_failed").to_string())
        })?;

        // Add assistant message to context
        self.context
            .add_assistant_message(content.to_string(), None);

        // Parse plan from assistant response
        let json_str_content = format_json_str(&content);
        let plan_json = serde_json::from_str::<Value>(&json_str_content).map_err(|e| {
            WorkflowError::Config(
                t!(
                    "workflow.react.plan_json_parse_failed",
                    json_string = json_str_content,
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;
        debug!("计划详情：{}", &content);

        // Extract plan details
        let plan_name = plan_json["plan_name"]
            .as_str()
            .ok_or_else(|| {
                WorkflowError::Config(t!("workflow.react.plan_name_parse_failed").to_string())
            })?
            .to_string();
        let plan_goal = plan_json["goal"]
            .as_str()
            .ok_or_else(|| {
                WorkflowError::Config(t!("workflow.react.plan_goal_parse_failed").to_string())
            })?
            .to_string();

        // Create plan
        let plan = self.create_plan(plan_name, plan_goal).await?;

        // Parse steps from plan JSON and store in plan
        let mut updated_plan = plan.clone();
        updated_plan
            .set_steps_from_json(plan_json.clone())
            .map_err(|e_str| {
                WorkflowError::Config(
                    t!("workflow.react.set_plan_steps_failed", error = e_str).to_string(),
                )
            })?;

        self.plan_manager.update_plan(updated_plan.clone()).await?;
        self.context.set_current_plan(updated_plan.clone());

        Ok(updated_plan)
    }

    /// Create a new plan
    ///
    /// # Arguments
    /// * `name` - Plan name
    /// * `goal` - Plan goal
    ///
    /// # Returns
    /// The newly created plan or an error if creation fails
    async fn create_plan(&mut self, name: String, goal: String) -> Result<Plan, WorkflowError> {
        let plan = self.plan_manager.create_plan(name, goal).await?;

        // clear messages
        self.context.clear_messages();

        // set current plan
        self.context.set_current_plan(plan.clone());

        Ok(plan)
    }

    /// Generate a summary of the plan execution
    ///
    /// # Arguments
    /// * `step_results` - Results of each step
    ///
    /// # Returns
    /// The summary or an error if generation fails
    async fn generate_summary(&mut self, step_results: Vec<Value>) -> Result<Value, WorkflowError> {
        info!("生成计划执行总结");
        // Clear previous context
        self.context.clear_messages();

        // Get current plan
        let current_plan = self.context.get_current_plan().ok_or_else(|| {
            WorkflowError::Config(t!("workflow.react.no_executing_plan").to_string())
        })?;

        // Add step results
        self.context.add_user_message(format!(
            "{}\n\n计划名称：{}\n\n计划目标：{}\n\n步骤执行结果：\n[Start of Steps Execution Results]\n{}\n[End of Steps Execution Results]",
            SUMMARY_PROMPT,
            current_plan.name,
            current_plan.goal,
            serde_json::to_string_pretty(&step_results).unwrap_or_default(),
        ));

        // Get messages for chat completion
        let messages = self.context.get_chat_messages(None);

        // Call chat completion API for summary generation
        let chat_response = self
            .chat_completion(ModelName::General, messages)
            .await
            .map_err(|e| {
                WorkflowError::Config(
                    t!(
                        "workflow.react.summary_chat_completion_failed",
                        error = e.to_string()
                    )
                    .to_string(),
                )
            })?;

        // Extract assistant response
        let content = chat_response["content"].as_str().unwrap_or("").to_string();

        // Add assistant message to context
        self.context.add_assistant_message(content.clone(), None);

        debug!("✅ 总结生成阶段调用聊天完成API成功: \n{}", content);

        // Create summary JSON
        let summary = json!({
            "plan_name": current_plan.name,
            "plan_goal": current_plan.goal,
            "step_results": step_results,
            "summary": content
        });

        Ok(summary)
    }

    /// Handles errors that occur during a ReAct phase (reasoning, acting, observing)
    ///
    /// # Arguments
    /// * `error` - The error that occurred
    /// * `phase_name` - Name of the phase (reasoning, acting, observing)
    /// * `step_index` - Index of the current step
    /// * `step_name` - Name of the current step
    /// * `step_goal` - Goal of the current step
    /// * `retry_count` - Current retry count (will be incremented)
    /// * `react_cycle_count` - Current ReAct cycle count
    /// * `tools_used` - List of tools used in this step
    ///
    /// # Returns
    /// Some(WorkflowError) if execution should stop, None if it should continue
    fn handle_phase_error(
        &mut self,
        error: WorkflowError,
        phase_name: &str,
        step_index: usize,
        step_name: &str,
        step_goal: &str,
        retry_count: &mut u32,
        react_cycle_count: u32,
        tools_used: &Vec<String>,
    ) -> Option<WorkflowError> {
        // Format error message
        let error_msg = t!(
            "workflow.react.phase_error_details",
            phase = phase_name,
            error = error.to_string()
        )
        .to_string();
        warn!("{}", error_msg);

        // Record error and increment retry count
        self.context.add_user_message(
            t!(
                "workflow.react.phase_error_user_message",
                phase = phase_name,
                error = error_msg
            )
            .to_string(),
        );
        *retry_count += 1;

        // If exceeded maximum retries, record step failure
        if *retry_count >= self.context.max_retries {
            // Record step failure
            self.context.add_step_execution_record(
                "failed".to_string(), // Action
                step_index,
                step_name.to_string(),
                step_goal.to_string(), // Goal
                Some(error_msg.clone()),
                Some(*retry_count),
                Some(self.context.max_retries),
                Some(react_cycle_count),
                Some(tools_used.clone()),
            );

            // Return error result
            let final_error_msg = t!(
                "workflow.react.step_max_retries_exceeded",
                step_name = step_name,
                error = error_msg
            )
            .to_string();
            return Some(WorkflowError::MaxRetriesExceeded(final_error_msg));
        }

        // If not exceeded maximum retries, continue to next iteration
        None
    }

    /// Determines if an error is fatal (requires plan failure)
    ///
    /// # Arguments
    /// * `error` - The error to check
    ///
    /// # Returns
    /// true if the error is fatal, false otherwise
    fn is_fatal_error(&self, error: &WorkflowError) -> bool {
        match error {
            WorkflowError::Io(_)
            | WorkflowError::Cancelled(_)
            | WorkflowError::Store(_)
            | WorkflowError::MaxRetriesExceeded(_) => true,
            _ => false,
        }
    }

    /// Get string value from JSON
    ///
    /// # Arguments
    /// * `value` - JSON value
    /// * `key` - Key to look up
    ///
    /// # Returns
    /// The value associated with the key or an empty string if the key is not found
    fn get_string_from_value(value: &serde_json::Value, key: &str) -> String {
        value
            .get(key)
            .and_then(|s| s.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default()
    }

    /// Get string value from map
    ///
    /// # Arguments
    /// * `value` - Map of values
    /// * `key` - Key to look up
    ///
    /// # Returns
    /// The value associated with the key or an empty string if the key is not found
    fn get_string_from_map(value: &serde_json::Map<String, Value>, key: &str) -> String {
        value
            .get(key)
            .and_then(|s| s.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default()
    }
}

mod tests {
    #[allow(unused_imports)]
    use super::*;

    #[tokio::test]
    async fn test_react_executor() {
        let db_path = crate::constants::STORE_DIR
            .read()
            .clone()
            .join("chatspeed.db");
        let main_store = Arc::new(std::sync::RwLock::new(MainStore::new(db_path).unwrap()));
        let chat_state =
            ChatState::new(Arc::new(crate::libs::window_channels::WindowChannels::new()));
        let mut exe = ReactExecutor::new(main_store, chat_state, 10)
            .await
            .unwrap();
        let plan = exe
            .execute("据说幻方的规模下降了不少，是真的吗？".to_string())
            .await
            .unwrap();
        println!("Plan: {:#?}", plan);
    }

    #[tokio::test]
    async fn test_web_search() {
        let db_path = crate::constants::STORE_DIR
            .read()
            .clone()
            .join("chatspeed.db");
        let main_store = Arc::new(std::sync::RwLock::new(MainStore::new(db_path).unwrap()));
        let chat_state =
            ChatState::new(Arc::new(crate::libs::window_channels::WindowChannels::new()));
        let exe = ReactExecutor::new(main_store, chat_state, 10)
            .await
            .unwrap();
        let ws = exe
            .context
            .function_manager
            .get_tool("web_search")
            .await
            .unwrap();
        let result = ws
            .call(json!({"provider":"google","kw": ["五粮液股票"]}))
            .await;
        println!("Result: {:#?}", result);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_web_crawler() {
        let db_path = crate::constants::STORE_DIR
            .read()
            .clone()
            .join("chatspeed.db");
        let main_store = Arc::new(std::sync::RwLock::new(MainStore::new(db_path).unwrap()));
        let chat_state =
            ChatState::new(Arc::new(crate::libs::window_channels::WindowChannels::new()));
        let exe = ReactExecutor::new(main_store, chat_state, 10)
            .await
            .unwrap();
        let ws = exe
            .context
            .function_manager
            .get_tool("web_crawler")
            .await
            .unwrap();
        let result = ws
            .call(json!({"url":"https://guba.eastmoney.com/list,000858,1370734419.html"}))
            .await;
        println!("Result: {:#?}", result);
        assert!(result.is_ok());
    }
}
