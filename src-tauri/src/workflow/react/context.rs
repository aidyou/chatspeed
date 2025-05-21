use std::sync::Arc;

use serde_json::Value;

use crate::workflow::{
    react::types::{Message, Plan, StepState},
    tool_manager::ToolManager,
};

use super::{
    types::{
        CompletionRecord, FunctionCall, PlanExecutionRecord, ReactCycleRecord, Step,
        StepExecutionRecord, StepRecord, StepRecordType, ToolCallRecord, ToolResultRecord,
    },
    MessageRole,
};

/// ReAct execution context
pub struct ReactContext {
    /// Conversation history
    pub messages: Vec<Message>,
    /// Current plan being executed
    pub current_plan: Option<Plan>,
    /// Function manager
    pub function_manager: Arc<ToolManager>,
    /// Maximum retries
    pub max_retries: u32,
    /// Step execution history
    pub step_history: Vec<serde_json::Value>,
    /// Step state
    pub step_state: StepState,
}

impl ReactContext {
    /// Creates a new execution context
    ///
    /// # Arguments
    /// * `function_manager` - Function manager
    /// * `chat_state` - Chat state
    /// * `main_store` - Main storage
    /// * `max_retries` - Maximum retries
    ///
    /// # Returns
    /// A new ReactContext
    pub fn new(function_manager: Arc<ToolManager>, max_retries: u32) -> Self {
        Self {
            messages: Vec::new(),
            current_plan: None,
            function_manager,
            max_retries,
            step_history: Vec::new(),
            step_state: StepState::new(),
        }
    }

    /// Adds a user message
    ///
    /// # Arguments
    /// * `content` - Message content
    ///
    /// # Returns
    /// The updated ReactContext
    pub fn add_user_message(&mut self, content: String) {
        self.messages.push(Message::user(content));
    }

    /// Adds an assistant message
    ///
    /// # Arguments
    /// * `content` - Message content
    /// * `function_call` - Optional function call information
    ///
    /// # Returns
    /// The updated ReactContext
    pub fn add_assistant_message(&mut self, content: String, function_call: Option<String>) {
        self.messages
            .push(Message::assistant(content, function_call));
    }

    /// Adds a function call result message
    ///
    /// # Arguments
    /// * `name` - Function name
    /// * `content` - Message content
    ///
    /// # Returns
    /// The updated ReactContext
    pub fn add_tool_message(&mut self, name: String, content: String, is_front: bool) {
        if is_front {
            self.messages.insert(0, Message::tool(name, content));
        } else {
            self.messages.push(Message::tool(name, content));
        }
    }

    /// Sets the current plan
    ///
    /// # Arguments
    /// * `plan` - Plan to set as current
    ///
    /// # Returns
    /// The updated ReactContext
    pub fn set_current_plan(&mut self, plan: Plan) {
        self.current_plan = Some(plan);
    }

    /// Gets the current plan
    ///
    /// # Returns
    /// The current plan
    pub fn get_current_plan(&self) -> Option<Plan> {
        self.current_plan.clone()
    }

    /// Gets the current step
    ///
    /// # Returns
    /// The current step
    pub fn get_current_step(&self) -> Option<&Step> {
        self.current_plan
            .as_ref()
            .and_then(|plan| plan.steps.get(self.step_state.step_index))
    }

    /// Gets the length of the current plan
    ///
    /// # Returns
    /// The length of the current plan
    pub fn get_step_len(&self) -> usize {
        self.current_plan
            .as_ref()
            .map_or(0, |plan| plan.steps.len())
    }

    /// Clears the message history
    ///
    /// # Returns
    /// The updated ReactContext
    pub fn clear_messages(&mut self) {
        self.messages.clear();
    }

    /// Keeps the last 5 tool call error messages
    ///
    /// # Returns
    /// The updated ReactContext
    pub fn keep_last_tool_call_error(&mut self) {
        let last_five_tool_messages: Vec<Message> = self
            .messages
            .iter()
            .rev()
            .take_while(|msg| msg.role == MessageRole::Tool)
            .take(5)
            .cloned()
            .collect();

        // clear messages
        self.messages.clear();

        // add last 5 tool call error messages
        self.messages
            .extend(last_five_tool_messages.into_iter().rev());
    }

    /// Gets the message history
    ///
    /// # Returns
    /// The message history
    pub fn get_messages(&self) -> Vec<Message> {
        self.messages.clone()
    }

