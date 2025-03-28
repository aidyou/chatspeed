use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fmt;

/// Plan status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PlanStatus {
    /// Waiting to execute
    Pending,
    /// Executing
    Running,
    /// Failed
    Failed,
    /// Completed
    Completed,
}

impl fmt::Display for PlanStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlanStatus::Pending => write!(f, "pending"),
            PlanStatus::Running => write!(f, "running"),
            PlanStatus::Failed => write!(f, "failed"),
            PlanStatus::Completed => write!(f, "completed"),
        }
    }
}
/// Plan structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    /// Plan ID, starting from 1
    pub id: u32,
    /// Plan name
    pub name: String,
    /// Plan goal
    pub goal: String,
    /// Execution status
    pub status: PlanStatus,
    /// Error message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    /// Retry count
    pub retry_count: u32,
    /// Plan steps
    pub steps: Vec<Step>,
    /// Created time
    pub created_at: DateTime<Utc>,
    /// Updated time
    pub updated_at: DateTime<Utc>,
}

impl Plan {
    /// Creates a new plan
    ///
    /// # Arguments
    /// * `id` - ID of the plan
    /// * `name` - Name of the plan
    /// * `goal` - Goal of the plan
    ///
    /// # Returns
    /// A new Plan with the specified ID, name, and goal
    pub fn new(id: u32, name: String, goal: String) -> Self {
        let now = Utc::now();
        Self {
            id,
            name,
            goal,
            status: PlanStatus::Pending,
            error_message: None,
            retry_count: 0,
            steps: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    /// Updates the status of the plan
    ///
    /// # Arguments
    /// * `status` - New status for the plan
    ///
    /// # Returns
    /// The updated Plan
    pub fn update_status(&mut self, status: PlanStatus) {
        self.status = status;
        self.updated_at = Utc::now();
    }

    /// Records an error for the plan
    ///
    /// # Arguments
    /// * `error` - Error message to record
    ///
    /// # Returns
    /// The updated Plan
    pub fn record_error(&mut self, error: String) {
        self.status = PlanStatus::Failed;
        self.error_message = Some(error);
        self.retry_count += 1;
        self.updated_at = Utc::now();
    }

    /// Sets the steps of the plan from a JSON value
    ///
    /// # Arguments
    /// * `plan_json` - JSON value containing plan steps
    ///
    /// # Returns
    /// The updated Plan
    pub fn set_steps_from_json(&mut self, plan_json: Value) -> Result<(), String> {
        if let Some(steps_array) = plan_json.get("steps").and_then(|s| s.as_array()) {
            self.steps.clear();

            for (index, step) in steps_array.iter().enumerate() {
                let name = step
                    .get("name")
                    .and_then(|n| n.as_str())
                    .ok_or_else(|| format!("步骤 {} 名称解析失败", index))?
                    .to_string();

                let goal = step
                    .get("goal")
                    .and_then(|g| g.as_str())
                    .ok_or_else(|| format!("步骤 {} 目标解析失败", index))?
                    .to_string();

                self.steps.push(Step::new(name, goal, index));
            }

            self.updated_at = Utc::now();
            Ok(())
        } else {
            Err("计划JSON中没有steps数组".to_string())
        }
    }

    /// Gets a step by index
    ///
    /// # Arguments
    /// * `index` - Index of the step to get
    ///
    /// # Returns
    /// The step at the specified index, or None if not found
    pub fn get_step(&self, index: usize) -> Option<&Step> {
        self.steps.get(index)
    }

    /// Gets a mutable step by index
    ///
    /// # Arguments
    /// * `index` - Index of the step to get
    ///
    /// # Returns
    /// The mutable step at the specified index, or None if not found
    pub fn get_step_mut(&mut self, index: usize) -> Option<&mut Step> {
        self.steps.get_mut(index)
    }

    /// Updates a step's result
    ///
    /// # Arguments
    /// * `index` - Index of the step to update
    /// * `result` - Result value to store with the step
    /// * `status` - Status of the step
    /// * `tools_used` - Tools used in this step
    /// * `retry_count` - Retry count for this step
    /// * `react_cycles` - React cycles for this step
    ///
    /// # Returns
    /// Result indicating success or failure
    pub fn update_step_result(
        &mut self,
        index: usize,
        result: Value,
        status: String,
        tools_used: Vec<String>,
        retry_count: u32,
        react_cycles: u32,
    ) -> Result<(), String> {
        if let Some(step) = self.get_step_mut(index) {
            step.set_result(result, status, tools_used, retry_count, react_cycles);
            self.updated_at = Utc::now();
            Ok(())
        } else {
            Err(format!("找不到索引为 {} 的步骤", index))
        }
    }

    /// Completes the plan
    ///
    /// # Arguments
    /// * `summary` - Optional summary value to store with the last step
    ///
    /// # Returns
    /// The completed Plan
    pub fn complete(&mut self, summary: Option<Value>) {
        self.status = PlanStatus::Completed;
        if let Some(summary_value) = summary {
            // 如果有步骤，将摘要添加到最后一个步骤的结果中
            if let Some(last_step) = self.steps.last_mut() {
                if let Some(result) = &mut last_step.result {
                    // 将摘要添加到现有结果中
                    if let Some(obj) = result.as_object_mut() {
                        obj.insert("summary".to_string(), summary_value.clone());
                    }
                } else {
                    // 如果没有结果，创建一个包含摘要的新结果
                    last_step.result = Some(json!({
                        "summary": summary_value
                    }));
                }
            }
        }
        self.updated_at = Utc::now();
    }

    /// Resets the plan status to pending
    ///
    /// # Returns
    /// The reset Plan
    pub fn reset(&mut self) {
        self.status = PlanStatus::Pending;
        self.error_message = None;
        self.updated_at = Utc::now();
    }
}

/// Step structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    /// Step name
    pub name: String,
    /// Step goal
    pub goal: String,
    /// Step index in the plan
    pub index: usize,
    /// Step execution result
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Step status
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Tools used in this step
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools_used: Option<Vec<String>>,
    /// Retry count for this step
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_count: Option<u32>,
    /// React cycles for this step
    #[serde(skip_serializing_if = "Option::is_none")]
    pub react_cycles: Option<u32>,
}

impl Step {
    /// Creates a new step
    ///
    /// # Arguments
    /// * `name` - Name of the step
    /// * `goal` - Goal of the step
    /// * `index` - Index of the step in the plan
    ///
    /// # Returns
    /// A new Step with the specified name, goal, index, and dependencies
    pub fn new(name: String, goal: String, index: usize) -> Self {
        Self {
            name,
            goal,
            index,
            result: None,
            status: None,
            tools_used: None,
            retry_count: None,
            react_cycles: None,
        }
    }

