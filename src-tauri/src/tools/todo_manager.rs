use async_trait::async_trait;
use serde_json::{json, Value};
use crate::tools::{ToolDefinition, NativeToolResult, ToolCallResult, ToolCategory};
use crate::ai::traits::chat::MCPToolDeclaration;

/// Todo list management (Full Claude Code Clone)
pub struct TodoCreateTool;
#[async_trait]
impl ToolDefinition for TodoCreateTool {
    fn name(&self) -> &str { "todo_create" }
    fn description(&self) -> &str { 
        "Use this tool to create a structured task list for your current coding session. This helps you track progress, organize complex tasks, and demonstrate thoroughness to the user.\n\
        It also helps the user understand the progress of the task and overall progress of their requests.\n\n\
        ## When to Use This Tool\n\
        Use this tool proactively in these scenarios:\n\
        - Complex multi-step tasks - When a task requires 3 or more distinct steps or actions\n\
        - Non-trivial and complex tasks - Tasks that require careful planning or multiple operations\n\
        - User explicitly requests todo list - When the user directly asks you to use the todo list\n\
        - User provides multiple tasks - When users provide a list of things to be done (numbered or comma-separated)\n\
        - After receiving new instructions - Immediately capture user requirements as tasks\n\
        - When you start working on a task - Mark it as in_progress BEFORE beginning work\n\
        - After completing a task - Mark it as completed and add any new follow-up tasks discovered during implementation\n\n\
        ## When NOT to Use This Tool\n\
        Skip using this tool when:\n\
        - There is only a single, straightforward task\n\
        - The task is trivial and tracking it provides no organizational benefit\n\
        - The task can be completed in less than 3 trivial steps\n\
        - The task is purely conversational or informational\n\n\
        ## Task Fields\n\
        - **subject**: A brief, actionable title in imperative form (e.g., \"Fix authentication bug in login flow\")\n\
        - **description**: Detailed description of what needs to be done, including context and acceptance criteria\n\
        - **activeForm**: Present continuous form shown in spinner when task is in_progress (e.g., \"Fixing authentication bug\"). This is displayed to the user while you work on the task."
    }
    fn category(&self) -> ToolCategory { ToolCategory::System }
    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "subject": { "type": "string", "description": "A brief title for the task" },
                    "description": { "type": "string", "description": "A detailed description of what needs to be done" },
                    "activeForm": { "type": "string", "description": "Present continuous form shown in spinner when in_progress (e.g., \"Running tests\")" }
                },
                "required": ["subject", "description"]
            }),
            output_schema: None,
            disabled: false,
        }
    }
    async fn call(&self, _params: Value) -> NativeToolResult {
        Ok(TodoCallResult::success(Some("Task created in todo list.".into()), None))
    }
}

pub struct TodoListTool;
#[async_trait]
impl ToolDefinition for TodoListTool {
    fn name(&self) -> &str { "todo_list" }
    fn description(&self) -> &str { 
        "Use this tool to list all tasks in the current session's todo list.\n\n\
        ## When to Use This Tool\n\
        - To see what tasks are available to work on (status: 'pending', no owner, not blocked)\n\
        - To check overall progress on the project\n\
        - To find tasks that are blocked and need dependencies resolved\n\
        - After completing a task, to check for newly unblocked work or claim the next available task\n\n\
        ## Output\n\
        Returns a summary of each task:\n\
        - **id**: Task identifier (use with todo_get, todo_update)\n\
        - **subject**: Brief description of the task\n\
        - **status**: 'pending', 'in_progress', or 'completed'"
    }
    fn category(&self) -> ToolCategory { ToolCategory::System }
    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({ "type": "object", "properties": {} }),
            output_schema: None,
            disabled: false,
        }
    }
    async fn call(&self, _params: Value) -> NativeToolResult {
        Ok(TodoCallResult::success(Some("Todo list retrieved.".into()), None))
    }
}

pub struct TodoUpdateTool;
#[async_trait]
impl ToolDefinition for TodoUpdateTool {
    fn name(&self) -> &str { "todo_update" }
    fn description(&self) -> &str { 
        "Use this tool to update a task in the todo list.\n\n\
        **Mark tasks as resolved:**\n\
        - When you have completed the work described in a task\n\
        - When a task is no longer needed or has been superseded\n\
        - IMPORTANT: Always mark your assigned tasks as resolved when you finish them\n\n\
        - ONLY mark a task as completed when you have FULLY accomplished it\n\n\
        **Delete tasks:**\n\
        - When a task is no longer relevant or was created in error\n\
        - Setting status to `deleted` permanently removes the task\n\n\
        **Update task details:**\n\
        - When requirements change or become clearer\n\n\
        ## Status Workflow\n\
        Status progresses: `pending` → `in_progress` → `completed`. Use `deleted` to permanently remove a task."
    }
    fn category(&self) -> ToolCategory { ToolCategory::System }
    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "todo_id": { "type": "string", "description": "The ID of the task to update" },
                    "subject": { "type": "string", "description": "New subject for the task" },
                    "description": { "type": "string", "description": "New description for the task" },
                    "activeForm": { "type": "string", "description": "Present continuous form shown in spinner when in_progress" },
                    "status": { 
                        "type": "string", 
                        "enum": ["pending", "in_progress", "completed", "deleted"],
                        "description": "New status for the task"
                    }
                },
                "required": ["todo_id"]
            }),
            output_schema: None,
            disabled: false,
        }
    }
    async fn call(&self, _params: Value) -> NativeToolResult {
        Ok(TodoCallResult::success(Some("Task updated.".into()), None))
    }
}

pub struct TodoGetTool;
#[async_trait]
impl ToolDefinition for TodoGetTool {
    fn name(&self) -> &str { "todo_get" }
    fn description(&self) -> &str { 
        "Use this tool to retrieve a task by its ID from the todo list.\n\n\
        ## When to Use This Tool\n\
        - When you need the full description and context before starting work on a task\n\
        - To understand task dependencies\n\
        - After being assigned a task, to get complete requirements"
    }
    fn category(&self) -> ToolCategory { ToolCategory::System }
    fn tool_calling_spec(&self) -> MCPToolDeclaration {
        MCPToolDeclaration {
            name: self.name().to_string(),
            description: self.description().to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "todo_id": { "type": "string", "description": "The ID of the task to retrieve" }
                },
                "required": ["todo_id"]
            }),
            output_schema: None,
            disabled: false,
        }
    }
    async fn call(&self, _params: Value) -> NativeToolResult {
        Ok(TodoCallResult::success(Some("Detailed task info retrieved.".into()), None))
    }
}

type TodoCallResult = crate::tools::ToolCallResult;