    /// Converts the conversation history to chat completion format
    ///
    /// # Returns
    /// The chat completion format
    pub fn get_chat_messages(&self, count: Option<usize>) -> Vec<serde_json::Value> {
        let max_len = count.unwrap_or(self.messages.len());
        self.messages
            .iter()
            .take(max_len)
            .map(|msg| {
                // Basic message structure
                let message = match msg.role {
                    // For function messages, format is {role: "function", name: "function_name", content: "result"}
                    MessageRole::Tool => {
                        if let Some(name) = &msg.tool_call_id {
                            serde_json::json!({
                                "role": msg.role.to_string(),
                                "content": msg.content,
                                "tool_call_id": name,
                            })
                        } else {
                            // Function messages must have a name, if not, use default name
                            serde_json::json!({
                                "role": msg.role.to_string(),
                                "content": msg.content,
                                "tool_call_id": "unknown_function",
                            })
                        }
                    }
                    // Assistant message format is {role: "assistant", content: "...", function_call: {...}}
                    MessageRole::Assistant => {
                        let mut base_msg = serde_json::json!({
                            "role": msg.role.to_string(),
                            "content": msg.content,
                        });

                        // Assistant message may contain function call
                        if let Some(function_call) = &msg.function_call {
                            base_msg["function_call"] =
                                serde_json::from_str(&function_call).unwrap_or_default();
                        }

                        base_msg
                    }
                    // Other message format is {role: "...", content: "..."}
                    _ => {
                        serde_json::json!({
                            "role": msg.role.to_string(),
                            "content": msg.content,
                        })
                    }
                };

                message
            })
            .collect()
    }

    /// Get step history
    ///
    /// # Returns
    /// The step history
    pub fn get_step_history(&self) -> &[serde_json::Value] {
        &self.step_history
    }

    /// Start a new ReAct cycle for a step
    ///
    /// # Arguments
    /// * `step_index` - Index of the step
    /// * `step_name` - Name of the step
    /// * `cycle_number` - Number of the cycle
    ///
    /// # Returns
    /// Updated ReactContext
    pub fn start_react_cycle(
        &mut self,
        step_index: usize,
        step_name: &str,
        cycle_number: u32,
    ) -> &mut Self {
        self.step_state.step_index = step_index;
        self.add_react_cycle_record(
            "start".to_string(),
            step_index,
            step_name.to_string(),
            cycle_number,
            None,
        )
    }

    /// End the current ReAct cycle for a step
    ///
    /// # Arguments
    /// * `step_index` - Index of the step
    /// * `step_name` - Name of the step
    /// * `cycle_number` - Number of the cycle
    /// * `status` - Status of the cycle
    ///
    /// # Returns
    /// Updated ReactContext
    pub fn end_react_cycle(
        &mut self,
        step_index: usize,
        step_name: &str,
        cycle_number: u32,
        status: &str,
    ) -> &mut Self {
        self.add_react_cycle_record(
            "end".to_string(),
            step_index,
            step_name.to_string(),
            cycle_number,
            Some(status.to_string()),
        )
    }

    // =================================================
    // Step Record Management
    // =================================================

    /// Add step execution record to history
    ///
    /// # Parameters
    /// * `record` - Step record to add
    ///
    /// # Returns
    /// Updated ReactContext
    pub fn add_step_record(&mut self, record: StepRecord) -> &mut Self {
        log::debug!(
            "Step Record: \n{}",
            serde_json::to_string_pretty(&record).unwrap_or_default()
        );
        if let Ok(record_json) = serde_json::to_value(&record) {
            self.step_history.push(record_json);
        } else {
            log::error!("Failed to serialize step record: {:?}", record); // Log serialization error
        }
        self
    }
    /// Add plan execution record to history
    ///
    /// # Arguments
    /// * `action` - Action to perform
    /// * `current_plan` - Current plan
    /// * `steps` - Steps of the plan
    ///
    /// # Returns
    /// Updated ReactContext
    pub fn add_plan_execution_record(
        &mut self,
        action: String,
        current_plan: &Plan,
        steps: &[Step],
    ) -> &mut Self {
        let plan_execution_record = PlanExecutionRecord {
            action,
            plan_name: current_plan.name.clone(),
            plan_id: current_plan.id,
            total_steps: Some(steps.len()),
        };

        let step_record = StepRecord::new(
            StepRecordType::PlanExecution,
            serde_json::to_value(plan_execution_record).unwrap_or_default(),
        );

        self.add_step_record(step_record);
        self
    }