    /// Sets the result of the step
    ///
    /// # Arguments
    /// * `result` - Result value to store with the step
    /// * `status` - Status of the step
    /// * `tools_used` - Tools used in this step
    /// * `retry_count` - Retry count for this step
    /// * `react_cycles` - React cycles for this step
    ///
    /// # Returns
    /// The updated Step
    pub fn set_result(
        &mut self,
        result: Value,
        status: String,
        tools_used: Vec<String>,
        retry_count: u32,
        react_cycles: u32,
    ) {
        self.result = Some(result);
        self.status = Some(status);
        self.tools_used = Some(tools_used);
        self.retry_count = Some(retry_count);
        self.react_cycles = Some(react_cycles);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum StepError {
    /// Function call failed
    FunctionCallFailed,
    /// Function not found
    FunctionNotFound,
    /// Other error
    Other(String),
}

impl fmt::Display for StepError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StepError::FunctionCallFailed => write!(f, "function call failed"),
            StepError::FunctionNotFound => write!(f, "function not found"),
            StepError::Other(msg) => write!(f, "other error: {}", msg),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum StepStatus {
    /// Step is running
    Running,
    /// Step is completed
    Completed,
    /// Action completed successfully, but step may need more actions
    Success,
    /// Step is failed
    Error,
    /// Unknown status
    Unknown,
    /// Step is failed
    Failed,
}

impl fmt::Display for StepStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StepStatus::Running => write!(f, "running"),
            StepStatus::Completed => write!(f, "completed"),
            StepStatus::Success => write!(f, "success"),
            StepStatus::Error => write!(f, "error"),
            StepStatus::Unknown => write!(f, "unknown"),
            StepStatus::Failed => write!(f, "failed"),
        }
    }
}