    /// Add step execution record to history
    ///
    /// # Arguments
    /// * `step_index` - Index of the step
    /// * `step_name` - Name of the step
    /// * `step_goal` - Goal of the step
    /// * `step_dependencies` - Dependencies of the step
    /// * `error` - Optional error message
    /// * `retry_count` - Optional retry count
    /// * `max_retries` - Optional maximum retries
    /// * `react_cycles` - Optional number of react cycles
    /// * `tools_used` - Optional tools used
    ///
    /// # Returns
    /// Updated ReactContext
    pub fn add_step_execution_record(
        &mut self,
        action: String,
        step_index: usize,
        step_name: String,
        step_goal: String,
        error: Option<String>,
        retry_count: Option<u32>,
        max_retries: Option<u32>,
        react_cycles: Option<u32>,
        tools_used: Option<Vec<String>>,
    ) -> &mut Self {
        let step_execution_record = StepExecutionRecord {
            action,
            step_index,
            step_name: step_name.clone(),
            step_goal: Some(step_goal.clone()),
            error,
            retry_count,
            max_retries,
            react_cycles,
            tools_used,
        };

        let step_record = StepRecord::new(
            StepRecordType::StepExecution,
            serde_json::to_value(step_execution_record).unwrap_or_default(),
        );

        self.add_step_record(step_record);
        self
    }

    /// Add React cycle record to history
    ///
    /// # Arguments
    /// * `action` - Action to perform
    /// * `step_index` - Index of the step
    /// * `step_name` - Name of the step
    /// * `cycle_number` - Number of the cycle
    /// * `result_status` - Optional result status
    ///
    /// # Returns
    /// Updated ReactContext
    pub fn add_react_cycle_record(
        &mut self,
        action: String,
        step_index: usize,
        step_name: String,
        cycle_number: u32,
        result_status: Option<String>,
    ) -> &mut Self {
        let step_record = StepRecord::new(
            StepRecordType::ReactCycle,
            serde_json::to_value(ReactCycleRecord {
                action,
                step_index,
                step_name,
                cycle_number,
                result_status,
            })
            .unwrap_or_default(),
        );

        self.add_step_record(step_record);
        self
    }

    /// Add completion record to history
    ///
    /// # Arguments
    /// * `status` - Completion status
    /// * `message` - Completion message
    ///
    /// # Returns
    /// Updated ReactContext
    pub fn add_completion_record(&mut self, status: String, message: String) -> &mut Self {
        let step_record = StepRecord::new(
            StepRecordType::Completion,
            serde_json::to_value(CompletionRecord { status, message }).unwrap_or_default(),
        );
        self.add_step_record(step_record);
        self
    }

    /// Add tool call record to history
    ///
    /// # Arguments
    /// * `function_call` - Function call to record
    ///
    /// # Returns
    /// Updated ReactContext
    pub fn add_toolcall_record(&mut self, function_call: FunctionCall) -> &mut Self {
        let tool_call_record = ToolCallRecord {
            tool: function_call.name.to_string(),
            parameters: function_call.arguments.clone().unwrap_or_default(),
        };

        let step_record = StepRecord::new(
            StepRecordType::ToolCall,
            serde_json::to_value(tool_call_record).unwrap_or_default(),
        );

        self.add_step_record(step_record);
        self
    }

    /// Add tool result record to history
    ///
    /// # Arguments
    /// * `function_call` - Function call to record
    /// * `status` - Status of the tool call
    /// * `result` - Optional result of the tool call
    /// * `error` - Optional error message
    ///
    /// # Returns
    /// Updated ReactContext
    pub fn add_toolresult_record(
        &mut self,
        function_call: FunctionCall,
        status: String,
        result: Option<Value>,
        error: Option<String>,
    ) -> &mut Self {
        let tool_result_record = ToolResultRecord {
            tool: function_call.name.clone(),
            arguments: function_call.arguments.clone().unwrap_or_default(),
            status,
            result,
            error,
        };

        // 使用 StepRecord::new 创建步骤记录
        let step_record = StepRecord::new(
            StepRecordType::ToolResult,
            serde_json::to_value(tool_result_record).unwrap_or_default(),
        );

        self.add_step_record(step_record);
        self
    }
}