impl From<&str> for StepStatus {
    fn from(s: &str) -> Self {
        match s {
            "running" => StepStatus::Running,
            "completed" => StepStatus::Completed,
            "success" => StepStatus::Success,
            "error" => StepStatus::Error,
            "failed" => StepStatus::Failed,
            _ => StepStatus::Unknown,
        }
    }
}

/// Function call structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: Option<Value>,
}

impl FunctionCall {
    pub fn new(name: String, arguments: Option<Value>) -> Self {
        Self { name, arguments }
    }
}
impl fmt::Display for FunctionCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            json!({"name": self.name, "arguments": self.arguments})
        )
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub status: StepStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_call: Option<FunctionCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observation_status: Option<StepStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_type: Option<StepError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
}

impl StepResult {
    pub fn new(
        status: StepStatus,
        function_call: Option<FunctionCall>,
        summary: Option<String>,
        action_result: Option<Value>,
        observation_status: Option<StepStatus>,
        error_message: Option<String>,
        error_type: Option<StepError>,
        reasoning: Option<String>,
    ) -> Self {
        Self {
            status,
            function_call,
            summary,
            action_result,
            observation_status,
            error_message,
            error_type,
            reasoning,
        }
    }
}

impl Default for StepResult {
    fn default() -> Self {
        Self {
            status: StepStatus::Running,
            function_call: None,
            summary: None,
            action_result: None,
            observation_status: None,
            error_message: None,
            error_type: None,
            reasoning: None,
        }
    }
}

/// Step record type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum StepRecordType {
    /// Plan execution
    PlanExecution,
    /// Step execution
    StepExecution,
    /// ReAct cycle
    ReactCycle,
    /// Tool call
    ToolCall,
    /// Tool result
    ToolResult,
    /// Completion
    Completion,
}

impl fmt::Display for StepRecordType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StepRecordType::PlanExecution => write!(f, "plan_execution"),
            StepRecordType::StepExecution => write!(f, "step_execution"),
            StepRecordType::ReactCycle => write!(f, "react_cycle"),
            StepRecordType::ToolCall => write!(f, "tool_call"),
            StepRecordType::ToolResult => write!(f, "tool_result"),
            StepRecordType::Completion => write!(f, "completion"),
        }
    }
}

/// Base step record structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepRecord {
    /// Record type
    pub record_type: StepRecordType,
    /// Timestamp
    pub timestamp: DateTime<Utc>,
    /// Additional data
    #[serde(flatten)]
    pub data: Value,
}

impl StepRecord {
    /// Creates a new step record
    ///
    /// # Arguments
    /// * `record_type` - Type of the record
    /// * `data` - Additional data for the record
    ///
    /// # Returns
    /// A new StepRecord with the specified type and data
    pub fn new(record_type: StepRecordType, data: Value) -> Self {
        Self {
            record_type,
            timestamp: Utc::now(),
            data,
        }
    }

    /// Creates a new plan execution record
    ///
    /// # Arguments
    /// * `record` - The plan execution record data
    ///
    /// # Returns
    /// A new StepRecord with PlanExecution type
    pub fn plan_execution(record: PlanExecutionRecord) -> Result<Self, serde_json::Error> {
        Ok(Self::new(
            StepRecordType::PlanExecution,
            serde_json::to_value(record)?,
        ))
    }

    /// Creates a new step execution record
    ///
    /// # Arguments
    /// * `record` - The step execution record data
    ///
    /// # Returns
    /// A new StepRecord with StepExecution type
    pub fn step_execution(record: StepExecutionRecord) -> Result<Self, serde_json::Error> {
        Ok(Self::new(
            StepRecordType::StepExecution,
            serde_json::to_value(record)?,
        ))
    }

    /// Creates a new react cycle record
    ///
    /// # Arguments
    /// * `record` - The react cycle record data
    ///
    /// # Returns
    /// A new StepRecord with ReactCycle type
    pub fn react_cycle(record: ReactCycleRecord) -> Result<Self, serde_json::Error> {
        Ok(Self::new(
            StepRecordType::ReactCycle,
            serde_json::to_value(record)?,
        ))
    }

    /// Creates a new completion record
    ///
    /// # Arguments
    /// * `record` - The completion record data
    ///
    /// # Returns
    /// A new StepRecord with Completion type
    pub fn completion(record: CompletionRecord) -> Result<Self, serde_json::Error> {
        Ok(Self::new(
            StepRecordType::Completion,
            serde_json::to_value(record)?,
        ))
    }
}

pub struct StepState {
    /// Completed sub-goals of the step
    pub goal_summaries: Vec<String>,
    /// Snippets of information related to the step goal
    pub snippets: Vec<Value>,
    /// Last search result
    pub last_search_result: Option<String>,
    /// Last tool result
    pub last_tool_result: Option<String>,
    /// Current step index
    pub step_index: usize,
}

impl StepState {
    /// Creates a new step state
    ///
    /// # Returns
    /// A new StepState
    pub fn new() -> Self {
        Self {
            goal_summaries: Vec::new(),
            snippets: Vec::new(),
            last_search_result: None,
            last_tool_result: None,
            step_index: 0,
        }
    }

    /// Initializes the step state
    ///
    /// # Arguments
    /// * `step_index` - The step index
    ///
    /// # Returns
    /// A new StepState
    pub fn init(&mut self, step_index: usize) {
        self.step_index = step_index;
        self.goal_summaries.clear();
        self.snippets.clear();
        self.last_search_result = None;
    }
}

/// Plan execution record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanExecutionRecord {
    /// Action (start or end)
    pub action: String,
    /// Plan name
    pub plan_name: String,
    /// Plan ID
    pub plan_id: u32,
    /// Total steps (for start action)
    pub total_steps: Option<usize>,
}

/// Step execution record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepExecutionRecord {
    /// Action (start, completed, or failed)
    pub action: String,
    /// Step index
    pub step_index: usize,
    /// Step name
    pub step_name: String,
    /// Step goal (for start action)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_goal: Option<String>,
    /// Error message (for failed action)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Retry count (for failed action)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_count: Option<u32>,
    /// Max retries (for failed action)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,
    /// React cycles (for completed action)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub react_cycles: Option<u32>,
    /// Tools used (for completed action)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools_used: Option<Vec<String>>,
}

/// ReAct cycle record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReactCycleRecord {
    /// Action (start or end)
    pub action: String,
    /// Step index
    pub step_index: usize,
    /// Step name
    pub step_name: String,
    /// Cycle number
    pub cycle_number: u32,
    /// Result status (for end action)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_status: Option<String>,
}

/// Tool call record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    /// Tool name
    pub tool: String,
    /// Parameters
    pub parameters: Value,
}

/// Tool result record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultRecord {
    /// Tool name
    pub tool: String,
    /// Arguments
    pub arguments: Value,
    /// Status
    pub status: String,
    /// Result (for success status)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Error message (for error status)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Completion record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRecord {
    /// Status
    pub status: String,
    /// Message
    pub message: String,
}

/// Message role
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MessageRole {
    /// User message
    User,
    /// Assistant message
    Assistant,
    /// Tool call result
    Tool,
    /// System message
    System,
}

impl fmt::Display for MessageRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MessageRole::User => write!(f, "user"),
            MessageRole::Assistant => write!(f, "assistant"),
            MessageRole::Tool => write!(f, "tool"),
            MessageRole::System => write!(f, "system"),
        }
    }
}

/// Dialog message
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    /// Message role
    pub role: MessageRole,
    /// Message content
    pub content: String,
    /// Function call information
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function_call: Option<String>,
    /// Tool call ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl Message {
    /// Creates a user message
    ///
    /// # Arguments
    /// * `content` - Message content
    ///
    /// # Returns
    /// A new Message with the specified content and user role
    pub fn user(content: String) -> Self {
        Self {
            role: MessageRole::User,
            content,
            function_call: None,
            tool_call_id: None,
        }
    }

    /// Creates an assistant message
    ///
    /// # Arguments
    /// * `content` - Message content
    /// * `function_call` - Optional function call information
    ///
    /// # Returns
    /// A new Message with the specified content and assistant role
    pub fn assistant(content: String, function_call: Option<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content,
            function_call,
            tool_call_id: None,
        }
    }

    /// Creates a tool call result message
    ///
    /// # Arguments
    /// * `name` - Tool name
    /// * `content` - Message content
    ///
    /// # Returns
    /// A new Message with the specified content and tool role
    pub fn tool(name: String, content: String) -> Self {
        Self {
            role: MessageRole::Tool,
            content,
            function_call: None,
            tool_call_id: Some(name),
        }
    }
}
